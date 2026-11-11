//! OSC 1.0 messages and bundles: what VRChat sends and receives.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Str(String),
    Blob(Vec<u8>),
    Bool(bool),
    Nil,
}

impl Arg {
    /// Python's `bool(value)`.
    pub fn truthy(&self) -> bool {
        match self {
            Arg::Int(i) => *i != 0,
            Arg::Long(i) => *i != 0,
            Arg::Float(f) => *f != 0.0,
            Arg::Double(f) => *f != 0.0,
            Arg::Str(s) => !s.is_empty(),
            Arg::Blob(b) => !b.is_empty(),
            Arg::Bool(b) => *b,
            Arg::Nil => false,
        }
    }

    /// As a JSON value (avatar parameters for the chatbox values).
    pub fn to_value(&self) -> Value {
        match self {
            Arg::Int(i) => Value::from(*i),
            Arg::Long(i) => Value::from(*i),
            Arg::Float(f) => Value::from(*f as f64),
            Arg::Double(f) => Value::from(*f),
            Arg::Str(s) => Value::from(s.clone()),
            Arg::Blob(b) => Value::from(b.clone()),
            Arg::Bool(b) => Value::from(*b),
            Arg::Nil => Value::Null,
        }
    }
}

impl From<bool> for Arg {
    fn from(b: bool) -> Self {
        Arg::Bool(b)
    }
}
impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Arg::Str(s.to_string())
    }
}
impl From<i32> for Arg {
    fn from(i: i32) -> Self {
        Arg::Int(i)
    }
}
impl From<f32> for Arg {
    fn from(f: f32) -> Self {
        Arg::Float(f)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub address: String,
    pub args: Vec<Arg>,
}

fn pad4(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn put_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
    pad4(buf);
}

/// One OSC message as bytes.
pub fn encode(address: &str, args: &[Arg]) -> Vec<u8> {
    let mut buf = Vec::new();
    put_str(&mut buf, address);
    let mut tags = String::from(",");
    let mut data = Vec::new();
    for a in args {
        match a {
            Arg::Int(i) => {
                tags.push('i');
                data.extend_from_slice(&i.to_be_bytes());
            }
            Arg::Long(i) => {
                tags.push('h');
                data.extend_from_slice(&i.to_be_bytes());
            }
            Arg::Float(f) => {
                tags.push('f');
                data.extend_from_slice(&f.to_be_bytes());
            }
            Arg::Double(f) => {
                tags.push('d');
                data.extend_from_slice(&f.to_be_bytes());
            }
            Arg::Str(s) => {
                tags.push('s');
                put_str(&mut data, s);
            }
            Arg::Blob(b) => {
                tags.push('b');
                data.extend_from_slice(&(b.len() as u32).to_be_bytes());
                data.extend_from_slice(b);
                pad4(&mut data);
            }
            Arg::Bool(b) => tags.push(if *b { 'T' } else { 'F' }),
            Arg::Nil => tags.push('N'),
        }
    }
    put_str(&mut buf, &tags);
    buf.extend_from_slice(&data);
    buf
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn string(&mut self) -> Option<String> {
        let rest = self.b.get(self.p..)?;
        let end = rest.iter().position(|&c| c == 0)?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.p += (end + 4) & !3;
        (self.p <= self.b.len()).then_some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }
}

fn decode_message(b: &[u8]) -> Option<Message> {
    let mut r = Reader { b, p: 0 };
    let address = r.string()?;
    if !address.starts_with('/') {
        return None;
    }
    if r.p >= b.len() {
        return Some(Message { address, args: Vec::new() }); // no type tags: no arguments
    }
    let tags = r.string()?;
    let mut args = Vec::new();
    for t in tags.strip_prefix(',')?.chars() {
        args.push(match t {
            'i' => Arg::Int(r.u32()? as i32),
            'h' => Arg::Long(r.u64()? as i64),
            'f' => Arg::Float(f32::from_bits(r.u32()?)),
            'd' => Arg::Double(f64::from_bits(r.u64()?)),
            's' | 'S' => Arg::Str(r.string()?),
            'b' => {
                let n = r.u32()? as usize;
                let data = r.take(n)?.to_vec();
                r.p = (r.p + 3) & !3;
                Arg::Blob(data)
            }
            'T' => Arg::Bool(true),
            'F' => Arg::Bool(false),
            'N' | 'I' => Arg::Nil,
            't' => Arg::Long(r.u64()? as i64),
            'c' | 'r' | 'm' => Arg::Int(r.u32()? as i32),
            _ => return None,
        });
    }
    Some(Message { address, args })
}

/// The messages in a packet (a message, or a bundle of them - also nested).
pub fn decode(b: &[u8]) -> Vec<Message> {
    let mut out = Vec::new();
    decode_into(b, &mut out, 0);
    out
}

fn decode_into(b: &[u8], out: &mut Vec<Message>, depth: u32) {
    if depth > 8 {
        return;
    }
    if b.starts_with(b"#bundle\0") {
        let mut r = Reader { b, p: 16 }; // "#bundle\0" + time tag
        while let Some(n) = r.u32() {
            match r.take(n as usize) {
                Some(part) => decode_into(part, out, depth + 1),
                None => break,
            }
        }
    } else if let Some(m) = decode_message(b) {
        out.push(m);
    }
}

/// OSC address pattern matching for the handlers' addresses (`*` = any part of one level).
pub fn matches(pattern: &str, address: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').collect();
    let a: Vec<&str> = address.split('/').collect();
    p.len() == a.len() && p.iter().zip(&a).all(|(p, a)| glob(p, a))
}

fn glob(p: &str, s: &str) -> bool {
    match p.split_once('*') {
        None => p == s,
        Some((pre, rest)) => {
            if !s.starts_with(pre) {
                return false;
            }
            let s = &s[pre.len()..];
            (0..=s.len()).filter(|&i| s.is_char_boundary(i)).any(|i| glob(rest, &s[i..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatbox_input_bytes() {
        // what python-osc sends for send_message("/chatbox/input", ["hi", True, False])
        let b = encode("/chatbox/input", &["hi".into(), true.into(), false.into()]);
        assert_eq!(b, b"/chatbox/input\0\0,sTF\0\0\0\0hi\0\0".to_vec());
    }

    #[test]
    fn roundtrip_and_bundles() {
        let m = encode("/avatar/parameters/VRMode", &[Arg::Int(1), Arg::Float(1.5), "x".into()]);
        let got = decode(&m);
        assert_eq!(got[0].args, vec![Arg::Int(1), Arg::Float(1.5), Arg::Str("x".into())]);
        let mut bundle = b"#bundle\0\0\0\0\0\0\0\0\x01".to_vec();
        for part in [&m, &encode("/avatar/parameters/AFK", &[true.into()])] {
            bundle.extend_from_slice(&(part.len() as u32).to_be_bytes());
            bundle.extend_from_slice(part);
        }
        let got = decode(&bundle);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].address, "/avatar/parameters/AFK");
        assert!(decode(b"garbage").is_empty());
    }

    #[test]
    fn patterns() {
        assert!(matches("/avatar/parameters/*", "/avatar/parameters/MuteSelf"));
        assert!(!matches("/avatar/parameters/*", "/avatar/parameters/a/b"));
        assert!(matches("/a/b*c", "/a/bxxc"));
    }
}
