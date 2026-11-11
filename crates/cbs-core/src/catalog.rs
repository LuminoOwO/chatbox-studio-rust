//! Everything that can go into the chatbox: keys, labels, categories, privacy groups.

use std::collections::HashMap;
use std::sync::LazyLock;

pub use crate::catalog_data::{OWN_KEYS, SYS_KEYS};

/// Value groups that only exist on Linux (WiVRn, Monado, the UPower battery list). They
/// aren't offered on Windows, so the editor doesn't show blocks that would stay empty.
pub const LINUX_ONLY_GROUPS: &[&str] = &["WiVRn", "Devices"];
pub const LINUX_ONLY_KEYS: &[&str] = &["hmd_device"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub key: &'static str,
    pub label: &'static str,
    pub category: &'static str,
}

fn offered(e: &(&str, &str, &str)) -> bool {
    !cfg!(windows) || !(LINUX_ONLY_GROUPS.contains(&e.2) || LINUX_ONLY_KEYS.contains(&e.0))
}

/// PC / VR / device values offered on this platform (sysinfo.PLACEHOLDERS).
pub fn sys_keys() -> &'static [Entry] {
    static V: LazyLock<Vec<Entry>> = LazyLock::new(|| {
        SYS_KEYS.iter().filter(|e| offered(e))
            .map(|&(key, label, category)| Entry { key, label, category }).collect()
    });
    &V
}

/// Own keys + system keys (blocks.CATALOG).
pub fn catalog() -> &'static [Entry] {
    static V: LazyLock<Vec<Entry>> = LazyLock::new(|| {
        OWN_KEYS.iter().map(|&(key, label, category)| Entry { key, label, category })
            .chain(sys_keys().iter().copied()).collect()
    });
    &V
}

/// key → (label, category)
pub fn key_info(key: &str) -> Option<&'static Entry> {
    static M: LazyLock<HashMap<&'static str, &'static Entry>> =
        LazyLock::new(|| catalog().iter().map(|e| (e.key, e)).collect());
    M.get(key).copied()
}

pub fn is_known(key: &str) -> bool {
    key_info(key).is_some()
}

/// "Speech", "Text & layout", then the value categories in catalog order.
pub fn categories() -> &'static [&'static str] {
    static V: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
        let mut v = vec!["Speech", "Text & layout"];
        for e in catalog() {
            if !v.contains(&e.category) {
                v.push(e.category);
            }
        }
        v
    });
    &V
}

/// Keys of a system-value category group (used to decide what to read).
pub fn keys_in(groups: &[&str]) -> Vec<&'static str> {
    sys_keys().iter().filter(|e| groups.contains(&e.category)).map(|e| e.key).collect()
}

pub fn vr_keys() -> Vec<&'static str> {
    keys_in(&["VR headset", "Controllers & trackers", "WiVRn"])
}

pub fn audio_keys() -> Vec<&'static str> {
    keys_in(&["Audio"])
}

pub fn device_keys() -> Vec<&'static str> {
    keys_in(&["Devices"])
}

const SHORT: &[(&str, &str)] =
    &[("cpu", "CPU %"), ("gpu", "GPU %"), ("ram", "RAM %"), ("time", "Time"), ("title", "Title")];

/// Short name shown on a block in the chatbox editor.
pub fn short_name(key: &str) -> String {
    if let Some((_, s)) = SHORT.iter().find(|(k, _)| *k == key) {
        return s.to_string();
    }
    let (label, cat) = key_info(key).map(|e| (e.label, e.category)).unwrap_or((key, ""));
    // drop a trailing "(...)" - but "(%)" becomes " %"
    static TAIL: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\s*\(([^)]*)\)$").unwrap());
    let mut d = TAIL
        .replace(label, |c: &regex::Captures| if &c[1] == "%" { " %".to_string() } else { String::new() })
        .into_owned();
    if d.is_empty() {
        d = key.to_string();
    }
    let pre = match cat {
        "CPU" => "CPU",
        "Graphics" => "GPU",
        _ => "",
    };
    let low = d.to_lowercase();
    if !pre.is_empty()
        && ![pre.to_lowercase().as_str(), "graphics", "vram", "driver", "pcie", "all "]
            .iter().any(|w| low.contains(w))
    {
        let chars: Vec<char> = d.chars().collect();
        let second_upper = chars.get(1).map(|c| c.is_uppercase()).unwrap_or(false);
        d = if second_upper {
            format!("{pre} {d}")
        } else {
            let first: String = chars[0].to_lowercase().collect();
            format!("{pre} {first}{}", chars[1..].iter().collect::<String>())
        };
    }
    d
}

/// Privacy groups (buttons under the chatbox title).
pub const PRIVACY_GROUPS: &[(&str, &str)] = &[
    ("speech", "💬 Speech"), ("text", "📝 Text"), ("time", "🕒 Time"), ("music", "🎵 Music"),
    ("status", "💤 Status"), ("pc", "💻 PC"), ("vr", "🥽 VR"), ("vrchat", "🎮 VRChat"),
];

pub fn privacy_group(key: &str) -> &'static str {
    match key_info(key).map(|e| e.category).unwrap_or("") {
        "Time & date" | "Countdown" => "time",
        "Music" => "music",
        "Status" => "status",
        "VR headset" | "WiVRn" | "Controllers & trackers" => "vr",
        "VRChat" => "vrchat",
        _ => "pc",
    }
}

/// Ready-made text blocks in the list ("Text & layout"): (label, text, description).
pub const TEXT_STARTERS: &[(&str, &str, &str)] = &[
    ("Text", "Text", "Your own text - click it to type"),
    (" · ", " · ", "Separator"),
    (" | ", " | ", "Separator"),
    (" — ", " — ", "Separator"),
    ("Space", " ", "A space"),
    ("CPU: {cpu}%", "CPU: {cpu}%", "Text with a value - only shown while it has one"),
    ("🎵 {title}[ — {artist}]", "🎵 {title}[ — {artist}]", "Song and artist"),
    ("🕒 {time}", "🕒 {time}", "Clock"),
    ("💤 AFK for {afk_time}", "💤 AFK for {afk_time}", "Only while you're AFK"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_matches_python() {
        assert_eq!(OWN_KEYS.len(), 27);
        assert_eq!(SYS_KEYS.len(), 117);
        assert_eq!(key_info("time").unwrap().category, "Time & date");
        assert_eq!(categories()[0], "Speech");
        assert_eq!(short_name("cpu"), "CPU %");
        assert_eq!(privacy_group("title"), "music");
        assert_eq!(privacy_group("hmd_battery"), "vr");
    }
}
