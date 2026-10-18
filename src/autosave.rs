//! Automatic saving and recovery after a crash.

use crate::logx;
use crate::tab::Tab;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct Meta {
    orig: String,
    title: String,
    saved_at: u64,
}

pub fn dir() -> PathBuf {
    let d = logx::data_dir().join("autosave");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Writes the current state of a modified tab. Returns the written file.
pub fn write(tab: &mut Tab) -> Option<PathBuf> {
    if !tab.dirty {
        return None;
    }
    // same bytes as a normal save: a password-protected document is not written unprotected in the recovery folder
    let bytes = tab.bytes_for_save().ok()?;
    let f = dir().join(format!("tab{}_{}.pdf", tab.id, std::process::id()));
    std::fs::write(&f, bytes).ok()?;
    crate::sys::restrict_to_owner(&f);
    let meta = Meta { orig: tab.path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(), title: tab.title.clone(), saved_at: logx::unix_secs() };
    let _ = std::fs::write(f.with_extension("json"), serde_json::to_string(&meta).unwrap_or_default());
    tab.autosave_file = Some(f.clone());
    Some(f)
}

/// Recovery files left by a previous session: (original path, backup file).
pub fn scan(skip_pid: u32) -> Vec<(PathBuf, PathBuf)> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(dir()) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().map(|x| x != "pdf").unwrap_or(true) {
            continue;
        }
        // ignore the backups of the current session
        if p.file_name().map(|n| n.to_string_lossy().ends_with(&format!("_{skip_pid}.pdf"))).unwrap_or(false) {
            continue;
        }
        let orig = std::fs::read_to_string(p.with_extension("json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Meta>(&s).ok())
            .map(|m| PathBuf::from(if m.orig.is_empty() { m.title } else { m.orig }))
            .unwrap_or_else(|| p.clone());
        out.push((orig, p));
    }
    out
}

pub fn remove(p: &Path) {
    let _ = std::fs::remove_file(p);
    let _ = std::fs::remove_file(p.with_extension("json"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_scan_roundtrip() {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        doc.pages_mut().create_page_at_end(pdfium_render::prelude::PdfPagePaperSize::a4()).unwrap();
        let mut tab = Tab::new(p, doc, Some(std::env::temp_dir().join("test_rp.pdf")), None, 1.0);
        assert!(write(&mut tab).is_none(), "nothing to save while the document is not modified");
        tab.dirty = true;
        let f = write(&mut tab).expect("backup file");
        assert!(f.exists());
        // seen from "another session"
        let found = scan(0);
        assert!(found.iter().any(|(o, a)| a == &f && o.ends_with("test_rp.pdf")), "{found:?}");
        // the current session is ignored
        assert!(!scan(std::process::id()).iter().any(|(_, a)| a == &f));
        remove(&f);
        assert!(!f.exists());
    }
}
