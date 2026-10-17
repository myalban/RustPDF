//! Page structure at file level (`lopdf`): order, deletion, rotation, insertion, merge,
//! split, crop, page size change. Forms, bookmarks and annotations are kept.

use crate::geom::R4;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

fn load(b: &[u8]) -> Result<Document, String> {
    Document::load_mem(b).map_err(|e| format!("read: {e}"))
}

fn save(doc: &mut Document) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| format!("write: {e}"))?;
    Ok(out)
}

fn f(v: f32) -> Object {
    Object::Real(v)
}

fn name(s: &str) -> Object {
    Object::Name(s.as_bytes().to_vec())
}

/// Page identifiers, in document order.
pub fn page_ids(doc: &Document) -> Vec<ObjectId> {
    doc.get_pages().values().copied().collect()
}

fn pages_root(doc: &Document) -> Result<ObjectId, String> {
    let root = doc.trailer.get(b"Root").and_then(|r| r.as_reference()).map_err(|e| e.to_string())?;
    doc.get_dictionary(root).and_then(|c| c.get(b"Pages")).and_then(|p| p.as_reference()).map_err(|e| e.to_string())
}

/// Inherited attributes (resources, boxes, rotation) copied onto each page, then the page tree flattened.
pub fn flatten_pages(doc: &mut Document) -> Result<(), String> {
    let ids = page_ids(doc);
    let root = pages_root(doc)?;
    for pid in &ids {
        let mut inherited: Vec<(&[u8], Object)> = vec![];
        for key in [&b"Resources"[..], b"MediaBox", b"CropBox", b"Rotate"] {
            let mut cur = *pid;
            for _ in 0..32 {
                let Ok(d) = doc.get_dictionary(cur) else { break };
                if let Ok(v) = d.get(key) {
                    inherited.push((key, v.clone()));
                    break;
                }
                match d.get(b"Parent").ok().and_then(|p| p.as_reference().ok()) {
                    Some(p) => cur = p,
                    None => break,
                }
            }
        }
        let pd = doc.get_object_mut(*pid).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
        for (k, v) in inherited {
            if !pd.has(k) {
                pd.set(k, v);
            }
        }
        pd.set("Parent", Object::Reference(root));
    }
    let rd = doc.get_object_mut(root).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
    rd.set("Kids", Object::Array(ids.iter().map(|i| Object::Reference(*i)).collect()));
    rd.set("Count", Object::Integer(ids.len() as i64));
    Ok(())
}

fn set_page_order(doc: &mut Document, order: &[ObjectId]) -> Result<(), String> {
    let root = pages_root(doc)?;
    let rd = doc.get_object_mut(root).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
    rd.set("Kids", Object::Array(order.iter().map(|i| Object::Reference(*i)).collect()));
    rd.set("Count", Object::Integer(order.len() as i64));
    Ok(())
}

/// Removes from the form fields the widgets whose page no longer exists.
fn cleanup_acroform(doc: &mut Document) {
    let keep: std::collections::HashSet<ObjectId> = page_ids(doc).into_iter().collect();
    let Ok(root) = doc.trailer.get(b"Root").and_then(|r| r.as_reference()) else { return };
    let Some(af) = doc.get_dictionary(root).ok().and_then(|c| c.get(b"AcroForm").ok().and_then(|a| a.as_reference().ok())) else { return };
    let fields = doc.get_dictionary(af).ok().and_then(|d| d.get(b"Fields").ok().and_then(|f| f.as_array().ok().cloned())).unwrap_or_default();
    let alive = |doc: &Document, id: ObjectId| -> bool {
        // a field is kept if at least one of its widgets is on a kept page (or without /P)
        let mut stack = vec![id];
        let mut any_widget = false;
        while let Some(cur) = stack.pop() {
            let Ok(d) = doc.get_dictionary(cur) else { continue };
            if let Ok(Object::Array(kids)) = d.get(b"Kids") {
                for k in kids {
                    if let Ok(r) = k.as_reference() {
                        stack.push(r);
                    }
                }
                continue;
            }
            any_widget = true;
            match d.get(b"P").ok().and_then(|p| p.as_reference().ok()) {
                Some(p) => {
                    if keep.contains(&p) {
                        return true;
                    }
                }
                None => return true,
            }
        }
        !any_widget
    };
    let kept: Vec<Object> = fields.into_iter().filter(|o| o.as_reference().map(|r| alive(doc, r)).unwrap_or(true)).collect();
    if let Ok(d) = doc.get_object_mut(af).and_then(|o| o.as_dict_mut()) {
        d.set("Fields", Object::Array(kept));
    }
}

/// Keeps only the pages `keep` (0-based indices), in the given order.
pub fn keep_pages(bytes: &[u8], keep: &[usize]) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    flatten_pages(&mut doc)?;
    let ids = page_ids(&doc);
    let mut order = vec![];
    for &i in keep {
        order.push(*ids.get(i).ok_or("page does not exist")?);
    }
    if order.is_empty() {
        return Err("no page to keep".into());
    }
    set_page_order(&mut doc, &order)?;
    cleanup_acroform(&mut doc);
    doc.prune_objects();
    save(&mut doc)
}

pub fn delete_pages(bytes: &[u8], del: &[usize]) -> Result<Vec<u8>, String> {
    let doc = load(bytes)?;
    let n = page_ids(&doc).len();
    let keep: Vec<usize> = (0..n).filter(|i| !del.contains(i)).collect();
    if keep.is_empty() {
        return Err("cannot delete all the pages".into());
    }
    keep_pages(bytes, &keep)
}

/// New order: `order[k]` = old index of the page placed at position k (permutation or subset).
pub fn reorder_pages(bytes: &[u8], order: &[usize]) -> Result<Vec<u8>, String> {
    keep_pages(bytes, order)
}

pub fn rotate_pages(bytes: &[u8], pages: &[usize], delta: i32) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    flatten_pages(&mut doc)?;
    let ids = page_ids(&doc);
    for &i in pages {
        let id = *ids.get(i).ok_or("page does not exist")?;
        let pd = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
        let cur = pd.get(b"Rotate").ok().and_then(|r| r.as_i64().ok()).unwrap_or(0);
        pd.set("Rotate", Object::Integer((cur + delta as i64).rem_euclid(360)));
    }
    save(&mut doc)
}

/// Inserts a blank page of `w`×`h` points at position `at`.
pub fn insert_blank(bytes: &[u8], at: usize, w: f32, h: f32) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    flatten_pages(&mut doc)?;
    let root = pages_root(&doc)?;
    let mut ids = page_ids(&doc);
    let content = doc.add_object(Stream::new(Dictionary::new(), Vec::new()));
    let mut pd = Dictionary::new();
    pd.set("Type", name("Page"));
    pd.set("Parent", Object::Reference(root));
    pd.set("MediaBox", Object::Array(vec![f(0.0), f(0.0), f(w), f(h)]));
    pd.set("Resources", Object::Dictionary(Dictionary::new()));
    pd.set("Contents", Object::Reference(content));
    let id = doc.add_object(Object::Dictionary(pd));
    ids.insert(at.min(ids.len()), id);
    set_page_order(&mut doc, &ids)?;
    save(&mut doc)
}

/// Imports pages from another document (0-based indices) starting at position `at`.
pub fn insert_from(bytes: &[u8], other: &[u8], other_pages: &[usize], at: usize) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    flatten_pages(&mut doc)?;
    let mut src = load(other)?;
    flatten_pages(&mut src)?;
    src.renumber_objects_with(doc.max_id + 1);
    let src_ids = page_ids(&src);
    let chosen: Vec<ObjectId> = other_pages.iter().map(|&i| src_ids.get(i).copied().ok_or("page does not exist")).collect::<Result<_, _>>()?;
    if chosen.is_empty() {
        return Err("no page to insert".into());
    }
    // form fields of the source document
    let src_fields = source_fields(&src, &chosen);
    let src_dr = source_dr(&src);
    doc.max_id = doc.max_id.max(src.max_id);
    for (id, obj) in src.objects.iter() {
        doc.objects.insert(*id, obj.clone());
    }
    let root = pages_root(&doc)?;
    for id in &chosen {
        if let Ok(pd) = doc.get_object_mut(*id).and_then(|o| o.as_dict_mut()) {
            pd.set("Parent", Object::Reference(root));
        }
    }
    let mut ids = page_ids(&doc);
    // `page_ids` still sees the old tree: rebuild from the old order + imported pages
    ids.retain(|i| !chosen.contains(i));
    let pos = at.min(ids.len());
    for (k, id) in chosen.iter().enumerate() {
        ids.insert(pos + k, *id);
    }
    set_page_order(&mut doc, &ids)?;
    merge_acroform(&mut doc, src_fields, src_dr)?;
    doc.prune_objects();
    save(&mut doc)
}

/// Form fields (references) of the source document that have a widget on one of the imported pages.
fn source_fields(src: &Document, pages: &[ObjectId]) -> Vec<Object> {
    let Ok(root) = src.trailer.get(b"Root").and_then(|r| r.as_reference()) else { return vec![] };
    let Some(af) = src.get_dictionary(root).ok().and_then(|c| c.get(b"AcroForm").ok().cloned()) else { return vec![] };
    let afd = match af {
        Object::Reference(r) => src.get_dictionary(r).ok().cloned(),
        Object::Dictionary(d) => Some(d),
        _ => None,
    };
    let Some(afd) = afd else { return vec![] };
    let fields = afd.get(b"Fields").ok().and_then(|f| f.as_array().ok().cloned()).unwrap_or_default();
    let on_pages = |id: ObjectId| -> bool {
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            let Ok(d) = src.get_dictionary(cur) else { continue };
            if let Ok(Object::Array(kids)) = d.get(b"Kids") {
                stack.extend(kids.iter().filter_map(|k| k.as_reference().ok()));
            } else if let Some(p) = d.get(b"P").ok().and_then(|p| p.as_reference().ok()) {
                if pages.contains(&p) {
                    return true;
                }
            }
        }
        false
    };
    fields.into_iter().filter(|o| o.as_reference().map(on_pages).unwrap_or(false)).collect()
}

fn source_dr(src: &Document) -> Option<Dictionary> {
    let root = src.trailer.get(b"Root").and_then(|r| r.as_reference()).ok()?;
    let af = src.get_dictionary(root).ok()?.get(b"AcroForm").ok()?.clone();
    let afd = match af {
        Object::Reference(r) => src.get_dictionary(r).ok()?.clone(),
        Object::Dictionary(d) => d,
        _ => return None,
    };
    match afd.get(b"DR").ok()? {
        Object::Dictionary(d) => Some(d.clone()),
        Object::Reference(r) => src.get_dictionary(*r).ok().cloned(),
        _ => None,
    }
}

fn merge_acroform(doc: &mut Document, fields: Vec<Object>, dr: Option<Dictionary>) -> Result<(), String> {
    if fields.is_empty() {
        return Ok(());
    }
    let root = doc.trailer.get(b"Root").and_then(|r| r.as_reference()).map_err(|e| e.to_string())?;
    let existing = doc.get_dictionary(root).ok().and_then(|c| c.get(b"AcroForm").ok().cloned());
    let af_id = match existing {
        Some(Object::Reference(r)) => r,
        _ => {
            let id = doc.add_object(Object::Dictionary(Dictionary::new()));
            doc.get_object_mut(root).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("AcroForm", Object::Reference(id));
            id
        }
    };
    // names already used
    let used: Vec<String> = doc
        .get_dictionary(af_id)
        .ok()
        .and_then(|d| d.get(b"Fields").ok().and_then(|f| f.as_array().ok().cloned()))
        .unwrap_or_default()
        .iter()
        .filter_map(|o| o.as_reference().ok())
        .filter_map(|r| doc.get_dictionary(r).ok().and_then(|d| d.get(b"T").ok().map(crate::pdfedit_annot::text_of)))
        .collect();
    for o in &fields {
        if let Ok(r) = o.as_reference() {
            if let Ok(d) = doc.get_object_mut(r).and_then(|o| o.as_dict_mut()) {
                if let Some(t) = d.get(b"T").ok().map(crate::pdfedit_annot::text_of) {
                    if used.contains(&t) {
                        d.set("T", crate::pdfedit_annot::pdf_text(&format!("{t}_2")));
                    }
                }
            }
        }
    }
    let afd = doc.get_object_mut(af_id).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
    let mut all = afd.get(b"Fields").ok().and_then(|f| f.as_array().ok().cloned()).unwrap_or_default();
    all.extend(fields);
    afd.set("Fields", Object::Array(all));
    if !afd.has(b"DA") {
        afd.set("DA", Object::String(b"/Helv 11 Tf 0 g".to_vec(), lopdf::StringFormat::Literal));
    }
    afd.set("NeedAppearances", Object::Boolean(true));
    if let Some(src_dr) = dr {
        let mut cur = match afd.get(b"DR") {
            Ok(Object::Dictionary(d)) => d.clone(),
            _ => Dictionary::new(),
        };
        for (k, v) in src_dr.iter() {
            if !cur.has(k) {
                cur.set(k.clone(), v.clone());
            } else if let (Ok(Object::Dictionary(a)), Object::Dictionary(b)) = (cur.get(k).cloned(), v.clone()) {
                let mut merged = a;
                for (kk, vv) in b.iter() {
                    if !merged.has(kk) {
                        merged.set(kk.clone(), vv.clone());
                    }
                }
                cur.set(k.clone(), Object::Dictionary(merged));
            }
        }
        afd.set("DR", Object::Dictionary(cur));
    }
    Ok(())
}

/// Merges several documents (in the given order).
pub fn merge_documents(files: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let first = files.first().ok_or("no file")?;
    let mut acc = first.clone();
    for other in &files[1..] {
        let n = page_ids(&load(&acc)?).len();
        let m = page_ids(&load(other)?).len();
        acc = insert_from(&acc, other, &(0..m).collect::<Vec<_>>(), n)?;
    }
    Ok(acc)
}

/// Splits into several documents; each entry is a list of pages (0-based).
pub fn split_documents(bytes: &[u8], groups: &[Vec<usize>]) -> Result<Vec<Vec<u8>>, String> {
    groups.iter().map(|g| keep_pages(bytes, g)).collect()
}

/// Crop box: margins (left, bottom, right, top) removed from the current box, or an absolute rectangle.
pub fn crop_pages(bytes: &[u8], pages: &[usize], margins: Option<[f32; 4]>, abs: Option<R4>) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    flatten_pages(&mut doc)?;
    let ids = page_ids(&doc);
    for &i in pages {
        let id = *ids.get(i).ok_or("page does not exist")?;
        let pd = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
        let cur = pd
            .get(b"CropBox")
            .ok()
            .or_else(|| pd.get(b"MediaBox").ok())
            .and_then(|o| o.as_array().ok())
            .map(|a| a.iter().filter_map(|x| x.as_float().ok()).collect::<Vec<f32>>())
            .filter(|v| v.len() == 4)
            .unwrap_or_else(|| vec![0.0, 0.0, 595.0, 842.0]);
        let base = [cur[0].min(cur[2]), cur[1].min(cur[3]), cur[0].max(cur[2]), cur[1].max(cur[3])];
        let new = match (abs, margins) {
            (Some(r), _) => r,
            (None, Some(m)) => [base[0] + m[0], base[1] + m[1], base[2] - m[2], base[3] - m[3]],
            _ => return Err("no margin".into()),
        };
        if new[2] - new[0] < 10.0 || new[3] - new[1] < 10.0 {
            return Err("the crop area is too small".into());
        }
        pd.set("CropBox", Object::Array(vec![f(new[0]), f(new[1]), f(new[2]), f(new[3])]));
    }
    save(&mut doc)
}

/// Changes the page size by scaling the content (centered, proportions kept if `fit`).
pub fn resize_pages(bytes: &[u8], pages: &[usize], new_w: f32, new_h: f32, fit: bool) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    flatten_pages(&mut doc)?;
    let ids = page_ids(&doc);
    for &i in pages {
        let id = *ids.get(i).ok_or("page does not exist")?;
        let (base, old_cm) = {
            let pd = doc.get_dictionary(id).map_err(|e| e.to_string())?;
            let mb = pd
                .get(b"CropBox")
                .ok()
                .or_else(|| pd.get(b"MediaBox").ok())
                .and_then(|o| o.as_array().ok())
                .map(|a| a.iter().filter_map(|x| x.as_float().ok()).collect::<Vec<f32>>())
                .filter(|v| v.len() == 4)
                .unwrap_or_else(|| vec![0.0, 0.0, 595.0, 842.0]);
            ([mb[0].min(mb[2]), mb[1].min(mb[3]), mb[0].max(mb[2]), mb[1].max(mb[3])], ())
        };
        let _ = old_cm;
        let (ow, oh) = (base[2] - base[0], base[3] - base[1]);
        let (sx, sy) = if fit {
            let s = (new_w / ow).min(new_h / oh);
            (s, s)
        } else {
            (new_w / ow, new_h / oh)
        };
        let tx = (new_w - ow * sx) / 2.0 - base[0] * sx;
        let ty = (new_h - oh * sy) / 2.0 - base[1] * sy;
        // wrap the content: "q sx 0 0 sy tx ty cm" … "Q"
        let pre = doc.add_object(Stream::new(Dictionary::new(), format!("q {sx:.5} 0 0 {sy:.5} {tx:.3} {ty:.3} cm\n").into_bytes()));
        let post = doc.add_object(Stream::new(Dictionary::new(), b"\nQ\n".to_vec()));
        let existing = doc.get_dictionary(id).ok().and_then(|d| d.get(b"Contents").ok().cloned());
        let mut arr = vec![Object::Reference(pre)];
        match existing {
            Some(Object::Reference(r)) => {
                // may reference an array or a stream
                match doc.get_object(r) {
                    Ok(Object::Array(a)) => arr.extend(a.clone()),
                    _ => arr.push(Object::Reference(r)),
                }
            }
            Some(Object::Array(a)) => arr.extend(a),
            _ => {}
        }
        arr.push(Object::Reference(post));
        // annotations: scale /Rect and /QuadPoints
        let annots = crate::pdfedit_annot::annots_array(&doc, id);
        for a in annots {
            let (Object::Reference(r), _) = (a, ()) else { continue };
            if let Ok(d) = doc.get_object_mut(r).and_then(|o| o.as_dict_mut()) {
                for key in [&b"Rect"[..], b"QuadPoints"] {
                    if let Ok(Object::Array(v)) = d.get(key).map(|o| o.clone()) {
                        let nv: Vec<Object> = v
                            .iter()
                            .enumerate()
                            .map(|(k, o)| {
                                let x = o.as_float().unwrap_or(0.0);
                                if k % 2 == 0 { f(x * sx + tx) } else { f(x * sy + ty) }
                            })
                            .collect();
                        d.set(key, Object::Array(nv));
                    }
                }
            }
        }
        let pd = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
        pd.set("Contents", Object::Array(arr));
        pd.set("MediaBox", Object::Array(vec![f(0.0), f(0.0), f(new_w), f(new_h)]));
        pd.remove(b"CropBox");
    }
    save(&mut doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfium_render::prelude::*;

    /// Document of `n` pages; page i contains the text "Page i".
    fn doc_bytes(n: usize) -> Vec<u8> {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        for i in 0..n {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
            let font = doc.fonts_mut().helvetica();
            page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), format!("Page {i}"), font, PdfPoints::new(24.0)).unwrap();
            page.regenerate_content().unwrap();
        }
        doc.save_to_bytes().unwrap()
    }

    fn texts(bytes: &[u8]) -> Vec<String> {
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(bytes.to_vec(), None).unwrap();
        (0..doc.pages().len()).map(|i| doc.pages().get(i).unwrap().text().unwrap().all().trim().to_string()).collect()
    }

    #[test]
    fn reorder_delete_keep() {
        let b = doc_bytes(4);
        assert_eq!(texts(&reorder_pages(&b, &[3, 0, 2, 1]).unwrap()), vec!["Page 3", "Page 0", "Page 2", "Page 1"]);
        assert_eq!(texts(&delete_pages(&b, &[1, 2]).unwrap()), vec!["Page 0", "Page 3"]);
        assert_eq!(texts(&keep_pages(&b, &[2]).unwrap()), vec!["Page 2"]);
        assert!(delete_pages(&b, &[0, 1, 2, 3]).is_err(), "cannot delete all the pages");
    }

    #[test]
    fn rotate_and_blank_and_crop() {
        let b = doc_bytes(2);
        let r = rotate_pages(&b, &[1], 90).unwrap();
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(r.clone(), None).unwrap();
        assert!(doc.pages().get(1).unwrap().width().value > doc.pages().get(1).unwrap().height().value, "page 2 rotated to landscape");
        assert!(doc.pages().get(0).unwrap().width().value < doc.pages().get(0).unwrap().height().value);
        let r2 = rotate_pages(&r, &[1], 270).unwrap();
        let doc = p.load_pdf_from_byte_vec(r2, None).unwrap();
        assert!(doc.pages().get(1).unwrap().width().value < doc.pages().get(1).unwrap().height().value, "back to portrait");
        let blank = insert_blank(&b, 1, 300.0, 400.0).unwrap();
        let doc = p.load_pdf_from_byte_vec(blank.clone(), None).unwrap();
        assert_eq!(doc.pages().len(), 3);
        assert!((doc.pages().get(1).unwrap().width().value - 300.0).abs() < 0.1);
        assert_eq!(texts(&blank), vec!["Page 0", "", "Page 1"]);
        let cropped = crop_pages(&b, &[0], Some([50.0, 50.0, 50.0, 50.0]), None).unwrap();
        let doc = p.load_pdf_from_byte_vec(cropped, None).unwrap();
        assert!((doc.pages().get(0).unwrap().width().value - (595.2756 - 100.0)).abs() < 0.5);
        assert!((doc.pages().get(1).unwrap().width().value - 595.2756).abs() < 0.5, "the other page is not cropped");
    }

    #[test]
    fn insert_merge_and_split() {
        let a = doc_bytes(2);
        let b = doc_bytes(3);
        let m = insert_from(&a, &b, &[0, 2], 1).unwrap();
        assert_eq!(texts(&m), vec!["Page 0", "Page 0", "Page 2", "Page 1"]);
        let merged = merge_documents(&[a.clone(), b.clone(), a.clone()]).unwrap();
        assert_eq!(texts(&merged).len(), 7);
        let parts = split_documents(&merged, &[vec![0, 1], vec![2, 3, 4], vec![5, 6]]).unwrap();
        assert_eq!(parts.iter().map(|p| texts(p).len()).collect::<Vec<_>>(), vec![2, 3, 2]);
        assert_eq!(texts(&parts[1]), vec!["Page 0", "Page 1", "Page 2"]);
    }

    #[test]
    fn resize_scales_content_and_page() {
        let b = doc_bytes(1);
        let r = resize_pages(&b, &[0], 420.0, 595.0, true).unwrap();
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(r, None).unwrap();
        let pg = doc.pages().get(0).unwrap();
        assert!((pg.width().value - 420.0).abs() < 0.1 && (pg.height().value - 595.0).abs() < 0.1);
        let t = pg.objects().get(0).unwrap().bounds().unwrap().to_rect();
        // the text (at x=72, 24 pt) is reduced by about 0.706
        assert!((t.left().value - 72.0 * 0.7057).abs() < 6.0, "{:?}", t);
        assert!(t.width().value < 150.0);
    }

    #[test]
    fn merging_keeps_form_fields() {
        // a document with a field + a simple document
        let p = crate::pdfx::test_pdfium();
        let mut d = p.create_new_pdf().unwrap();
        d.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let mut t = crate::tab::Tab::new(p, d, None, None, 1.0);
        t.create_field(
            &crate::pdfedit_forms::NewField {
                page: 0,
                kind: crate::model::FieldKind::Text,
                rect: [100.0, 700.0, 300.0, 720.0],
                name: "name".into(),
                options: vec![],
                value: "x".into(),
                read_only: false,
                required: false,
                multiline: false,
            },
            100,
        )
        .unwrap();
        let with_field = t.doc.save_to_bytes().unwrap();
        let plain = doc_bytes(1);
        // field in second position: it must stay usable; two fields with the same name are renamed
        let m = merge_documents(&[plain, with_field.clone(), with_field]).unwrap();
        let doc = p.load_pdf_from_byte_vec(m, None).unwrap();
        assert_eq!(doc.pages().len(), 3);
        let mut t2 = crate::tab::Tab::new(p, doc, None, None, 1.0);
        let names: Vec<String> = (0..3).flat_map(|i| t2.info(i).map(|inf| inf.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>()).unwrap_or_default()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert_ne!(names[0], names[1], "distinct names after merging: {names:?}");
    }
}
