//! A tab = an open document: PDF, history, view, selection, search.

use crate::geom::R4;
use crate::logx;
use crate::model::*;
use crate::pdfx;
use crate::textsel::TextSel;
use eframe::egui::{Pos2, Rect, Vec2};
use pdfium_render::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

pub struct HistEntry {
    pub label: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FitMode {
    None,
    Width,
    Page,
}

/// Position and zoom of a document's view.
pub struct ViewState {
    /// Current page (the most visible one).
    pub page: usize,
    /// Zoom factor (1.0 = 100 % = 96 dpi).
    pub zoom: f32,
    pub fit: FitMode,
    /// Current scroll offset (filled in at every frame).
    pub offset: Vec2,
    /// Go to a page: (page, fraction of its height to place at the top).
    pub scroll_to: Option<usize>,
    /// Force an offset (zoom centered on the cursor, jump to a position).
    pub set_offset: Option<Vec2>,
    /// View point (screen) to keep fixed during a zoom.
    pub zoom_pivot: Option<Pos2>,
    pub zoom_changed_at: Option<Instant>,
    /// Size of the visible area.
    pub avail: Vec2,
    /// Scale (pixels per point) of the previous frame, for the zoom centered on the cursor.
    pub pivot_old_scale: Option<f32>,
    /// Top-left corner of the scrolling content, on screen.
    pub origin_screen: Pos2,
    /// Pages drawn in the last frame.
    pub visible: Vec<usize>,
}

impl ViewState {
    pub fn new(zoom: f32) -> Self {
        ViewState {
            page: 0,
            zoom,
            fit: FitMode::None,
            offset: Vec2::ZERO,
            scroll_to: None,
            set_offset: None,
            zoom_pivot: None,
            zoom_changed_at: None,
            avail: Vec2::new(800.0, 600.0),
            pivot_old_scale: None,
            origin_screen: Pos2::ZERO,
            visible: vec![],
        }
    }
}

/// Selected items (all on the same page).
#[derive(Default, Clone, Debug)]
pub struct Selection {
    pub page: usize,
    /// Indices of the items (`ObjInfo::idx`).
    pub items: Vec<usize>,
    pub field: Option<usize>,
    pub annot: Option<usize>,
    pub link: Option<usize>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.field.is_none() && self.annot.is_none() && self.link.is_none()
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.field = None;
        self.annot = None;
        self.link = None;
    }
    pub fn has_object(&self) -> bool {
        !self.items.is_empty()
    }
    pub fn contains(&self, page: usize, idx: usize) -> bool {
        self.page == page && self.items.contains(&idx)
    }
}

/// An occurrence found by the search (may span several rectangles).
#[derive(Clone)]
pub struct SearchHit {
    pub page: usize,
    pub rects: Vec<R4>,
}

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub case: bool,
    pub word: bool,
    pub replace: String,
    pub matches: Vec<SearchHit>,
    pub cur: Option<usize>,
    pub stale: bool,
    pub scroll: bool,
}

/// Gesture in progress with the mouse.
#[derive(Clone, Debug, Default)]
pub enum Drag {
    #[default]
    None,
    /// Move of the selection (accumulated in screen pixels).
    Move { page: usize, accum: Vec2 },
    /// Resize: original and current screen rectangle; `hx`/`hy` ∈ {-1, 0, 1}.
    Resize { page: usize, hx: i8, hy: i8, orig: Rect, cur: Rect },
    /// Selection rectangle (screen pixels).
    Marquee { page: usize, start: Pos2, cur: Pos2 },
    /// Area drawn (export, redaction, link, field, image…) in page space.
    Zone { page: usize, a: [f32; 2], b: [f32; 2] },
    /// Text selection in progress.
    TextSel { page: usize, anchor: usize },
    /// Shape being drawn (two corners, page space).
    Shape { page: usize, a: [f32; 2], b: [f32; 2] },
    /// Freehand stroke (points in page space).
    Ink { page: usize, pts: Vec<[f32; 2]> },
    /// Move of a point of a vector path.
    PathPoint { page: usize, item: usize, seg: usize, kind: crate::pathdata::PtKind, pos: [f32; 2] },
    Pan,
}

/// Text editor directly on the page.
#[derive(Clone, Debug)]
pub enum InlineKind {
    /// Existing text block (item index).
    Object(usize),
    /// Form field (index in `PageInfo::fields`).
    Field(usize),
    /// New text to create at (x, y) page space.
    NewText { x: f32, y: f32 },
    /// Content of a note/comment (annotation index).
    Note(usize),
}

#[derive(Clone, Debug)]
pub struct Inline {
    pub page: usize,
    pub kind: InlineKind,
    pub text: String,
    pub focus: bool,
}

pub struct Tab {
    pub id: u64,
    pub doc: PdfDocument<'static>,
    pub pdfium: &'static Pdfium,
    pub path: Option<PathBuf>,
    pub password: Option<String>,
    pub title: String,
    pub dirty: bool,
    pub undo: Vec<HistEntry>,
    pub redo: Vec<HistEntry>,
    /// Versions: incremented at every structure change (pages added, deleted, moved…).
    pub doc_ver: u64,
    /// Content version of each page.
    pub page_ver: Vec<u64>,
    /// Sizes of the pages in view space (rotation applied), in points.
    pub sizes: Vec<(f32, f32)>,
    pub infos: HashMap<usize, Rc<PageInfo>>,
    pub paragraphs: bool,
    /// Calculation/format rules of the fields (document version, rules).
    pub calc: Option<(u64, Vec<crate::pdfedit_forms::CalcRule>)>,
    /// Protection to apply on save (password, rights).
    pub security: Option<crate::pdfedit_doc::Security>,
    /// Protection of the file as it was opened (password, permissions): restored on save if an edit stripped it.
    pub original_protection: Option<crate::pdfedit_doc::Security>,
    pub view: ViewState,
    pub sel: Selection,
    pub search: SearchState,
    pub drag: Drag,
    pub inline: Option<Inline>,
    pub text_sel: Option<TextSel>,
    /// Measurement in progress (points in page space, page).
    pub measure: Vec<[f32; 2]>,
    pub measure_page: usize,
    /// Redaction areas waiting to be applied: (page, rectangle).
    pub redactions: Vec<(usize, [f32; 4])>,
    /// Polygon being entered (page space).
    pub poly: Vec<[f32; 2]>,
    pub last_edit: Option<Instant>,
    /// Bytes of the document in its current state, prepared in advance (so that the next edit is instant).
    pub snap_cache: Option<(u64, Vec<u8>)>,
    /// Change counter (invalidates `snap_cache`).
    pub mut_count: u64,
    pub last_autosave: Instant,
    pub autosave_file: Option<PathBuf>,
    pub opened_at: Instant,
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Tab {
    pub fn new(pdfium: &'static Pdfium, doc: PdfDocument<'static>, path: Option<PathBuf>, password: Option<String>, zoom: f32) -> Tab {
        let title = path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let mut t = Tab {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            doc,
            pdfium,
            path,
            password,
            title,
            dirty: false,
            undo: vec![],
            redo: vec![],
            doc_ver: 1,
            page_ver: vec![],
            sizes: vec![],
            infos: HashMap::new(),
            paragraphs: true,
            calc: None,
            security: None,
            original_protection: None,
            view: ViewState::new(zoom),
            sel: Selection::default(),
            search: SearchState::default(),
            drag: Drag::None,
            inline: None,
            text_sel: None,
            measure: vec![],
            measure_page: 0,
            redactions: vec![],
            poly: vec![],
            last_edit: None,
            snap_cache: None,
            mut_count: 0,
            last_autosave: Instant::now(),
            autosave_file: None,
            opened_at: Instant::now(),
        };
        t.refresh_sizes();
        t
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    /// Recomputes the page sizes and resets the versions (after a structure change).
    pub fn refresh_sizes(&mut self) {
        let n = self.doc.pages().len() as usize;
        let mut sizes = Vec::with_capacity(n);
        for i in 0..n {
            let s = self
                .doc
                .pages()
                .page_size(i as i32)
                .map(|r| (r.width().value, r.height().value))
                .unwrap_or((595.0, 842.0));
            sizes.push(s);
        }
        self.sizes = sizes;
        self.page_ver = vec![self.doc_ver; n];
        self.infos.clear();
        if self.view.page >= n {
            self.view.page = n.saturating_sub(1);
        }
    }

    /// Size of a single page after a change (rotation, crop).
    fn refresh_one_size(&mut self, i: usize) {
        if let Ok(p) = self.doc.pages().get(i as i32) {
            if i < self.sizes.len() {
                self.sizes[i] = (p.width().value, p.height().value);
            }
        }
    }

    /// Inventory of the page (cached until the next change).
    pub fn info(&mut self, page: usize) -> Option<Rc<PageInfo>> {
        if page >= self.page_count() {
            return None;
        }
        if let Some(i) = self.infos.get(&page) {
            if i.ver == self.page_ver[page] && i.paragraphs == self.paragraphs {
                return Some(i.clone());
            }
        }
        let t0 = Instant::now();
        let p = self.read_page(page).ok()?;
        let info = Rc::new(pdfx::scan_page(&p, page, self.page_ver[page], self.paragraphs));
        if t0.elapsed().as_millis() > 60 {
            logx::info(format!("slow page analysis {}: {} ms ({} objects)", page + 1, t0.elapsed().as_millis(), info.objects.len()));
        }
        self.infos.insert(page, info.clone());
        Some(info)
    }

    pub fn set_paragraph_mode(&mut self, on: bool) {
        if self.paragraphs != on {
            self.paragraphs = on;
            self.infos.clear();
            self.sel.clear();
        }
    }

    // ------------------------------------------------------------ history

    fn mem_used(&self) -> usize {
        self.undo.iter().chain(self.redo.iter()).map(|e| e.bytes.len()).sum()
    }

    /// Remembers the current state before a change (undoable).
    pub fn snapshot(&mut self, label: &str, mem_cap_mb: u32) {
        let cached = match self.snap_cache.take() {
            Some((stamp, b)) if stamp == self.mut_count => Some(b),
            _ => None,
        };
        let saved = match cached {
            Some(b) => Ok(b),
            None => self.doc.save_to_bytes(),
        };
        // a change always follows a snapshot
        self.mut_count += 1;
        match saved {
            Ok(b) => {
                self.undo.push(HistEntry { label: label.to_string(), bytes: b });
                self.redo.clear();
                let cap = mem_cap_mb as usize * 1024 * 1024;
                while self.undo.len() > 1 && self.mem_used() > cap {
                    self.undo.remove(0);
                }
            }
            Err(e) => logx::warn(format!("snapshot failed: {e:?}")),
        }
        self.dirty = true;
        self.last_edit = Some(Instant::now());
    }

    /// Prepares (during an idle moment) the document bytes, to speed up the next change.
    pub fn prepare_snapshot(&mut self) {
        if matches!(&self.snap_cache, Some((s, _)) if *s == self.mut_count) {
            return;
        }
        self.snap_cache = self.doc.save_to_bytes().ok().map(|b| (self.mut_count, b));
    }

    pub fn snapshot_ready(&self) -> bool {
        matches!(&self.snap_cache, Some((s, _)) if *s == self.mut_count)
    }

    pub(crate) fn reload(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        match pdfx::open_bytes(self.pdfium, bytes, self.password.as_deref()) {
            Ok(d) => {
                self.mut_count += 1;
                self.snap_cache = None;
                self.doc = d;
                self.doc_ver += 1;
                self.refresh_sizes();
                self.sel.clear();
                self.search.stale = true;
                Ok(())
            }
            Err(e) => Err(format!("{e:?}")),
        }
    }

    /// Undoes the last action; returns its label.
    pub fn undo(&mut self) -> Option<String> {
        let e = self.undo.pop()?;
        let cur = self.doc.save_to_bytes().ok();
        if let Err(m) = self.reload(e.bytes) {
            logx::error(format!("undo failed: {m}"));
            return None;
        }
        if let Some(c) = cur {
            self.redo.push(HistEntry { label: e.label.clone(), bytes: c });
        }
        self.dirty = true;
        Some(e.label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let e = self.redo.pop()?;
        let cur = self.doc.save_to_bytes().ok();
        if let Err(m) = self.reload(e.bytes) {
            logx::error(format!("redo failed: {m}"));
            return None;
        }
        if let Some(c) = cur {
            self.undo.push(HistEntry { label: e.label.clone(), bytes: c });
        }
        self.dirty = true;
        Some(e.label)
    }

    /// Goes back to the state preceding action number `n` of the undo stack (0 = the oldest).
    pub fn undo_to(&mut self, n: usize) {
        while self.undo.len() > n {
            if self.undo().is_none() {
                break;
            }
        }
    }

    // ------------------------------------------------------------ changes

    /// Modifies a page: snapshot, closure `f`, content regeneration, cache invalidation.
    pub fn edit_page<F>(&mut self, page: usize, label: &str, mem_cap_mb: u32, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut PdfPage<'static>, &mut PdfDocument<'static>) -> Result<(), PdfiumError>,
    {
        self.edit_page_impl(page, label, mem_cap_mb, true, f)
    }

    /// Like `edit_page`, but the snapshot (undo) is the caller's responsibility.
    pub fn edit_page_impl<F>(&mut self, page: usize, label: &str, mem_cap_mb: u32, snap: bool, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut PdfPage<'static>, &mut PdfDocument<'static>) -> Result<(), PdfiumError>,
    {
        if page >= self.page_count() {
            return Err("page does not exist".into());
        }
        let t0 = Instant::now();
        if snap {
            self.snapshot(label, mem_cap_mb);
        }
        let t_snap = t0.elapsed();
        // Temporarily detach the document to be able to borrow page and document separately.
        let r = (|| {
            let mut pg = self.doc.pages().get(page as i32)?;
            // A single regeneration at the end (not at every object change).
            pg.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
            f(&mut pg, &mut self.doc)?;
            pg.regenerate_content()?;
            Ok::<(), PdfiumError>(())
        })();
        match r {
            Ok(_) => {
                self.mut_count += 1;
                self.page_ver[page] += 1;
                self.infos.remove(&page);
                self.refresh_one_size(page);
                self.search.stale = true;
                if t0.elapsed().as_millis() > 100 {
                    logx::info(format!("slow \"{label}\": snapshot {} ms, total {} ms", t_snap.as_millis(), t0.elapsed().as_millis()));
                }
                Ok(())
            }
            Err(e) => {
                // The snapshot was not used.
                if snap {
                    self.undo.pop();
                }
                Err(format!("{e:?}"))
            }
        }
    }

    /// Structure change of the document (pages, bookmarks…): everything is recomputed.
    pub fn edit_doc<F>(&mut self, label: &str, mem_cap_mb: u32, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut PdfDocument<'static>) -> Result<(), PdfiumError>,
    {
        self.snapshot(label, mem_cap_mb);
        match f(&mut self.doc) {
            Ok(_) => {
                self.doc_ver += 1;
                self.refresh_sizes();
                self.sel.clear();
                self.search.stale = true;
                Ok(())
            }
            Err(e) => {
                self.undo.pop();
                Err(format!("{e:?}"))
            }
        }
    }

    /// Replaces the whole document with new bytes (`lopdf` operations).
    pub fn replace_with_bytes(&mut self, label: &str, mem_cap_mb: u32, bytes: Vec<u8>) -> Result<(), String> {
        self.snapshot(label, mem_cap_mb);
        match self.reload(bytes) {
            Ok(_) => Ok(()),
            Err(e) => {
                self.undo.pop();
                Err(e)
            }
        }
    }

    pub fn save_to(&mut self, path: &std::path::Path) -> Result<(), String> {
        let bytes = self.bytes_for_save()?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())?;
        self.path = Some(path.to_path_buf());
        self.title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.dirty = false;
        Ok(())
    }

    /// Opens a page read-only: automatic content regeneration is disabled.
    /// (The crate temporarily modifies the matrix of the images it reads, then would regenerate the content when
    /// the page is closed, which would alter the document.)
    pub fn read_page(&self, i: usize) -> Result<PdfPage<'static>, String> {
        let mut p = self.doc.pages().get(i as i32).map_err(|e| format!("{e:?}"))?;
        p.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
        Ok(p)
    }

    /// Move (vx, vy) expressed in view space (screen) → page space, for the given page.
    pub fn sel_view_delta(&self, page: usize, vx: f32, vy: f32) -> (f32, f32) {
        match self.infos.get(&page) {
            Some(i) => i.geom.from_view.apply_vec(vx, vy),
            None => (vx, -vy),
        }
    }

    pub fn goto_page(&mut self, page: usize) {
        let p = page.min(self.page_count().saturating_sub(1));
        self.view.page = p;
        self.view.scroll_to = Some(p);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn tab_with_text(text: &str) -> Tab {
        let p = pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let font = doc.fonts_mut().helvetica();
        page.objects_mut()
            .create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), text, font, PdfPoints::new(14.0))
            .unwrap();
        page.regenerate_content().unwrap();
        drop(page);
        Tab::new(p, doc, None, None, 1.0)
    }

    #[test]
    fn edit_undo_redo_cycle() {
        let mut t = tab_with_text("Hello");
        assert_eq!(t.info(0).unwrap().objects.len(), 1);
        t.edit_page(0, "Addition", 100, |page, doc| {
            let font = doc.fonts_mut().helvetica();
            page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(600.0), "Second", font, PdfPoints::new(14.0))?;
            Ok(())
        })
        .unwrap();
        assert!(t.dirty);
        assert_eq!(t.info(0).unwrap().objects.len(), 2, "cache invalidated after an edit");
        assert_eq!(t.undo.len(), 1);
        assert_eq!(t.undo().as_deref(), Some("Addition"));
        assert_eq!(t.info(0).unwrap().objects.len(), 1);
        assert_eq!(t.redo.len(), 1);
        assert_eq!(t.redo().as_deref(), Some("Addition"));
        assert_eq!(t.info(0).unwrap().objects.len(), 2);
    }

    #[test]
    fn history_respects_memory_cap() {
        let mut t = tab_with_text("x");
        for i in 0..5 {
            t.edit_page(0, &format!("a{i}"), 0, |_, _| Ok(())).unwrap();
        }
        assert_eq!(t.undo.len(), 1, "with a zero cap at least one entry is kept");
    }

    #[test]
    fn failed_edit_leaves_no_history() {
        let mut t = tab_with_text("x");
        let r = t.edit_page(5, "missing page", 100, |_, _| Ok(()));
        assert!(r.is_err());
        assert!(t.undo.is_empty());
    }

    #[test]
    fn structure_edit_resets_versions() {
        let mut t = tab_with_text("x");
        let v = t.doc_ver;
        t.edit_doc("Blank page", 100, |d| {
            d.pages_mut().create_page_at_end(PdfPagePaperSize::a4())?;
            Ok(())
        })
        .unwrap();
        assert_eq!(t.page_count(), 2);
        assert!(t.doc_ver > v);
        t.undo();
        assert_eq!(t.page_count(), 1);
    }
}
