//! Translations. The function `t` takes a key and returns the text in the current language.
//!
//! `i18n_en.rs` is the reference (and the default language); every other language lives in its own
//! `i18n_xx.rs` file (key → text) and falls back to English for a missing key.
//! Tests check that every key used in the code exists and that each table is complete.

use crate::i18n_en::T as EN;
use crate::prefs::Lang;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

static LANG: AtomicU8 = AtomicU8::new(0);

/// Translations (everything except English), in the order of `Lang::index` starting at 1.
fn extra_tables() -> [&'static [(&'static str, &'static str)]; 6] {
    [crate::i18n_fr::T, crate::i18n_es::T, crate::i18n_de::T, crate::i18n_it::T, crate::i18n_pt::T, crate::i18n_ru::T]
}

/// Sets the language; `Lang::Auto` follows the language of the computer.
pub fn set_lang(l: Lang) {
    LANG.store(l.resolve().index(), Ordering::Relaxed);
}

/// The language currently in use (never `Auto`).
pub fn lang() -> Lang {
    Lang::from_index(LANG.load(Ordering::Relaxed))
}

fn english() -> &'static HashMap<&'static str, &'static str> {
    static M: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    M.get_or_init(|| EN.iter().copied().collect())
}

fn extra(i: usize) -> &'static HashMap<&'static str, &'static str> {
    static M: OnceLock<Vec<HashMap<&'static str, &'static str>>> = OnceLock::new();
    &M.get_or_init(|| extra_tables().iter().map(|t| t.iter().copied().collect()).collect())[i]
}

/// Translated text; an unknown key is returned as is.
pub fn t(key: &'static str) -> &'static str {
    let Some(en) = english().get(key).copied() else { return key };
    let l = lang();
    match l {
        Lang::En | Lang::Auto => en,
        _ => extra(l.index() as usize - 1).get(key).copied().unwrap_or(en),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_duplicate_keys() {
        let mut seen = std::collections::HashSet::new();
        for (k, _) in EN {
            assert!(seen.insert(*k), "duplicate key: {k}");
        }
    }

    #[test]
    fn all_used_keys_exist() {
        let mut missing = vec![];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.extension().map(|e| e != "rs").unwrap_or(true) {
                continue;
            }
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if name.starts_with("i18n_") || name == "icons.rs" {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            let b = src.as_bytes();
            let mut i = 0;
            while let Some(pos) = src[i..].find("t(\"") {
                let start = i + pos;
                let ok_prefix = start == 0 || !(b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'_');
                let key_start = start + 3;
                if let Some(end) = src[key_start..].find('"') {
                    let key = &src[key_start..key_start + end];
                    if ok_prefix
                        && !key.is_empty()
                        && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
                        && src[key_start + end..].starts_with("\")")
                        && !EN.iter().any(|(k, _)| *k == key)
                    {
                        missing.push(format!("{name}: {key}"));
                    }
                    i = key_start + end;
                } else {
                    break;
                }
            }
        }
        assert!(missing.is_empty(), "missing translation keys: {missing:?}");
    }

    #[test]
    fn language_switch() {
        set_lang(Lang::En);
        assert_eq!(t("cmd_undo"), "Undo");
        set_lang(Lang::Fr);
        assert_eq!(t("cmd_undo"), "Annuler");
        assert_eq!(t("unknown key"), "unknown key");
        set_lang(Lang::En);
    }

    #[test]
    fn extra_languages_are_complete_and_clean() {
        for (n, table) in extra_tables().iter().enumerate() {
            let code = Lang::from_index(n as u8 + 1).code();
            let mut seen = std::collections::HashSet::new();
            for (k, v) in table.iter() {
                assert!(seen.insert(*k), "{code}: duplicate key {k}");
                assert!(EN.iter().any(|(tk, _)| tk == k), "{code}: unknown key {k}");
                assert!(!v.trim().is_empty(), "{code}: empty text for {k}");
            }
            let missing: Vec<&str> = EN.iter().map(|(k, _)| *k).filter(|k| !seen.contains(k)).collect();
            assert!(missing.is_empty(), "{code}: missing keys {missing:?}");
        }
    }

    #[test]
    fn placeholders_are_preserved() {
        for (k, en) in EN {
            for ph in ["{n}", "{N}", "{date}", "{title}", "{file}"] {
                if en.contains(ph) {
                    for (n, table) in extra_tables().iter().enumerate() {
                        if let Some((_, v)) = table.iter().find(|(tk, _)| tk == k) {
                            assert!(v.contains(ph), "{}: {k} lost {ph}", Lang::from_index(n as u8 + 1).code());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn locale_detection() {
        assert_eq!(Lang::from_locale("fr-FR"), Lang::Fr);
        assert_eq!(Lang::from_locale("es_MX"), Lang::Es);
        assert_eq!(Lang::from_locale("pt-BR"), Lang::Pt);
        assert_eq!(Lang::from_locale("de"), Lang::De);
        assert_eq!(Lang::from_locale("ja-JP"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
    }
}
