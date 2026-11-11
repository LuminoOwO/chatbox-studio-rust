//! Templates: `{placeholder}` is replaced; a part in `[square brackets]` disappears when a
//! placeholder inside it is empty, e.g. `🎵 {title}[ — {artist}]`.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::pystr::strip;

pub type Values = HashMap<String, String>;

/// `\{(\w+)\}`
pub static PH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(\w+)\}").unwrap());
/// `\[([^\[\]]*)\]`
pub static GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\[\]]*)\]").unwrap());

/// Names of the placeholders in a template, in order (with repeats).
pub fn placeholders(template: &str) -> Vec<String> {
    PH.captures_iter(template).map(|c| c[1].to_string()).collect()
}

fn filled(values: &Values, name: &str) -> bool {
    values.get(name).map(|v| !strip(v).is_empty()).unwrap_or(false)
}

/// Fills a template. Unknown placeholders stay as written; `\n` (typed as two
/// characters) becomes a line break; the result is stripped.
pub fn fill(template: &str, values: &Values) -> String {
    let t = GROUP.replace_all(template, |c: &Captures| {
        let inner = &c[1];
        let names = placeholders(inner);
        if names.is_empty() {
            return c[0].to_string(); // plain text in brackets, e.g. "[EN]", stays
        }
        if names.iter().any(|n| !filled(values, n)) {
            return String::new();
        }
        inner.to_string()
    });
    let t = PH.replace_all(&t, |c: &Captures| match values.get(&c[1]) {
        Some(v) => v.clone(),
        None => c[0].to_string(),
    });
    strip(&t.replace("\\n", "\n")).to_string()
}

/// Python's `template.format_map(SafeDict(...))`: `{key}` replaced, unknown keys kept as
/// `{key}`, `{{` and `}}` give braces.
pub fn format_map(template: &str, values: &Values) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            if chars.get(i + 1) == Some(&'{') {
                out.push('{');
                i += 2;
                continue;
            }
            if let Some(end) = chars[i + 1..].iter().position(|&x| x == '}') {
                let key: String = chars[i + 1..i + 1 + end].iter().collect();
                let name = key.split([':', '!']).next().unwrap_or("");
                match values.get(name) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push('{');
                        out.push_str(&key);
                        out.push('}');
                    }
                }
                i += end + 2;
                continue;
            }
        } else if c == '}' && chars.get(i + 1) == Some(&'}') {
            out.push('}');
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// One translation's part of the chatbox message (`chatbox_template` with `{text}`,
/// `{original}`, `{src}`, `{dst}`). Untranslated text ignores formats that would show
/// `{original}` twice.
pub fn format_chatbox(template: &str, text: &str, translated: bool, dst: &str, src: &str,
                      original: &str) -> String {
    if template.contains("{original}") && (!translated || original.is_empty()) {
        return text.to_string();
    }
    let tpl = if template.is_empty() { "{text}" } else { template };
    let mut v = Values::new();
    v.insert("text".into(), text.into());
    v.insert("original".into(), original.into());
    v.insert("src".into(), src.to_uppercase());
    v.insert("dst".into(), dst.to_uppercase());
    let msg = format_map(tpl, &v).replace("\\n", "\n");
    let msg = strip(&msg);
    if msg.is_empty() { text.to_string() } else { msg.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(pairs: &[(&str, &str)]) -> Values {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn groups_vanish_when_empty() {
        let v = vals(&[("title", "Song"), ("artist", "")]);
        assert_eq!(fill("🎵 {title}[ — {artist}]", &v), "🎵 Song");
        let v = vals(&[("title", "Song"), ("artist", "Band")]);
        assert_eq!(fill("🎵 {title}[ — {artist}]", &v), "🎵 Song — Band");
        assert_eq!(fill("[EN] {x}", &vals(&[])), "[EN] {x}");
    }

    #[test]
    fn chatbox_format() {
        assert_eq!(format_chatbox("{text} ({original})", "Hola", true, "es", "en", "Hi"),
                   "Hola (Hi)");
        assert_eq!(format_chatbox("{text} ({original})", "Hi", false, "en", "en", "Hi"), "Hi");
        assert_eq!(format_chatbox("[{src}→{dst}] {text}", "Hola", true, "es", "en", "Hi"),
                   "[EN→ES] Hola");
        assert_eq!(format_chatbox("{text} {nope}", "a", true, "x", "y", "b"), "a {nope}");
        assert_eq!(format_chatbox("{original}\\n{text}", "B", true, "x", "y", "A"), "A\nB");
    }
}
