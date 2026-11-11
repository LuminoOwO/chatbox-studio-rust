//! Formatting helpers and the display modes of chatbox blocks (ticker, marquee, bounce,
//! lines, pages, typewriter) - a line-by-line port of chatbox.py, with Python's string
//! semantics (lengths and slices in code points).

use crate::jv::{Obj, ObjExt};
use crate::pystr::{clen, head, rstrip, slice, split_ws, splitlines, strip, ulen};

/// VRChat chatbox length: 144 UTF-16 units (an emoji counts 2).
pub const LIMIT: usize = 144;

pub const SAMPLE_BASE: &str = "This sample shows how much text fits in your chatbox at once. \
Every word here counts toward the limit, so you can see the real size. ";

/// Exactly n characters of readable sample text.
pub fn sample_text(n: usize) -> String {
    let n = n.clamp(1, LIMIT);
    let body = format!("[{n} characters] {}", SAMPLE_BASE.repeat(3));
    let body = crate::pystr::ljust(&head(&body, n), n, '.');
    if body.ends_with(' ') {
        format!("{}.", head(&body, clen(&body) - 1))
    } else {
        body
    }
}

/// `{icon} {text}` (just the text when either is empty).
pub fn with_icon(icon: &str, text: &str) -> String {
    let icon = strip(icon);
    if !icon.is_empty() && !text.is_empty() {
        format!("{icon} {text}")
    } else {
        text.to_string()
    }
}

/// "2d 3h", "1h 5m", "4m 2s", "<1m".
pub fn fmt_duration(seconds: f64, with_seconds: bool) -> String {
    let mut s = seconds.max(0.0) as i64;
    let d = s / 86400;
    s %= 86400;
    let h = s / 3600;
    s %= 3600;
    let m = s / 60;
    s %= 60;
    if d > 0 {
        return format!("{d}d {h}h");
    }
    if h > 0 {
        return format!("{h}h {m}m");
    }
    if with_seconds {
        return if m > 0 { format!("{m}m {s}s") } else { format!("{s}s") };
    }
    if m > 0 { format!("{m}m") } else { "<1m".into() }
}

/// "1:23" / "1:02:03".
pub fn fmt_clock(seconds: Option<f64>) -> String {
    let Some(sec) = seconds else { return String::new() };
    let mut s = sec.max(0.0) as i64;
    let h = s / 3600;
    s %= 3600;
    let m = s / 60;
    s %= 60;
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// "━━━●──────"
pub fn progress_bar(pos: Option<f64>, dur: Option<f64>, width: usize) -> String {
    let (Some(pos), Some(dur)) = (pos, dur) else { return String::new() };
    if dur == 0.0 {
        return String::new();
    }
    let k = ((pos / dur * width as f64) as i64).clamp(0, width as i64 - 1) as usize;
    format!("{}●{}", "━".repeat(k), "─".repeat(width - 1 - k))
}

/// Cuts to n characters with "…".
pub fn truncate(text: &str, n: usize) -> String {
    if n > 0 && clen(text) > n {
        format!("{}…", rstrip(&head(text, n.saturating_sub(1).max(1))))
    } else {
        text.to_string()
    }
}

/// Cuts to n UTF-16 units without splitting a character (VRChat would).
pub fn truncate16(text: &str, n: usize) -> String {
    if ulen(text) <= n {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = if (ch as u32) > 0xFFFF { 2 } else { 1 };
        if used + w > n.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    format!("{}…", rstrip(&out))
}

/// Splits text into pages of at most `width` characters, at word boundaries.
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(10);
    let mut pages: Vec<String> = Vec::new();
    let mut cur = String::new();
    for raw_line in text.split('\n') {
        let mut line_start = true;
        for w0 in split_ws(raw_line) {
            let mut w = w0.to_string();
            while clen(&w) > width {
                if !cur.is_empty() {
                    pages.push(std::mem::take(&mut cur));
                }
                pages.push(head(&w, width));
                w = crate::pystr::tail(&w, width);
            }
            let sep = if cur.is_empty() { "" } else if line_start { "\n" } else { " " };
            if clen(&cur) + clen(sep) + clen(&w) <= width {
                cur.push_str(sep);
                cur.push_str(&w);
            } else {
                pages.push(std::mem::replace(&mut cur, w));
            }
            line_start = false;
        }
    }
    if !cur.is_empty() {
        pages.push(cur);
    }
    if pages.is_empty() {
        pages.push(String::new());
    }
    pages
}

/// Sliding windows over the words: each next window starts `step` words later (speech
/// "Scroll").
pub fn scroll_windows(text: &str, width: usize, step: usize) -> Vec<String> {
    let replaced = text.replace('\n', " \n ");
    let words: Vec<&str> = replaced.split(' ').filter(|w| !w.is_empty()).collect();
    let mut out = Vec::new();
    let step = step.max(1);
    let mut i = 0;
    while i < words.len() {
        let mut cur = String::new();
        let mut j = i;
        while j < words.len() {
            let w = words[j];
            let add = if cur.is_empty() || cur.ends_with('\n') || w == "\n" {
                w.to_string()
            } else {
                format!(" {w}")
            };
            if clen(&cur) + clen(&add) > width && !cur.is_empty() {
                break;
            }
            cur.push_str(&add);
            j += 1;
        }
        out.push(strip(&cur.replace(" \n", "\n").replace("\n ", "\n")).to_string());
        if j >= words.len() {
            break;
        }
        i += step;
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Lines joined with " · " (blank lines dropped).
fn flat(text: &str) -> String {
    splitlines(text)
        .iter()
        .map(|l| strip(l))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A ticker: a window of `width` characters moving along `step` words at a time and
/// starting over at the end.
pub fn ticker_frames(text: &str, width: usize, step: usize) -> Vec<String> {
    let flat = flat(text);
    if clen(&flat) <= width {
        return vec![flat];
    }
    let mut words: Vec<String> = Vec::new();
    for w in flat.split(' ').chain(std::iter::once("•")) {
        if ["·", "•", "|", "-", "—", "/"].contains(&w) && !words.is_empty() {
            let last = words.last_mut().unwrap();
            last.push(' ');
            last.push_str(w);
        } else if !w.is_empty() {
            words.push(w.to_string());
        }
    }
    let n = words.len();
    let mut out = Vec::new();
    let mut start = 0;
    while start < n {
        let mut cur = String::new();
        let mut k = 0;
        while k < n {
            let w = &words[(start + k) % n];
            let add = if cur.is_empty() { w.clone() } else { format!(" {w}") };
            if clen(&cur) + clen(&add) > width {
                if cur.is_empty() {
                    cur = head(w, width);
                }
                break;
            }
            cur.push_str(&add);
            k += 1;
        }
        out.push(cur);
        start += step.max(1);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Letter by letter, around and around like a LED sign.
pub fn marquee_frames(text: &str, width: usize, step: usize) -> Vec<String> {
    let flat = flat(text);
    if clen(&flat) <= width {
        return vec![flat];
    }
    let lp = format!("{flat}   •   ");
    let twice: Vec<char> = lp.repeat(2).chars().collect();
    let n = clen(&lp);
    (0..n)
        .step_by(step.max(1))
        .map(|i| twice[i..(i + width).min(twice.len())].iter().collect())
        .collect()
}

/// Slides to the end, then back to the start.
pub fn bounce_frames(text: &str, width: usize, step: usize) -> Vec<String> {
    let flat = flat(text);
    let len = clen(&flat);
    if len <= width {
        return vec![flat];
    }
    let last = len - width;
    let mut pos: Vec<usize> = (0..last).step_by(step.max(1)).collect();
    pos.push(last);
    if pos.len() == 1 {
        pos.push(pos[0]);
    } else {
        // pos + pos[-2:0:-1]
        let back: Vec<usize> = pos[1..pos.len() - 1].iter().rev().copied().collect();
        pos.extend(back);
    }
    pos.iter().map(|&p| slice(&flat, p, p + width)).collect()
}

/// Like credits: `lines` lines at a time, moving up one line per step.
pub fn line_frames(text: &str, width: usize, lines: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for line in splitlines(text) {
        let t = strip(&line);
        if !t.is_empty() {
            if clen(&line) > width {
                rows.extend(wrap_words(t, width));
            } else {
                rows.push(t.to_string());
            }
        }
    }
    let lines = lines.max(1);
    if rows.len() <= lines {
        return vec![rows.join("\n")];
    }
    let mut lp = rows.clone();
    lp.extend(rows[..lines - 1].iter().cloned());
    (0..rows.len()).map(|i| lp[i..i + lines].join("\n")).collect()
}

/// Words appear a few at a time; a full window stays for one more step, then the next.
pub fn typewriter_frames(text: &str, width: usize, step: usize) -> Vec<String> {
    let step = step.max(1);
    let mut out: Vec<String> = Vec::new();
    let join = |toks: &[String]| -> String {
        let mut t = String::new();
        for tok in toks {
            if tok == "\n" || t.is_empty() || t.ends_with('\n') {
                t.push_str(tok);
            } else {
                t.push(' ');
                t.push_str(tok);
            }
        }
        strip(&t).to_string()
    };
    for page in wrap_words(text, width) {
        let toks = tokens(&page);
        let idx: Vec<usize> =
            toks.iter().enumerate().filter(|(_, t)| t.as_str() != "\n").map(|(i, _)| i).collect();
        let mut k = step;
        while k < idx.len() + step {
            let upto = idx[k.min(idx.len()) - 1] + 1;
            out.push(join(&toks[..upto]));
            k += step;
        }
        if let Some(last) = out.last().cloned() {
            out.push(last);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// `re.findall(r"\S+|\n", text)`
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c == '\n' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push("\n".into());
        } else if crate::pystr::is_space(c) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn frames_for(mode: &str, text: &str, width: usize, step: usize, lines: usize) -> Vec<String> {
    match mode {
        "scroll" => ticker_frames(text, width, step),
        "marquee" => marquee_frames(text, width, step),
        "bounce" => bounce_frames(text, width, step),
        "lines" => line_frames(text, width, lines),
        "typewriter" => typewriter_frames(text, width, step),
        _ => wrap_words(text, width),
    }
}

/// Applies a block's window setting (ticker, marquee, pages, ...) to its text.
pub fn window_text(text: &str, el: &Obj, now_wall: f64, min_interval: f64) -> String {
    let mode = el.s("show_mode", "full");
    if mode == "full" || text.is_empty() {
        return text.to_string();
    }
    let width = el.i_or("win_chars", 40).clamp(5, LIMIT as i64) as usize;
    let secs = min_interval.max(el.f_or("win_seconds", 2.0));
    let frames = frames_for(
        &mode,
        text,
        width,
        el.i_or("win_step", 1).max(0) as usize,
        el.i_or("win_lines", 1).max(0) as usize,
    );
    let idx = ((now_wall / secs).floor() as i64).rem_euclid(frames.len() as i64) as usize;
    frames[idx].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(30.0, false), "<1m");
        assert_eq!(fmt_duration(30.0, true), "30s");
        assert_eq!(fmt_duration(3700.0, false), "1h 1m");
        assert_eq!(fmt_duration(90000.0, false), "1d 1h");
        assert_eq!(fmt_clock(Some(83.0)), "1:23");
        assert_eq!(fmt_clock(Some(3723.0)), "1:02:03");
        assert_eq!(progress_bar(Some(50.0), Some(100.0), 10), "━━━━━●────");
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate16(&"🎵".repeat(80), 144).encode_utf16().count(), 143);
    }

    #[test]
    fn wrapping() {
        assert_eq!(wrap_words("aaa bbb ccc ddd", 10), vec!["aaa bbb", "ccc ddd"]);
        assert_eq!(wrap_words("", 10), vec![""]);
    }
}
