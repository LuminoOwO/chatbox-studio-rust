//! `settings.json` - the same file, keys and defaults as the Python version.
//!
//! Loading is forgiving: every field is read on its own, so one odd value (say, a number
//! saved as a string) falls back to its default instead of losing all settings. Keys this
//! version doesn't know are kept and written back unchanged.
//!
//! Writing matches Python's `json.dumps(indent=2)`: 2-space indent and non-ASCII characters
//! escaped as `\uXXXX`, so the Python version can still read the file on any Windows code
//! page.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths;

/// One translation route: speech in `src` goes to the chatbox in `dst`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub dst: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

fn yes() -> bool {
    true
}

impl Route {
    pub fn new(src: &str, dst: &str) -> Self {
        Route { src: src.into(), dst: dst.into(), enabled: true, other: Map::new() }
    }
}

macro_rules! settings {
    ($( $(#[$m:meta])* $name:ident : $ty:ty = $default:expr ),* $(,)?) => {
        /// All settings. Field names are the JSON keys.
        #[derive(Debug, Clone, PartialEq, Serialize)]
        pub struct Settings {
            $( $(#[$m])* pub $name: $ty, )*
            /// Keys this version doesn't know, kept as they are.
            #[serde(flatten)]
            pub other: Map<String, Value>,
        }

        impl Default for Settings {
            fn default() -> Self {
                Settings { $( $name: $default, )* other: Map::new() }
            }
        }

        impl Settings {
            /// From a JSON object; each field falls back to its default on its own.
            pub fn from_map(mut map: Map<String, Value>) -> Self {
                let mut s = Settings::default();
                $(
                    if let Some(v) = map.remove(stringify!($name)) {
                        if let Ok(x) = serde_json::from_value::<$ty>(v.clone()) {
                            s.$name = x;
                        } else if let Some(x) = lenient::<$ty>(&v) {
                            s.$name = x;
                        }
                    }
                )*
                s.other = map;
                s
            }

            /// The known keys, in file order.
            pub const KEYS: &'static [&'static str] = &[ $( stringify!($name), )* ];
        }
    };
}

settings! {
    mic_device: Value = Value::Null,
    gpu_index: i64 = 0,
    speech_device: String = String::new(),
    model: String = "large-v3".into(),
    compute_type: String = "auto".into(),
    routes: Vec<Route> = vec![Route::new("en", "en")],
    join_with: String = "newline".into(),
    translator_model: String = "nllb-200-distilled-600M".into(),
    translator_device: String = "same".into(),
    nllb_for_english: bool = false,
    chatbox_template: String = "{text}".into(),
    translate_typed: bool = false,
    beam_size: i64 = 1,
    vad_threshold: f64 = 0.5,
    end_silence_ms: i64 = 500,
    min_speech_ms: i64 = 250,
    max_utterance_s: f64 = 10.0,
    osc_enabled: bool = true,
    osc_host: String = "127.0.0.1".into(),
    osc_port: i64 = 9000,
    osc_listen_port: i64 = 9001,
    oscquery: bool = true,
    typing_indicator: bool = true,
    notify_sound: bool = false,
    follow_vrc_mute: bool = true,
    ui_theme: String = "dark".into(),
    ui_scale: f64 = 1.0,
    min_send_interval_s: f64 = 1.5,
    filters_enabled: bool = true,
    remove_fillers: bool = true,
    min_words: i64 = 0,
    filter_typed: bool = false,
    replace_rules: Vec<Value> = Vec::new(),
    block_rules: Vec<Value> = Vec::new(),
    chatbox: Value = Value::Object(Map::new()),
    earshot: Value = Value::Object(Map::new()),
    extra: Map<String, Value> = Map::new(),
}

/// Values Python would have accepted: numbers in strings, ints for floats, 0/1 for bools.
fn lenient<T: serde::de::DeserializeOwned>(v: &Value) -> Option<T> {
    let alt = match v {
        Value::String(s) => {
            let t = s.trim();
            if let Ok(i) = t.parse::<i64>() {
                Value::from(i)
            } else if let Ok(f) = t.parse::<f64>() {
                serde_json::Number::from_f64(f).map(Value::Number)?
            } else if t.eq_ignore_ascii_case("true") {
                Value::Bool(true)
            } else if t.eq_ignore_ascii_case("false") {
                Value::Bool(false)
            } else {
                return None;
            }
        }
        Value::Number(n) => {
            // a float where an int is expected (e.g. 500.0) or a number for a bool
            let f = n.as_f64()?;
            if let Ok(x) = serde_json::from_value::<T>(Value::from(f as i64)) {
                return Some(x);
            }
            Value::Bool(f != 0.0)
        }
        _ => return None,
    };
    serde_json::from_value(alt).ok()
}

impl Settings {
    /// Reads a settings file. `Ok(None)` when there is none yet (first start).
    pub fn load_from(path: &Path) -> io::Result<Option<Settings>> {
        let text = match std::fs::read(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        match serde_json::from_slice::<Value>(&text) {
            Ok(Value::Object(map)) => Ok(Some(Settings::from_map(map))),
            Ok(_) | Err(_) => Ok(None),     // broken file: start with defaults
        }
    }

    /// The settings of this user (`None` on first start).
    pub fn load() -> Option<Settings> {
        Settings::load_from(&paths::settings_file()).ok().flatten()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Python-compatible JSON text (indent 2, ASCII only).
    pub fn to_json(&self) -> String {
        to_python_json(&self.to_value())
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // write next to it, then replace: a crash while saving never leaves half a file
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(&tmp, path)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&paths::settings_file())
    }

    /// An entry of `extra` (per-app conveniences like `onboarded`).
    pub fn extra_bool(&self, key: &str) -> bool {
        self.extra.get(key).map(truthy).unwrap_or(false)
    }
}

/// Python truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `json.dumps(value, indent=2)` as Python writes it: `", "`-free pretty print with
/// `": "`, 2 spaces, non-ASCII as `\uXXXX` (surrogate pairs above U+FFFF), floats always
/// with a fraction (`10.0`).
pub fn to_python_json(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, 0, &mut out);
    out
}

fn write_value(v: &Value, indent: usize, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_f64() {
                let f = n.as_f64().unwrap_or(0.0);
                if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e16 {
                    out.push_str(&format!("{f:.1}"));
                } else {
                    out.push_str(&format!("{f}"));
                }
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::String(s) => write_string(s, out),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                out.push_str(&" ".repeat(indent + 2));
                write_value(x, indent + 2, out);
            }
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            out.push(']');
        }
        Value::Object(o) => {
            if o.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                out.push_str(&" ".repeat(indent + 2));
                write_string(k, out);
                out.push_str(": ");
                write_value(x, indent + 2, out);
            }
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            out.push('}');
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7f => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_python() {
        let s = Settings::default();
        assert_eq!(s.model, "large-v3");
        assert_eq!(s.routes, vec![Route::new("en", "en")]);
        assert_eq!(s.end_silence_ms, 500);
        assert_eq!(s.osc_port, 9000);
        assert_eq!(Settings::KEYS.len(), 37);
    }

    #[test]
    fn forgiving_load_keeps_unknown_keys() {
        let v: Value = serde_json::json!({
            "model": "medium", "end_silence_ms": "700", "beam_size": 5.0,
            "osc_enabled": 0, "routes": "broken", "something_new": [1, 2]
        });
        let s = Settings::from_map(v.as_object().unwrap().clone());
        assert_eq!(s.model, "medium");
        assert_eq!(s.end_silence_ms, 700);
        assert_eq!(s.beam_size, 5);
        assert!(!s.osc_enabled);
        assert_eq!(s.routes, Settings::default().routes);
        assert_eq!(s.other.get("something_new"), Some(&serde_json::json!([1, 2])));
        let back = s.to_value();
        assert_eq!(back["something_new"], serde_json::json!([1, 2]));
    }

    #[test]
    fn python_style_json() {
        let v = serde_json::json!({"a": 1, "b": 10.0, "c": "é🎵", "d": [], "e": {"x": true}});
        assert_eq!(
            to_python_json(&v),
            "{\n  \"a\": 1,\n  \"b\": 10.0,\n  \"c\": \"\\u00e9\\ud83c\\udfb5\",\n  \"d\": [],\n  \"e\": {\n    \"x\": true\n  }\n}"
        );
    }
}
