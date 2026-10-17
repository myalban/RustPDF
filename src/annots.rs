//! Annotations: text markup, notes, stamps, permanent redaction.

use crate::geom::*;
use crate::model::*;
use crate::pathdata::{PathData, build_path};
use crate::pdfedit_annot;
use crate::tab::Tab;
use crate::tools::Tool;
use pdfium_render::prelude::*;

fn err<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

fn pdf_rect(r: &R4) -> PdfRect {
    PdfRect::new(PdfPoints::new(r[1]), PdfPoints::new(r[0]), PdfPoints::new(r[3]), PdfPoints::new(r[2]))
}

pub fn markup_kind(t: Tool) -> Option<AnnotKind> {
    match t {
        Tool::Highlight => Some(AnnotKind::Highlight),
        Tool::Underline => Some(AnnotKind::Underline),
        Tool::StrikeOut => Some(AnnotKind::StrikeOut),
        Tool::Squiggly => Some(AnnotKind::Squiggly),
        _ => None,
    }
}

impl Tab {
    /// Bytes of the current document (starting point of the `lopdf` operations).
    fn bytes(&self) -> Result<Vec<u8>, String> {
        self.doc.save_to_bytes().map_err(err)
    }

    /// Creates a markup (highlight, underline, strikeout, squiggly) covering the given rectangles.
    pub fn add_markup(&mut self, page: usize, kind: AnnotKind, rects: &[R4], color: [u8; 3], alpha: u8, author: &str, mem: u32) -> Result<(), String> {
        if rects.is_empty() {
            return Err("no area".into());
        }
        let label = match kind {
            AnnotKind::Highlight => "Highlight",
            AnnotKind::Underline => "Underline",
            AnnotKind::StrikeOut => "Strikeout",
            _ => "Squiggly",
        };
        let a = pdfedit_annot::NewAnnot { page, kind, rects: rects.to_vec(), color, alpha, author: author.to_string(), contents: String::new() };
        let out = pdfedit_annot::add_annotation(&self.bytes()?, &a)?;
        self.replace_with_bytes(label, mem, out)
    }

    /// Creates a note (icon + comment) at (x, y).
    pub fn add_note(&mut self, page: usize, x: f32, y: f32, text: &str, author: &str, mem: u32) -> Result<(), String> {
        let a = pdfedit_annot::NewAnnot { page, kind: AnnotKind::Note, rects: vec![[x - 12.0, y - 12.0, x + 12.0, y + 12.0]], color: [255, 200, 0], alpha: 255, author: author.to_string(), contents: text.to_string() };
        let out = pdfedit_annot::add_annotation(&self.bytes()?, &a)?;
        self.replace_with_bytes("Note", mem, out)
    }

    /// Index in the /Annots array of annotation `ai` (PDFium index) of a page.
    fn annot_file_index(&mut self, bytes: &[u8], page: usize, ai: usize) -> Result<usize, String> {
        let info = self.info(page).ok_or("page not found")?;
        let a = info.annots.iter().find(|a| a.idx == ai).ok_or("annotation not found")?;
        let st = match a.kind {
            AnnotKind::Highlight => "Highlight",
            AnnotKind::Underline => "Underline",
            AnnotKind::StrikeOut => "StrikeOut",
            AnnotKind::Squiggly => "Squiggly",
            AnnotKind::Note => "Text",
            AnnotKind::FreeText => "FreeText",
            AnnotKind::Ink => "Ink",
            AnnotKind::Square => "Square",
            AnnotKind::Circle => "Circle",
            AnnotKind::Line => "Line",
            AnnotKind::Stamp => "Stamp",
            _ => "",
        };
        pdfedit_annot::find_annotation(bytes, page, st, &a.rect).ok_or_else(|| "annotation not found in the file".to_string())
    }

    pub fn annot_set_contents(&mut self, page: usize, ai: usize, text: &str, mem: u32) -> Result<(), String> {
        let b = self.bytes()?;
        let idx = self.annot_file_index(&b, page, ai)?;
        let out = pdfedit_annot::set_annotation_contents(&b, page, idx, text)?;
        self.replace_with_bytes("Comment", mem, out)
    }

    pub fn annot_set_color(&mut self, page: usize, ai: usize, color: [u8; 3], alpha: u8, mem: u32) -> Result<(), String> {
        let b = self.bytes()?;
        let idx = self.annot_file_index(&b, page, ai)?;
        let out = pdfedit_annot::set_annotation_color(&b, page, idx, color, alpha)?;
        self.replace_with_bytes("Annotation color", mem, out)
    }

    pub fn annot_move(&mut self, page: usize, ai: usize, dx: f32, dy: f32, mem: u32) -> Result<(), String> {
        let b = self.bytes()?;
        let idx = self.annot_file_index(&b, page, ai)?;
        let out = pdfedit_annot::move_annotation(&b, page, idx, dx, dy)?;
        self.replace_with_bytes("Move annotation", mem, out)
    }

    pub fn annot_delete(&mut self, page: usize, ai: usize, mem: u32) -> Result<(), String> {
        let b = self.bytes()?;
        let idx = self.annot_file_index(&b, page, ai)?;
        let out = pdfedit_annot::delete_annotation(&b, page, idx)?;
        self.replace_with_bytes("Delete annotation", mem, out)
    }

    /// All annotations of the document (comments), read from the file.
    pub fn list_comments(&self) -> Result<Vec<pdfedit_annot::AnnotRec>, String> {
        pdfedit_annot::list_annotations(&self.bytes()?)
    }

    /// Replies to an annotation (index in the page's /Annots array).
    pub fn reply_to(&mut self, page: usize, file_index: usize, text: &str, author: &str, mem: u32) -> Result<(), String> {
        let out = pdfedit_annot::add_reply(&self.bytes()?, page, file_index, text, author)?;
        self.replace_with_bytes("Reply", mem, out)
    }

    /// Changes the text of an annotation identified by its index in the file.
    pub fn set_comment_by_file_index(&mut self, page: usize, file_index: usize, text: &str, mem: u32) -> Result<(), String> {
        let out = pdfedit_annot::set_annotation_contents(&self.bytes()?, page, file_index, text)?;
        self.replace_with_bytes("Comment", mem, out)
    }

    pub fn delete_comment_by_file_index(&mut self, page: usize, file_index: usize, mem: u32) -> Result<(), String> {
        let out = pdfedit_annot::delete_annotation(&self.bytes()?, page, file_index)?;
        self.replace_with_bytes("Delete annotation", mem, out)
    }

    /// Stamp: rounded frame + text, grouped into a single movable item.
    pub fn add_stamp(&mut self, page: usize, x: f32, y: f32, text: &str, color: [u8; 3], mem: u32) -> Result<usize, String> {
        let text = text.to_string();
        let size = 22.0f32;
        let w = (text.chars().count() as f32 * size * 0.62 + 24.0).max(80.0);
        let h = size + 18.0;
        let rect = [x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0];
        let col = [color[0], color[1], color[2], 255];
        let mut first = 0usize;
        self.edit_page(page, "Stamp", mem, |pg, doc| {
            let tok = doc.fonts_mut().helvetica_bold();
            let mut objs = pg.objects_mut();
            first = objs.len() as usize;
            let mut frame = PathData::rect(rect);
            frame.stroke = Some(col);
            frame.width = 3.0;
            frame.fill = None;
            build_path(doc, &mut objs, &frame)?;
            let mut t = objs.create_text_object(PdfPoints::new(rect[0] + 12.0), PdfPoints::new(rect[1] + 9.0), &text, tok, PdfPoints::new(size))?;
            t.set_fill_color(PdfColor::new(col[0], col[1], col[2], 255))?;
            Ok(())
        })?;
        // group the frame and the text
        let info = self.info(page).ok_or("page not found")?;
        let items: Vec<usize> = info.objects.iter().filter(|o| o.idx >= first).map(|o| o.idx).collect();
        if items.len() >= 2 {
            return self.group_items(page, &items, mem);
        }
        Ok(first)
    }

    // ------------------------------------------------------------- redaction

    /// Permanent redaction of an area: removes the text, blacks out the images (pixels) and the annotations
    /// located in the area, then draws a black rectangle.
    pub fn redact_rects(&mut self, page: usize, rects: &[R4], mem: u32) -> Result<usize, String> {
        let rects = rects.to_vec();
        let rects_for_annots = rects.clone();
        let mut removed = 0usize;
        self.snapshot("Redact", mem);
        let r = self.edit_page_impl(page, "Redact", mem, false, |pg, doc| {
            // 1. text: remove the characters inside the areas, recreating the segments that are kept
            let page_text = pg.text()?;
            let mut text_jobs: Vec<(usize, Vec<(f32, String)>, PdfFontToken, f32, [u8; 3], f32)> = vec![];
            {
                let objs = pg.objects();
                for i in 0..objs.len() {
                    let o = objs.get(i)?;
                    let Some(t) = o.as_text_object() else { continue };
                    let Ok(b) = o.bounds() else { continue };
                    let br = b.to_rect();
                    let orect = [br.left().value, br.bottom().value, br.right().value, br.top().value];
                    if !rects.iter().any(|r| r4_intersects(&orect, r)) {
                        continue;
                    }
                    let m = o.matrix()?;
                    let base = m.f();
                    let size = t.scaled_font_size().value;
                    let color = o.fill_color().map(|c| [c.red(), c.green(), c.blue()]).unwrap_or([0, 0, 0]);
                    let tok = t.font().token();
                    let chars = t.chars(&page_text)?;
                    let mut kept: Vec<(f32, String)> = vec![];
                    let mut cur: Option<(f32, String)> = None;
                    for c in chars.iter() {
                        let ch = c.unicode_char();
                        let cr = c.loose_bounds().ok().map(|b| [b.left().value, b.bottom().value, b.right().value, b.top().value]);
                        let inside = match (&cr, ch) {
                            (Some(cr), _) => {
                                let (cx, cy) = r4_center(cr);
                                rects.iter().any(|r| r4_contains(r, cx, cy))
                            }
                            _ => false,
                        };
                        match (inside, ch) {
                            (false, Some(ch)) if !ch.is_control() => {
                                let ox = c.origin_x().map(|v| v.value).unwrap_or(cr.map(|r| r[0]).unwrap_or(0.0));
                                match cur.as_mut() {
                                    Some((_, s)) => s.push(ch),
                                    None => cur = Some((ox, ch.to_string())),
                                }
                            }
                            _ => {
                                if let Some(seg) = cur.take() {
                                    kept.push(seg);
                                }
                            }
                        }
                    }
                    if let Some(seg) = cur.take() {
                        kept.push(seg);
                    }
                    text_jobs.push((i, kept, tok, size, color, base));
                }
            }
            drop(page_text);
            let mut objs = pg.objects_mut();
            // remove the objects (descending indices) then recreate the kept segments
            let mut idxs: Vec<usize> = text_jobs.iter().map(|j| j.0).collect();
            idxs.sort_unstable_by(|a, b| b.cmp(a));
            for i in idxs {
                let o = objs.get(i)?;
                objs.remove_object(o)?;
                removed += 1;
            }
            for (_, kept, tok, size, color, base) in &text_jobs {
                for (x, s) in kept {
                    let mut o = objs.create_text_object(PdfPoints::new(*x), PdfPoints::new(*base), s, *tok, PdfPoints::new(*size))?;
                    o.set_fill_color(PdfColor::new(color[0], color[1], color[2], 255))?;
                }
            }
            // 2. images: blacken the pixels inside the area; remove if entirely inside
            let n = objs.len() as usize;
            for i in (0..n).rev() {
                let mut o = objs.get(i)?;
                if o.as_image_object().is_none() {
                    continue;
                }
                let Ok(b) = o.bounds() else { continue };
                let br = b.to_rect();
                let orect = [br.left().value, br.bottom().value, br.right().value, br.top().value];
                let hits: Vec<R4> = rects.iter().filter(|r| r4_intersects(&orect, r)).copied().collect();
                if hits.is_empty() {
                    continue;
                }
                if hits.iter().any(|r| r4_inside(&orect, r, 0.5)) {
                    objs.remove_object(o)?;
                    removed += 1;
                    continue;
                }
                if let Some(img) = crate::edit::processed_image(&mut o, doc) {
                    let mut rgba = img.to_rgba8();
                    let (iw, ih) = (rgba.width() as f32, rgba.height() as f32);
                    let (ow, oh) = (orect[2] - orect[0], orect[3] - orect[1]);
                    for r in &hits {
                        let x0 = (((r[0] - orect[0]) / ow) * iw).floor().clamp(0.0, iw) as u32;
                        let x1 = (((r[2] - orect[0]) / ow) * iw).ceil().clamp(0.0, iw) as u32;
                        // the image's Y axis is inverted relative to the page
                        let y0 = (((orect[3] - r[3]) / oh) * ih).floor().clamp(0.0, ih) as u32;
                        let y1 = (((orect[3] - r[1]) / oh) * ih).ceil().clamp(0.0, ih) as u32;
                        for y in y0..y1 {
                            for x in x0..x1 {
                                rgba.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                            }
                        }
                    }
                    if let Some(io) = o.as_image_object_mut() {
                        io.set_image(&image::DynamicImage::ImageRgba8(rgba))?;
                    }
                    removed += 1;
                }
            }
            // 3. paths entirely inside the area
            let n = objs.len() as usize;
            for i in (0..n).rev() {
                let o = objs.get(i)?;
                if !matches!(o.object_type(), PdfPageObjectType::Path) {
                    continue;
                }
                let Ok(b) = o.bounds() else { continue };
                let br = b.to_rect();
                let orect = [br.left().value, br.bottom().value, br.right().value, br.top().value];
                if rects.iter().any(|r| r4_inside(&orect, r, 0.5)) {
                    objs.remove_object(o)?;
                    removed += 1;
                }
            }
            // 4. black rectangles
            for r in &rects {
                let mut d = PathData::rect(*r);
                d.fill = Some([0, 0, 0, 255]);
                d.stroke = None;
                build_path(doc, &mut objs, &d)?;
            }
            Ok(())
        });
        if let Err(e) = r {
            self.undo.pop();
            return Err(e);
        }
        // 5. annotations, links and fields inside the area (through the file: safer than the annotation API)
        let (out, n) = pdfedit_annot::delete_annotations_in(&self.bytes()?, page, &rects_for_annots)?;
        if n > 0 {
            self.reload(out)?;
            removed += n;
        }
        Ok(removed)
    }

    /// Applies all pending redaction areas.
    pub fn apply_redactions(&mut self, mem: u32) -> Result<usize, String> {
        let pending = std::mem::take(&mut self.redactions);
        let mut total = 0;
        let mut pages: Vec<usize> = pending.iter().map(|(p, _)| *p).collect();
        pages.sort_unstable();
        pages.dedup();
        for p in pages {
            let rects: Vec<R4> = pending.iter().filter(|(pp, _)| *pp == p).map(|(_, r)| *r).collect();
            total += self.redact_rects(p, &rects, mem)?;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tab::Tab;

    fn tab_with_text(text: &str) -> Tab {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let font = doc.fonts_mut().helvetica();
        {
            let objs = page.objects_mut();
            objs.create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), text, font, PdfPoints::new(20.0)).unwrap();
            let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(40, 40, image::Rgba([255, 0, 0, 255])));
            objs.create_image_object(PdfPoints::new(300.0), PdfPoints::new(500.0), &img, Some(PdfPoints::new(100.0)), Some(PdfPoints::new(100.0))).unwrap();
        }
        page.regenerate_content().unwrap();
        drop(page);
        Tab::new(p, doc, None, None, 1.0)
    }

    #[test]
    fn highlight_is_created_read_back_and_drawn() {
        let mut t = tab_with_text("Highlight text");
        let r = {
            let info = t.info(0).unwrap();
            info.objects.iter().find(|o| o.kind == ObjKind::Text).unwrap().rect
        };
        t.add_markup(0, AnnotKind::Highlight, &[r], [255, 235, 59], 150, "Me", 100).unwrap();
        let info = t.info(0).unwrap();
        let a = info.annots.iter().find(|a| a.kind == AnnotKind::Highlight).expect("highlight read back");
        assert!(a.rect[0] <= r[0] + 1.0 && a.rect[2] >= r[2] - 1.0);
        // the rendering is yellow under the text (white background originally)
        let pg = t.read_page(0).unwrap();
        let img = crate::pdfx::render_full(&pg, 595, true).unwrap();
        // look for a yellow pixel in the text area
        let (x0, x1) = (r[0] as usize, r[2] as usize);
        let (y0, y1) = ((842.0 - r[3]) as usize, (842.0 - r[1]) as usize);
        let mut yellow = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let px = &img.data[(y * img.w + x) * 4..][..3];
                if px[0] > 200 && px[1] > 200 && px[2] < 200 {
                    yellow += 1;
                }
            }
        }
        assert!(yellow > 50, "yellow pixels expected under the text, found {yellow}; annotations: {:?}", info.annots);
    }

    #[test]
    fn all_markup_kinds_note_and_delete() {
        let mut t = tab_with_text("Several markups");
        let r = t.info(0).unwrap().objects.iter().find(|o| o.kind == ObjKind::Text).unwrap().rect;
        for k in [AnnotKind::Underline, AnnotKind::StrikeOut, AnnotKind::Squiggly] {
            t.add_markup(0, k, &[r], [255, 0, 0], 255, "Me", 100).unwrap();
        }
        t.add_note(0, 400.0, 600.0, "A comment", "Me", 100).unwrap();
        let info = t.info(0).unwrap();
        for k in [AnnotKind::Underline, AnnotKind::StrikeOut, AnnotKind::Squiggly, AnnotKind::Note] {
            assert!(info.annots.iter().any(|a| a.kind == k), "{k:?} missing in {:?}", info.annots.iter().map(|a| a.kind).collect::<Vec<_>>());
        }
        let note = info.annots.iter().find(|a| a.kind == AnnotKind::Note).unwrap();
        assert_eq!(note.contents, "A comment");
        let ai = note.idx;
        t.annot_set_contents(0, ai, "Edited", 100).unwrap();
        assert_eq!(t.info(0).unwrap().annots.iter().find(|a| a.kind == AnnotKind::Note).unwrap().contents, "Edited");
        let before = t.info(0).unwrap().annots.len();
        let ai = t.info(0).unwrap().annots.iter().find(|a| a.kind == AnnotKind::Note).unwrap().idx;
        t.annot_delete(0, ai, 100).unwrap();
        assert_eq!(t.info(0).unwrap().annots.len(), before - 1);
    }

    #[test]
    fn stamp_is_one_grouped_item() {
        let mut t = tab_with_text("x");
        let idx = t.add_stamp(0, 300.0, 400.0, "APPROVED", [30, 140, 60], 100).unwrap();
        let info = t.info(0).unwrap();
        let s = info.object(idx).expect("stamp");
        assert_eq!(s.kind, ObjKind::Form, "frame + text grouped");
        assert!(s.rect[2] - s.rect[0] > 80.0);
    }

    #[test]
    fn redaction_removes_text_partially_and_blackens_image() {
        let mut t = tab_with_text("PUBLIC SECRET1234 END");
        let full = t.page_text(0);
        assert!(full.contains("SECRET1234"));
        // area covering "SECRET1234": locate its characters to determine the rectangle
        let a = full.find("SECRET1234").unwrap();
        let (_, rects) = t.text_range(0, a, a + 9);
        let r = r4_expand(&rects[0], -0.5);
        t.redact_rects(0, &[[r[0], r[1] - 1.0, r[2], r[3] + 1.0], [330.0, 530.0, 380.0, 580.0]], 100).unwrap();
        let after = t.page_text(0);
        assert!(!after.contains("SECRET") && !after.contains("1234"), "redacted text still present: {after:?}");
        assert!(after.contains("PUBLIC") && after.contains("END"), "the rest is kept: {after:?}");
        // image: pixels blackened inside the area, red elsewhere
        let pg = t.read_page(0).unwrap();
        let img = crate::pdfx::render_full(&pg, 595, false).unwrap();
        let px = |x: f32, y: f32| {
            let (xi, yi) = (x as usize, (842.0 - y) as usize);
            img.data[(yi * img.w + xi) * 4..][..3].to_vec()
        };
        let black = px(355.0, 555.0);
        assert!(black[0] < 40 && black[1] < 40, "redacted area is black: {black:?}");
        let red = px(310.0, 510.0);
        assert!(red[0] > 200 && red[1] < 60, "rest of the image is red: {red:?}");
    }
}




#[cfg(test)]
mod redact_annot_tests {
    use super::*;

    #[test]
    fn redaction_also_removes_annotations_in_the_zone() {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let font = doc.fonts_mut().helvetica();
        page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), "Confidential", font, PdfPoints::new(20.0)).unwrap();
        page.regenerate_content().unwrap();
        drop(page);
        let mut t = Tab::new(p, doc, None, None, 1.0);
        t.add_note(0, 100.0, 710.0, "secret note", "Me", 100).unwrap();
        t.add_note(0, 400.0, 300.0, "note elsewhere", "Me", 100).unwrap();
        assert_eq!(t.list_comments().unwrap().len(), 2);
        let n = t.redact_rects(0, &[[60.0, 690.0, 200.0, 725.0]], 100).unwrap();
        assert!(n >= 2, "text + note removed: {n}");
        let left = t.list_comments().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].contents, "note elsewhere");
        assert!(!t.page_text(0).contains("Confidential"));
        // a single undo reverts the whole redaction
        assert_eq!(t.undo().as_deref(), Some("Redact"));
        assert_eq!(t.list_comments().unwrap().len(), 2);
        assert!(t.page_text(0).contains("Confidential"));
    }
}
