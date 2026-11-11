//! Reading loosely typed JSON the way the Python version does (`int(b.get("prio", 2))`,
//! `float(o.get("offset") or 0)`, `str(...)`). Blocks and the chatbox config stay plain
//! JSON objects, so every key - also ones this version doesn't know - survives a save.

use serde_json::{Map, Value};

pub type Obj = Map<String, Value>;

/// `str(v)` like Python for the values that appear in settings.
pub fn to_str(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "True".into() } else { "False".into() },
        Value::Number(n) => {
            if n.is_f64() {
                let f = n.as_f64().unwrap_or(0.0);
                if f.fract() == 0.0 && f.abs() < 1e16 { format!("{f:.1}") } else { format!("{f}") }
            } else {
                n.to_string()
            }
        }
        other => other.to_string(),
    }
}

/// `int(v)`; None when Python's int() would fail.
pub fn to_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Value::Bool(b) => Some(*b as i64),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// `float(v)`; None when Python's float() would fail.
pub fn to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(*b as i64 as f64),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Python truthiness.
pub fn truthy(v: &Value) -> bool {
    crate::settings::truthy(v)
}

pub trait ObjExt {
    /// `str(d.get(k, default))`
    fn s(&self, k: &str, default: &str) -> String;
    /// `int(d.get(k) or default)` - falsy values (0, "", None) give the default
    fn i_or(&self, k: &str, default: i64) -> i64;
    /// `int(d.get(k, default))` - a present 0 stays 0
    fn i(&self, k: &str, default: i64) -> i64;
    /// `float(d.get(k) or default)`
    fn f_or(&self, k: &str, default: f64) -> f64;
    /// `float(d.get(k, default))`
    fn f(&self, k: &str, default: f64) -> f64;
    /// `bool(d.get(k, default))`
    fn b(&self, k: &str, default: bool) -> bool;
    fn obj(&self, k: &str) -> Option<&Obj>;
    fn set(&mut self, k: &str, v: impl Into<Value>);
    /// `d.setdefault(k, v)`
    fn setdefault(&mut self, k: &str, v: impl Into<Value>);
}

impl ObjExt for Obj {
    fn s(&self, k: &str, default: &str) -> String {
        match self.get(k) {
            Some(v) => to_str(v),
            None => default.to_string(),
        }
    }

    fn i_or(&self, k: &str, default: i64) -> i64 {
        match self.get(k) {
            Some(v) if truthy(v) => to_i64(v).unwrap_or(default),
            _ => default,
        }
    }

    fn i(&self, k: &str, default: i64) -> i64 {
        match self.get(k) {
            Some(v) => to_i64(v).unwrap_or(default),
            None => default,
        }
    }

    fn f_or(&self, k: &str, default: f64) -> f64 {
        match self.get(k) {
            Some(v) if truthy(v) => to_f64(v).unwrap_or(default),
            _ => default,
        }
    }

    fn f(&self, k: &str, default: f64) -> f64 {
        match self.get(k) {
            Some(v) => to_f64(v).unwrap_or(default),
            None => default,
        }
    }

    fn b(&self, k: &str, default: bool) -> bool {
        match self.get(k) {
            Some(v) => truthy(v),
            None => default,
        }
    }

    fn obj(&self, k: &str) -> Option<&Obj> {
        self.get(k).and_then(Value::as_object)
    }

    fn set(&mut self, k: &str, v: impl Into<Value>) {
        self.insert(k.to_string(), v.into());
    }

    fn setdefault(&mut self, k: &str, v: impl Into<Value>) {
        if !self.contains_key(k) {
            self.insert(k.to_string(), v.into());
        }
    }
}

/// A list of strings from a JSON array (non-strings converted with `str`).
pub fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().map(to_str).collect(),
        _ => Vec::new(),
    }
}

/// 8 hex characters, like Python's `uuid.uuid4().hex[:8]`.
pub fn new_id() -> String {
    use rand::Rng;
    format!("{:08x}", rand::thread_rng().gen::<u32>())
}
