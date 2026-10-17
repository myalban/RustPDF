//! User preferences (JSON in the data folder) and list of recent files.

use crate::logx;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Interface language. `Auto` follows the language of the computer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum Lang {
    #[default]
    Auto,
    En,
    Fr,
    Es,
    De,
    It,
    Pt,
    Ru,
}

impl Lang {
    /// Selectable languages (after `Auto`).
    pub const ALL: [Lang; 7] = [Lang::En, Lang::Fr, Lang::Es, Lang::De, Lang::It, Lang::Pt, Lang::Ru];

    /// Name of the language in that language.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Auto => "Auto",
            Lang::En => "English",
            Lang::Fr => "Français",
            Lang::Es => "Español",
            Lang::De => "Deutsch",
            Lang::It => "Italiano",
            Lang::Pt => "Português",
            Lang::Ru => "Русский",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Auto => "auto",
            Lang::En => "en",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::De => "de",
            Lang::It => "it",
            Lang::Pt => "pt",
            Lang::Ru => "ru",
        }
    }

    /// Index used by the translation tables (0 = English, 1 = French, 2.. = extra languages).
    pub fn index(self) -> u8 {
        match self {
            Lang::Auto | Lang::En => 0,
            Lang::Fr => 1,
            Lang::Es => 2,
            Lang::De => 3,
            Lang::It => 4,
            Lang::Pt => 5,
            Lang::Ru => 6,
        }
    }

    pub fn from_index(i: u8) -> Lang {
        match i {
            1 => Lang::Fr,
            2 => Lang::Es,
            3 => Lang::De,
            4 => Lang::It,
            5 => Lang::Pt,
            6 => Lang::Ru,
            _ => Lang::En,
        }
    }

    /// Language for a locale name such as `fr-FR` or `pt_BR`; English when unsupported.
    pub fn from_locale(locale: &str) -> Lang {
        let code = locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        Lang::ALL.into_iter().find(|l| l.code() == code).unwrap_or(Lang::En)
    }

    /// The language of the computer (English when it cannot be determined or is unsupported).
    pub fn system() -> Lang {
        sys_locale::get_locale().map(|l| Lang::from_locale(&l)).unwrap_or(Lang::En)
    }

    /// Replaces `Auto` by the language of the computer.
    pub fn resolve(self) -> Lang {
        if self == Lang::Auto { Lang::system() } else { self }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum ThemePref {
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum Unit {
    Pt,
    #[default]
    Mm,
    Cm,
    In,
}

impl Unit {
    pub const ALL: [Unit; 4] = [Unit::Pt, Unit::Mm, Unit::Cm, Unit::In];

    pub fn label(self) -> &'static str {
        match self {
            Unit::Pt => "pt",
            Unit::Mm => "mm",
            Unit::Cm => "cm",
            Unit::In => "in",
        }
    }
    /// PDF points per unit.
    pub fn pt_per_unit(self) -> f32 {
        match self {
            Unit::Pt => 1.0,
            Unit::Mm => 72.0 / 25.4,
            Unit::Cm => 72.0 / 2.54,
            Unit::In => 72.0,
        }
    }
    pub fn from_pt(self, pt: f32) -> f32 {
        pt / self.pt_per_unit()
    }
    pub fn to_pt(self, v: f32) -> f32 {
        v * self.pt_per_unit()
    }
    pub fn format(self, pt: f32) -> String {
        let v = self.from_pt(pt);
        match self {
            Unit::Pt => format!("{v:.1} pt"),
            Unit::Mm => format!("{v:.1} mm"),
            Unit::Cm => format!("{v:.2} cm"),
            Unit::In => format!("{v:.2} in"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum ViewMode {
    Single,
    #[default]
    Continuous,
    TwoPages,
    TwoPagesContinuous,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub lang: Lang,
    pub theme: ThemePref,
    /// Inverts the page colors on screen (night reading).
    pub night_mode: bool,
    /// 0 = fit to width, otherwise factor (1.0 = 100 %).
    pub default_zoom: f32,
    pub view_mode: ViewMode,
    /// In two-page view: the first page alone (cover).
    pub cover_page: bool,
    pub export_dpi: f32,
    pub default_dir: String,
    pub check_updates: bool,
    pub update_url: String,
    /// 0 = disabled.
    pub autosave_secs: u32,
    pub units: Unit,
    pub show_rulers: bool,
    pub show_boxes: bool,
    pub show_fields: bool,
    pub undo_mem_mb: u32,
    pub cache_mb: u32,
    pub author: String,
    pub restore_view: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            lang: Lang::Auto,
            theme: ThemePref::Auto,
            night_mode: false,
            default_zoom: 0.0,
            view_mode: ViewMode::Continuous,
            cover_page: false,
            export_dpi: 300.0,
            default_dir: String::new(),
            check_updates: false,
            update_url: String::new(),
            autosave_secs: 120,
            units: Unit::Mm,
            show_rulers: false,
            show_boxes: true,
            show_fields: true,
            undo_mem_mb: 400,
            cache_mb: 512,
            author: std::env::var("USERNAME").unwrap_or_default(),
            restore_view: true,
        }
    }
}

fn prefs_path() -> PathBuf {
    logx::data_dir().join("prefs.json")
}

impl Prefs {
    pub fn load() -> Prefs {
        match std::fs::read_to_string(prefs_path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                logx::warn(format!("unreadable preferences ({e}), default values"));
                Prefs::default()
            }),
            Err(_) => Prefs::default(),
        }
    }

    pub fn save(&self) {
        match serde_json::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = std::fs::write(prefs_path(), s) {
                    logx::warn(format!("preferences not saved: {e}"));
                }
            }
            Err(e) => logx::warn(format!("preferences not serialized: {e}")),
        }
    }
}

// ----------------------------------------------------------------- recent

pub const MAX_RECENT: usize = 15;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecentEntry {
    pub path: String,
    /// Last page displayed (0-based).
    #[serde(default)]
    pub page: usize,
    /// Last zoom (0 = default value).
    #[serde(default)]
    pub zoom: f32,
    #[serde(default)]
    pub opened: u64,
}

impl RecentEntry {
    pub fn path_buf(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Recent {
    pub entries: Vec<RecentEntry>,
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// Path without the Windows `\\?\` prefix, for comparing and displaying.
pub fn clean_path(p: &Path) -> PathBuf {
    let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    PathBuf::from(c.to_string_lossy().trim_start_matches(r"\\?\").to_string())
}

fn recent_path() -> PathBuf {
    logx::data_dir().join("recent.json")
}

impl Recent {
    pub fn load() -> Recent {
        if let Ok(s) = std::fs::read_to_string(recent_path()) {
            if let Ok(r) = serde_json::from_str::<Recent>(&s) {
                return r;
            }
        }
        // Migration from the old text format (one path per line).
        let old = logx::data_dir().join("recent.txt");
        if let Ok(s) = std::fs::read_to_string(&old) {
            let entries = s
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .take(MAX_RECENT)
                .map(|l| RecentEntry { path: l.to_string(), page: 0, zoom: 0.0, opened: 0 })
                .collect();
            let r = Recent { entries };
            r.save();
            let _ = std::fs::remove_file(old);
            return r;
        }
        Recent::default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(recent_path(), s);
        }
    }

    pub fn add(&mut self, path: &Path) {
        let p = clean_path(path);
        let prev = self.get(&p).cloned();
        self.entries.retain(|e| !same_path(&e.path_buf(), &p));
        self.entries.insert(
            0,
            RecentEntry {
                path: p.to_string_lossy().into_owned(),
                page: prev.as_ref().map(|e| e.page).unwrap_or(0),
                zoom: prev.as_ref().map(|e| e.zoom).unwrap_or(0.0),
                opened: logx::unix_secs(),
            },
        );
        self.entries.truncate(MAX_RECENT);
        self.save();
    }

    pub fn get(&self, path: &Path) -> Option<&RecentEntry> {
        self.entries.iter().find(|e| same_path(&e.path_buf(), path))
    }

    pub fn remove(&mut self, path: &Path) {
        self.entries.retain(|e| !same_path(&e.path_buf(), path));
        self.save();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    /// Remembers the page and zoom of a file (without changing the order).
    pub fn remember_view(&mut self, path: &Path, page: usize, zoom: f32) {
        let p = clean_path(path);
        if let Some(e) = self.entries.iter_mut().find(|e| same_path(&e.path_buf(), &p)) {
            if e.page != page || (e.zoom - zoom).abs() > 0.001 {
                e.page = page;
                e.zoom = zoom;
                self.save_soon();
            }
        }
    }

    fn save_soon(&self) {
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_roundtrip() {
        for u in Unit::ALL {
            let pt = 123.4;
            assert!((u.to_pt(u.from_pt(pt)) - pt).abs() < 1e-3);
        }
        assert!((Unit::Mm.from_pt(72.0) - 25.4).abs() < 1e-3);
        assert_eq!(Unit::In.format(72.0), "1.00 in");
    }

    #[test]
    fn prefs_missing_fields_use_defaults() {
        let p: Prefs = serde_json::from_str(r#"{"lang":"En"}"#).unwrap();
        assert_eq!(p.lang, Lang::En);
        assert_eq!(p.cache_mb, 512);
        assert_eq!(p.view_mode, ViewMode::Continuous);
    }

    #[test]
    fn recent_dedupe_and_view_memory() {
        let mut r = Recent::default();
        let dir = std::env::temp_dir();
        let (a, b) = (dir.join("rp_a.pdf"), dir.join("rp_b.pdf"));
        // Work in memory: avoid writing to the real data folder.
        let push = |r: &mut Recent, p: &Path| {
            let p = clean_path(p);
            let prev = r.get(&p).cloned();
            r.entries.retain(|e| !same_path(&e.path_buf(), &p));
            r.entries.insert(0, RecentEntry { path: p.to_string_lossy().into(), page: prev.map(|e| e.page).unwrap_or(0), zoom: 0.0, opened: 1 });
        };
        push(&mut r, &a);
        push(&mut r, &b);
        r.entries[1].page = 7;
        push(&mut r, &a);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].page, 7, "the remembered page is kept");
        assert!(r.entries[1].path.ends_with("rp_b.pdf"));
    }
}
