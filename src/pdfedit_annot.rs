//! Annotations written with `lopdf`: dictionary + appearance stream, for identical rendering in every reader.

use crate::geom::{R4, r4_union};
use crate::model::AnnotKind;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

/// PDF text string (UTF-16BE with BOM as soon as there are non-ASCII characters).
pub fn pdf_text(s: &str) -> Object {
    if s.is_ascii() {
        Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
    } else {
        let mut v = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        Object::String(v, StringFormat::Literal)
    }
}

/// Decodes a PDF text string (PDFDocEncoding ≈ Latin-1, or UTF-16BE with BOM).
pub fn text_of(o: &Object) -> String {
    match o {
        Object::String(b, _) => {
            if b.len() >= 2 && b[0] == 0xFE && b[1] == 0xFF {
                let u: Vec<u16> = b[2..].chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&u)
            } else if let Ok(s) = std::str::from_utf8(b) {
                // many tools write raw UTF-8; an accented Latin-1 text is almost never valid UTF-8
                s.to_string()
            } else {
                b.iter().map(|&c| c as char).collect()
            }
        }
        Object::Name(n) => String::from_utf8_lossy(n).into_owned(),
        _ => String::new(),
    }
}

fn f(v: f32) -> Object {
    Object::Real(v)
}

fn pdf_date() -> Object {
    let t = crate::logx::timestamp(); // "YYYY-MM-DD HH:MM:SS"
    let d: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    Object::String(format!("D:{d}Z").into_bytes(), StringFormat::Literal)
}

fn rect_obj(r: &R4) -> Object {
    Object::Array(vec![f(r[0]), f(r[1]), f(r[2]), f(r[3])])
}

fn obj_to_rect(o: &Object) -> Option<R4> {
    let a = o.as_array().ok()?;
    if a.len() < 4 {
        return None;
    }
    let v: Vec<f32> = a.iter().take(4).filter_map(|x| x.as_float().ok()).collect();
    if v.len() < 4 {
        return None;
    }
    Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

/// Content (appearance stream) of a text markup.
fn markup_ap_content(kind: AnnotKind, rects: &[R4], color: [u8; 3]) -> String {
    let (r, g, b) = (color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0);
    let mut s = String::new();
    match kind {
        AnnotKind::Highlight => {
            s.push_str(&format!("/GS gs {r:.3} {g:.3} {b:.3} rg\n"));
            for q in rects {
                s.push_str(&format!("{:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l {:.2} {:.2} l h f\n", q[0], q[3], q[2], q[3], q[2], q[1], q[0], q[1]));
            }
        }
        AnnotKind::Underline | AnnotKind::StrikeOut => {
            s.push_str(&format!("/GS gs {r:.3} {g:.3} {b:.3} RG 1 w\n"));
            for q in rects {
                let y = if kind == AnnotKind::Underline { q[1] + 1.0 } else { (q[1] + q[3]) * 0.5 };
                s.push_str(&format!("{:.2} {:.2} m {:.2} {:.2} l S\n", q[0], y, q[2], y));
            }
        }
        AnnotKind::Squiggly => {
            s.push_str(&format!("/GS gs {r:.3} {g:.3} {b:.3} RG 0.8 w\n"));
            for q in rects {
                let y = q[1] + 1.0;
                let mut x = q[0];
                s.push_str(&format!("{x:.2} {y:.2} m "));
                let mut up = true;
                while x < q[2] {
                    x += 2.0;
                    let yy = if up { y + 1.8 } else { y };
                    s.push_str(&format!("{:.2} {yy:.2} l ", x.min(q[2])));
                    up = !up;
                }
                s.push_str("S\n");
            }
        }
        _ => {}
    }
    s
}

/// Comment bubble of 24×24.
fn note_ap_content(color: [u8; 3]) -> String {
    let (r, g, b) = (color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0);
    format!("q {r:.3} {g:.3} {b:.3} rg 0.3 0.25 0 RG 1 w 2 6 20 16 re B Q q 0.3 0.25 0 RG 1 w 6 6 m 6 2 l 10 6 l S Q q 0.3 0.25 0 RG 1 w 5 17 m 19 17 l 5 13 m 19 13 l 5 9.5 m 14 9.5 l S Q\n")
}

fn make_ap(doc: &mut Document, bbox: R4, content: String, alpha: f32, multiply: bool) -> ObjectId {
    let mut gs = Dictionary::new();
    gs.set("Type", Object::Name(b"ExtGState".to_vec()));
    gs.set("CA", f(alpha));
    gs.set("ca", f(alpha));
    if multiply {
        gs.set("BM", Object::Name(b"Multiply".to_vec()));
    }
    let mut egs = Dictionary::new();
    egs.set("GS", Object::Dictionary(gs));
    let mut res = Dictionary::new();
    res.set("ExtGState", Object::Dictionary(egs));
    let mut d = Dictionary::new();
    d.set("Type", Object::Name(b"XObject".to_vec()));
    d.set("Subtype", Object::Name(b"Form".to_vec()));
    d.set("BBox", rect_obj(&bbox));
    d.set("Resources", Object::Dictionary(res));
    doc.add_object(Stream::new(d, content.into_bytes()))
}

pub(crate) fn page_id(doc: &Document, page: usize) -> Result<ObjectId, String> {
    doc.get_pages().get(&((page + 1) as u32)).copied().ok_or_else(|| "page not found".to_string())
}

/// Entries of the page's /Annots array (references or direct dictionaries).
pub(crate) fn annots_array(doc: &Document, pid: ObjectId) -> Vec<Object> {
    let Ok(pd) = doc.get_dictionary(pid) else { return vec![] };
    match pd.get(b"Annots") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => doc.get_object(*r).ok().and_then(|o| o.as_array().ok().cloned()).unwrap_or_default(),
        _ => vec![],
    }
}

pub(crate) fn set_annots_array(doc: &mut Document, pid: ObjectId, arr: Vec<Object>) -> Result<(), String> {
    let target = doc.get_dictionary(pid).ok().and_then(|d| if let Ok(Object::Reference(r)) = d.get(b"Annots") { Some(*r) } else { None });
    if let Some(r) = target {
        doc.objects.insert(r, Object::Array(arr));
    } else {
        let pd = doc.get_object_mut(pid).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
        pd.set("Annots", Object::Array(arr));
    }
    Ok(())
}

pub(crate) fn annot_dict_of(doc: &Document, o: &Object) -> Option<(Option<ObjectId>, Dictionary)> {
    match o {
        Object::Reference(r) => doc.get_dictionary(*r).ok().map(|d| (Some(*r), d.clone())),
        Object::Dictionary(d) => Some((None, d.clone())),
        _ => None,
    }
}

fn new_name(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    format!("{prefix}-{}-{}", crate::logx::unix_secs(), N.fetch_add(1, Ordering::Relaxed))
}

fn save(doc: &mut Document) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| format!("write: {e}"))?;
    Ok(out)
}

pub struct NewAnnot {
    pub page: usize,
    pub kind: AnnotKind,
    /// Rectangles (markups) or anchor point of the icon (notes: rectangle whose center is used).
    pub rects: Vec<R4>,
    pub color: [u8; 3],
    pub alpha: u8,
    pub author: String,
    pub contents: String,
}

/// Adds a markup or a note, with its appearance stream.
pub fn add_annotation(bytes: &[u8], a: &NewAnnot) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| format!("read: {e}"))?;
    let pid = page_id(&doc, a.page)?;
    let first = *a.rects.first().ok_or("no area")?;
    let bounds = a.rects.iter().skip(1).fold(first, |x, y| r4_union(&x, y));
    let alpha = a.alpha as f32 / 255.0;
    let mut d = Dictionary::new();
    d.set("Type", Object::Name(b"Annot".to_vec()));
    let (subtype, ap, rect): (&[u8], ObjectId, R4) = match a.kind {
        AnnotKind::Highlight | AnnotKind::Underline | AnnotKind::StrikeOut | AnnotKind::Squiggly => {
            let st: &[u8] = match a.kind {
                AnnotKind::Highlight => b"Highlight",
                AnnotKind::Underline => b"Underline",
                AnnotKind::StrikeOut => b"StrikeOut",
                _ => b"Squiggly",
            };
            let mut quads = vec![];
            for r in &a.rects {
                // Acrobat order: top-left, top-right, bottom-left, bottom-right
                for v in [r[0], r[3], r[2], r[3], r[0], r[1], r[2], r[1]] {
                    quads.push(f(v));
                }
            }
            d.set("QuadPoints", Object::Array(quads));
            let ap = make_ap(&mut doc, bounds, markup_ap_content(a.kind, &a.rects, a.color), alpha, a.kind == AnnotKind::Highlight);
            (st, ap, bounds)
        }
        AnnotKind::Note => {
            let (cx, cy) = ((first[0] + first[2]) * 0.5, (first[1] + first[3]) * 0.5);
            let ap = make_ap(&mut doc, [0.0, 0.0, 24.0, 24.0], note_ap_content(a.color), 1.0, false);
            d.set("Name", Object::Name(b"Comment".to_vec()));
            (b"Text", ap, [cx - 12.0, cy - 12.0, cx + 12.0, cy + 12.0])
        }
        _ => return Err("unsupported annotation type".into()),
    };
    d.set("Subtype", Object::Name(subtype.to_vec()));
    d.set("Rect", rect_obj(&rect));
    d.set("C", Object::Array(vec![f(a.color[0] as f32 / 255.0), f(a.color[1] as f32 / 255.0), f(a.color[2] as f32 / 255.0)]));
    d.set("CA", f(alpha));
    d.set("F", Object::Integer(4));
    d.set("T", pdf_text(&a.author));
    d.set("Contents", pdf_text(&a.contents));
    d.set("M", pdf_date());
    d.set("CreationDate", pdf_date());
    d.set("NM", pdf_text(&new_name("RP")));
    d.set("P", Object::Reference(pid));
    let mut apd = Dictionary::new();
    apd.set("N", Object::Reference(ap));
    d.set("AP", Object::Dictionary(apd));
    let id = doc.add_object(Object::Dictionary(d));
    let mut arr = annots_array(&doc, pid);
    arr.push(Object::Reference(id));
    set_annots_array(&mut doc, pid, arr)?;
    save(&mut doc)
}

/// An annotation read from the file (comments panel, edits).
#[derive(Clone, Debug)]
pub struct AnnotRec {
    pub page: usize,
    /// Position in the page's /Annots array.
    pub index: usize,
    pub id: Option<ObjectId>,
    pub subtype: String,
    pub contents: String,
    pub author: String,
    pub date: String,
    pub rect: R4,
    /// Parent annotation (for a reply).
    pub irt: Option<ObjectId>,
    pub color: [u8; 3],
}

pub fn list_annotations(bytes: &[u8]) -> Result<Vec<AnnotRec>, String> {
    let doc = Document::load_mem(bytes).map_err(|e| format!("read: {e}"))?;
    let mut out = vec![];
    for (pn, pid) in doc.get_pages() {
        for (i, o) in annots_array(&doc, pid).iter().enumerate() {
            let Some((id, d)) = annot_dict_of(&doc, o) else { continue };
            let subtype = d.get(b"Subtype").ok().map(text_of).unwrap_or_default();
            if matches!(subtype.as_str(), "Link" | "Widget" | "Popup") {
                continue;
            }
            let rect = d.get(b"Rect").ok().and_then(obj_to_rect).unwrap_or([0.0; 4]);
            let col = d
                .get(b"C")
                .ok()
                .and_then(|c| c.as_array().ok())
                .filter(|a| a.len() >= 3)
                .map(|a| [(a[0].as_float().unwrap_or(0.0) * 255.0) as u8, (a[1].as_float().unwrap_or(0.0) * 255.0) as u8, (a[2].as_float().unwrap_or(0.0) * 255.0) as u8])
                .unwrap_or([255, 220, 0]);
            out.push(AnnotRec {
                page: pn as usize - 1,
                index: i,
                id,
                subtype,
                contents: d.get(b"Contents").ok().map(text_of).unwrap_or_default(),
                author: d.get(b"T").ok().map(text_of).unwrap_or_default(),
                date: d.get(b"M").ok().map(text_of).unwrap_or_default(),
                rect,
                irt: d.get(b"IRT").ok().and_then(|r| r.as_reference().ok()),
                color: col,
            });
        }
    }
    Ok(out)
}

/// Finds the index (/Annots array) of an annotation from its type and rectangle.
pub fn find_annotation(bytes: &[u8], page: usize, subtype: &str, rect: &R4) -> Option<usize> {
    let recs = list_annotations(bytes).ok()?;
    let close = |a: &R4, b: &R4| (0..4).all(|k| (a[k] - b[k]).abs() < 1.5);
    recs.iter().filter(|r| r.page == page && r.subtype == subtype && close(&r.rect, rect)).map(|r| r.index).next()
}

fn modify_annot<F>(bytes: &[u8], page: usize, index: usize, fun: F) -> Result<Vec<u8>, String>
where
    F: FnOnce(&mut Document, &mut Dictionary) -> Result<(), String>,
{
    let mut doc = Document::load_mem(bytes).map_err(|e| format!("read: {e}"))?;
    let pid = page_id(&doc, page)?;
    let arr = annots_array(&doc, pid);
    let o = arr.get(index).ok_or("annotation not found")?.clone();
    match o {
        Object::Reference(r) => {
            let mut d = doc.get_dictionary(r).map_err(|e| e.to_string())?.clone();
            fun(&mut doc, &mut d)?;
            doc.objects.insert(r, Object::Dictionary(d));
        }
        Object::Dictionary(mut d) => {
            fun(&mut doc, &mut d)?;
            let mut arr = arr.clone();
            arr[index] = Object::Dictionary(d);
            set_annots_array(&mut doc, pid, arr)?;
        }
        _ => return Err("unreadable annotation".into()),
    }
    save(&mut doc)
}

pub fn set_annotation_contents(bytes: &[u8], page: usize, index: usize, text: &str) -> Result<Vec<u8>, String> {
    modify_annot(bytes, page, index, |_, d| {
        d.set("Contents", pdf_text(text));
        d.set("M", pdf_date());
        Ok(())
    })
}

/// Changes the color of a markup or a note by regenerating its appearance stream.
pub fn set_annotation_color(bytes: &[u8], page: usize, index: usize, color: [u8; 3], alpha: u8) -> Result<Vec<u8>, String> {
    modify_annot(bytes, page, index, |doc, d| {
        let subtype = d.get(b"Subtype").ok().map(text_of).unwrap_or_default();
        let kind = match subtype.as_str() {
            "Highlight" => AnnotKind::Highlight,
            "Underline" => AnnotKind::Underline,
            "StrikeOut" => AnnotKind::StrikeOut,
            "Squiggly" => AnnotKind::Squiggly,
            "Text" => AnnotKind::Note,
            _ => return Err("color not editable for this type".into()),
        };
        let rect = d.get(b"Rect").ok().and_then(obj_to_rect).unwrap_or([0.0; 4]);
        let a = alpha as f32 / 255.0;
        let ap = if kind == AnnotKind::Note {
            make_ap(doc, [0.0, 0.0, 24.0, 24.0], note_ap_content(color), 1.0, false)
        } else {
            let q = d.get(b"QuadPoints").ok().and_then(|o| o.as_array().ok()).cloned().unwrap_or_default();
            let v: Vec<f32> = q.iter().filter_map(|x| x.as_float().ok()).collect();
            let rects: Vec<R4> = v
                .chunks(8)
                .filter(|c| c.len() == 8)
                .map(|c| {
                    let xs = [c[0], c[2], c[4], c[6]];
                    let ys = [c[1], c[3], c[5], c[7]];
                    [xs.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max), ys.iter().cloned().fold(f32::MIN, f32::max)]
                })
                .collect();
            make_ap(doc, rect, markup_ap_content(kind, &rects, color), a, kind == AnnotKind::Highlight)
        };
        let mut apd = Dictionary::new();
        apd.set("N", Object::Reference(ap));
        d.set("AP", Object::Dictionary(apd));
        d.set("C", Object::Array(vec![f(color[0] as f32 / 255.0), f(color[1] as f32 / 255.0), f(color[2] as f32 / 255.0)]));
        d.set("CA", f(a));
        Ok(())
    })
}

/// Moves an annotation by (dx, dy): rectangle, anchor points and appearance.
pub fn move_annotation(bytes: &[u8], page: usize, index: usize, dx: f32, dy: f32) -> Result<Vec<u8>, String> {
    modify_annot(bytes, page, index, |doc, d| {
        let is_note = d.get(b"Subtype").ok().map(text_of).as_deref() == Some("Text");
        if let Some(r) = d.get(b"Rect").ok().and_then(obj_to_rect) {
            d.set("Rect", rect_obj(&[r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy]));
        }
        if let Ok(Object::Array(q)) = d.get(b"QuadPoints") {
            let moved: Vec<Object> = q.iter().enumerate().map(|(i, o)| f(o.as_float().unwrap_or(0.0) + if i % 2 == 0 { dx } else { dy })).collect();
            d.set("QuadPoints", Object::Array(moved));
        }
        // The appearance of a markup is in page coordinates: translate it through its matrix (the notes follow the Rect).
        if !is_note {
            if let Ok(Object::Dictionary(ap)) = d.get(b"AP") {
                if let Ok(Object::Reference(n)) = ap.get(b"N") {
                    if let Ok(Object::Stream(s)) = doc.get_object_mut(*n) {
                        let (cur_e, cur_f) = s
                            .dict
                            .get(b"Matrix")
                            .ok()
                            .and_then(|m| m.as_array().ok())
                            .filter(|m| m.len() == 6)
                            .map(|m| (m[4].as_float().unwrap_or(0.0), m[5].as_float().unwrap_or(0.0)))
                            .unwrap_or((0.0, 0.0));
                        s.dict.set("Matrix", Object::Array(vec![f(1.0), f(0.0), f(0.0), f(1.0), f(cur_e + dx), f(cur_f + dy)]));
                    }
                }
            }
        }
        Ok(())
    })
}

pub fn delete_annotation(bytes: &[u8], page: usize, index: usize) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| format!("read: {e}"))?;
    let pid = page_id(&doc, page)?;
    let mut arr = annots_array(&doc, pid);
    if index >= arr.len() {
        return Err("annotation not found".into());
    }
    let removed = arr.remove(index);
    // also removes the replies that reference it
    if let Object::Reference(rid) = removed {
        let replies: Vec<usize> = arr
            .iter()
            .enumerate()
            .filter(|(_, o)| annot_dict_of(&doc, o).and_then(|(_, d)| d.get(b"IRT").ok().and_then(|r| r.as_reference().ok())) == Some(rid))
            .map(|(i, _)| i)
            .collect();
        for i in replies.into_iter().rev() {
            arr.remove(i);
        }
    }
    set_annots_array(&mut doc, pid, arr)?;
    save(&mut doc)
}

/// Adds a reply to an annotation (discussion thread): hidden Text annotation with /IRT.
pub fn add_reply(bytes: &[u8], page: usize, parent_index: usize, text: &str, author: &str) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| format!("read: {e}"))?;
    let pid = page_id(&doc, page)?;
    let mut arr = annots_array(&doc, pid);
    let parent = arr.get(parent_index).ok_or("annotation not found")?.clone();
    let Object::Reference(pr) = parent else { return Err("annotation without identifier".into()) };
    let prect = doc.get_dictionary(pr).ok().and_then(|d| d.get(b"Rect").ok().and_then(obj_to_rect)).unwrap_or([0.0, 0.0, 24.0, 24.0]);
    let mut d = Dictionary::new();
    d.set("Type", Object::Name(b"Annot".to_vec()));
    d.set("Subtype", Object::Name(b"Text".to_vec()));
    d.set("Rect", rect_obj(&[prect[0], prect[3] - 24.0, prect[0] + 24.0, prect[3]]));
    d.set("Contents", pdf_text(text));
    d.set("T", pdf_text(author));
    d.set("IRT", Object::Reference(pr));
    d.set("RT", Object::Name(b"R".to_vec()));
    d.set("F", Object::Integer(2 | 4)); // hidden on display, printable
    d.set("M", pdf_date());
    d.set("NM", pdf_text(&new_name("RPR")));
    d.set("P", Object::Reference(pid));
    let id = doc.add_object(Object::Dictionary(d));
    arr.push(Object::Reference(id));
    set_annots_array(&mut doc, pid, arr)?;
    save(&mut doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfium_render::prelude::*;

    fn base() -> Vec<u8> {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let font = doc.fonts_mut().helvetica();
        page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), "Highlight text", font, PdfPoints::new(20.0)).unwrap();
        page.regenerate_content().unwrap();
        drop(page);
        doc.save_to_bytes().unwrap()
    }

    fn count_color(bytes: Vec<u8>, ok: impl Fn(&[u8]) -> bool, region: (usize, usize, usize, usize)) -> usize {
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(bytes, None).unwrap();
        let pg = doc.pages().get(0).unwrap();
        let img = crate::pdfx::render_full(&pg, 595, true).unwrap();
        let mut n = 0;
        for y in region.1..region.3 {
            for x in region.0..region.2 {
                if ok(&img.data[(y * img.w + x) * 4..][..3]) {
                    n += 1;
                }
            }
        }
        n
    }

    const REGION: (usize, usize, usize, usize) = (72, 125, 190, 150);

    #[test]
    fn highlight_is_drawn_by_pdfium() {
        let b = base();
        let none = count_color(b.clone(), |p| p[2] < 200 && p[0] > 200 && p[1] > 150, REGION);
        let a = NewAnnot { page: 0, kind: AnnotKind::Highlight, rects: vec![[72.0, 695.0, 190.0, 715.0]], color: [255, 235, 59], alpha: 255, author: "Me".into(), contents: String::new() };
        let out = add_annotation(&b, &a).unwrap();
        let yellow = count_color(out, |p| p[2] < 200 && p[0] > 200 && p[1] > 150, REGION);
        assert!(yellow > none + 1500, "yellow highlight expected: {none} → {yellow}");
    }

    #[test]
    fn underline_strikeout_squiggly_and_note_are_drawn() {
        let b = base();
        for (k, name) in [(AnnotKind::Underline, "Underline"), (AnnotKind::StrikeOut, "StrikeOut"), (AnnotKind::Squiggly, "Squiggly")] {
            let a = NewAnnot { page: 0, kind: k, rects: vec![[72.0, 695.0, 190.0, 715.0]], color: [255, 0, 0], alpha: 255, author: "Me".into(), contents: String::new() };
            let out = add_annotation(&b, &a).unwrap();
            let red = count_color(out, |p| p[0] as i32 > p[1] as i32 + 50 && p[0] as i32 > p[2] as i32 + 50, REGION);
            assert!(red > 40, "{name}: {red} red pixels");
        }
        let n = NewAnnot { page: 0, kind: AnnotKind::Note, rects: vec![[300.0, 500.0, 320.0, 520.0]], color: [255, 200, 0], alpha: 255, author: "Me".into(), contents: "Hello é".into() };
        let out = add_annotation(&b, &n).unwrap();
        let recs = list_annotations(&out).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].subtype, "Text");
        assert_eq!(recs[0].contents, "Hello é", "Unicode text read back");
        assert_eq!(recs[0].author, "Me");
        // the note is drawn (yellow icon around (310, 510))
        let yellow = count_color(out, |p| p[0] > 200 && p[1] > 150 && p[2] < 80, (298, 842 - 524, 322, 842 - 496));
        assert!(yellow > 100, "note icon drawn: {yellow}");
    }

    #[test]
    fn edit_move_recolor_reply_delete() {
        let b = base();
        let a = NewAnnot { page: 0, kind: AnnotKind::Highlight, rects: vec![[72.0, 695.0, 190.0, 715.0]], color: [255, 235, 59], alpha: 255, author: "Me".into(), contents: "first".into() };
        let b = add_annotation(&b, &a).unwrap();
        let idx = find_annotation(&b, 0, "Highlight", &[72.0, 695.0, 190.0, 715.0]).expect("found by its rectangle");
        let b = set_annotation_contents(&b, 0, idx, "edited").unwrap();
        assert_eq!(list_annotations(&b).unwrap()[0].contents, "edited");
        // move: the yellow ends up lower
        let b = move_annotation(&b, 0, idx, 0.0, -40.0).unwrap();
        let moved_rect = list_annotations(&b).unwrap()[0].rect;
        assert!((moved_rect[1] - 655.0).abs() < 0.01, "{moved_rect:?}");
        let yellow_old = count_color(b.clone(), |p| p[2] < 200 && p[0] > 200 && p[1] > 150, REGION);
        let yellow_new = count_color(b.clone(), |p| p[2] < 200 && p[0] > 200 && p[1] > 150, (72, 165, 190, 190));
        assert!(yellow_new > 1500 && yellow_old < 200, "appearance moved: old={yellow_old} new={yellow_new}");
        // recoloring to green
        let b = set_annotation_color(&b, 0, idx, [0, 200, 0], 255).unwrap();
        let green = count_color(b.clone(), |p| p[1] > 150 && p[0] < 100 && p[2] < 100, (72, 165, 190, 190));
        assert!(green > 1500, "green: {green}");
        // reply then deletion of the parent: the reply disappears with it
        let b = add_reply(&b, 0, idx, "my reply", "Other").unwrap();
        let recs = list_annotations(&b).unwrap();
        assert_eq!(recs.len(), 2);
        let reply = recs.iter().find(|r| r.contents == "my reply").unwrap();
        assert_eq!(reply.irt, recs.iter().find(|r| r.contents == "edited").unwrap().id);
        let b = delete_annotation(&b, 0, idx).unwrap();
        assert!(list_annotations(&b).unwrap().is_empty());
    }
}

/// Deletes all the annotations (including links and fields) whose rectangle touches one of the areas.
/// Returns the new bytes and the number of annotations removed.
pub fn delete_annotations_in(bytes: &[u8], page: usize, rects: &[R4]) -> Result<(Vec<u8>, usize), String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| format!("read: {e}"))?;
    let pid = page_id(&doc, page)?;
    let arr = annots_array(&doc, pid);
    let mut keep = vec![];
    let mut removed = 0;
    for o in arr {
        let hit = annot_dict_of(&doc, &o)
            .and_then(|(_, d)| d.get(b"Rect").ok().and_then(obj_to_rect))
            .map(|r| rects.iter().any(|z| crate::geom::r4_intersects(&r, z)))
            .unwrap_or(false);
        if hit {
            removed += 1;
        } else {
            keep.push(o);
        }
    }
    if removed == 0 {
        return Ok((bytes.to_vec(), 0));
    }
    set_annots_array(&mut doc, pid, keep)?;
    Ok((save(&mut doc)?, removed))
}
