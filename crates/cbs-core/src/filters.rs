//! Text filters applied before a sentence goes to the VRChat chatbox:
//!
//! * filler words - drops "um", "uh", "äh", ... that Whisper sometimes writes out
//! * replacements - find → replace, plain text or regex, whole word or anywhere
//! * block rules  - a sentence containing one of these is not sent at all
//! * min words    - ignore very short sentences ("Okay.", "Hm.")
//!
//! Regex rules use Python's syntax (lookarounds included) and Python-style replacements
//! (`\1`, `\g<name>`), as typed in the Python version.

use std::sync::LazyLock;

use fancy_regex::{Captures, Regex};
use serde_json::Value;

use crate::jv::{to_str, ObjExt};
use crate::pystr::{head, is_alnum, is_lower, is_upper, strip_chars, tail};
use crate::settings::Settings;

pub const FILLERS: &[&str] = &[
    "um", "umm", "uh", "uhh", "uhm", "erm", "er", "hmm", "hm", "mhm", "äh", "ähm", "öhm", "ehm",
];

/// Case-insensitive like the Python version's `re.I`. The flag goes inline: fancy-regex
/// loses the builder's flag inside lookarounds.
fn build(pattern: &str) -> Result<Regex, String> {
    Regex::new(&format!("(?i:{pattern})")).map_err(|e| e.to_string())
}

static FILLER_RE: LazyLock<Regex> = LazyLock::new(|| {
    build(&format!(r"(?<!\w)(?:{})(?!\w)[,.…]*\s*", FILLERS.join("|"))).unwrap()
});
static SPACE_BEFORE_PUNCT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+([,.!?;:])").unwrap());
static REPEATED_PUNCT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([,;:])(?:\s*[,;:])+").unwrap());
static MULTI_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w+").unwrap());

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FilterOutcome {
    pub text: String,
    pub blocked: bool,
    pub reason: String,
    pub changed: bool,
}

impl FilterOutcome {
    fn pass(text: String) -> Self {
        FilterOutcome { text, ..Default::default() }
    }
    fn block(text: String, reason: String) -> Self {
        FilterOutcome { text, blocked: true, reason, changed: false }
    }
}

#[derive(Debug, Clone)]
struct Replace {
    pattern: Regex,
    replacement: String,
    regex: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FilterSet {
    pub enabled: bool,
    pub remove_fillers: bool,
    pub min_words: i64,
    replace: Vec<Replace>,
    block: Vec<(Regex, String)>,
    /// Rules that couldn't be compiled, for the log.
    pub errors: Vec<String>,
}

/// Escapes text for a regex like Python's `re.escape`.
fn escape(text: &str) -> String {
    fancy_regex::escape(text).into_owned()
}

/// A rule as a regex: plain text is escaped; whole word adds `(?<!\w)…(?!\w)`.
pub fn compile_rule(text: &str, whole_word: bool, regex: bool) -> Result<Regex, String> {
    if regex {
        build(text)?; // report errors against the pattern as typed
    }
    let mut body = if regex { text.to_string() } else { escape(text.trim()) };
    if whole_word {
        body = format!(r"(?<!\w)(?:{body})(?!\w)");
    }
    build(&body)
}

/// Python replacement syntax → fancy-regex/regex syntax: `\1` and `\g<1>` / `\g<name>`
/// become `${1}` / `${name}`, `\n` `\t` `\\` are escapes, `$` is literal.
pub fn python_replacement(repl: &str) -> String {
    let chars: Vec<char> = repl.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' {
            out.push_str("$$");
            i += 1;
            continue;
        }
        if c != '\\' || i + 1 >= chars.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let n = chars[i + 1];
        if n.is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && j < i + 3 && chars[j].is_ascii_digit() {
                j += 1;
            }
            let num: String = chars[i + 1..j].iter().collect();
            out.push_str(&format!("${{{num}}}"));
            i = j;
            continue;
        }
        if n == 'g' && chars.get(i + 2) == Some(&'<') {
            if let Some(end) = chars[i + 3..].iter().position(|&x| x == '>') {
                let name: String = chars[i + 3..i + 3 + end].iter().collect();
                out.push_str(&format!("${{{name}}}"));
                i += end + 4;
                continue;
            }
        }
        match n {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '\\' => out.push('\\'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
        i += 2;
    }
    out
}

/// Plain replacements keep the capitalisation of what they replace.
fn keep_case(found: &str, repl: &str) -> String {
    if is_upper(found) && found.chars().count() > 1 {
        return repl.to_uppercase();
    }
    let f1 = head(found, 1);
    let r1 = head(repl, 1);
    if is_upper(&f1) && is_lower(&r1) {
        return format!("{}{}", r1.to_uppercase(), tail(repl, 1));
    }
    repl.to_string()
}

fn rule_bool(r: &serde_json::Map<String, Value>, k: &str, default: bool) -> bool {
    r.b(k, default)
}

impl FilterSet {
    pub fn from_settings(s: &Settings) -> FilterSet {
        let mut fs = FilterSet {
            enabled: s.filters_enabled,
            remove_fillers: s.remove_fillers,
            min_words: s.min_words,
            ..Default::default()
        };
        for (i, r) in s.replace_rules.iter().enumerate() {
            let Some(r) = r.as_object() else { continue };
            let find = r.get("find").map(to_str).unwrap_or_default();
            if !rule_bool(r, "enabled", true) || find.trim().is_empty() {
                continue;
            }
            let regex = rule_bool(r, "regex", false);
            match compile_rule(&find, rule_bool(r, "whole_word", true), regex) {
                Ok(pattern) => fs.replace.push(Replace {
                    pattern,
                    replacement: r.get("replace").map(to_str).unwrap_or_default(),
                    regex,
                }),
                Err(e) => fs.errors.push(format!("Replace rule {} ('{find}'): {e}", i + 1)),
            }
        }
        for (i, r) in s.block_rules.iter().enumerate() {
            let Some(r) = r.as_object() else { continue };
            let pat = r.get("pattern").map(to_str).unwrap_or_default();
            if !rule_bool(r, "enabled", true) || pat.trim().is_empty() {
                continue;
            }
            match compile_rule(&pat, rule_bool(r, "whole_word", true), rule_bool(r, "regex", false)) {
                Ok(re) => fs.block.push((re, pat)),
                Err(e) => fs.errors.push(format!("Block rule {} ('{pat}'): {e}", i + 1)),
            }
        }
        fs
    }

    fn blocked_by(&self, text: &str) -> Option<String> {
        self.block
            .iter()
            .find(|(re, _)| re.is_match(text).unwrap_or(false))
            .map(|(_, label)| format!("contains “{label}”"))
    }

    pub fn apply(&self, text: &str) -> FilterOutcome {
        if !self.enabled {
            return FilterOutcome::pass(text.to_string());
        }
        let original = text;
        if let Some(why) = self.blocked_by(text) {
            return FilterOutcome::block(text.to_string(), why);
        }
        let mut text = text.to_string();
        if self.remove_fillers {
            text = FILLER_RE.replace_all(&text, "").into_owned();
        }
        for r in &self.replace {
            text = if r.regex {
                r.pattern.replace_all(&text, python_replacement(&r.replacement).as_str()).into_owned()
            } else {
                let repl = r.replacement.clone();
                r.pattern
                    .replace_all(&text, |c: &Captures| keep_case(&c[0], &repl))
                    .into_owned()
            };
        }
        // tidy up spacing / punctuation left behind by removals
        text = SPACE_BEFORE_PUNCT.replace_all(&text, "$1").into_owned();
        text = REPEATED_PUNCT.replace_all(&text, "$1").into_owned();
        text = MULTI_SPACE.replace_all(&text, " ").into_owned();
        text = strip_chars(&text, " ,;:").to_string();
        if is_lower(&head(&text, 1)) && is_upper(&head(original, 1)) {
            text = format!("{}{}", head(&text, 1).to_uppercase(), tail(&text, 1));
        }
        // a replacement could have produced a blocked word
        if let Some(why) = self.blocked_by(&text) {
            return FilterOutcome::block(text, why);
        }
        if !text.chars().any(is_alnum) {
            return FilterOutcome::block(text, "nothing left after filtering".into());
        }
        if self.min_words > 0 && (WORD.find_iter(&text).count() as i64) < self.min_words {
            return FilterOutcome::block(text, format!("shorter than {} words", self.min_words));
        }
        let changed = text != original;
        FilterOutcome { text, changed, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(rep: Value, block: Value) -> FilterSet {
        let mut s = Settings::default();
        s.replace_rules = rep.as_array().unwrap().clone();
        s.block_rules = block.as_array().unwrap().clone();
        FilterSet::from_settings(&s)
    }

    #[test]
    fn fillers_and_tidy() {
        let fs = set(json!([]), json!([]));
        assert_eq!(fs.apply("Um, so, uh, I think so.").text, "So, I think so.");
    }

    #[test]
    fn keep_case_and_regex() {
        let fs = set(json!([
            {"find": "vr chat", "replace": "VRChat"},
            {"find": "colour", "replace": "color"},
            {"find": r"(\d+) euros?", "replace": r"€\1", "regex": true, "whole_word": false}
        ]), json!([]));
        assert_eq!(fs.apply("vr chat is fun").text, "VRChat is fun");
        assert_eq!(fs.apply("Colour and COLOUR").text, "Color and COLOR");
        assert_eq!(fs.apply("It costs 5 euros").text, "It costs €5");
    }

    #[test]
    fn blocking_and_min_words() {
        let fs = set(json!([]), json!([{"pattern": "secret"}]));
        let o = fs.apply("this is secret stuff");
        assert!(o.blocked);
        assert_eq!(o.reason, "contains “secret”");
        let mut s = Settings::default();
        s.min_words = 3;
        let fs = FilterSet::from_settings(&s);
        assert!(fs.apply("Okay.").blocked);
        assert!(fs.apply("um").blocked);
    }
}
