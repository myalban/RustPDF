//! Data model of a page: inventory of objects, form fields, annotations, links.

use crate::geom::{PageGeom, R4};
use crate::textgroup::Block;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjKind {
    Text,
    Image,
    Path,
    Shading,
    Form,
    Other,
}

#[derive(Clone, Debug)]
pub struct ObjInfo {
    /// Index of the PDF object (for a text block: the first piece).
    pub idx: usize,
    pub kind: ObjKind,
    /// Espace page PDF.
    pub rect: R4,
    /// All the PDF objects making up this item (a single one except for text).
    pub members: Vec<usize>,
    /// Text block (lines, alignment…), for `ObjKind::Text`.
    pub block: Option<Block>,
    /// Underlined / struck-through text (thin lines detected under or through the text, included in `members`).
    pub underline: bool,
    pub strike: bool,
}

impl ObjInfo {
    pub fn text(&self) -> &str {
        self.block.as_ref().map(|b| b.text.as_str()).unwrap_or("")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldKind {
    Text,
    Check,
    Radio,
    Combo,
    List,
    Button,
    Signature,
    Other,
}

#[derive(Clone, Debug)]
pub struct FieldInfo {
    /// Index of the widget annotation in the page.
    pub ann: usize,
    pub kind: FieldKind,
    pub rect: R4,
    pub name: String,
    pub value: String,
    pub checked: bool,
    pub read_only: bool,
    /// For lists: (label, selected).
    pub options: Vec<(String, bool)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnnotKind {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    Note,
    FreeText,
    Ink,
    Square,
    Circle,
    Line,
    Stamp,
    Redact,
    Other,
}

#[derive(Clone, Debug)]
pub struct AnnotInfo {
    pub idx: usize,
    pub kind: AnnotKind,
    pub rect: R4,
    pub contents: String,
    pub author: String,
    pub date: String,
    pub color: [u8; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    Uri(String),
    Page(usize),
    None,
}

#[derive(Clone, Debug)]
pub struct LinkInfo {
    pub rect: R4,
    pub target: LinkTarget,
}

/// Complete inventory of a page at a given version.
#[derive(Clone, Debug)]
pub struct PageInfo {
    pub page: usize,
    pub ver: u64,
    pub paragraphs: bool,
    pub geom: PageGeom,
    pub objects: Vec<ObjInfo>,
    pub fields: Vec<FieldInfo>,
    pub annots: Vec<AnnotInfo>,
    pub links: Vec<LinkInfo>,
}

impl PageInfo {
    pub fn object(&self, idx: usize) -> Option<&ObjInfo> {
        self.objects.iter().find(|o| o.idx == idx)
    }
}
