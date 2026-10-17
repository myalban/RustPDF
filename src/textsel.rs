//! Text selection with the mouse: character under the cursor, ranges, rectangles per line, copied text.

use crate::geom::{R4, r4_union};
use crate::tab::Tab;
use pdfium_render::prelude::*;

/// Text selection: PDFium character indices (inclusive), on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSel {
    pub page: usize,
    pub a: usize,
    pub b: usize,
}

impl TextSel {
    pub fn ordered(&self) -> (usize, usize) {
        (self.a.min(self.b), self.a.max(self.b))
    }
}

fn char_rect(c: &PdfPageTextChar) -> Option<R4> {
    let b = c.loose_bounds().or_else(|_| c.tight_bounds()).ok()?;
    let r = [b.left().value, b.bottom().value, b.right().value, b.top().value];
    if r[2] - r[0] <= 0.0 && r[3] - r[1] <= 0.0 {
        None
    } else {
        Some(r)
    }
}

impl Tab {
    /// Character nearest to a page point (PDF page space).
    pub fn char_near(&self, page: usize, x: f32, y: f32, tol: f32) -> Option<usize> {
        let pg = self.read_page(page).ok()?;
        let text = pg.text().ok()?;
        let chars = text.chars();
        if let Some(c) = chars.get_char_at_point(PdfPoints::new(x), PdfPoints::new(y)) {
            return Some(c.index());
        }
        // Otherwise: nearest character within the tolerance (get_char_near_point returns nothing here).
        let mut best: Option<(usize, f32)> = None;
        for i in 0..chars.len() {
            let Ok(c) = chars.get(i) else { continue };
            if c.unicode_char().map(|ch| ch.is_control()).unwrap_or(true) {
                continue;
            }
            let Some(r) = char_rect(&c) else { continue };
            let dx = (r[0] - x).max(x - r[2]).max(0.0);
            let dy = (r[1] - y).max(y - r[3]).max(0.0);
            let d = (dx * dx + dy * dy).sqrt();
            if d <= tol && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Text and rectangles (one per line) of a character range.
    pub fn text_range(&self, page: usize, a: usize, b: usize) -> (String, Vec<R4>) {
        let (lo, hi) = (a.min(b), a.max(b));
        let Ok(pg) = self.read_page(page) else { return (String::new(), vec![]) };
        let Ok(text) = pg.text() else { return (String::new(), vec![]) };
        let chars = text.chars();
        let mut s = String::new();
        let mut rects: Vec<R4> = vec![];
        let mut cur: Option<R4> = None;
        for i in lo..=hi {
            let Ok(c) = chars.get(i) else { continue };
            if let Some(ch) = c.unicode_char() {
                s.push(ch);
            }
            let generated = c.is_generated().unwrap_or(false);
            let ctrl = c.unicode_char().map(|ch| ch.is_control()).unwrap_or(true);
            if generated || ctrl {
                if let Some(r) = cur.take() {
                    rects.push(r);
                }
                continue;
            }
            let Some(r) = char_rect(&c) else { continue };
            match cur {
                None => cur = Some(r),
                Some(u) => {
                    let (h, ov) = ((u[3] - u[1]).min(r[3] - r[1]), u[3].min(r[3]) - u[1].max(r[1]));
                    if h > 0.0 && ov > 0.5 * h && r[0] >= u[0] - 1.0 {
                        cur = Some(r4_union(&u, &r));
                    } else {
                        rects.push(u);
                        cur = Some(r);
                    }
                }
            }
        }
        if let Some(r) = cur {
            rects.push(r);
        }
        (s, rects)
    }

    /// Word around a character (double-click).
    pub fn word_at(&self, page: usize, idx: usize) -> Option<(usize, usize)> {
        let pg = self.read_page(page).ok()?;
        let text = pg.text().ok()?;
        let chars = text.chars();
        let is_word = |i: usize| chars.get(i).ok().and_then(|c| c.unicode_char()).map(|c| c.is_alphanumeric() || c == '_' || c == '\'').unwrap_or(false);
        if !is_word(idx) {
            return Some((idx, idx));
        }
        let mut lo = idx;
        while lo > 0 && is_word(lo - 1) {
            lo -= 1;
        }
        let mut hi = idx;
        let n = chars.len();
        while hi + 1 < n && is_word(hi + 1) {
            hi += 1;
        }
        Some((lo, hi))
    }

    /// Whole line around a character (triple-click).
    pub fn line_at(&self, page: usize, idx: usize) -> Option<(usize, usize)> {
        let pg = self.read_page(page).ok()?;
        let text = pg.text().ok()?;
        let chars = text.chars();
        let n = chars.len();
        let is_nl = |i: usize| chars.get(i).ok().and_then(|c| c.unicode_char()).map(|c| c == '\n' || c == '\r').unwrap_or(false);
        let mut lo = idx;
        while lo > 0 && !is_nl(lo - 1) {
            lo -= 1;
        }
        let mut hi = idx;
        while hi + 1 < n && !is_nl(hi + 1) {
            hi += 1;
        }
        Some((lo, hi))
    }

    /// All the text of the page.
    pub fn page_text(&self, page: usize) -> String {
        self.read_page(page).ok().and_then(|p| p.text().ok().map(|t| t.all())).unwrap_or_default()
    }

    pub fn char_count(&self, page: usize) -> usize {
        self.read_page(page).ok().and_then(|p| p.text().ok().map(|t| t.chars().len())).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tab::Tab;

    fn tab() -> Tab {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let font = doc.fonts_mut().helvetica();
        let objs = page.objects_mut();
        objs.create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), "Hello big world", font, PdfPoints::new(20.0)).unwrap();
        objs.create_text_object(PdfPoints::new(72.0), PdfPoints::new(660.0), "Second line", font, PdfPoints::new(20.0)).unwrap();
        page.regenerate_content().unwrap();
        drop(page);
        Tab::new(p, doc, None, None, 1.0)
    }

    #[test]
    fn char_under_pointer_and_range_text() {
        let t = tab();
        // "Hello big world" at 20 pt: "w" of "world" around x ≈ 72 + 20*(0.55*10) ≈ 180
        let page_text = t.page_text(0);
        assert!(page_text.contains("Hello big world"), "{page_text:?}");
        let w_idx = page_text.find("world").unwrap();
        let (txt, rects) = t.text_range(0, w_idx, w_idx + 4);
        assert_eq!(txt, "world");
        assert_eq!(rects.len(), 1);
        // the center of the rectangle finds these characters
        let (cx, cy) = ((rects[0][0] + rects[0][2]) / 2.0, (rects[0][1] + rects[0][3]) / 2.0);
        let idx = t.char_near(0, cx, cy, 2.0).expect("character found");
        assert!(idx >= w_idx && idx <= w_idx + 4, "{idx}");
    }

    #[test]
    fn selection_across_two_lines_gives_two_rects() {
        let t = tab();
        let all = t.page_text(0);
        let a = all.find("world").unwrap();
        let b = all.find("Second").unwrap() + 5;
        let (txt, rects) = t.text_range(0, a, b);
        assert!(txt.contains("world") && txt.contains("Secon"), "{txt:?}");
        assert_eq!(rects.len(), 2, "one area per line: {rects:?}");
        assert!(rects[0][1] > rects[1][1], "the first line is above");
    }

    #[test]
    fn word_and_line_expansion() {
        let t = tab();
        let all = t.page_text(0);
        let big = all.find("big").unwrap();
        let (lo, hi) = t.word_at(0, big + 1).unwrap();
        assert_eq!(t.text_range(0, lo, hi).0, "big");
        let (lo, hi) = t.line_at(0, big).unwrap();
        assert_eq!(t.text_range(0, lo, hi).0.trim(), "Hello big world");
    }
}

