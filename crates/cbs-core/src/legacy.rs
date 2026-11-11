//! Converting the "elements" chatbox setups of 1.18 and older to blocks.

use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::{json, Value};

use crate::blocks::{display_defaults, media_filter_defaults, new_block, speech_defaults};
use crate::jv::{new_id, to_str, truthy, Obj, ObjExt};
use crate::pystr::{is_alnum, strip};
use crate::template::PH;

const OLD_INFO_TYPES: &[&str] = &["specs", "stats", "vr"];
const PRIORITIES: &[&str] = &["low", "normal", "high", "keep"];
const DISPLAY_MODES: &[&str] = &["full", "scroll", "marquee", "bounce", "lines", "pages", "typewriter"];

fn element_defaults(kind: &str) -> Option<Obj> {
    let v = match kind {
        "speech" => {
            let mut d = speech_defaults();
            d.set("icon", "");
            return Some(d);
        }
        "text" => json!({"text": "Hi! I'm using a voice translator 🌍", "icon": "", "gpu_index": 0}),
        "rotating" => json!({"messages": "Hello!\nHave a nice day ✨", "interval": 10.0,
                             "random": false, "icon": ""}),
        "time" => json!({"timezone": "local", "style": "24h", "format": "{time}", "icon": "🕒"}),
        "countdown" => json!({"target": "2026-12-24 18:00", "label": "Christmas",
                              "format": "{label} in {left}", "done": "🎉 {label}!", "icon": "⏳"}),
        "media" => json!({"format": "{title}[ — {artist}]", "fallback_format": "{app}",
                          "icon": "🎵", "fallback_icon": "🔊", "show_paused": false,
                          "max_title": 40, "filters": null}),
        "lyrics" => json!({"format": "{line}", "icon": "🎤", "online": false, "offset": 0.0,
                           "folder": ""}),
        "afk" => json!({"format": "AFK for {duration}", "icon": "💤"}),
        "playtime" => json!({"since": "vrchat", "format": "{elapsed}", "icon": "⏱"}),
        "window" => json!({"format": "{app}", "show_title": false, "icon": "🖥"}),
        _ => return None,
    };
    v.as_object().cloned()
}

fn default_priority(kind: &str) -> &'static str {
    match kind {
        "speech" | "afk" => "high",
        "rotating" | "media" | "playtime" | "window" | "lyrics" => "low",
        _ => "normal",
    }
}

fn new_element(kind: &str, over: Value) -> Obj {
    let mut el = Obj::new();
    el.set("id", new_id());
    el.set("type", kind);
    el.set("enabled", true);
    el.set("visible", true);
    el.set("priority", default_priority(kind));
    for (k, v) in element_defaults(kind).unwrap_or_default() {
        el.insert(k, v);
    }
    if kind != "speech" {
        for (k, v) in display_defaults() {
            el.insert(k, v);
        }
    }
    if kind == "media" {
        el.set("filters", Value::Object(media_filter_defaults()));
    }
    if let Some(o) = over.as_object() {
        for (k, v) in o {
            el.insert(k.clone(), v.clone());
        }
    }
    el
}

fn default_config() -> Obj {
    let els = vec![
        Value::Object(new_element("speech", json!({}))),
        Value::Object(new_element("time", json!({"enabled": false}))),
        Value::Object(new_element("media", json!({"enabled": false}))),
        Value::Object(new_element("text", json!({"enabled": false}))),
    ];
    let mut c = Obj::new();
    c.set("elements", Value::Array(els));
    c.set("separator", "newline");
    c.set("custom_separator", " | ");
    c.set("speech_alone", false);
    c.set("min_interval", 1.2);
    c.set("keepalive", 20.0);
    c.set("hide_extras", false);
    c
}

/// '🕒 {time}' → ('🕒', '{time}'). Used to convert formats saved before 1.9.
pub fn split_icon(fmt: &str) -> (String, String) {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^\s*(\S{1,3})\s+(.*)$").unwrap());
    if let Some(c) = RE.captures(fmt) {
        if !c[1].chars().any(|ch| is_alnum(ch) || ch == '{' || ch == '[') {
            return (c[1].to_string(), c[2].to_string());
        }
    }
    (String::new(), fmt.to_string())
}

/// Fills in missing keys of the old (1.18 and older) element setup.
pub fn normalize_elements(cfg: Obj) -> Obj {
    let base = default_config();
    if cfg.is_empty() {
        return base;
    }
    let mut cfg = cfg;
    for (k, v) in base {
        cfg.entry(k).or_insert(v);
    }
    let mut els: Vec<Value> = Vec::new();
    let list = cfg.get("elements").and_then(Value::as_array).cloned().unwrap_or_default();
    for v in list {
        let Some(mut el) = v.as_object().cloned() else { continue };
        let mut kind = el.s("type", "");
        if OLD_INFO_TYPES.contains(&kind.as_str()) {
            // 1.16: PC specs, PC stats and VR headset are one "Custom text & info" element
            let mut fmt = el.s("format", "");
            if !el.contains_key("icon") {
                let (icon, rest) = split_icon(&fmt);
                el.set("icon", icon);
                fmt = rest;
            }
            el.remove("format");
            kind = "text".into();
            el.set("type", "text");
            el.set("text", fmt);
            el.setdefault("priority", "low");
        }
        let Some(defaults) = element_defaults(&kind) else { continue };
        el.setdefault("id", new_id());
        el.setdefault("enabled", true);
        el.setdefault("visible", true);
        let pr = el.get("priority").and_then(Value::as_str).unwrap_or("");
        if !PRIORITIES.contains(&pr) {
            el.set("priority", default_priority(&kind));
        }
        if kind == "speech" {
            let flow = el.get("flow").and_then(Value::as_str).unwrap_or("");
            if flow != "live" && flow != "queue" {
                el.set("flow", "live"); // 1.10: live is the default
                el.remove("queue");
            }
        }
        if !el.contains_key("icon") && el.contains_key("format") {
            let (icon, fmt) = split_icon(&el.s("format", ""));
            el.set("icon", icon);
            el.set("format", fmt);
            if kind == "media" && el.contains_key("fallback_format") {
                let (icon, fmt) = split_icon(&el.s("fallback_format", ""));
                el.set("fallback_icon", icon);
                el.set("fallback_format", fmt);
            }
        }
        for (k, v) in defaults {
            el.entry(k).or_insert(v);
        }
        if kind != "speech" {
            for (k, v) in display_defaults() {
                el.entry(k).or_insert(v);
            }
            let mode = el.get("show_mode").and_then(Value::as_str).unwrap_or("");
            if !DISPLAY_MODES.contains(&mode) {
                el.set("show_mode", "full");
            }
        }
        if kind == "media" {
            let mut f = match el.get("filters") {
                Some(Value::Object(o)) if !o.is_empty() => o.clone(),
                _ => Obj::new(),
            };
            for (k, v) in media_filter_defaults() {
                f.entry(k).or_insert(v);
            }
            el.set("filters", Value::Object(f));
        }
        els.push(Value::Object(el));
    }
    if !els.iter().any(|e| e.get("type").and_then(Value::as_str) == Some("speech")) {
        els.insert(0, Value::Object(new_element("speech", json!({}))));
    }
    cfg.set("elements", Value::Array(els));
    cfg
}

fn prio_num(p: &str) -> i64 {
    match p {
        "keep" => 0,
        "high" => 1,
        "normal" => 2,
        "low" => 3,
        _ => 2,
    }
}

fn rename_table(kind: &str) -> Vec<(&'static str, &'static str)> {
    match kind {
        "countdown" => vec![("label", "countdown_label"), ("left", "countdown_left"),
                            ("date", "countdown_date")],
        "media" => vec![("app", "player")],
        "lyrics" => vec![("line", "lyric_line"), ("next", "lyric_next")],
        "afk" => vec![("duration", "afk_time")],
        "window" => vec![("app", "active_app"), ("title", "window_title")],
        _ => vec![],
    }
}

fn rename(fmt: &str, kind: &str, extra: &[(&'static str, &'static str)]) -> String {
    let mut table = rename_table(kind);
    for (k, v) in extra {
        table.retain(|(a, _)| a != k);
        table.push((k, v));
    }
    PH.replace_all(fmt, |c: &Captures| {
        let name = &c[1];
        let new = table.iter().find(|(a, _)| *a == name).map(|(_, b)| *b).unwrap_or(name);
        format!("{{{new}}}")
    })
    .into_owned()
}

fn icon(icon: &str, text: &str) -> String {
    let icon = strip(icon);
    if !icon.is_empty() && !text.is_empty() { format!("{icon} {text}") } else { text.to_string() }
}

fn str_or(el: &Obj, k: &str, default: &str) -> String {
    // `el.get(k, default) or ""` followed by str()
    match el.get(k) {
        None => default.to_string(),
        Some(v) if !truthy(v) => String::new(),
        Some(v) => to_str(v),
    }
}

fn val(el: &Obj, k: &str, default: Value) -> Value {
    el.get(k).cloned().unwrap_or(default)
}

/// 1.18 'elements' → 1.19 'blocks'. The old list is kept as 'elements_old'.
pub fn migrate(cfg: Obj) -> Obj {
    let old = normalize_elements(cfg.clone());
    let mut cfg = cfg;
    let mut blocks: Vec<Obj> = Vec::new();
    let sep = old.s("separator", "newline");
    let sep_text = match sep.as_str() {
        "bar" => " | ".to_string(),
        "dot" => " · ".to_string(),
        "space" => "  ".to_string(),
        _ => old.s("custom_separator", " | ").replace("\\n", "\n"),
    };
    let mut media_filters: Option<Value> = None;
    let mut lyrics: Option<Obj> = None;
    let mut gpu_index: i64 = 0;
    let mut speech_seen = false;

    let add_sep = |blocks: &mut Vec<Obj>| {
        let Some(last) = blocks.last() else { return };
        if sep == "newline" || sep_text == "\n" {
            blocks.push(new_block("newline", Obj::new()));
        } else {
            let prio = last.get("prio").cloned().unwrap_or(json!(2));
            let mut o = Obj::new();
            o.set("text", sep_text.clone());
            o.set("prio", prio);
            blocks.push(new_block("text", o));
        }
    };

    let elements = old.get("elements").and_then(Value::as_array).cloned().unwrap_or_default();
    for v in elements {
        let Some(el) = v.as_object() else { continue };
        let kind = el.s("type", "");
        let prio = prio_num(&el.s("priority", "normal"));
        let mut common = Obj::new();
        common.set("prio", prio);
        common.set("visible", val(el, "visible", json!(true)));
        for k in display_defaults().keys() {
            if let Some(x) = el.get(k) {
                common.insert(k.clone(), x.clone());
            }
        }
        let with = |extra: Vec<(&str, Value)>| {
            let mut o = Obj::new();
            for (k, v) in extra {
                o.set(k, v);
            }
            for (k, v) in &common {
                o.insert(k.clone(), v.clone());
            }
            o
        };
        if kind == "media" && media_filters.is_none() {
            media_filters = Some(val(el, "filters", Value::Null));
        }
        if kind == "lyrics" {
            let mut l = Obj::new();
            l.set("online", el.b("online", false));
            l.set("folder", val(el, "folder", json!("")));
            l.set("offset", el.f_or("offset", 0.0));
            lyrics = Some(l);
        }
        if kind == "text" {
            gpu_index = el.i_or("gpu_index", gpu_index);
        }
        let enabled = el.b("enabled", true);
        if kind == "speech" {
            speech_seen = true;
            let mut over = Obj::new();
            for k in speech_defaults().keys() {
                if let Some(x) = el.get(k) {
                    over.insert(k.clone(), x.clone());
                }
            }
            let mut b = new_block("speech", over);
            b.set("prio", prio);
            b.set("enabled", enabled);
            b.set("visible", val(el, "visible", json!(true)));
            if el.get("icon").map(truthy).unwrap_or(false) {
                b.set("icon", el["icon"].clone());
            }
            if !enabled {
                blocks.push(b); // kept (switched off) so its settings survive
                continue;
            }
            add_sep(&mut blocks);
            blocks.push(b);
            continue;
        }
        if !enabled {
            continue;
        }
        add_sep(&mut blocks);
        let ic = el.s("icon", "");
        let fmt = |d: &str| str_or(el, "format", d);
        match kind.as_str() {
            "text" => {
                let text = el.s("text", "");
                let unescaped = text.replace("\\n", "\n");
                let lines: Vec<&str> = unescaped.split('\n').collect();
                if el.s("show_mode", "full") == "full" && lines.len() > 1 {
                    for (n, line) in lines.iter().enumerate() {
                        if n > 0 {
                            blocks.push(new_block("newline", Obj::new()));
                        }
                        let t = if n == 0 { icon(&ic, line) } else { line.to_string() };
                        blocks.push(new_block("text", with(vec![("text", t.into())])));
                    }
                } else {
                    blocks.push(new_block("text", with(vec![("text", icon(&ic, &text).into())])));
                }
            }
            "rotating" => {
                blocks.push(new_block("rotating", with(vec![
                    ("messages", val(el, "messages", json!(""))),
                    ("interval", val(el, "interval", json!(10.0))),
                    ("random", el.b("random", false).into()),
                ])));
                if !ic.is_empty() {
                    let mut o = Obj::new();
                    o.set("text", format!("{ic} "));
                    o.set("prio", prio);
                    let at = blocks.len() - 1;
                    blocks.insert(at, new_block("text", o));
                }
            }
            "time" => {
                let opts = json!({"timezone": val(el, "timezone", json!("local")),
                                  "style": val(el, "style", json!("24h"))});
                blocks.push(new_block("text", with(vec![
                    ("text", icon(&ic, &el.s("format", "{time}")).into()), ("opts", opts)])));
            }
            "countdown" => {
                let opts = json!({"target": val(el, "target", json!("")),
                                  "label": val(el, "label", json!(""))});
                blocks.push(new_block("text", with(vec![
                    ("text", icon(&ic, &rename(&fmt(""), &kind, &[])).into()),
                    ("opts", opts.clone())])));
                let done = str_or(el, "done", "");
                if !strip(&done).is_empty() {
                    let t = rename(&done, &kind, &[("label", "countdown_done")]);
                    blocks.push(new_block("text", with(vec![("text", t.into()), ("opts", opts)])));
                }
            }
            "media" => {
                blocks.push(new_block("text", with(vec![
                    ("text", icon(&ic, &rename(&fmt(""), &kind, &[])).into()),
                    ("opts", json!({"max_title": el.i_or("max_title", 0)}))])));
                let fallback = el.obj("filters").map(|f| f.b("fallback", true)).unwrap_or(true);
                if fallback {
                    let fb = rename(&el.s("fallback_format", "{app}"), &kind, &[("app", "audio_app")]);
                    let fi = el.s("fallback_icon", "");
                    blocks.push(new_block("text", with(vec![("text", icon(&fi, &fb).into())])));
                }
            }
            "lyrics" | "afk" => {
                blocks.push(new_block("text", with(vec![
                    ("text", icon(&ic, &rename(&fmt(""), &kind, &[])).into())])));
            }
            "playtime" => {
                let key = if el.s("since", "vrchat") == "vrchat" { "playtime" } else { "app_time" };
                blocks.push(new_block("text", with(vec![
                    ("text", icon(&ic, &rename(&fmt(""), &kind, &[("elapsed", key)])).into())])));
            }
            "window" => {
                let mut f = rename(&el.s("format", "{app}"), &kind, &[]);
                if !el.b("show_title", false) {
                    f = f.replace("{window_title}", "");
                }
                blocks.push(new_block("text", with(vec![("text", icon(&ic, strip(&f)).into())])));
            }
            _ => {}
        }
    }
    if !speech_seen {
        blocks.insert(0, new_block("speech", Obj::new()));
    }
    // drop separators that ended up at the very end
    while blocks.last().map(|b| b.s("type", "") == "newline").unwrap_or(false) {
        blocks.pop();
    }
    cfg.set("blocks", Value::Array(blocks.into_iter().map(Value::Object).collect()));
    let old_els = cfg.remove("elements").unwrap_or(json!([]));
    cfg.set("elements_old", old_els);
    let mf = match media_filters {
        Some(v) if truthy(&v) => v,
        _ => Value::Object(media_filter_defaults()),
    };
    cfg.set("media_filters", mf);
    cfg.setdefault("lyrics", json!({"online": false, "folder": "", "offset": 0.0}));
    if let (Some(l), Some(Value::Object(dst))) = (lyrics, cfg.get_mut("lyrics")) {
        for (k, v) in l {
            dst.insert(k, v);
        }
    }
    cfg.setdefault("gpu_index", gpu_index);
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{blocks, normalize};

    #[test]
    fn icons() {
        assert_eq!(split_icon("🕒 {time}"), ("🕒".into(), "{time}".into()));
        assert_eq!(split_icon("CPU {cpu}"), ("".into(), "CPU {cpu}".into()));
    }

    #[test]
    fn old_elements_become_blocks() {
        let old = json!({"separator": "bar", "elements": [
            {"type": "speech"},
            {"type": "time", "icon": "🕒", "format": "{time}", "style": "12h"},
            {"type": "afk", "icon": "💤", "format": "AFK for {duration}", "priority": "keep"},
            {"type": "media", "enabled": false},
            {"type": "stats", "format": "💻 CPU {cpu}%"},
        ]});
        let n = normalize(Some(&old));
        let texts: Vec<String> = blocks(&n).iter().map(|b| b.s("text", "")).collect();
        assert_eq!(texts, vec!["", " | ", "🕒 {time}", " | ", "💤 AFK for {afk_time}", " | ",
                               "💻 CPU {cpu}%"]);
        assert_eq!(blocks(&n)[2].obj("opts").unwrap().s("style", ""), "12h");
        assert_eq!(blocks(&n)[4].i("prio", -1), 0);
        assert_eq!(blocks(&n)[6].i("prio", -1), 3);
        assert!(n.contains_key("elements_old") && !n.contains_key("elements"));
    }
}
