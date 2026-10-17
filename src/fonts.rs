//! Fonts: catalog of the system fonts (TrueType) and matching with the fonts of a PDF.

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Builtin {
    Helvetica,
    Times,
    Courier,
}

/// Font family chosen for a text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FontFamily {
    /// Keep the document's font.
    Original,
    Builtin(Builtin),
    /// System font (family name): it will be embedded in the PDF.
    System(String),
}

impl FontFamily {
    pub fn label(&self) -> String {
        match self {
            FontFamily::Original => crate::i18n::t("font_original").to_string(),
            FontFamily::Builtin(Builtin::Helvetica) => "Helvetica".into(),
            FontFamily::Builtin(Builtin::Times) => "Times".into(),
            FontFamily::Builtin(Builtin::Courier) => "Courier".into(),
            FontFamily::System(s) => s.clone(),
        }
    }
}

/// Family, bold and italic deduced from the PostScript name of a PDF font
/// (ex. `ABCDEF+Calibri-BoldItalic` → `("Calibri", true, true)`).
pub fn parse_pdf_font_name(name: &str) -> (String, bool, bool) {
    let n = strip_subset_prefix(name);
    let lower = n.to_lowercase();
    let bold = lower.contains("bold") || lower.contains("black") || lower.contains("heavy") || lower.contains("semibold");
    let italic = lower.contains("italic") || lower.contains("oblique");
    // family = before the first hyphen or comma
    let fam = n
        .split(|c| c == '-' || c == ',')
        .next()
        .unwrap_or(n)
        .trim_end_matches("MT")
        .trim_end_matches("PS")
        .to_string();
    (insert_spaces(&fam), bold, italic)
}

/// `CourierNew` → `Courier New`, `TimesNewRoman` → `Times New Roman`.
fn insert_spaces(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_lowercase();
        out.push(c);
    }
    out
}

/// Removes the subset prefix `ABCDEF+`.
pub fn strip_subset_prefix(name: &str) -> &str {
    let b = name.as_bytes();
    if b.len() > 7 && b[6] == b'+' && b[..6].iter().all(|c| c.is_ascii_uppercase()) {
        &name[7..]
    } else {
        name
    }
}

pub fn is_subset_name(name: &str) -> bool {
    strip_subset_prefix(name).len() != name.len()
}

pub struct Catalog {
    db: fontdb::Database,
    /// Usable TrueType families, sorted.
    pub families: Vec<String>,
}

fn is_plain_truetype(data: &[u8]) -> bool {
    data.len() > 4 && (data[..4] == [0, 1, 0, 0] || &data[..4] == b"true")
}

impl Catalog {
    fn load() -> Catalog {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut fams: Vec<String> = vec![];
        for f in db.faces() {
            if let Some((name, _)) = f.families.first() {
                if f.index == 0 && !fams.contains(name) {
                    fams.push(name.clone());
                }
            }
        }
        fams.sort_by_key(|s| s.to_lowercase());
        Catalog { db, families: fams }
    }

    /// TTF bytes of a system font, if it exists as simple TrueType (not .ttc, not CFF).
    pub fn find(&self, family: &str, bold: bool, italic: bool) -> Option<Vec<u8>> {
        let q = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
            stretch: fontdb::Stretch::Normal,
            style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
        };
        let id = self.db.query(&q)?;
        let info = self.db.face(id)?;
        if info.index != 0 {
            return None;
        }
        let fam_ok = info.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(family));
        if !fam_ok {
            return None;
        }
        self.db.with_face_data(id, |data, _| if is_plain_truetype(data) { Some(data.to_vec()) } else { None }).flatten()
    }

    pub fn has_family(&self, family: &str) -> bool {
        self.families.iter().any(|f| f.eq_ignore_ascii_case(family))
    }
}

pub fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(Catalog::load)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pdf_font_names() {
        assert_eq!(parse_pdf_font_name("ABCDEF+Calibri-BoldItalic"), ("Calibri".into(), true, true));
        assert_eq!(parse_pdf_font_name("Arial-BoldMT"), ("Arial".into(), true, false));
        assert_eq!(parse_pdf_font_name("TimesNewRomanPS-ItalicMT"), ("Times New Roman".into(), false, true));
        assert_eq!(parse_pdf_font_name("Helvetica"), ("Helvetica".into(), false, false));
        assert!(is_subset_name("ABCDEF+Calibri"));
        assert!(!is_subset_name("Calibri"));
        assert_eq!(strip_subset_prefix("QWERTY+Foo"), "Foo");
    }

    #[test]
    fn system_catalog_finds_common_fonts() {
        let c = catalog();
        assert!(!c.families.is_empty(), "no system font found");
        // Arial exists on every Windows
        if c.has_family("Arial") {
            let regular = c.find("Arial", false, false).expect("Arial normal");
            let bold = c.find("Arial", true, false).expect("Arial bold");
            assert!(is_plain_truetype(&regular) && is_plain_truetype(&bold));
            assert_ne!(regular.len(), bold.len());
        }
    }
}
