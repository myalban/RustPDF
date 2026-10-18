//! Operation dialog boxes (pages, document, signatures): common types and helpers.

use crate::pageops::{HAlign, SplitMode, VAlign};
use crate::pdfedit_doc::{CleanOpts, DocInfo, Security};
use crate::sign::{Identity, SigReport, SignOptions};
use crate::signature_pad::SigStrokes;
use eframe::egui::{self, Align2};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

pub const PAGE_SIZES: &[(&str, f32, f32)] = &[("A3", 841.89, 1190.55), ("A4", 595.28, 841.89), ("A5", 419.53, 595.28), ("Letter", 612.0, 792.0), ("Legal", 612.0, 1008.0), ("Tabloid", 792.0, 1224.0)];

pub struct InsertBlank {
    pub size: usize, // index in PAGE_SIZES; usize::MAX = same as the current page; usize::MAX-1 = custom
    pub landscape: bool,
    pub custom: [f32; 2],
    pub count: usize,
    pub position: usize, // 0 after the current page, 1 before, 2 at the start, 3 at the end
}

pub struct InsertFile {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub n_other: usize,
    pub pages: String,
    pub position: usize,
    pub page_no: usize,
}

pub struct Extract {
    pub pages: String,
    pub remove_after: bool,
}

pub struct Split {
    pub mode: usize, // 0 every N, 1 ranges, 2 max size, 3 bookmarks
    pub every: usize,
    pub ranges: String,
    pub max_mb: f32,
}

pub struct Merge {
    pub files: Vec<PathBuf>,
    pub include_current: bool,
    pub new_document: bool,
}

pub struct Crop {
    pub pages: String,
    pub margins: [f32; 4], // left, top, right, bottom (units)
}

pub struct PageSizeD {
    pub pages: String,
    pub preset: usize,
    pub landscape: bool,
    pub custom: [f32; 2],
    pub fit: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StampMode {
    Numbers,
    HeaderFooter,
}

pub struct StampD {
    pub mode: StampMode,
    pub pages: String,
    pub format: String,
    pub pos: (HAlign, VAlign),
    pub header: [String; 3],
    pub footer: [String; 3],
    pub family: usize, // 0 Helvetica 1 Times 2 Courier
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 3],
    pub margin: f32, // points
    pub start: i32,
    pub skip_first: bool,
}

pub struct WmD {
    pub text_mode: bool,
    pub text: String,
    pub image: Option<(PathBuf, image::DynamicImage)>,
    pub pages: String,
    pub size: f32,
    pub color: [u8; 3],
    pub opacity: f32,
    pub rotation: f32,
    pub behind: bool,
}

pub struct PagesAction {
    pub delete: bool,
    pub pages: String,
    pub angle: i32,
}

pub struct InfoD {
    pub info: DocInfo,
}

pub struct SecD {
    pub sec: Security,
    pub confirm: String,
    pub existing: Option<crate::pdfedit_doc::SecurityInfo>,
}

pub struct CleanD {
    pub opts: CleanOpts,
    pub report: Option<Vec<String>>,
}

pub struct SignD {
    pub identity: Option<Identity>,
    pub p12_path: Option<PathBuf>,
    pub password: String,
    pub error: String,
    pub opts: SignOptions,
    pub visible: bool,
    pub page: usize,
    pub corner: usize, // 0 bottom left 1 bottom right 2 top left 3 top right
    pub creating: Option<CreateId>,
    pub generating: Option<Receiver<Result<Identity, String>>>,
}

pub struct CreateId {
    pub name: String,
    pub org: String,
    pub email: String,
    pub years: u32,
    pub password: String,
}

pub struct NewFieldD {
    pub tool: crate::tools::Tool,
    pub page: usize,
    pub rect: [f32; 4],
    pub name: String,
    pub options: String,
    pub value: String,
    pub read_only: bool,
    pub required: bool,
    pub multiline: bool,
}

pub struct LinkD {
    pub page: usize,
    pub rect: [f32; 4],
    pub use_url: bool,
    pub url: String,
    pub target_page: usize,
}

#[derive(Clone)]
pub enum SigPayload {
    Strokes(SigStrokes),
    Typed { text: String, font: String },
    Image(image::DynamicImage),
}

pub struct SigD {
    pub tab: usize, // 0 draw 1 saved 2 type 3 image
    pub strokes: Vec<Vec<[f32; 2]>>,
    pub name: String,
    pub save: bool,
    pub saved: Vec<SigStrokes>,
    pub text: String,
    pub font: String,
    pub color: [u8; 3],
}

pub struct ExportD {
    pub kind: usize, // 0 pages as images 1 all images 2 text
    pub pages: String,
    pub dpi: f32,
    pub png: bool,
}

pub struct ChoiceD {
    pub page: usize,
    pub field: usize,
    pub options: Vec<(String, bool)>,
}

pub enum Dlg2 {
    InsertBlank(InsertBlank),
    InsertFile(InsertFile),
    Extract(Extract),
    Split(Split),
    Merge(Merge),
    Crop(Crop),
    PageSize(PageSizeD),
    Stamp(StampD),
    Watermark(WmD),
    Pages(PagesAction),
    Info(InfoD),
    Security(SecD),
    Clean(CleanD),
    Sign(Box<SignD>),
    Verify(Vec<SigReport>),
    NewField(NewFieldD),
    Link(LinkD),
    Signature(Box<SigD>),
    Export(ExportD),
    Choice(ChoiceD),
    Updates(String),
}

pub fn modal(title: &str) -> egui::Window<'static> {
    egui::Window::new(title.trim_end_matches('…').to_string()).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).order(egui::Order::Foreground)
}

/// Paper size (width, height) of a preset, taking the orientation into account.
pub fn preset_size(i: usize, landscape: bool) -> (f32, f32) {
    let (_, w, h) = PAGE_SIZES[i.min(PAGE_SIZES.len() - 1)];
    if landscape { (h, w) } else { (w, h) }
}

pub fn split_mode(s: &Split) -> Result<SplitMode, String> {
    Ok(match s.mode {
        0 => SplitMode::EveryN(s.every.max(1)),
        1 => SplitMode::Ranges(vec![]),
        2 => SplitMode::MaxBytes((s.max_mb * 1_048_576.0) as usize),
        _ => SplitMode::ByBookmarks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets() {
        let (w, h) = preset_size(1, false);
        assert!((w - 595.28).abs() < 0.01 && (h - 841.89).abs() < 0.01);
        let (w, h) = preset_size(1, true);
        assert!(w > h);
    }
}
