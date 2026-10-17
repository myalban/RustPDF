//! Operations on pages and on the document, applied to a tab (undoable).

use crate::deskew;
use crate::edit_text::TextStyle;
use crate::fonts::Builtin;
use crate::geom::*;
use crate::model::*;
use crate::pdfedit_doc::{self, Bm, CleanOpts, DocInfo, Layer, LinkDest, Security, WatermarkSpec};
use crate::pdfedit_pages as pp;
use crate::tab::Tab;
use pdfium_render::prelude::*;

fn err<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

/// Parses a page list "1-3, 5, 8-" (numbering from 1) → 0-based indices, without duplicates, sorted.
pub fn parse_ranges(s: &str, n: usize) -> Result<Vec<usize>, String> {
    let mut out: Vec<usize> = vec![];
    for part in s.split(|c| c == ',' || c == ';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim(), Some(b.trim())),
            None => (part, None),
        };
        let lo: usize = if a.is_empty() { 1 } else { a.parse().map_err(|_| format!("\"{part}\" is not a valid page"))? };
        let hi: usize = match b {
            None => lo,
            Some("") => n,
            Some(b) => b.parse().map_err(|_| format!("\"{part}\" is not a valid page"))?,
        };
        if lo == 0 || hi == 0 || lo > hi || hi > n {
            return Err(format!("\"{part}\" is outside the document (1–{n})"));
        }
        out.extend((lo - 1)..hi);
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err("no page specified".into());
    }
    Ok(out)
}

/// Inverse of `parse_ranges`: [0,1,2,4] → "1-3, 5".
pub fn format_ranges(pages: &[usize]) -> String {
    let mut v = pages.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut parts = vec![];
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
            j += 1;
        }
        parts.push(if i == j { format!("{}", v[i] + 1) } else { format!("{}-{}", v[i] + 1, v[j] + 1) });
        i = j + 1;
    }
    parts.join(", ")
}

#[derive(Clone, Debug, PartialEq)]
pub enum SplitMode {
    /// Each entry is a list of pages.
    Ranges(Vec<Vec<usize>>),
    EveryN(usize),
    /// Maximum size of a file (bytes).
    MaxBytes(usize),
    /// One file per top-level bookmark.
    ByBookmarks,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VAlign {
    Top,
    Bottom,
}

#[derive(Clone, Debug)]
pub struct StampItem {
    /// Text template: {n} number, {N} total, {date}, {file}, {title}.
    pub template: String,
    pub h: HAlign,
    pub v: VAlign,
}

#[derive(Clone, Debug)]
pub struct StampSpec {
    pub items: Vec<StampItem>,
    pub style: TextStyle,
    pub margin: f32,
    pub start: i32,
    pub skip_first: bool,
}

/// Approximate width of a text for a standard font.
pub fn text_width(text: &str, size: f32, family: Builtin) -> f32 {
    match family {
        Builtin::Courier => text.chars().count() as f32 * size * 0.6,
        Builtin::Times => pdfedit_doc::helvetica_width(text, size) * 0.92,
        Builtin::Helvetica => pdfedit_doc::helvetica_width(text, size),
    }
}

fn today() -> String {
    let t = crate::logx::timestamp(); // AAAA-MM-JJ HH:MM:SS
    let d: Vec<&str> = t[..10].split('-').collect();
    format!("{}/{}/{}", d[2], d[1], d[0])
}

impl Tab {
    fn apply_bytes<F: FnOnce(&[u8]) -> Result<Vec<u8>, String>>(&mut self, label: &str, mem: u32, f: F) -> Result<(), String> {
        let b = self.doc.save_to_bytes().map_err(err)?;
        let out = f(&b)?;
        self.replace_with_bytes(label, mem, out)
    }

    // ------------------------------------------------------------------ pages

    /// Moves pages: they are placed before page `to` (index in the current order; `n` = at the end).
    pub fn move_pages_to(&mut self, pages: &[usize], to: usize, mem: u32) -> Result<(), String> {
        let n = self.page_count();
        let mut moved: Vec<usize> = pages.iter().copied().filter(|&p| p < n).collect();
        moved.sort_unstable();
        moved.dedup();
        if moved.is_empty() {
            return Ok(());
        }
        let rest: Vec<usize> = (0..n).filter(|p| !moved.contains(p)).collect();
        let before = moved.iter().filter(|&&p| p < to).count();
        let pos = to.saturating_sub(before).min(rest.len());
        let mut order = rest;
        for (k, p) in moved.iter().enumerate() {
            order.insert(pos + k, *p);
        }
        if order.iter().enumerate().all(|(i, p)| i == *p) {
            return Ok(());
        }
        let first_moved_new = pos;
        self.apply_bytes("Move pages", mem, |b| pp::reorder_pages(b, &order))?;
        self.view.page = first_moved_new.min(self.page_count().saturating_sub(1));
        self.view.scroll_to = Some(self.view.page);
        Ok(())
    }

    pub fn delete_page_list(&mut self, pages: &[usize], mem: u32) -> Result<(), String> {
        self.apply_bytes("Delete pages", mem, |b| pp::delete_pages(b, pages))?;
        let n = self.page_count();
        self.view.page = self.view.page.min(n.saturating_sub(1));
        self.view.scroll_to = Some(self.view.page);
        Ok(())
    }

    pub fn rotate_page_list(&mut self, pages: &[usize], delta: i32, mem: u32) -> Result<(), String> {
        self.apply_bytes("Rotate pages", mem, |b| pp::rotate_pages(b, pages, delta))
    }

    pub fn insert_blank_at(&mut self, at: usize, w: f32, h: f32, mem: u32) -> Result<(), String> {
        self.apply_bytes("Insert a blank page", mem, |b| pp::insert_blank(b, at, w, h))?;
        self.view.page = at.min(self.page_count().saturating_sub(1));
        self.view.scroll_to = Some(self.view.page);
        Ok(())
    }

    pub fn insert_from_bytes(&mut self, other: &[u8], pages: &[usize], at: usize, mem: u32) -> Result<(), String> {
        self.apply_bytes("Insert pages", mem, |b| pp::insert_from(b, other, pages, at))
    }

    pub fn merge_bytes(&mut self, others: &[Vec<u8>], mem: u32) -> Result<(), String> {
        self.apply_bytes("Merge documents", mem, |b| {
            let mut all = vec![b.to_vec()];
            all.extend(others.iter().cloned());
            pp::merge_documents(&all)
        })
    }

    /// Bytes of a document containing only the requested pages.
    pub fn extract_bytes(&self, pages: &[usize]) -> Result<Vec<u8>, String> {
        let b = self.doc.save_to_bytes().map_err(err)?;
        pp::keep_pages(&b, pages)
    }

    /// Computes the page groups of a split.
    pub fn split_groups(&self, mode: &SplitMode) -> Result<Vec<Vec<usize>>, String> {
        let n = self.page_count();
        match mode {
            SplitMode::Ranges(r) => Ok(r.clone()),
            SplitMode::EveryN(k) => {
                if *k == 0 {
                    return Err("invalid number of pages".into());
                }
                Ok((0..n).collect::<Vec<_>>().chunks(*k).map(|c| c.to_vec()).collect())
            }
            SplitMode::ByBookmarks => {
                let b = self.doc.save_to_bytes().map_err(err)?;
                let mut starts: Vec<usize> = pdfedit_doc::read_bookmarks(&b).iter().map(|x| x.page.min(n.saturating_sub(1))).collect();
                starts.sort_unstable();
                starts.dedup();
                if starts.first() != Some(&0) {
                    starts.insert(0, 0);
                }
                let mut groups = vec![];
                for (i, s) in starts.iter().enumerate() {
                    let e = starts.get(i + 1).copied().unwrap_or(n);
                    groups.push((*s..e).collect());
                }
                Ok(groups)
            }
            SplitMode::MaxBytes(limit) => {
                let b = self.doc.save_to_bytes().map_err(err)?;
                let mut groups: Vec<Vec<usize>> = vec![];
                let mut cur: Vec<usize> = vec![];
                for p in 0..n {
                    let mut trial = cur.clone();
                    trial.push(p);
                    let size = pp::keep_pages(&b, &trial)?.len();
                    if size > *limit && !cur.is_empty() {
                        groups.push(std::mem::take(&mut cur));
                        cur.push(p);
                    } else {
                        cur = trial;
                    }
                }
                if !cur.is_empty() {
                    groups.push(cur);
                }
                Ok(groups)
            }
        }
    }

    pub fn split_bytes(&self, groups: &[Vec<usize>]) -> Result<Vec<Vec<u8>>, String> {
        let b = self.doc.save_to_bytes().map_err(err)?;
        pp::split_documents(&b, groups)
    }

    pub fn crop_page_list(&mut self, pages: &[usize], margins: Option<[f32; 4]>, abs: Option<R4>, mem: u32) -> Result<(), String> {
        self.apply_bytes("Crop pages", mem, |b| pp::crop_pages(b, pages, margins, abs))
    }

    pub fn resize_page_list(&mut self, pages: &[usize], w: f32, h: f32, fit: bool, mem: u32) -> Result<(), String> {
        self.apply_bytes("Format de page", mem, |b| pp::resize_pages(b, pages, w, h, fit))
    }

    // ------------------------------------------- page numbers, headers, footers

    /// Adds texts (page numbers, headers, footers) on the given pages.
    pub fn stamp_pages(&mut self, pages: &[usize], spec: &StampSpec, mem: u32) -> Result<usize, String> {
        self.snapshot("Headers and footers", mem);
        let total = self.page_count();
        let file = self.title.clone();
        let title = pdfedit_doc::read_info(&self.doc.save_to_bytes().map_err(err)?).title;
        let date = today();
        let mut count = 0;
        for (k, &p) in pages.iter().enumerate() {
            if spec.skip_first && k == 0 {
                continue;
            }
            let num = spec.start + k as i32 - if spec.skip_first { 1 } else { 0 };
            let info = self.info(p).ok_or("page not found")?;
            let geom = info.geom;
            let rot = self.page_rotation(p);
            let st = spec.style.clone();
            let items = spec.items.clone();
            let margin = spec.margin;
            let fam = match &st.family {
                crate::fonts::FontFamily::Builtin(b) => *b,
                _ => Builtin::Helvetica,
            };
            let r = self.edit_page_impl(p, "Headers and footers", mem, false, |pg, doc| {
                let tok = {
                    let f = doc.fonts_mut();
                    match (fam, st.bold, st.italic) {
                        (Builtin::Helvetica, false, false) => f.helvetica(),
                        (Builtin::Helvetica, true, false) => f.helvetica_bold(),
                        (Builtin::Helvetica, false, true) => f.helvetica_oblique(),
                        (Builtin::Helvetica, true, true) => f.helvetica_bold_oblique(),
                        (Builtin::Times, false, false) => f.times_roman(),
                        (Builtin::Times, true, false) => f.times_bold(),
                        (Builtin::Times, false, true) => f.times_italic(),
                        (Builtin::Times, true, true) => f.times_bold_italic(),
                        (Builtin::Courier, false, false) => f.courier(),
                        (Builtin::Courier, true, false) => f.courier_bold(),
                        (Builtin::Courier, false, true) => f.courier_oblique(),
                        (Builtin::Courier, true, true) => f.courier_bold_oblique(),
                    }
                };
                let objs = pg.objects_mut();
                for it in &items {
                    let text = it.template.replace("{n}", &num.to_string()).replace("{N}", &total.to_string()).replace("{date}", &date).replace("{file}", &file).replace("{title}", &title);
                    if text.trim().is_empty() {
                        continue;
                    }
                    let w = text_width(&text, st.size, fam);
                    let x = match it.h {
                        HAlign::Left => margin,
                        HAlign::Center => (geom.w - w) / 2.0,
                        HAlign::Right => geom.w - margin - w,
                    };
                    let y = match it.v {
                        VAlign::Top => margin + st.size * 0.8,
                        VAlign::Bottom => geom.h - margin,
                    };
                    let (px, py) = geom.to_view_inverse(x, y);
                    let mut o = objs.create_text_object(PdfPoints::new(px), PdfPoints::new(py), &text, tok, PdfPoints::new(st.size))?;
                    o.set_fill_color(PdfColor::new(st.color[0], st.color[1], st.color[2], 255))?;
                    if rot != 0 {
                        // the text must appear upright in the view: inverse of the page rotation around its origin
                        let a = (rot as f32).to_radians();
                        let (s, c) = a.sin_cos();
                        o.transform(c, s, -s, c, px - px * c + py * s, py - px * s - py * c)?;
                    }
                }
                Ok(())
            });
            match r {
                Ok(_) => count += 1,
                Err(e) => {
                    self.undo.pop();
                    return Err(e);
                }
            }
        }
        Ok(count)
    }

    fn page_rotation(&self, p: usize) -> i32 {
        self.read_page(p)
            .ok()
            .and_then(|pg| pg.rotation().ok())
            .map(|r| match r {
                PdfPageRenderRotation::None => 0,
                PdfPageRenderRotation::Degrees90 => 90,
                PdfPageRenderRotation::Degrees180 => 180,
                PdfPageRenderRotation::Degrees270 => 270,
            })
            .unwrap_or(0)
    }

    pub fn add_watermark(&mut self, spec: &WatermarkSpec, mem: u32) -> Result<(), String> {
        self.apply_bytes("Watermark", mem, |b| pdfedit_doc::add_watermark(b, spec))
    }

    // -------------------------------------------------------- scan deskewing

    /// Straightens the large images (scans) of the given pages; returns (corrected images, average angle).
    pub fn deskew_pages(&mut self, pages: &[usize], mem: u32) -> Result<(usize, f32), String> {
        let mut fixes: Vec<(usize, usize, image::DynamicImage, f32)> = vec![];
        for &p in pages {
            let info = self.info(p).ok_or("page not found")?;
            let area = info.geom.w * info.geom.h;
            let ids: Vec<usize> = info.objects.iter().filter(|o| o.kind == ObjKind::Image && r4_area(&o.rect) > area * 0.35).map(|o| o.idx).collect();
            for idx in ids {
                let img = self.read_image(p, idx)?;
                let (fixed, angle) = deskew::deskew(&img.to_rgba8());
                if angle.abs() >= 0.15 {
                    fixes.push((p, idx, image::DynamicImage::ImageRgba8(fixed), angle));
                }
            }
        }
        if fixes.is_empty() {
            return Ok((0, 0.0));
        }
        self.snapshot("Deskew scans", mem);
        let n = fixes.len();
        let avg = fixes.iter().map(|f| f.3).sum::<f32>() / n as f32;
        for (p, idx, img, _) in fixes {
            let r = self.edit_page_impl(p, "Deskew scans", mem, false, |pg, _| {
                let objs = pg.objects_mut();
                let mut o = objs.get(idx)?;
                if let Some(io) = o.as_image_object_mut() {
                    io.set_image(&img)?;
                }
                Ok(())
            });
            if let Err(e) = r {
                self.undo.pop();
                return Err(e);
            }
        }
        Ok((n, avg))
    }

    // ------------------------------------------------------- properties, cleaning, security

    pub fn doc_info(&self) -> DocInfo {
        self.doc.save_to_bytes().map(|b| pdfedit_doc::read_info(&b)).unwrap_or_default()
    }

    pub fn set_doc_info(&mut self, info: &DocInfo, mem: u32) -> Result<(), String> {
        self.apply_bytes("Document properties", mem, |b| pdfedit_doc::write_info(b, info))
    }

    pub fn clean_metadata(&mut self, opts: &CleanOpts, mem: u32) -> Result<Vec<String>, String> {
        let mut report = vec![];
        self.apply_bytes("Clean metadata", mem, |b| {
            let (out, r) = pdfedit_doc::clean(b, opts)?;
            report = r;
            Ok(out)
        })?;
        Ok(report)
    }

    /// Bytes to write on save: encrypted if a protection is active.
    pub fn bytes_for_save(&self) -> Result<Vec<u8>, String> {
        let b = self.doc.save_to_bytes().map_err(err)?;
        match &self.security {
            Some(s) if s.is_active() => pdfedit_doc::encrypt(&b, s),
            _ => Ok(b),
        }
    }

    pub fn set_security(&mut self, s: Option<Security>) {
        self.security = s.filter(|x| x.is_active());
        self.dirty = true;
    }

    // ------------------------------------------------------------ bookmarks, links, layers

    pub fn bookmarks(&self) -> Vec<Bm> {
        self.doc.save_to_bytes().map(|b| pdfedit_doc::read_bookmarks(&b)).unwrap_or_default()
    }

    pub fn set_bookmarks(&mut self, nodes: &[Bm], mem: u32) -> Result<(), String> {
        self.apply_bytes("Bookmarks", mem, |b| pdfedit_doc::write_bookmarks(b, nodes))
    }

    pub fn add_link(&mut self, page: usize, rect: R4, dest: &LinkDest, mem: u32) -> Result<(), String> {
        self.apply_bytes("Create a link", mem, |b| pdfedit_doc::add_link(b, page, rect, dest))
    }

    pub fn delete_link_at(&mut self, page: usize, rect: R4, mem: u32) -> Result<(), String> {
        let b = self.doc.save_to_bytes().map_err(err)?;
        let idx = crate::pdfedit_annot::find_annotation(&b, page, "Link", &rect).ok_or("link not found")?;
        let out = crate::pdfedit_annot::delete_annotation(&b, page, idx)?;
        self.replace_with_bytes("Delete link", mem, out)
    }

    pub fn layers(&self) -> Vec<Layer> {
        self.doc.save_to_bytes().map(|b| pdfedit_doc::list_layers(&b)).unwrap_or_default()
    }

    pub fn set_layer(&mut self, id: u32, visible: bool, mem: u32) -> Result<(), String> {
        self.apply_bytes("Layers", mem, |b| pdfedit_doc::set_layer_visible(b, id, visible))
    }

    // --------------------------------------------------------------- attachments

    pub fn attachments(&self) -> Vec<(String, usize)> {
        self.doc.attachments().iter().map(|a| (a.name(), a.len())).collect()
    }

    pub fn add_attachment(&mut self, name: &str, data: &[u8], mem: u32) -> Result<(), String> {
        self.snapshot("Add an attachment", mem);
        let r = self.doc.attachments_mut().create_attachment_from_bytes(name, data);
        match r {
            Ok(_) => Ok(()),
            Err(e) => {
                self.undo.pop();
                Err(err(e))
            }
        }
    }

    pub fn delete_attachment(&mut self, index: usize, mem: u32) -> Result<(), String> {
        self.snapshot("Delete an attachment", mem);
        let r = self.doc.attachments_mut().delete_at_index(index as _);
        match r {
            Ok(_) => Ok(()),
            Err(e) => {
                self.undo.pop();
                Err(err(e))
            }
        }
    }

    pub fn attachment_bytes(&self, index: usize) -> Result<Vec<u8>, String> {
        let a = self.doc.attachments().get(index as _).map_err(err)?;
        a.save_to_bytes().map_err(err)
    }
}

impl PageGeom {
    /// View → page: readable alias of `pt_from_view`.
    pub fn to_view_inverse(&self, x: f32, y: f32) -> (f32, f32) {
        self.pt_from_view(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tab::Tab;

    fn tab(n: usize) -> Tab {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        for i in 0..n {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
            let font = doc.fonts_mut().helvetica();
            page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), format!("Page {i}"), font, PdfPoints::new(24.0)).unwrap();
            page.regenerate_content().unwrap();
        }
        Tab::new(p, doc, None, None, 1.0)
    }

    fn texts(t: &Tab) -> Vec<String> {
        (0..t.page_count()).map(|i| t.page_text(i).trim().to_string()).collect()
    }

    #[test]
    fn ranges_parse_and_format() {
        assert_eq!(parse_ranges("1-3, 5, 8-", 10).unwrap(), vec![0, 1, 2, 4, 7, 8, 9]);
        assert_eq!(parse_ranges("-2", 10).unwrap(), vec![0, 1]);
        assert_eq!(parse_ranges("3;1", 5).unwrap(), vec![0, 2]);
        assert!(parse_ranges("0", 5).is_err() && parse_ranges("2-9", 5).is_err() && parse_ranges("a", 5).is_err() && parse_ranges("", 5).is_err() && parse_ranges("4-2", 5).is_err());
        assert_eq!(format_ranges(&[0, 1, 2, 4, 7, 8]), "1-3, 5, 8-9");
    }

    #[test]
    fn move_delete_rotate_insert_with_undo() {
        let mut t = tab(4);
        t.move_pages_to(&[3], 0, 100).unwrap();
        assert_eq!(texts(&t), vec!["Page 3", "Page 0", "Page 1", "Page 2"]);
        t.move_pages_to(&[0, 1], 4, 100).unwrap(); // the first two go to the end
        assert_eq!(texts(&t), vec!["Page 1", "Page 2", "Page 3", "Page 0"]);
        t.delete_page_list(&[0], 100).unwrap();
        assert_eq!(t.page_count(), 3);
        t.rotate_page_list(&[1], 90, 100).unwrap();
        assert!(t.sizes[1].0 > t.sizes[1].1, "page 2 in landscape");
        t.insert_blank_at(1, 300.0, 300.0, 100).unwrap();
        assert_eq!(t.page_count(), 4);
        assert_eq!(t.view.page, 1);
        // each operation is undoable
        for _ in 0..5 {
            t.undo();
        }
        assert_eq!(texts(&t), vec!["Page 0", "Page 1", "Page 2", "Page 3"]);
    }

    #[test]
    fn extract_split_merge() {
        let mut t = tab(5);
        let part = t.extract_bytes(&[1, 3]).unwrap();
        let p = crate::pdfx::test_pdfium();
        let d = p.load_pdf_from_byte_vec(part, None).unwrap();
        assert_eq!(d.pages().len(), 2);
        let g = t.split_groups(&SplitMode::EveryN(2)).unwrap();
        assert_eq!(g, vec![vec![0, 1], vec![2, 3], vec![4]]);
        let parts = t.split_bytes(&g).unwrap();
        assert_eq!(parts.len(), 3);
        let big = t.split_groups(&SplitMode::MaxBytes(1)).unwrap();
        assert_eq!(big.len(), 5, "one file per page when the limit is tiny");
        let all = t.split_groups(&SplitMode::MaxBytes(10_000_000)).unwrap();
        assert_eq!(all.len(), 1);
        t.merge_bytes(&parts[..2], 100).unwrap();
        assert_eq!(t.page_count(), 9);
    }

    #[test]
    fn split_by_bookmarks() {
        let mut t = tab(6);
        t.set_bookmarks(&[Bm { title: "A".into(), page: 0, children: vec![] }, Bm { title: "B".into(), page: 2, children: vec![] }, Bm { title: "C".into(), page: 5, children: vec![] }], 100).unwrap();
        assert_eq!(t.split_groups(&SplitMode::ByBookmarks).unwrap(), vec![vec![0, 1], vec![2, 3, 4], vec![5]]);
        assert_eq!(t.bookmarks().len(), 3);
    }

    #[test]
    fn page_numbers_and_headers() {
        let mut t = tab(3);
        let style = TextStyle { size: 10.0, ..TextStyle::default() };
        let spec = StampSpec {
            items: vec![
                StampItem { template: "Page {n} / {N}".into(), h: HAlign::Center, v: VAlign::Bottom },
                StampItem { template: "{file}".into(), h: HAlign::Left, v: VAlign::Top },
            ],
            style,
            margin: 30.0,
            start: 1,
            skip_first: false,
        };
        t.title = "doc.pdf".into();
        t.stamp_pages(&[0, 1, 2], &spec, 100).unwrap();
        for i in 0..3 {
            let txt = t.page_text(i);
            assert!(txt.contains(&format!("Page {} / 3", i + 1)), "{txt:?}");
            assert!(txt.contains("doc.pdf"), "{txt:?}");
        }
        // position: the number is centered at the bottom
        let info = t.info(0).unwrap();
        let num = info.objects.iter().find(|o| o.text().starts_with("Page 1 /")).unwrap();
        let g = info.geom;
        let v = g.rect_to_view(&num.rect);
        assert!(((v[0] + v[2]) / 2.0 - g.w / 2.0).abs() < 4.0, "centered: {v:?}");
        assert!(v[3] > g.h - 40.0, "at the bottom: {v:?}");
        // a single undo removes everything
        t.undo();
        assert!(!t.page_text(1).contains("Page 2 / 3"));
        // first page skipped, numbering starting at 5
        let spec2 = StampSpec { items: vec![StampItem { template: "{n}".into(), h: HAlign::Right, v: VAlign::Bottom }], style: TextStyle::default(), margin: 20.0, start: 5, skip_first: true };
        t.stamp_pages(&[0, 1, 2], &spec2, 100).unwrap();
        assert!(!t.page_text(0).contains('5'));
        assert!(t.page_text(1).contains('5') && t.page_text(2).contains('6'));
    }

    #[test]
    fn stamps_stay_upright_on_rotated_pages() {
        let mut t = tab(1);
        t.rotate_page_list(&[0], 90, 100).unwrap();
        let spec = StampSpec { items: vec![StampItem { template: "BOTTOM".into(), h: HAlign::Center, v: VAlign::Bottom }], style: TextStyle { size: 12.0, ..TextStyle::default() }, margin: 20.0, start: 1, skip_first: false };
        t.stamp_pages(&[0], &spec, 100).unwrap();
        let info = t.info(0).unwrap();
        let o = info.objects.iter().find(|o| o.text() == "BOTTOM").expect("stamp");
        let v = info.geom.rect_to_view(&o.rect);
        assert!(v[2] - v[0] > v[3] - v[1], "horizontal text in the view: {v:?}");
        assert!(v[3] > info.geom.h - 40.0, "bottom of the view: {v:?}");
    }

    #[test]
    fn deskew_straightens_a_scanned_page() {
        let mut t = tab(1);
        // a "scanned page": full-page image of lines skewed by 2°
        let mut base = image::RgbaImage::from_pixel(600, 800, image::Rgba([255, 255, 255, 255]));
        for (k, y0) in (60..740).step_by(36).enumerate() {
            for y in y0..y0 + 10 {
                for x in 60..(60 + 380 + (k as u32 * 31) % 120) {
                    if (x / 13) % 5 != 4 {
                        base.put_pixel(x, y, image::Rgba([20, 20, 20, 255]));
                    }
                }
            }
        }
        let skewed = deskew::rotate_image(&base, 2.0, image::Rgba([255, 255, 255, 255]));
        t.add_image(0, 0.0, 842.0, &image::DynamicImage::ImageRgba8(skewed), 595.0, 100).unwrap();
        let (n, avg) = t.deskew_pages(&[0], 100).unwrap();
        assert_eq!(n, 1);
        assert!((avg - 2.0).abs() < 0.5, "{avg}");
        // second pass: nothing left to fix
        let (n2, _) = t.deskew_pages(&[0], 100).unwrap();
        assert_eq!(n2, 0);
    }

    #[test]
    fn attachments_info_and_security_on_save() {
        let mut t = tab(1);
        t.add_attachment("note.txt", b"hello", 100).unwrap();
        assert_eq!(t.attachments(), vec![("note.txt".to_string(), 5)]);
        assert_eq!(t.attachment_bytes(0).unwrap(), b"hello");
        t.delete_attachment(0, 100).unwrap();
        assert!(t.attachments().is_empty());
        t.set_doc_info(&DocInfo { title: "Title".into(), author: "Me".into(), ..Default::default() }, 100).unwrap();
        assert_eq!(t.doc_info().title, "Title");
        t.set_security(Some(Security { user_password: "pwd".into(), owner_password: "master".into(), ..Security::default() }));
        let saved = t.bytes_for_save().unwrap();
        let p = crate::pdfx::test_pdfium();
        assert!(matches!(crate::pdfx::open_bytes(p, saved.clone(), None), Err(crate::pdfx::OpenErr::Password)));
        assert!(crate::pdfx::open_bytes(p, saved, Some("pwd")).is_ok());
        t.set_security(None);
        assert!(crate::pdfx::open_bytes(p, t.bytes_for_save().unwrap(), None).is_ok());
    }
}
