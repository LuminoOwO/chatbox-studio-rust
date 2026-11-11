//! Languages the app offers: Whisper code, display name, NLLB-200 (FLORES) code.
//! English first (the default), then all others alphabetically - no language is preferred.

use std::sync::OnceLock;

/// The route target "room": Earshot's room language.
pub const ROOM: &str = "room";

const TABLE: &[(&str, &str, &str)] = &[
    ("en", "English", "eng_Latn"),
    ("de", "German", "deu_Latn"),
    ("fr", "French", "fra_Latn"),
    ("es", "Spanish", "spa_Latn"),
    ("it", "Italian", "ita_Latn"),
    ("pt", "Portuguese", "por_Latn"),
    ("nl", "Dutch", "nld_Latn"),
    ("pl", "Polish", "pol_Latn"),
    ("cs", "Czech", "ces_Latn"),
    ("sv", "Swedish", "swe_Latn"),
    ("da", "Danish", "dan_Latn"),
    ("no", "Norwegian", "nob_Latn"),
    ("fi", "Finnish", "fin_Latn"),
    ("hu", "Hungarian", "hun_Latn"),
    ("ro", "Romanian", "ron_Latn"),
    ("hr", "Croatian", "hrv_Latn"),
    ("bg", "Bulgarian", "bul_Cyrl"),
    ("el", "Greek", "ell_Grek"),
    ("ru", "Russian", "rus_Cyrl"),
    ("uk", "Ukrainian", "ukr_Cyrl"),
    ("tr", "Turkish", "tur_Latn"),
    ("ar", "Arabic", "arb_Arab"),
    ("he", "Hebrew", "heb_Hebr"),
    ("hi", "Hindi", "hin_Deva"),
    ("ja", "Japanese", "jpn_Jpan"),
    ("ko", "Korean", "kor_Hang"),
    ("zh", "Chinese (Simplified)", "zho_Hans"),
    ("th", "Thai", "tha_Thai"),
    ("vi", "Vietnamese", "vie_Latn"),
    ("id", "Indonesian", "ind_Latn"),
    ("ms", "Malay", "zsm_Latn"),
    ("tl", "Filipino", "tgl_Latn"),
    ("ca", "Catalan", "cat_Latn"),
    ("sk", "Slovak", "slk_Latn"),
    ("sl", "Slovenian", "slv_Latn"),
    ("sr", "Serbian", "srp_Cyrl"),
    ("lt", "Lithuanian", "lit_Latn"),
    ("lv", "Latvian", "lvs_Latn"),
    ("et", "Estonian", "est_Latn"),
    ("is", "Icelandic", "isl_Latn"),
    ("cy", "Welsh", "cym_Latn"),
    ("af", "Afrikaans", "afr_Latn"),
    ("fa", "Persian", "pes_Arab"),
    ("ur", "Urdu", "urd_Arab"),
    ("bn", "Bengali", "ben_Beng"),
    ("ta", "Tamil", "tam_Taml"),
    ("sw", "Swahili", "swh_Latn"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    pub flores: &'static str,
}

/// All languages: English first, then by name.
pub fn all() -> &'static [Language] {
    static LIST: OnceLock<Vec<Language>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut v: Vec<Language> = TABLE
            .iter()
            .map(|&(code, name, flores)| Language { code, name, flores })
            .collect();
        v[1..].sort_by(|a, b| a.name.cmp(b.name));
        v
    })
}

pub fn get(code: &str) -> Option<&'static Language> {
    all().iter().find(|l| l.code == code)
}

pub fn is_known(code: &str) -> bool {
    get(code).is_some()
}

/// Display name; "the room language" for `room`, the code itself when unknown.
pub fn name(code: &str) -> String {
    if code == ROOM {
        return "the room language".into();
    }
    get(code).map(|l| l.name.to_string()).unwrap_or_else(|| code.to_string())
}

/// NLLB-200 (FLORES) code.
pub fn nllb(code: &str) -> Option<&'static str> {
    get(code).map(|l| l.flores)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_is_english_then_alphabetical() {
        let l = all();
        assert_eq!(l.len(), 47);
        assert_eq!(l[0].code, "en");
        assert_eq!(l[1].name, "Afrikaans");
        assert!(l[1..].windows(2).all(|w| w[0].name <= w[1].name));
        assert_eq!(name("room"), "the room language");
        assert_eq!(nllb("zh"), Some("zho_Hans"));
    }
}
