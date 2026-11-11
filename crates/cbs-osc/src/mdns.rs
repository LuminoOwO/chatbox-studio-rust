//! A small mDNS (multicast DNS) endpoint: answers for our own services and browses for
//! others. Only what OSCQuery needs.
//!
//! Like python-zeroconf (what the Python version used), it listens on every IPv4 interface,
//! loopback included, and sends each announcement on all of them. Our services always
//! advertise 127.0.0.1, so VRChat on the same PC finds them whichever interface it listens on.

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, SockRef, Socket, Type};

use crate::dns::{
    same_name, Packet, Question, RData, Record, CLASS_IN, TOP_BIT, TYPE_A, TYPE_AAAA, TYPE_ANY,
    TYPE_PTR, TYPE_SRV, TYPE_TXT,
};

pub const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
pub const PORT: u16 = 5353;

/// IPv4 addresses of this PC's interfaces (loopback included).
pub fn interface_ips() -> Vec<Ipv4Addr> {
    let mut out = vec![Ipv4Addr::LOCALHOST];
    if let Ok(list) = if_addrs::get_if_addrs() {
        for i in list {
            if let IpAddr::V4(ip) = i.ip() {
                if !out.contains(&ip) {
                    out.push(ip);
                }
            }
        }
    }
    out
}

/// Sends packets: multicast on every interface, or unicast back to someone.
#[derive(Clone)]
pub struct Sender {
    sock: Arc<UdpSocket>,
    ifaces: Arc<Vec<Ipv4Addr>>,
    lock: Arc<Mutex<()>>,
}

impl Sender {
    pub fn multicast(&self, p: &Packet) {
        let data = p.encode();
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let sref = SockRef::from(&*self.sock);
        for ip in self.ifaces.iter() {
            if sref.set_multicast_if_v4(ip).is_ok() {
                let _ = self.sock.send_to(&data, SocketAddrV4::new(GROUP, PORT));
            }
        }
    }

    pub fn unicast(&self, p: &Packet, to: SocketAddr) {
        let _ = self.sock.send_to(&p.encode(), to);
    }
}

pub trait Handler: Send + 'static {
    fn packet(&mut self, out: &Sender, p: &Packet, from: SocketAddr);
    /// Called a few times a second (timers).
    fn tick(&mut self, out: &Sender, now: Instant);
    /// Called once before the endpoint closes (goodbye packets).
    fn stopping(&mut self, _out: &Sender) {}
}

/// A running endpoint; stops when dropped.
pub struct Mdns {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn open_socket(ifaces: &[Ipv4Addr]) -> io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    s.set_reuse_port(true)?;
    s.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT).into())?;
    let mut joined = 0;
    for ip in ifaces {
        if s.join_multicast_v4(&GROUP, ip).is_ok() {
            joined += 1;
        }
    }
    if joined == 0 {
        s.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)?;
    }
    s.set_multicast_loop_v4(true)?;
    s.set_multicast_ttl_v4(255)?;
    s.set_read_timeout(Some(Duration::from_millis(200)))?;
    Ok(s.into())
}

impl Mdns {
    pub fn start<H: Handler>(mut handler: H) -> io::Result<Mdns> {
        let ifaces = interface_ips();
        let sock = Arc::new(open_socket(&ifaces)?);
        let out = Sender { sock: sock.clone(), ifaces: Arc::new(ifaces), lock: Arc::default() };
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = std::thread::Builder::new().name("mdns".into()).spawn(move || {
            let mut buf = vec![0u8; 9000];
            handler.tick(&out, Instant::now());
            while !stop2.load(Ordering::Relaxed) {
                if let Ok((n, from)) = sock.recv_from(&mut buf) {
                    if let Some(p) = Packet::decode(&buf[..n]) {
                        handler.packet(&out, &p, from);
                    }
                }
                handler.tick(&out, Instant::now());
            }
            handler.stopping(&out);
        })?;
        Ok(Mdns { stop, thread: Some(thread) })
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            if t.thread().id() != std::thread::current().id() {
                let _ = t.join();
            }
        }
    }
}

impl Drop for Mdns {
    fn drop(&mut self) {
        self.stop();
    }
}

// --------------------------------------------------------------------------- //
// Advertising
// --------------------------------------------------------------------------- //

/// A service we announce, e.g. `ChatboxStudio-1A2B._osc._udp.local.` on port 9001.
#[derive(Debug, Clone)]
pub struct Service {
    /// "_osc._udp.local."
    pub ty: String,
    /// "ChatboxStudio-1A2B._osc._udp.local."
    pub instance: String,
    /// "ChatboxStudio-1A2B.local."
    pub host: String,
    pub addr: Ipv4Addr,
    pub port: u16,
}

impl Service {
    pub fn new(name: &str, ty: &str, addr: Ipv4Addr, port: u16) -> Service {
        Service { ty: ty.into(), instance: format!("{name}.{ty}"), host: format!("{name}.local."),
                  addr, port }
    }

    fn ptr(&self, ttl: u32) -> Record {
        Record { name: self.ty.clone(), class: CLASS_IN, ttl, data: RData::Ptr(self.instance.clone()) }
    }
    fn srv(&self, ttl: u32) -> Record {
        Record { name: self.instance.clone(), class: CLASS_IN | TOP_BIT, ttl,
                 data: RData::Srv { priority: 0, weight: 0, port: self.port, target: self.host.clone() } }
    }
    fn txt(&self, ttl: u32) -> Record {
        Record { name: self.instance.clone(), class: CLASS_IN | TOP_BIT, ttl, data: RData::Txt(vec![0]) }
    }
    fn a(&self, ttl: u32) -> Record {
        Record { name: self.host.clone(), class: CLASS_IN | TOP_BIT, ttl, data: RData::A(self.addr) }
    }
}

const TTL_OTHER: u32 = 4500;
const TTL_HOST: u32 = 120;

fn push_unique(list: &mut Vec<Record>, r: Record) {
    if !list.contains(&r) {
        list.push(r);
    }
}

/// Answers queries for our services and announces them (start) and says goodbye (stop).
pub struct Responder {
    services: Vec<Service>,
    announce: Vec<Instant>,
}

impl Responder {
    pub fn new(services: Vec<Service>) -> Responder {
        let now = Instant::now();
        let announce = [0, 1, 3].iter().map(|s| now + Duration::from_secs(*s)).collect();
        Responder { services, announce }
    }

    fn all_records(&self, ttl_scale: u32) -> Packet {
        let mut p = Packet { flags: 0x8400, ..Default::default() };
        for s in &self.services {
            p.answers.push(s.ptr(TTL_OTHER * ttl_scale));
            p.answers.push(s.srv(TTL_HOST * ttl_scale));
            p.answers.push(s.txt(TTL_OTHER * ttl_scale));
            push_unique(&mut p.answers, s.a(TTL_HOST * ttl_scale));
        }
        p
    }

    fn answer(&self, q: &Question, p: &mut Packet) {
        let any = q.qtype == TYPE_ANY;
        if (q.qtype == TYPE_PTR || any) && same_name(&q.name, "_services._dns-sd._udp.local.") {
            for s in &self.services {
                push_unique(&mut p.answers, Record { name: q.name.clone(), class: CLASS_IN,
                                                     ttl: TTL_OTHER, data: RData::Ptr(s.ty.clone()) });
            }
        }
        for s in &self.services {
            if (q.qtype == TYPE_PTR || any) && same_name(&q.name, &s.ty) {
                push_unique(&mut p.answers, s.ptr(TTL_OTHER));
                push_unique(&mut p.additionals, s.srv(TTL_HOST));
                push_unique(&mut p.additionals, s.txt(TTL_OTHER));
                push_unique(&mut p.additionals, s.a(TTL_HOST));
            }
            if same_name(&q.name, &s.instance) {
                if q.qtype == TYPE_SRV || any {
                    push_unique(&mut p.answers, s.srv(TTL_HOST));
                    push_unique(&mut p.additionals, s.a(TTL_HOST));
                }
                if q.qtype == TYPE_TXT || any {
                    push_unique(&mut p.answers, s.txt(TTL_OTHER));
                }
            }
            if same_name(&q.name, &s.host) && (q.qtype == TYPE_A || any) {
                push_unique(&mut p.answers, s.a(TTL_HOST));
            }
        }
    }
}

impl Handler for Responder {
    fn packet(&mut self, out: &Sender, p: &Packet, from: SocketAddr) {
        if p.is_response() || p.questions.is_empty() {
            return;
        }
        let mut r = Packet { flags: 0x8400, ..Default::default() };
        for q in &p.questions {
            self.answer(q, &mut r);
        }
        if r.answers.is_empty() {
            return;
        }
        r.additionals.retain(|x| !r.answers.contains(x));
        if from.port() != PORT {
            // a simple resolver asking directly: reply to it, with its id and question
            r.id = p.id;
            r.questions = p.questions.clone();
            out.unicast(&r, from);
        } else if p.questions.iter().all(|q| q.qclass & TOP_BIT != 0) {
            out.unicast(&r, from);
        } else {
            out.multicast(&r);
        }
    }

    fn tick(&mut self, out: &Sender, now: Instant) {
        if self.announce.first().is_some_and(|t| now >= *t) {
            self.announce.remove(0);
            out.multicast(&self.all_records(1));
        }
    }

    fn stopping(&mut self, out: &Sender) {
        let bye = self.all_records(0);
        out.multicast(&bye);
        std::thread::sleep(Duration::from_millis(100));
        out.multicast(&bye);
    }
}

// --------------------------------------------------------------------------- //
// Browsing
// --------------------------------------------------------------------------- //

#[derive(Debug, Clone, PartialEq)]
pub enum BrowseEvent {
    /// A service was found and resolved: (full name, port, addresses).
    Resolved(String, u16, HashSet<IpAddr>),
    /// A service went away (full name).
    Removed(String),
}

struct Pending {
    name: String,
    deadline: Instant,
    asked_addr: bool,
}

/// Looks for services of one type and resolves them (port + addresses), like
/// python-zeroconf's ServiceBrowser + get_service_info(timeout=3 s).
pub struct Browser<F: FnMut(BrowseEvent) + Send + 'static> {
    ty: String,
    filter: Box<dyn Fn(&str) -> bool + Send>,
    on_event: F,
    known: HashMap<String, (String, Instant, u32)>, // lowercased → (name, expires, ttl)
    srv: HashMap<String, (u16, String)>,
    addrs: HashMap<String, HashSet<IpAddr>>,
    pending: HashMap<String, Pending>,
    next_query: Instant,
    interval: Duration,
    refreshed: HashMap<String, Instant>,
}

impl<F: FnMut(BrowseEvent) + Send + 'static> Browser<F> {
    /// `filter` picks the instances to resolve by full name.
    pub fn new(ty: &str, filter: impl Fn(&str) -> bool + Send + 'static, on_event: F) -> Self {
        Browser {
            ty: ty.into(),
            filter: Box::new(filter),
            on_event,
            known: HashMap::new(),
            srv: HashMap::new(),
            addrs: HashMap::new(),
            pending: HashMap::new(),
            next_query: Instant::now(),
            interval: Duration::from_secs(1),
            refreshed: HashMap::new(),
        }
    }

    fn query(out: &Sender, questions: Vec<(String, u16)>) {
        let p = Packet {
            questions: questions.into_iter()
                .map(|(name, qtype)| Question { name, qtype, qclass: CLASS_IN })
                .collect(),
            ..Default::default()
        };
        out.multicast(&p);
    }

    fn remove(&mut self, inst: &str) {
        let key = inst.to_lowercase();
        if let Some((name, ..)) = self.known.remove(&key) {
            self.pending.remove(&key);
            self.srv.remove(&key);
            (self.on_event)(BrowseEvent::Removed(name));
        }
    }

    fn try_resolve(&mut self, out: &Sender) {
        let keys: Vec<String> = self.pending.keys().cloned().collect();
        for k in keys {
            let Some((port, target)) = self.srv.get(&k).cloned() else { continue };
            let addrs = self.addrs.get(&target.to_lowercase()).cloned().unwrap_or_default();
            if addrs.is_empty() || port == 0 {
                let p = self.pending.get_mut(&k).unwrap();
                if !p.asked_addr {
                    p.asked_addr = true;
                    Self::query(out, vec![(target.clone(), TYPE_A), (target, TYPE_AAAA)]);
                }
                continue;
            }
            let p = self.pending.remove(&k).unwrap();
            (self.on_event)(BrowseEvent::Resolved(p.name, port, addrs));
        }
    }
}

impl<F: FnMut(BrowseEvent) + Send + 'static> Handler for Browser<F> {
    fn packet(&mut self, out: &Sender, p: &Packet, _from: SocketAddr) {
        if !p.is_response() {
            return;
        }
        let now = Instant::now();
        let mut added: Vec<String> = Vec::new();
        let mut removed: Vec<String> = Vec::new();
        for r in p.records() {
            let name = r.name.to_lowercase();
            match &r.data {
                RData::A(ip) => {
                    let set = self.addrs.entry(name).or_default();
                    if r.ttl == 0 { set.remove(&IpAddr::V4(*ip)); } else { set.insert(IpAddr::V4(*ip)); }
                }
                RData::Aaaa(ip) => {
                    let set = self.addrs.entry(name).or_default();
                    if r.ttl == 0 { set.remove(&IpAddr::V6(*ip)); } else { set.insert(IpAddr::V6(*ip)); }
                }
                RData::Srv { port, target, .. } if r.ttl > 0 => {
                    self.srv.insert(name, (*port, target.clone()));
                }
                RData::Ptr(inst) if same_name(&r.name, &self.ty) => {
                    if !(self.filter)(inst) {
                        continue;
                    }
                    let key = inst.to_lowercase();
                    if r.ttl == 0 {
                        removed.push(inst.clone());
                    } else {
                        let new = !self.known.contains_key(&key);
                        self.known.insert(key, (inst.clone(), now + Duration::from_secs(r.ttl as u64),
                                                r.ttl));
                        if new {
                            added.push(inst.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        for inst in removed {
            self.remove(&inst);
        }
        for inst in added {
            let key = inst.to_lowercase();
            self.pending.insert(key.clone(), Pending { name: inst.clone(),
                                                       deadline: now + Duration::from_secs(3),
                                                       asked_addr: false });
            if !self.srv.contains_key(&key) {
                Self::query(out, vec![(inst.clone(), TYPE_SRV), (inst, TYPE_TXT)]);
            }
        }
        self.try_resolve(out);
    }

    fn tick(&mut self, out: &Sender, now: Instant) {
        if now >= self.next_query {
            Self::query(out, vec![(self.ty.clone(), TYPE_PTR)]);
            self.next_query = now + self.interval;
            self.interval = (self.interval * 2).min(Duration::from_secs(60));
        }
        self.pending.retain(|_, p| now < p.deadline);
        let mut expired = Vec::new();
        let mut refresh = false;
        for (k, (_, exp, ttl)) in &self.known {
            if now >= *exp {
                expired.push(k.clone());
            } else if now + Duration::from_secs((*ttl as u64) / 5) >= *exp
                && self.refreshed.get(k).is_none_or(|t| now.duration_since(*t) > Duration::from_secs(1))
            {
                refresh = true;
                self.refreshed.insert(k.clone(), now);
            }
        }
        if refresh {
            Self::query(out, vec![(self.ty.clone(), TYPE_PTR)]);
        }
        for k in expired {
            self.remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responder_answers_ptr_queries() {
        let r = Responder::new(vec![Service::new("App-1A2B", "_osc._udp.local.", Ipv4Addr::LOCALHOST, 9002)]);
        let mut p = Packet::default();
        r.answer(&Question { name: "_osc._udp.local".into(), qtype: TYPE_PTR, qclass: CLASS_IN }, &mut p);
        assert_eq!(p.answers[0].data, RData::Ptr("App-1A2B._osc._udp.local.".into()));
        assert!(p.additionals.iter().any(|x| matches!(x.data, RData::Srv { port: 9002, .. })));
        assert!(p.additionals.iter().any(|x| x.data == RData::A(Ipv4Addr::LOCALHOST)));
    }
}
