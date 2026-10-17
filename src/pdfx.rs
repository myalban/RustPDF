//! PDFium helpers: opening, page geometry, inventory, tiled rendering.

use crate::geom::{Affine, PageGeom, R4};
use crate::logx;
use crate::model::*;
use crate::textgroup::{self, Run};
use pdfium_render::prelude::*;
use std::path::Path;

/// Initializes PDFium: looks for `pdfium.dll` next to the executable, in the current folder, then in the system.
pub fn load_pdfium() -> anyhow::Result<&'static Pdfium> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    let candidates = [
        Pdfium::pdfium_platform_library_name_at_path(&exe_dir),
        Pdfium::pdfium_platform_library_name_at_path("./"),
    ];
    let mut last = None;
    for c in candidates {
        match Pdfium::bind_to_library(c) {
            Ok(b) => return Ok(Box::leak(Box::new(Pdfium::new(b)))),
            Err(e) => last = Some(e),
        }
    }
    match Pdfium::bind_to_system_library() {
        Ok(b) => Ok(Box::leak(Box::new(Pdfium::new(b)))),
        Err(e) => Err(anyhow::anyhow!("pdfium.dll not found: {:?} / {:?}", last, e)),
    }
}

// ------------------------------------------------------------- opening

#[derive(Debug)]
pub enum OpenErr {
    /// Password required or incorrect.
    Password,
    /// Damaged file or invalid format.
    Damaged,
    /// File not found or unreadable.
    Missing(String),
    Other(String),
}

fn map_open_err(e: PdfiumError) -> OpenErr {
    match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => OpenErr::Password,
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => OpenErr::Damaged,
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FileError) => OpenErr::Missing(format!("{e:?}")),
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::SecurityError) => {
            OpenErr::Other("The document is protected (security restrictions)".into())
        }
        other => OpenErr::Other(format!("{other:?}")),
    }
}

pub fn open_path(pdfium: &'static Pdfium, path: &Path, password: Option<&str>) -> Result<PdfDocument<'static>, OpenErr> {
    if !path.exists() {
        return Err(OpenErr::Missing(path.display().to_string()));
    }
    let bytes = std::fs::read(path).map_err(|e| OpenErr::Missing(e.to_string()))?;
    open_bytes(pdfium, bytes, password)
}

pub fn open_bytes(pdfium: &'static Pdfium, bytes: Vec<u8>, password: Option<&str>) -> Result<PdfDocument<'static>, OpenErr> {
    pdfium.load_pdf_from_byte_vec(bytes, password).map_err(map_open_err)
}

/// Tries to repair a damaged PDF (rebuilding the cross-reference table) with `lopdf`.
pub fn repair_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut doc = lopdf::Document::load_mem(bytes).ok()?;
    let mut out = Vec::new();
    doc.save_to(&mut out).ok()?;
    Some(out)
}

// -------------------------------------------------------------- geometry

/// Computes the page → view transformation by querying PDFium (handles rotation, offset CropBox).
pub fn page_geom(page: &PdfPage) -> PageGeom {
    let w = page.width().value;
    let h = page.height().value;
    const S: i32 = 1_000_000;
    let cfg = PdfRenderConfig::new().set_fixed_size(S, S);
    let sample = |x: f32, y: f32| -> Option<(f32, f32)> {
        page.points_to_pixels(PdfPoints::new(x), PdfPoints::new(y), &cfg)
            .ok()
            .map(|(px, py)| (px as f32 * w / S as f32, py as f32 * h / S as f32))
    };
    // Reference points far from each other for precision.
    let d = 100.0;
    let (o, px, py) = match (sample(0.0, 0.0), sample(d, 0.0), sample(0.0, d)) {
        (Some(o), Some(px), Some(py)) => (o, px, py),
        _ => return PageGeom::simple(w, h),
    };
    let to_view = Affine { a: (px.0 - o.0) / d, b: (px.1 - o.1) / d, c: (py.0 - o.0) / d, d: (py.1 - o.1) / d, e: o.0, f: o.1 };
    match to_view.inverse() {
        Some(from_view) => PageGeom { w, h, to_view, from_view },
        None => PageGeom::simple(w, h),
    }
}

// ------------------------------------------------------------- inventory

fn rect_of(r: &PdfRect) -> R4 {
    [r.left().value, r.bottom().value, r.right().value, r.top().value]
}

pub fn scan_page(page: &PdfPage, idx: usize, ver: u64, paragraphs: bool) -> PageInfo {
    let geom = page_geom(page);
    let mut objects: Vec<ObjInfo> = vec![];
    let mut runs: Vec<Run> = vec![];
    // a single text page for all the objects (recreating it for each object is very expensive)
    let page_text = page.text().ok();
    for (i, obj) in page.objects().iter().enumerate() {
        let Ok(b) = obj.bounds() else { continue };
        let rect = rect_of(&b.to_rect());
        let kind = match obj.object_type() {
            PdfPageObjectType::Text => ObjKind::Text,
            PdfPageObjectType::Image => ObjKind::Image,
            PdfPageObjectType::Path => ObjKind::Path,
            PdfPageObjectType::Shading => ObjKind::Shading,
            _ => {
                if obj.as_x_object_form_object().is_some() {
                    ObjKind::Form
                } else {
                    ObjKind::Other
                }
            }
        };
        if kind == ObjKind::Text {
            if let Some(t) = obj.as_text_object() {
                let text = match &page_text {
                    Some(pt) => pt.for_object(&t),
                    None => t.text(),
                };
                if text.trim().is_empty() {
                    continue;
                }
                let color = obj.fill_color().map(|c| [c.red(), c.green(), c.blue()]).unwrap_or([0, 0, 0]);
                let base = obj.matrix().map(|m| m.f()).unwrap_or(rect[1]);
                let tt = text.trim();
                let leader = tt.chars().count() >= 3 && tt.chars().all(|ch| matches!(ch, '.' | '_' | '-' | '…' | '·' | ' '));
                runs.push(Run {
                    idx: i,
                    text,
                    font: t.font().name(),
                    size: t.scaled_font_size().value,
                    color,
                    base,
                    rect,
                    leader,
                });
            }
        } else {
            objects.push(ObjInfo { idx: i, kind, rect, members: vec![i], block: None, underline: false, strike: false });
        }
    }
    let blocks = if paragraphs { textgroup::group(runs) } else { textgroup::group_as_lines(runs) };
    let mut text_objs: Vec<ObjInfo> = blocks
        .into_iter()
        .map(|b| ObjInfo { idx: b.leader_idx(), kind: ObjKind::Text, rect: b.rect, members: b.members(), block: Some(b), underline: false, strike: false })
        .collect();
    attach_decorations(&mut text_objs, &mut objects);
    objects.extend(text_objs);
    objects.sort_by_key(|o| o.idx);

    // Form fields and annotations
    let mut fields = vec![];
    let mut annots = vec![];
    for (i, ann) in page.annotations().iter().enumerate() {
        let Ok(b) = ann.bounds() else { continue };
        let rect = rect_of(&b);
        if let Some(f) = ann.as_form_field() {
            let mut info = FieldInfo {
                ann: i,
                kind: FieldKind::Other,
                rect,
                name: f.name().unwrap_or_default(),
                value: String::new(),
                checked: false,
                read_only: f.is_read_only(),
                options: vec![],
            };
            match f.field_type() {
                PdfFormFieldType::Text => {
                    info.kind = FieldKind::Text;
                    info.value = f.as_text_field().and_then(|t| t.value()).unwrap_or_default();
                }
                PdfFormFieldType::Checkbox => {
                    info.kind = FieldKind::Check;
                    info.checked = f.as_checkbox_field().and_then(|c| c.is_checked().ok()).unwrap_or(false);
                }
                PdfFormFieldType::RadioButton => {
                    info.kind = FieldKind::Radio;
                    info.checked = f.as_radio_button_field().and_then(|c| c.is_checked().ok()).unwrap_or(false);
                }
                PdfFormFieldType::ComboBox => {
                    info.kind = FieldKind::Combo;
                    if let Some(c) = f.as_combo_box_field() {
                        info.value = c.value().unwrap_or_default();
                        info.options = c.options().iter().map(|o| (o.label().cloned().unwrap_or_default(), o.is_set())).collect();
                    }
                }
                PdfFormFieldType::ListBox => {
                    info.kind = FieldKind::List;
                    if let Some(c) = f.as_list_box_field() {
                        info.value = c.value().unwrap_or_default();
                        info.options = c.options().iter().map(|o| (o.label().cloned().unwrap_or_default(), o.is_set())).collect();
                    }
                }
                PdfFormFieldType::PushButton => info.kind = FieldKind::Button,
                PdfFormFieldType::Signature => info.kind = FieldKind::Signature,
                _ => {}
            }
            fields.push(info);
            continue;
        }
        let tname = format!("{:?}", ann.annotation_type());
        let kind = match tname.as_str() {
            "Highlight" => AnnotKind::Highlight,
            "Underline" => AnnotKind::Underline,
            "Strikeout" | "StrikeOut" => AnnotKind::StrikeOut,
            "Squiggly" => AnnotKind::Squiggly,
            "Text" => AnnotKind::Note,
            "FreeText" => AnnotKind::FreeText,
            "Ink" => AnnotKind::Ink,
            "Square" => AnnotKind::Square,
            "Circle" => AnnotKind::Circle,
            "Line" => AnnotKind::Line,
            "Stamp" => AnnotKind::Stamp,
            "Redacted" | "Redact" => AnnotKind::Redact,
            "Link" | "Popup" | "Widget" | "XfaWidget" => continue,
            _ => AnnotKind::Other,
        };
        // Do not call `stroke_color()` / `fill_color()`: PDFium crashes on annotations that have an appearance stream.
        // The exact color is read from the file (lopdf) when needed.
        let color = [255, 220, 0];
        annots.push(AnnotInfo {
            idx: i,
            kind,
            rect,
            contents: ann.contents().unwrap_or_default(),
            author: ann.creator().unwrap_or_default(),
            date: ann.modification_date().or_else(|| ann.creation_date()).unwrap_or_default(),
            color,
        });
    }

    // Links
    let mut links = vec![];
    for link in page.links().iter() {
        let Ok(r) = link.rect() else { continue };
        let mut target = LinkTarget::None;
        if let Some(action) = link.action() {
            if let Some(u) = action.as_uri_action() {
                if let Ok(uri) = u.uri() {
                    target = LinkTarget::Uri(uri);
                }
            } else if let Some(l) = action.as_local_destination_action() {
                if let Ok(d) = l.destination() {
                    if let Ok(p) = d.page_index() {
                        target = LinkTarget::Page(p as usize);
                    }
                }
            }
        }
        if target == LinkTarget::None {
            if let Some(d) = link.destination() {
                if let Ok(p) = d.page_index() {
                    target = LinkTarget::Page(p as usize);
                }
            }
        }
        links.push(LinkInfo { rect: rect_of(&r), target });
    }

    PageInfo { page: idx, ver, paragraphs, geom, objects, fields, annots, links }
}

/// Attaches to the text blocks the thin lines (underline, strikethrough) that go with them:
/// they are removed from the list of standalone objects and added to the block's `members`.
pub fn attach_decorations(texts: &mut [ObjInfo], others: &mut Vec<ObjInfo>) {
    let mut taken = vec![false; others.len()];
    for t in texts.iter_mut() {
        let Some(block) = t.block.clone() else { continue };
        for line in &block.lines {
            let size = line.first().size.max(1.0);
            let base = line.base;
            for (k, o) in others.iter().enumerate() {
                if taken[k] || o.kind != ObjKind::Path {
                    continue;
                }
                let (w, h) = (o.rect[2] - o.rect[0], o.rect[3] - o.rect[1]);
                if h > (0.16 * size).max(2.5) || w < 4.0 {
                    continue;
                }
                let lw = line.rect[2] - line.rect[0];
                if o.rect[0] < line.rect[0] - 3.0 || o.rect[2] > line.rect[2] + 3.0 || w < 0.6 * lw {
                    continue;
                }
                let yc = (o.rect[1] + o.rect[3]) * 0.5;
                if yc >= base - 0.35 * size && yc <= base - 0.02 * size {
                    t.underline = true;
                } else if yc > base + 0.18 * size && yc < base + 0.45 * size {
                    t.strike = true;
                } else {
                    continue;
                }
                taken[k] = true;
                t.members.push(o.idx);
                t.rect = crate::geom::r4_union(&t.rect, &o.rect);
            }
        }
    }
    let mut k = 0;
    others.retain(|_| {
        let keep = !taken[k];
        k += 1;
        keep
    });
}

// ------------------------------------------------------------------ rendering

/// Raw RGBA image.
pub struct Rgba {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

/// Renders the whole page at the given width (preview).
pub fn render_full(page: &PdfPage, width_px: i32, forms: bool) -> Option<Rgba> {
    let cfg = PdfRenderConfig::new()
        .set_target_width(width_px.max(8))
        .render_form_data(forms)
        .set_clear_color(PdfColor::new(255, 255, 255, 255));
    let bmp = page.render_with_config(&cfg).ok()?;
    Some(Rgba { w: bmp.width() as usize, h: bmp.height() as usize, data: bmp.as_rgba_bytes() })
}

/// Renders a tile `(x0, y0, w, h)` of a page whose total size in pixels is `(full_w, full_h)`.
pub fn render_tile(page: &PdfPage, full_w: i32, full_h: i32, x0: i32, y0: i32, w: i32, h: i32, _forms: bool) -> Option<Rgba> {
    let mut bmp = PdfBitmap::empty(w, h, PdfBitmapFormat::BGRA).ok()?;
    let cfg = PdfRenderConfig::new()
        .set_target_size(full_w, full_h)
        .set_origin(-x0, -y0)
        // The "with forms" mode is the only one that honors the origin; it is therefore always on for tiles.
        .render_form_data(true)
        .set_clear_color(PdfColor::new(255, 255, 255, 255));
    page.render_into_bitmap_with_config(&mut bmp, &cfg).map_err(|e| logx::warn(format!("tile: {e:?}"))).ok()?;
    Some(Rgba { w: w as usize, h: h as usize, data: bmp.as_rgba_bytes() })
}

/// Single PDFium instance shared by all the tests (PDFium only initializes once per process).
#[cfg(test)]
pub fn test_pdfium() -> &'static Pdfium {
    static P: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let p = *P.get_or_init(|| load_pdfium().expect("pdfium.dll") as *const Pdfium as usize);
    unsafe { &*(p as *const Pdfium) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdfium() -> &'static Pdfium {
        test_pdfium()
    }

    fn sample_doc(rotation: Option<PdfPageRenderRotation>) -> PdfDocument<'static> {
        let mut doc = pdfium().create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let font = doc.fonts_mut().helvetica();
        page.objects_mut()
            .create_text_object(PdfPoints::new(100.0), PdfPoints::new(700.0), "Hello", font, PdfPoints::new(20.0))
            .unwrap();
        if let Some(r) = rotation {
            page.set_rotation(r);
        }
        page.regenerate_content().unwrap();
        drop(page);
        doc
    }

    #[test]
    fn geometry_unrotated_flips_y() {
        let doc = sample_doc(None);
        let page = doc.pages().get(0).unwrap();
        let g = page_geom(&page);
        let (x, y) = g.to_view.apply(100.0, 700.0);
        assert!((x - 100.0).abs() < 0.1, "{x}");
        assert!((y - (page.height().value - 700.0)).abs() < 0.1, "{y}");
    }

    #[test]
    fn geometry_rotated_90_swaps_axes() {
        let doc = sample_doc(Some(PdfPageRenderRotation::Degrees90));
        let page = doc.pages().get(0).unwrap();
        let g = page_geom(&page);
        // The A4 portrait page rotated by 90° becomes landscape.
        assert!(g.w > g.h, "{} {}", g.w, g.h);
        let b = g.rect_to_view(&[100.0, 700.0, 200.0, 720.0]);
        assert!(b[0] >= -0.1 && b[2] <= g.w + 0.1 && b[1] >= -0.1 && b[3] <= g.h + 0.1, "{b:?} within {}x{}", g.w, g.h);
        // round trip
        let r = g.rect_from_view(&b);
        assert!((r[0] - 100.0).abs() < 0.2 && (r[3] - 720.0).abs() < 0.2, "{r:?}");
    }

    #[test]
    fn scan_finds_text_block() {
        let doc = sample_doc(None);
        let page = doc.pages().get(0).unwrap();
        let info = scan_page(&page, 0, 1, true);
        let t = info.objects.iter().find(|o| o.kind == ObjKind::Text).expect("text block");
        assert_eq!(t.text(), "Hello");
        assert!(t.rect[2] > t.rect[0]);
    }

    #[test]
    fn tile_equals_crop_of_full_render() {
        let doc = sample_doc(None);
        let page = doc.pages().get(0).unwrap();
        let (fw, fh) = (1190, 1684);
        let full = render_tile(&page, fw, fh, 0, 0, fw, fh, false).unwrap();
        let (x0, y0, tw, th) = (150, 250, 300, 200);
        let tile = render_tile(&page, fw, fh, x0, y0, tw, th, false).unwrap();
        let mut diff = 0u64;
        for y in 0..th as usize {
            for x in 0..tw as usize {
                let a = &full.data[((y + y0 as usize) * full.w + x + x0 as usize) * 4..][..3];
                let b = &tile.data[(y * tile.w + x) * 4..][..3];
                for c in 0..3 {
                    diff += (a[c] as i32 - b[c] as i32).unsigned_abs() as u64;
                }
            }
        }
        assert_eq!(diff, 0, "the tile must be identical to the crop of the full render");
    }

    #[test]
    fn open_errors_are_classified() {
        let p = pdfium();
        assert!(matches!(open_path(p, Path::new("n_existe_pas.pdf"), None), Err(OpenErr::Missing(_))));
        assert!(matches!(open_bytes(p, b"this is not a pdf".to_vec(), None), Err(OpenErr::Damaged)));
    }
}

