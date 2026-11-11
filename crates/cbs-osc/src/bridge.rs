//! Sends chatbox messages to VRChat and listens for VRChat's mute state, AFK and avatar
//! parameters.
//!
//! Mute-sync states (reported through `on_listen_state`):
//!
//! * `off`      - not listening
//! * `waiting`  - listening, but VRChat hasn't sent its mute state yet
//! * `ok`       - VRChat's mute state is being received
//! * `oscquery` - our port was taken; listening on another one, announced with OSCQuery
//! * `busy`     - another program (or a second copy of this app) holds the port; retried
//!   every few seconds

use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use cbs_core::pystr::head;
use cbs_core::routes::CHATBOX_LIMIT;
use cbs_core::settings::Settings;
use serde_json::Value;

use crate::osc::{self, Arg};
use crate::oscquery::{OscQueryService, VrchatFinder};

/// The app's name as announced with OSCQuery (spaces removed, like the Python version).
pub const OSCQUERY_NAME: &str = "ChatboxStudio";

type Cb<T> = Arc<dyn Fn(T) + Send + Sync>;
type ParamCb = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// What the bridge reports back.
#[derive(Clone)]
pub struct Events {
    pub log: Cb<String>,
    pub on_mute: Cb<bool>,
    pub on_listen_state: Cb<&'static str>,
    pub on_afk: Cb<bool>,
    pub on_param: ParamCb,
}

impl Default for Events {
    fn default() -> Self {
        Events {
            log: Arc::new(|_| {}),
            on_mute: Arc::new(|_| {}),
            on_listen_state: Arc::new(|_| {}),
            on_afk: Arc::new(|_| {}),
            on_param: Arc::new(|_, _| {}),
        }
    }
}

/// The OS's text for an error, like Python's `e.strerror` ("Address already in use").
pub fn strerror(e: &io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            if t.thread().id() != std::thread::current().id() {
                let _ = t.join();
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Default)]
struct Listen {
    server: Option<Server>,
    oscq: Option<OscQueryService>,
    actual_port: u16,
    via_oscquery: bool,
    busy_logged: bool,
}

struct Inner {
    s: Arc<RwLock<Settings>>,
    ev: Events,
    found_port: Mutex<Option<u16>>,
    finder: Mutex<Option<VrchatFinder>>,
    client: Mutex<(Option<SocketAddr>, Option<UdpSocket>)>,
    listen: Mutex<Listen>,
    listen_state: Mutex<&'static str>,
    logged_target: Mutex<Option<(String, u16)>>,
    alive: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    fn settings(&self) -> std::sync::RwLockReadGuard<'_, Settings> {
        self.s.read().unwrap_or_else(|e| e.into_inner())
    }

    fn log(&self, msg: String) {
        (self.ev.log)(msg);
    }

    fn send_port(&self) -> u16 {
        let s = self.settings();
        match *lock(&self.found_port) {
            Some(p) if s.oscquery => p,
            _ => s.osc_port as u16,
        }
    }

    fn retarget(&self) {
        let host = self.settings().osc_host.clone();
        let port = self.send_port();
        let addr = (host.as_str(), port).to_socket_addrs().ok().and_then(|mut a| a.next());
        let sock = addr.and_then(|a| {
            let bind = if a.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
            UdpSocket::bind(bind).ok()
        });
        *lock(&self.client) = (addr, sock);
    }

    fn send(&self, address: &str, args: &[Arg]) -> io::Result<()> {
        let c = lock(&self.client);
        match (&c.0, &c.1) {
            (Some(a), Some(s)) => s.send_to(&osc::encode(address, args), a).map(|_| ()),
            _ => Err(io::Error::new(io::ErrorKind::AddrNotAvailable,
                                    "the chatbox address can't be resolved")),
        }
    }

    fn set_listen_state(&self, st: &'static str) {
        let mut cur = lock(&self.listen_state);
        if *cur != st {
            *cur = st;
            drop(cur);
            (self.ev.on_listen_state)(st);
        }
    }

    fn found(&self, port: Option<u16>) {
        let before = self.send_port();
        *lock(&self.found_port) = port;
        self.retarget();
        let (host, own) = {
            let s = self.settings();
            (s.osc_host.clone(), s.osc_port)
        };
        if let Some(p) = port {
            let note = if p as i64 == own {
                String::new()
            } else {
                format!(" (not {own} - VRChat uses another port here)")
            };
            self.log(format!("Found VRChat with OSCQuery: chatbox goes to {host}:{p}{note}."));
        } else if before as i64 != own {
            self.log(format!("VRChat closed - chatbox goes to port {own} again."));
        }
    }

    fn handle(&self, m: &osc::Message) {
        let Some(first) = m.args.first() else { return };
        if m.address == "/avatar/parameters/MuteSelf" {
            self.set_listen_state("ok");
            (self.ev.on_mute)(first.truthy());
        }
        if m.address == "/avatar/parameters/AFK" {
            self.set_listen_state("ok");
            (self.ev.on_afk)(first.truthy());
        }
        if osc::matches("/avatar/parameters/*", &m.address) {
            let name = m.address.rsplit('/').next().unwrap_or("");
            (self.ev.on_param)(name, first.to_value());
        }
    }
}

fn stop_server(l: &mut Listen) {
    if let Some(mut q) = l.oscq.take() {
        q.stop();
    }
    if let Some(mut s) = l.server.take() {
        s.shutdown();
    }
}

fn serve(sock: UdpSocket, inner: Weak<Inner>) -> Server {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    let thread = std::thread::Builder::new()
        .name("osc-listen".into())
        .spawn(move || {
            let mut buf = vec![0u8; 65536];
            while !stop2.load(Ordering::Relaxed) && inner.strong_count() > 0 {
                if let Ok((n, _)) = sock.recv_from(&mut buf) {
                    let Some(inner) = inner.upgrade() else { break };
                    for m in osc::decode(&buf[..n]) {
                        inner.handle(&m);
                    }
                }
            }
        })
        .ok();
    Server { stop, thread }
}

/// See [`OscBridge::set_listening`].
fn set_listening(i: &Arc<Inner>, on: bool) {
    let mut l = lock(&i.listen);
    if !on {
        stop_server(&mut l);
        l.actual_port = 0;
        drop(l);
        i.set_listen_state("off");
        return;
    }
    if l.server.is_some() {
        return;
    }
    let (port, oscquery) = {
        let s = i.settings();
        (s.osc_listen_port as u16, s.oscquery)
    };
    let mut fallback = false;
    let sock = match UdpSocket::bind(("127.0.0.1", port)) {
        Ok(s) => s,
        Err(e) => {
            if !oscquery {
                if !l.busy_logged {
                    l.busy_logged = true;
                    i.log(format!(
                        "Can't listen on port {port}: {}. Another program (an OSC router like \
                         VOR, face tracking, ...) uses it. Set the listen port to the port that \
                         program forwards to, or turn on 'Find VRChat with OSCQuery'. Retrying...",
                        strerror(&e)));
                }
                drop(l);
                i.set_listen_state("busy");
                return;
            }
            fallback = true;
            match UdpSocket::bind(("127.0.0.1", 0)) {
                Ok(s) => s,
                Err(e) => {
                    i.log(format!("Can't listen for VRChat: {}.", strerror(&e)));
                    drop(l);
                    i.set_listen_state("busy");
                    return;
                }
            }
        }
    };
    l.actual_port = sock.local_addr().map(|a| a.port()).unwrap_or(0);
    l.server = Some(serve(sock, Arc::downgrade(i)));
    l.via_oscquery = false;
    if oscquery && (fallback || l.actual_port != 9001) {
        let mut q = OscQueryService::new(OSCQUERY_NAME, l.actual_port);
        match q.start() {
            Ok(_) => {
                l.oscq = Some(q);
                l.via_oscquery = true;
            }
            Err(e) => i.log(format!("OSCQuery could not start ({}).", strerror(&e))),
        }
    }
    if fallback {
        let msg = format!("Port {port} is used by another program (for example an OSC router \
                           like VOR). Listening on port {} instead", l.actual_port);
        i.log(msg + if l.via_oscquery { " and telling VRChat about it via OSCQuery." }
                    else { ". VRChat won't find it without OSCQuery." });
    } else if l.busy_logged {
        i.log(format!("Port {port} is free again - mute sync works now."));
    }
    l.busy_logged = fallback;
    let st = if l.via_oscquery && fallback { "oscquery" } else { "waiting" };
    drop(l);
    i.set_listen_state(st);
}

/// VRChat's OSC: chatbox out, mute / AFK / avatar parameters in.
pub struct OscBridge {
    inner: Arc<Inner>,
}

impl OscBridge {
    pub fn new(settings: Arc<RwLock<Settings>>, events: Events) -> OscBridge {
        let inner = Arc::new(Inner {
            s: settings,
            ev: events,
            found_port: Mutex::new(None),
            finder: Mutex::new(None),
            client: Mutex::new((None, None)),
            listen: Mutex::new(Listen::default()),
            listen_state: Mutex::new("off"),
            logged_target: Mutex::new(None),
            alive: AtomicBool::new(true),
        });
        inner.retarget();
        let b = OscBridge { inner };
        let weak = Arc::downgrade(&b.inner);
        let _ = std::thread::Builder::new().name("osc-retry".into()).spawn(move || loop {
            for _ in 0..25 {
                std::thread::sleep(Duration::from_millis(200));
                match weak.upgrade() {
                    Some(i) if i.alive.load(Ordering::Relaxed) => {}
                    _ => return,
                }
            }
            let Some(i) = weak.upgrade() else { return };
            if *lock(&i.listen_state) == "busy" {
                set_listening(&i, true);
            }
        });
        b.set_listening(true); // always: mute sync and the AFK status both need it
        b.start_finder();
        b
    }

    pub fn send_port(&self) -> u16 {
        self.inner.send_port()
    }

    /// After changing the host or port.
    pub fn retarget(&self) {
        self.inner.retarget();
    }

    pub fn listen_state(&self) -> &'static str {
        *lock(&self.inner.listen_state)
    }

    /// The UDP port we really listen on (0 = not listening).
    pub fn actual_port(&self) -> u16 {
        lock(&self.inner.listen).actual_port
    }

    pub fn via_oscquery(&self) -> bool {
        lock(&self.inner.listen).via_oscquery
    }

    pub fn found_port(&self) -> Option<u16> {
        *lock(&self.inner.found_port)
    }

    /// Looks for VRChat's real chatbox port with OSCQuery (live, while the app runs).
    pub fn start_finder(&self) {
        self.stop_finder();
        let i = &self.inner;
        if !i.settings().oscquery {
            if lock(&i.found_port).take().is_some() {
                i.retarget();
            }
            return;
        }
        let weak = Arc::downgrade(i);
        match VrchatFinder::start(move |p| {
            if let Some(i) = weak.upgrade() {
                i.found(p);
            }
        }) {
            Ok(f) => *lock(&i.finder) = Some(f),
            Err(e) => {
                let port = i.settings().osc_port;
                i.log(format!("OSCQuery search for VRChat not available ({}). Sending to port {port}.",
                              strerror(&e)));
            }
        }
    }

    pub fn stop_finder(&self) {
        let f = lock(&self.inner.finder).take();
        if let Some(mut f) = f {
            f.stop();
        }
    }

    /// Start or stop listening for VRChat's avatar parameters (live, no restart needed).
    ///
    /// 1. Listen on the chosen port (9001 = VRChat's default).
    /// 2. If it's taken (e.g. an OSC router like VOR), listen on a free port instead and
    ///    announce it with OSCQuery, so VRChat sends to us as well.
    /// 3. Without OSCQuery: show "busy" and retry every few seconds.
    pub fn set_listening(&self, on: bool) {
        set_listening(&self.inner, on);
    }

    /// After changing the listen port or the OSCQuery option.
    pub fn restart_listener(&self) {
        {
            let mut l = lock(&self.inner.listen);
            stop_server(&mut l);
            *lock(&self.inner.listen_state) = "";
        }
        self.set_listening(true);
    }

    pub fn typing(&self, on: bool) {
        let s = self.inner.settings();
        if s.osc_enabled && s.typing_indicator {
            drop(s);
            let _ = self.inner.send("/chatbox/typing", &[Arg::Bool(on)]);
        }
    }

    /// Sends one finished chatbox message (timing is handled by the composer).
    pub fn send_chatbox(&self, text: &str) {
        let i = &self.inner;
        let (enabled, host, oscquery, notify) = {
            let s = i.settings();
            (s.osc_enabled, s.osc_host.clone(), s.oscquery, s.notify_sound)
        };
        if !enabled {
            return;
        }
        let target = (host, i.send_port());
        let mut logged = lock(&i.logged_target);
        if logged.as_ref() != Some(&target) {
            *logged = Some(target.clone());
            drop(logged);
            let how = if oscquery && lock(&i.found_port).is_some() {
                "found with OSCQuery"
            } else if target.1 == 9000 {
                "VRChat's default"
            } else {
                "your setting; VRChat's default is 9000"
            };
            i.log(format!("Chatbox messages go to {}:{} ({how}).", target.0, target.1));
        }
        let args = [Arg::Str(head(text, CHATBOX_LIMIT)), Arg::Bool(true), Arg::Bool(notify)];
        if let Err(e) = i.send("/chatbox/input", &args) {
            i.log(format!("OSC send failed: {}", strerror(&e)));
        }
    }

    /// Stops listening and searching (also done when the bridge is dropped).
    pub fn close(&self) {
        self.inner.alive.store(false, Ordering::Relaxed);
        self.stop_finder();
        let mut l = lock(&self.inner.listen);
        stop_server(&mut l);
    }
}

impl Drop for OscBridge {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn settings(listen: u16, send: u16) -> Arc<RwLock<Settings>> {
        let mut s = Settings::default();
        s.osc_listen_port = listen as i64;
        s.osc_port = send as i64;
        s.oscquery = false;
        Arc::new(RwLock::new(s))
    }

    fn free_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[test]
    fn sends_and_receives() {
        let vrc = UdpSocket::bind("127.0.0.1:0").unwrap();
        vrc.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let listen = free_port();
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        let ev = Events {
            on_mute: Arc::new(move |m| { let _ = tx.send(format!("mute {m}")); }),
            on_param: Arc::new(move |n, v| { let _ = tx2.send(format!("{n}={v}")); }),
            ..Default::default()
        };
        let b = OscBridge::new(settings(listen, vrc.local_addr().unwrap().port()), ev);
        assert_eq!(b.listen_state(), "waiting");
        b.send_chatbox(&"é".repeat(200));
        let mut buf = [0u8; 2048];
        let (n, from) = vrc.recv_from(&mut buf).unwrap();
        let m = &osc::decode(&buf[..n])[0];
        assert_eq!(m.address, "/chatbox/input");
        assert_eq!(m.args[0], Arg::Str("é".repeat(144)));
        let _ = from;
        vrc.send_to(&osc::encode("/avatar/parameters/MuteSelf", &[true.into()]),
                    ("127.0.0.1", listen)).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "mute true");
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "MuteSelf=true");
        assert_eq!(b.listen_state(), "ok");
        b.set_listening(false);
        assert_eq!(b.listen_state(), "off");
    }

    #[test]
    fn busy_port_without_oscquery() {
        let taken = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let b = OscBridge::new(settings(port, 9), Events::default());
        assert_eq!(b.listen_state(), "busy");
        drop(taken);
        b.set_listening(true);
        assert_eq!(b.listen_state(), "waiting");
        assert_eq!(b.actual_port(), port);
    }
}
