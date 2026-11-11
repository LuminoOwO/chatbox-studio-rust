//! The parts of DNS messages mDNS service discovery needs (RFC 1035 / 6762 / 6763):
//! questions and A, AAAA, PTR, SRV, TXT records.

use std::net::{Ipv4Addr, Ipv6Addr};

pub const TYPE_A: u16 = 1;
pub const TYPE_PTR: u16 = 12;
pub const TYPE_TXT: u16 = 16;
pub const TYPE_AAAA: u16 = 28;
pub const TYPE_SRV: u16 = 33;
pub const TYPE_ANY: u16 = 255;
pub const CLASS_IN: u16 = 1;
/// In a question: "answer by unicast". In a record: "cache flush" (unique record).
pub const TOP_BIT: u16 = 0x8000;

#[derive(Debug, Clone, PartialEq)]
pub enum RData {
    A(Ipv4Addr),
    Aaaa(Ipv6Addr),
    Ptr(String),
    Srv { priority: u16, weight: u16, port: u16, target: String },
    Txt(Vec<u8>),
    Other(u16),
}

impl RData {
    pub fn rtype(&self) -> u16 {
        match self {
            RData::A(_) => TYPE_A,
            RData::Aaaa(_) => TYPE_AAAA,
            RData::Ptr(_) => TYPE_PTR,
            RData::Srv { .. } => TYPE_SRV,
            RData::Txt(_) => TYPE_TXT,
            RData::Other(t) => *t,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub name: String,
    pub class: u16,
    pub ttl: u32,
    pub data: RData,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Packet {
    pub id: u16,
    pub flags: u16,
    pub questions: Vec<Question>,
    pub answers: Vec<Record>,
    pub authorities: Vec<Record>,
    pub additionals: Vec<Record>,
}

impl Packet {
    pub fn is_response(&self) -> bool {
        self.flags & 0x8000 != 0
    }

    pub fn records(&self) -> impl Iterator<Item = &Record> {
        self.answers.iter().chain(&self.authorities).chain(&self.additionals)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(512);
        for v in [self.id, self.flags, self.questions.len() as u16, self.answers.len() as u16,
                  self.authorities.len() as u16, self.additionals.len() as u16] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        for q in &self.questions {
            put_name(&mut b, &q.name);
            b.extend_from_slice(&q.qtype.to_be_bytes());
            b.extend_from_slice(&q.qclass.to_be_bytes());
        }
        for r in self.records() {
            put_name(&mut b, &r.name);
            b.extend_from_slice(&r.data.rtype().to_be_bytes());
            b.extend_from_slice(&r.class.to_be_bytes());
            b.extend_from_slice(&r.ttl.to_be_bytes());
            let mut d = Vec::new();
            match &r.data {
                RData::A(ip) => d.extend_from_slice(&ip.octets()),
                RData::Aaaa(ip) => d.extend_from_slice(&ip.octets()),
                RData::Ptr(n) => put_name(&mut d, n),
                RData::Srv { priority, weight, port, target } => {
                    d.extend_from_slice(&priority.to_be_bytes());
                    d.extend_from_slice(&weight.to_be_bytes());
                    d.extend_from_slice(&port.to_be_bytes());
                    put_name(&mut d, target);
                }
                RData::Txt(t) => d.extend_from_slice(t),
                RData::Other(_) => {}
            }
            b.extend_from_slice(&(d.len() as u16).to_be_bytes());
            b.extend_from_slice(&d);
        }
        b
    }

    pub fn decode(b: &[u8]) -> Option<Packet> {
        let mut r = Cursor { b, p: 0 };
        let id = r.u16()?;
        let flags = r.u16()?;
        let (qd, an, ns, ar) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
        let mut p = Packet { id, flags, ..Default::default() };
        for _ in 0..qd {
            let name = r.name()?;
            p.questions.push(Question { name, qtype: r.u16()?, qclass: r.u16()? });
        }
        for (n, list) in [(an, &mut p.answers), (ns, &mut p.authorities), (ar, &mut p.additionals)] {
            for _ in 0..n {
                list.push(r.record()?);
            }
        }
        Some(p)
    }
}

/// Writes a name ("a.b.local." or "a.b.local") without compression.
fn put_name(b: &mut Vec<u8>, name: &str) {
    for label in name.trim_end_matches('.').split('.').filter(|l| !l.is_empty()) {
        let bytes = &label.as_bytes()[..label.len().min(63)];
        b.push(bytes.len() as u8);
        b.extend_from_slice(bytes);
    }
    b.push(0);
}

struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
}

impl Cursor<'_> {
    fn u16(&mut self) -> Option<u16> {
        let v = u16::from_be_bytes(self.b.get(self.p..self.p + 2)?.try_into().ok()?);
        self.p += 2;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_be_bytes(self.b.get(self.p..self.p + 4)?.try_into().ok()?);
        self.p += 4;
        Some(v)
    }

    /// A name, following compression pointers. Ends with ".".
    fn name(&mut self) -> Option<String> {
        let (name, end) = read_name(self.b, self.p)?;
        self.p = end;
        Some(name)
    }

    fn record(&mut self) -> Option<Record> {
        let name = self.name()?;
        let rtype = self.u16()?;
        let class = self.u16()?;
        let ttl = self.u32()?;
        let len = self.u16()? as usize;
        let start = self.p;
        let raw = self.b.get(start..start + len)?;
        let data = match rtype {
            TYPE_A if len == 4 => RData::A(Ipv4Addr::new(raw[0], raw[1], raw[2], raw[3])),
            TYPE_AAAA if len == 16 => RData::Aaaa(Ipv6Addr::from(<[u8; 16]>::try_from(raw).ok()?)),
            TYPE_PTR => RData::Ptr(read_name(self.b, start)?.0),
            TYPE_SRV if len >= 7 => {
                let mut c = Cursor { b: self.b, p: start };
                let (priority, weight, port) = (c.u16()?, c.u16()?, c.u16()?);
                RData::Srv { priority, weight, port, target: c.name()? }
            }
            TYPE_TXT => RData::Txt(raw.to_vec()),
            t => RData::Other(t),
        };
        self.p = start + len;
        Some(Record { name, class, ttl, data })
    }
}

/// (name, position after the name in the original place).
fn read_name(b: &[u8], mut p: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut end = None;
    let mut jumps = 0;
    loop {
        let len = *b.get(p)? as usize;
        if len == 0 {
            p += 1;
            break;
        }
        if len & 0xc0 == 0xc0 {
            let off = ((len & 0x3f) << 8) | *b.get(p + 1)? as usize;
            if end.is_none() {
                end = Some(p + 2);
            }
            jumps += 1;
            if jumps > 32 {
                return None;
            }
            p = off;
            continue;
        }
        let label = b.get(p + 1..p + 1 + len)?;
        out.push_str(&String::from_utf8_lossy(label));
        out.push('.');
        p += 1 + len;
    }
    if out.is_empty() {
        out.push('.');
    }
    Some((out, end.unwrap_or(p)))
}

/// Names compare without case and without the trailing dot.
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim_end_matches('.').eq_ignore_ascii_case(b.trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = Packet {
            id: 0,
            flags: 0x8400,
            questions: vec![],
            answers: vec![Record { name: "_osc._udp.local.".into(), class: CLASS_IN, ttl: 4500,
                                   data: RData::Ptr("X-1._osc._udp.local.".into()) }],
            authorities: vec![],
            additionals: vec![
                Record { name: "X-1._osc._udp.local.".into(), class: CLASS_IN | TOP_BIT, ttl: 120,
                         data: RData::Srv { priority: 0, weight: 0, port: 9001,
                                            target: "X-1.local.".into() } },
                Record { name: "X-1.local.".into(), class: CLASS_IN | TOP_BIT, ttl: 120,
                         data: RData::A(Ipv4Addr::LOCALHOST) },
                Record { name: "X-1._osc._udp.local.".into(), class: CLASS_IN | TOP_BIT, ttl: 4500,
                         data: RData::Txt(vec![0]) },
            ],
        };
        assert_eq!(Packet::decode(&p.encode()).unwrap(), p);
    }

    #[test]
    fn compressed_names() {
        // header, one answer: name "a.local" then PTR rdata "b" + pointer to "local"
        let mut b = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        b.extend_from_slice(b"\x01a\x05local\x00"); // at 12; "local" at 14
        b.extend_from_slice(&[0, 12, 0, 1, 0, 0, 0, 10, 0, 4]);
        b.extend_from_slice(b"\x01b\xc0\x0e");
        let p = Packet::decode(&b).unwrap();
        assert_eq!(p.answers[0].name, "a.local.");
        assert_eq!(p.answers[0].data, RData::Ptr("b.local.".into()));
        assert!(same_name("B.Local", "b.local."));
    }
}
