//! Chatbox blocks: the chatbox is a row of small blocks you drag into place.
//!
//! | type | is |
//! |---|---|
//! | `speech` | your transcribed / translated sentences (only one) |
//! | `value` | one value, e.g. `{gpu_name}`, `{time}`, `{title}` |
//! | `text` | your own text; it can hold values ("CPU: {cpu}%") and then only shows when all of them have something. Parts in `[ ]` disappear on their own. |
//! | `rotating` | messages that take turns |
//! | `newline` | starts a new line |
//!
//! Blocks are kept as JSON objects (like the Python version's dicts), so every key
//! survives load and save. Priority: a number per block; speech 1, everything else 2. When
//! the chatbox runs out of room, the highest number goes first, all blocks with that number
//! together. 0 = never removed.

use serde_json::{json, Value};

use crate::catalog::{self, key_info};
use crate::jv::{new_id, to_i64, Obj, ObjExt};
use crate::template::placeholders;

pub const TYPES: &[&str] = &["speech", "value", "text", "rotating", "newline"];

pub fn speech_defaults() -> Obj {
    json!({"width": 144, "mode": "pages", "page_seconds": 0.0, "scroll_words": 3,
           "scroll_seconds": 1.6, "hold_seconds": 6.0, "flow": "live", "limit": 0})
        .as_object().unwrap().clone()
}

pub fn display_defaults() -> Obj {
    json!({"show_mode": "full", "win_chars": 40, "win_step": 2, "win_seconds": 2.0,
           "win_lines": 1})
        .as_object().unwrap().clone()
}

pub fn opt_defaults() -> Obj {
    json!({"timezone": "local", "style": "24h", "target": "2026-12-24 18:00",
           "label": "Christmas", "max_title": 40})
        .as_object().unwrap().clone()
}

/// media.default_filters()
pub fn media_filter_defaults() -> Obj {
    json!({"mode": "hide", "rules": [], "fallback": true,
           "fallback_ignore": ["VRChat", "Discord", "WEBRTC VoiceEngine", "Steam",
                               "steamwebhelper", "wine", "vrserver", "vrcompositor",
                               "vrmonitor", "SteamVR", "OVRServer", "Oculus", "System sounds",
                               "speech-dispatcher", "TeamSpeak", "Mumble", "Zoom", "Teams"],
           "fallback_threshold": 0.02})
        .as_object().unwrap().clone()
}

fn merged(base: Obj, over: &Obj) -> Obj {
    let mut b = base;
    for (k, v) in over {
        b.insert(k.clone(), v.clone());
    }
    b
}

/// A new block of a kind, with `over` applied on top (`opts` merged with the defaults).
pub fn new_block(kind: &str, over: Obj) -> Obj {
    let mut b = Obj::new();
    b.set("id", new_id());
    b.set("type", kind);
    b.set("prio", if kind == "speech" { 1 } else { 2 });
    b.set("visible", true);
    match kind {
        "speech" => {
            b = merged(b, &speech_defaults());
            b.set("enabled", true);
        }
        "value" => b.set("key", "time"),
        "text" => b.set("text", "Text"),
        "rotating" => {
            b.set("messages", "Hello!\nHave a nice day ✨");
            b.set("interval", 10.0);
            b.set("random", false);
        }
        _ => {}
    }
    if kind != "speech" && kind != "newline" {
        b = merged(b, &display_defaults());
        b.set("opts", Value::Object(opt_defaults()));
    }
    let opts_over = over.get("opts").and_then(Value::as_object).cloned();
    b = merged(b, &over);
    if let Some(o) = opts_over {
        b.set("opts", Value::Object(merged(opt_defaults(), &o)));
    }
    b
}

pub fn default_blocks() -> Vec<Value> {
    vec![Value::Object(new_block("speech", Obj::new()))]
}

/// The value keys a block uses.
pub fn keys_of(b: &Obj) -> Vec<String> {
    match b.s("type", "").as_str() {
        "value" => vec![b.s("key", "")],
        "text" => placeholders(&b.s("text", "")),
        "rotating" => placeholders(&b.s("messages", "")),
        _ => Vec::new(),
    }
}

/// The privacy groups a block belongs to.
pub fn groups_of(b: &Obj) -> Vec<&'static str> {
    let t = b.s("type", "");
    if t == "speech" {
        return vec!["speech"];
    }
    let mut out: Vec<&'static str> = Vec::new();
    for k in keys_of(b) {
        if catalog::is_known(&k) {
            let g = catalog::privacy_group(&k);
            if !out.contains(&g) {
                out.push(g);
            }
        }
    }
    if out.is_empty() && (t == "text" || t == "rotating") {
        out.push("text");
    }
    out
}

/// Option sections a block shows in the editor, from the values in it.
pub fn option_sets(b: &Obj) -> Vec<&'static str> {
    let keys = keys_of(b);
    let cats: Vec<&str> = keys.iter().filter_map(|k| key_info(k)).map(|e| e.category).collect();
    let mut out = Vec::new();
    if cats.contains(&"Time & date") {
        out.push("time");
    }
    if cats.contains(&"Countdown") {
        out.push("countdown");
    }
    if keys.iter().any(|k| k == "title" || k == "artist") {
        out.push("media");
    }
    if cats.contains(&"Graphics") {
        out.push("gpu");
    }
    out
}

/// Fills in what's missing, drops unknown block types, keeps exactly one speech block
/// (first), converts 1.18 configs. The result is what the composer and editor work on.
pub fn normalize(cfg: Option<&Value>) -> Obj {
    let mut cfg: Obj = cfg.and_then(Value::as_object).cloned().unwrap_or_default();
    if !cfg.contains_key("blocks")
        && cfg.get("elements").map(crate::jv::truthy).unwrap_or(false)
    {
        cfg = crate::legacy::migrate(cfg);
    }
    cfg.setdefault("blocks", Value::Array(default_blocks()));
    cfg.setdefault("speech_alone", false);
    cfg.setdefault("min_interval", 1.2);
    cfg.setdefault("keepalive", 20.0);
    cfg.setdefault("hide_extras", false);
    cfg.setdefault("hidden_groups", json!([]));
    cfg.setdefault("lyrics", json!({"online": false, "folder": "", "offset": 0.0}));
    cfg.setdefault("gpu_index", 0);
    cfg.setdefault("show_paused", false);
    let mut f = cfg.obj("media_filters").cloned().unwrap_or_default();
    for (k, v) in media_filter_defaults() {
        f.entry(k).or_insert(v);
    }
    cfg.set("media_filters", Value::Object(f));

    let blocks = cfg.get("blocks").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out: Vec<Value> = Vec::new();
    let mut speech = false;
    for v in blocks {
        let Some(mut b) = v.as_object().cloned() else { continue };
        let t = b.s("type", "");
        if !TYPES.contains(&t.as_str()) {
            continue;
        }
        if t == "speech" {
            if speech {
                continue;
            }
            speech = true;
            for (k, v) in speech_defaults() {
                b.entry(k).or_insert(v);
            }
            b.setdefault("enabled", true);
            let flow = b.s("flow", "");
            if flow != "live" && flow != "queue" {
                b.set("flow", "live");
            }
        }
        b.setdefault("id", new_id());
        b.setdefault("visible", true);
        let default_prio = if t == "speech" { 1 } else { 2 };
        let prio = match b.get("prio") {
            None => Some(default_prio),
            Some(v) => to_i64(v),
        };
        b.set("prio", prio.map(|p| p.max(0)).unwrap_or(2));
        if t != "speech" && t != "newline" {
            for (k, v) in display_defaults() {
                b.entry(k).or_insert(v);
            }
            let o = b.obj("opts").cloned().unwrap_or_default();
            b.set("opts", Value::Object(merged(opt_defaults(), &o)));
        }
        if t == "value" {
            b.setdefault("key", "time");
        }
        if t == "text" {
            b.setdefault("text", "");
        }
        out.push(Value::Object(b));
    }
    if !speech {
        out.insert(0, Value::Object(new_block("speech", Obj::new())));
    }
    cfg.set("blocks", Value::Array(out));
    cfg
}

/// The blocks of a normalised config.
pub fn blocks(cfg: &Obj) -> Vec<&Obj> {
    cfg.get("blocks")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_object).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn with(pairs: Value) -> Obj {
        pairs.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn normalize_fills_and_dedups() {
        let cfg = json!({"blocks": [
            {"type": "value", "key": "cpu"},
            {"type": "speech"},
            {"type": "speech"},
            {"type": "bogus"},
            {"type": "text", "text": "Hi", "prio": "3"},
        ]});
        let n = normalize(Some(&cfg));
        let bs = blocks(&n);
        assert_eq!(bs.len(), 3);
        assert_eq!(bs[0].s("type", ""), "value");
        assert_eq!(bs[0].i("prio", -1), 2);
        assert_eq!(bs[1].s("flow", ""), "live");
        assert_eq!(bs[2].i("prio", -1), 3);
        assert_eq!(bs[2].obj("opts").unwrap().s("style", ""), "24h");
        assert_eq!(n.obj("media_filters").unwrap().s("mode", ""), "hide");
    }

    #[test]
    fn empty_config_gets_speech() {
        let n = normalize(None);
        assert_eq!(blocks(&n).len(), 1);
        assert_eq!(blocks(&n)[0].s("type", ""), "speech");
    }

    #[test]
    fn groups() {
        let b = new_block("text", with(json!({"text": "🎵 {title} {cpu}"})));
        assert_eq!(groups_of(&b), vec!["music", "pc"]);
        let t = new_block("text", Map::new());
        assert_eq!(groups_of(&t), vec!["text"]);
        assert_eq!(option_sets(&b), vec!["media"]);
    }
}
