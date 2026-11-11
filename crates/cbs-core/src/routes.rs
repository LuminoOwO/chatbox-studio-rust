//! Translation routes and splitting long text into chatbox-sized messages.

use std::sync::LazyLock;

use regex::Regex;

use crate::languages::{self, ROOM};
use crate::pystr::{rfind_before, strip};
use crate::settings::Route;

/// VRChat's chatbox limit as the Python version applies it when splitting speech (code
/// points; the composer's own limit counts UTF-16 units).
pub const CHATBOX_LIMIT: usize = 144;

/// The switched-on routes as (src, dst) pairs, without duplicates. `room` targets become
/// the current room language (`room_language()`).
pub fn active_routes(routes: &[Route], room_language: impl Fn() -> String) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for r in routes {
        let mut key = (r.src.clone(), r.dst.clone());
        if key.1 == ROOM {
            key.1 = room_language();
        }
        if r.enabled && languages::is_known(&key.0) && languages::is_known(&key.1)
            && !out.contains(&key)
        {
            out.push(key);
        }
    }
    out
}

/// The source languages Whisper listens for, in route order.
pub fn sources(active: &[(String, String)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (s, _) in active {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}

static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]+").unwrap());

/// Splits text into messages of at most `limit` characters, at spaces or line breaks.
pub fn split_for_chatbox(text: &str, limit: usize) -> Vec<String> {
    let text = SPACES.replace_all(text, " ");
    let mut rest: Vec<char> = strip(&text).chars().collect();
    let mut parts = Vec::new();
    while rest.len() > limit {
        let a = rfind_before(&rest, ' ', limit);
        let b = rfind_before(&rest, '\n', limit);
        let cut = match (a, b) {
            (Some(x), Some(y)) => x.max(y),
            (Some(x), None) | (None, Some(x)) => x,
            (None, None) => 0,
        };
        let cut = if cut == 0 { limit } else { cut };
        let piece: String = rest[..cut].iter().collect();
        parts.push(strip(&piece).to_string());
        let after: String = rest[cut..].iter().collect();
        rest = strip(&after).chars().collect();
    }
    if !rest.is_empty() {
        parts.push(rest.into_iter().collect());
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes() {
        let rs = vec![Route::new("en", "es"), Route::new("en", "es"), Route::new("xx", "en"),
                      Route::new("es", ROOM)];
        assert_eq!(active_routes(&rs, || "fr".into()),
                   vec![("en".into(), "es".into()), ("es".into(), "fr".into())]);
    }

    #[test]
    fn splitting() {
        let long = "word ".repeat(40);
        let parts = split_for_chatbox(&long, 144);
        assert!(parts.iter().all(|p| p.chars().count() <= 144));
        assert_eq!(parts.join(" "), strip(&long));
        assert_eq!(split_for_chatbox(&"x".repeat(150), 144).len(), 2);
    }
}
