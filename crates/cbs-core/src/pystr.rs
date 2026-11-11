//! Small helpers that behave like Python's string functions.
//!
//! The Python version counts and slices strings by *code points* (`len`, `s[a:b]`), and
//! measures the VRChat limit in UTF-16 units. These helpers keep that behaviour exactly,
//! so text is split and cut at the same places as before.

/// Number of code points (Python `len`).
pub fn clen(s: &str) -> usize {
    s.chars().count()
}

/// Length the way VRChat counts it: UTF-16 code units (an emoji counts 2).
pub fn ulen(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Python `s[a:b]` on code points (indices clamped, no negatives).
pub fn slice(s: &str, a: usize, b: usize) -> String {
    if b <= a {
        return String::new();
    }
    s.chars().skip(a).take(b - a).collect()
}

/// Python `s[:n]`.
pub fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Python `s[n:]`.
pub fn tail(s: &str, n: usize) -> String {
    s.chars().skip(n).collect()
}

/// Python `str.isspace` for one character.
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\x1c'..='\x1f')
}

/// Python `str.strip()` (whitespace).
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// Python `str.rstrip()`.
pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

/// Python `str.strip(chars)`.
pub fn strip_chars<'a>(s: &'a str, chars: &str) -> &'a str {
    s.trim_matches(|c| chars.contains(c))
}

/// Python `str.split()` (runs of whitespace, no empty parts).
pub fn split_ws(s: &str) -> Vec<&str> {
    s.split(is_space).filter(|p| !p.is_empty()).collect()
}

/// Python `str.splitlines()` (without the line endings).
pub fn splitlines(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            '\n' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}'
            | '\u{2029}' => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn cased(c: char) -> bool {
    c.is_uppercase() || c.is_lowercase()
}

/// Python `str.isupper()`: at least one cased character and all cased ones upper case.
pub fn is_upper(s: &str) -> bool {
    s.chars().any(cased) && !s.chars().any(|c| c.is_lowercase())
}

/// Python `str.islower()`.
pub fn is_lower(s: &str) -> bool {
    s.chars().any(cased) && !s.chars().any(|c| c.is_uppercase())
}

/// Python `str.isalnum()` for one character.
pub fn is_alnum(c: char) -> bool {
    c.is_alphanumeric()
}

/// Python `str.upper()`.
pub fn upper(s: &str) -> String {
    s.to_uppercase()
}

/// Python `text.rfind(needle, 0, end)` on code points; None when not found.
pub fn rfind_before(chars: &[char], needle: char, end: usize) -> Option<usize> {
    let end = end.min(chars.len());
    chars[..end].iter().rposition(|&c| c == needle)
}

/// Python `str.ljust(n, fill)`.
pub fn ljust(s: &str, n: usize, fill: char) -> String {
    let mut out = s.to_string();
    let len = clen(s);
    for _ in len..n {
        out.push(fill);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(clen("a🎵b"), 3);
        assert_eq!(ulen("a🎵b"), 4);
        assert_eq!(slice("héllo", 1, 3), "él");
    }

    #[test]
    fn splitting() {
        assert_eq!(split_ws("  a  b\tc\n"), vec!["a", "b", "c"]);
        assert_eq!(splitlines("a\r\nb\rc\nd"), vec!["a", "b", "c", "d"]);
        assert_eq!(splitlines("a\n"), vec!["a"]);
        assert_eq!(splitlines("a\n\nb"), vec!["a", "", "b"]);
    }

    #[test]
    fn case() {
        assert!(is_upper("ABC1"));
        assert!(!is_upper("123"));
        assert!(!is_upper("AbC"));
        assert!(is_lower("abc"));
    }
}
