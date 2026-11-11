//! OSCQuery advertising, so VRChat finds this app by itself.
//!
//! VRChat normally sends avatar parameters (mute, AFK, ...) only to UDP port 9001. If another
//! app already uses 9001 (an OSC router like VOR, face tracking, ...), we listen on a free
//! port instead and announce it with OSCQuery: an mDNS service plus a tiny HTTP server that
//! says "send /avatar/parameters to this UDP port". VRChat then sends to us as well.
//!
//! Same approach as VRChat's own vrc-oscquery-lib and the VrcAdvert tool.

use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};

use crate::mdns::{interface_ips, BrowseEvent, Browser, Mdns, Responder, Service};

pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

fn tree() -> Value {
    json!({
        "DESCRIPTION": "root node", "FULL_PATH": "/", "ACCESS": 0,
        "CONTENTS": {
            "avatar": {
                "FULL_PATH": "/avatar", "ACCESS": 0,
                "CONTENTS": {
                    "parameters": {"DESCRIPTION": "", "FULL_PATH": "/avatar/parameters",
                                   "ACCESS": 2, "TYPE": "b"},
                },
            },
        },
    })
}

/// `json.dumps(value)` with Python's default separators and ASCII escapes.
pub fn py_dumps(v: &Value) -> String {
    let mut out = String::new();
    dump(v, &mut out);
    out
}

fn dump(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    '\u{8}' => out.push_str("\\b"),
                    '\u{c}' => out.push_str("\\f"),
                    c if (c as u32) < 0x20 || (c as u32) > 0x7f => {
                        let mut buf = [0u16; 2];
                        for u in c.encode_utf16(&mut buf) {
                            out.push_str(&format!("\\u{u:04x}"));
                        }
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                dump(x, out);
            }
            out.push(']');
        }
        Value::Object(o) => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                dump(&Value::String(k.clone()), out);
                out.push_str(": ");
                dump(x, out);
            }
            out.push('}');
        }
    }
}

/// A random 4-character hex suffix, upper case (unique per run).
fn suffix() -> String {
    use rand::Rng;
    format!("{:04X}", rand::thread_rng().gen::<u16>())
}

struct Http {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn respond(mut s: TcpStream, host_info: &str) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut req = Vec::new();
    let mut buf = [0u8; 2048];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") && req.len() < 65536 {
        match s.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => req.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&req);
    let first = text.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let reply = if method == "GET" {
        let body = if path.contains("HOST_INFO") { host_info.to_string() } else { py_dumps(&tree()) };
        format!("HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len())
    } else {
        "HTTP/1.0 501 Unsupported method\r\nContent-Length: 0\r\n\r\n".to_string()
    };
    let _ = s.write_all(reply.as_bytes());
    let _ = s.shutdown(Shutdown::Both);
}

impl Http {
    fn start(host_info: String) -> io::Result<Http> {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = l.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let info = Arc::new(host_info);
        let thread = std::thread::Builder::new().name("oscquery".into()).spawn(move || {
            for conn in l.incoming() {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(s) = conn {
                    let info = info.clone();
                    std::thread::spawn(move || respond(s, &info));
                }
            }
        })?;
        Ok(Http { port, stop, thread: Some(thread) })
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, self.port).into(),
                                           Duration::from_secs(1));
        if let Some(t) = self.thread.take() {
            if t.thread().id() != std::thread::current().id() {
                let _ = t.join();
            }
        }
    }
}

/// Tells VRChat (via mDNS + HTTP) to send avatar parameters to our UDP port too.
pub struct OscQueryService {
    pub name: String,
    pub osc_port: u16,
    http: Option<Http>,
    mdns: Option<Mdns>,
}

impl OscQueryService {
    pub fn new(name: &str, osc_port: u16) -> OscQueryService {
        OscQueryService { name: format!("{name}-{}", suffix()), osc_port, http: None, mdns: None }
    }

    pub fn host_info(&self) -> Value {
        json!({"NAME": self.name, "OSC_IP": "127.0.0.1", "OSC_PORT": self.osc_port,
               "OSC_TRANSPORT": "UDP",
               "EXTENSIONS": {"ACCESS": true, "CLIPMODE": false, "RANGE": true,
                              "TYPE": true, "VALUE": true}})
    }

    /// Starts the HTTP server and the mDNS announcements; returns the HTTP port.
    pub fn start(&mut self) -> io::Result<u16> {
        let http = Http::start(py_dumps(&self.host_info()))?;
        let http_port = http.port;
        self.http = Some(http);
        let lo = Ipv4Addr::LOCALHOST;
        let services = vec![
            Service::new(&self.name, "_oscjson._tcp.local.", lo, http_port),
            Service::new(&self.name, "_osc._udp.local.", lo, self.osc_port),
        ];
        match Mdns::start(Responder::new(services)) {
            Ok(m) => self.mdns = Some(m),
            Err(e) => {
                self.stop();
                return Err(e);
            }
        }
        Ok(http_port)
    }

    pub fn stop(&mut self) {
        if let Some(mut m) = self.mdns.take() {
            m.stop();
        }
        if let Some(mut h) = self.http.take() {
            h.stop();
        }
    }
}

impl Drop for OscQueryService {
    fn drop(&mut self) {
        self.stop();
    }
}

/// This PC's IPv4 addresses.
pub fn local_ips() -> HashSet<IpAddr> {
    interface_ips().into_iter().map(IpAddr::V4).collect()
}

/// Finds the UDP port VRChat really listens on (it announces it with OSCQuery as
/// 'VRChat-Client-XXXXXX' under _osc._udp). Normally 9000, but different when VRChat was
/// started with --osc=... (common with OSC routers like VOR) or 9000 was taken.
/// Only VRChat on this PC counts, not other people's VRChat on the same network.
pub struct VrchatFinder {
    pub port: Arc<Mutex<Option<u16>>>,
    mdns: Option<Mdns>,
}

impl VrchatFinder {
    pub fn start(on_change: impl Fn(Option<u16>) + Send + 'static) -> io::Result<VrchatFinder> {
        let port = Arc::new(Mutex::new(None));
        let port2 = port.clone();
        let mut names: Vec<(String, u16)> = Vec::new();
        let update = move |names: &Vec<(String, u16)>| {
            let p = names.first().map(|(_, p)| *p);
            let mut cur = port2.lock().unwrap_or_else(|e| e.into_inner());
            if p != *cur {
                *cur = p;
                drop(cur);
                on_change(p);
            }
        };
        let browser = Browser::new(
            "_osc._udp.local.",
            |name| name.to_lowercase().starts_with("vrchat-client"),
            move |ev| match ev {
                BrowseEvent::Removed(name) => {
                    names.retain(|(n, _)| *n != name);
                    update(&names);
                }
                BrowseEvent::Resolved(name, p, addrs) => {
                    if !addrs.is_empty() && addrs.is_disjoint(&local_ips()) {
                        return; // someone else's VRChat on the network
                    }
                    match names.iter_mut().find(|(n, _)| *n == name) {
                        Some(e) => e.1 = p,
                        None => names.push((name, p)),
                    }
                    update(&names);
                }
            },
        );
        Ok(VrchatFinder { port, mdns: Some(Mdns::start(browser)?) })
    }

    pub fn stop(&mut self) {
        if let Some(mut m) = self.mdns.take() {
            m.stop();
        }
    }
}

impl Drop for VrchatFinder {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_like_python() {
        assert_eq!(py_dumps(&json!({"A": 1, "B": [true, null], "C": "\u{e4}\u{7f}"})),
                   "{\"A\": 1, \"B\": [true, null], \"C\": \"\\u00e4\u{7f}\"}");
    }

    #[test]
    fn http_answers() {
        let mut h = Http::start("{\"NAME\": \"x\"}".into()).unwrap();
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, h.port)).unwrap();
        s.write_all(b"GET /?HOST_INFO HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut r = String::new();
        s.read_to_string(&mut r).unwrap();
        assert!(r.starts_with("HTTP/1.0 200 OK") && r.ends_with("{\"NAME\": \"x\"}"), "{r}");
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, h.port)).unwrap();
        s.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        let mut r = String::new();
        s.read_to_string(&mut r).unwrap();
        assert!(r.contains("\"FULL_PATH\": \"/avatar/parameters\""));
        h.stop();
    }

    /// Our advertisement, found by our own browser over real multicast (skipped where the
    /// machine has no multicast, e.g. some CI sandboxes).
    #[test]
    fn advertise_and_find() {
        let mut svc = OscQueryService::new("VRChat-Client-Test", 9123);
        if svc.start().is_err() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let f = VrchatFinder::start(move |p| { let _ = tx.send(p); });
        let Ok(mut f) = f else { return };
        let got = rx.recv_timeout(Duration::from_secs(6));
        eprintln!("advertise_and_find: {got:?}");
        if let Ok(p) = got {
            assert_eq!(p, Some(9123));
            svc.stop();
            // goodbye packet → removed
            let gone = rx.recv_timeout(Duration::from_secs(4));
            eprintln!("after goodbye: {gone:?}");
            if let Ok(p) = gone {
                assert_eq!(p, None);
            }
        }
        f.stop();
    }
}
