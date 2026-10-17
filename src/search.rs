//! Search in the document, navigation between results, and "find / replace".

use crate::edit_text::TextStyle;
use crate::geom::*;
use crate::model::ObjKind;
use crate::tab::{SearchHit, Tab};
use pdfium_render::prelude::*;

impl Tab {
    /// Searches for all the occurrences in the whole document.
    pub fn collect_matches(&self) -> Vec<SearchHit> {
        let q = self.search.query.trim();
        let mut hits = vec![];
        if q.is_empty() {
            return hits;
        }
        let opts = PdfSearchOptions::new().match_case(self.search.case).match_whole_word(self.search.word);
        for pi in 0..self.page_count() {
            let Ok(page) = self.read_page(pi) else { continue };
            let Ok(text) = page.text() else { continue };
            let Ok(search) = text.search(q, &opts) else { continue };
            for segs in search.iter(PdfSearchDirection::SearchForward) {
                let rects: Vec<R4> = segs
                    .iter()
                    .map(|s| {
                        let b = s.bounds();
                        [b.left().value, b.bottom().value, b.right().value, b.top().value]
                    })
                    .collect();
                if !rects.is_empty() {
                    hits.push(SearchHit { page: pi, rects });
                }
            }
        }
        hits
    }

    /// Restarts the search (text modified) and goes to the first occurrence from the current page.
    pub fn run_search(&mut self) -> Option<usize> {
        self.search.stale = false;
        self.search.matches = self.collect_matches();
        if self.search.matches.is_empty() {
            self.search.cur = None;
            return None;
        }
        let page = self.view.page;
        let start = self.search.matches.iter().position(|m| m.page >= page).unwrap_or(0);
        self.goto_match(start);
        Some(start)
    }

    /// Recomputes the occurrences after a document change, without moving the view.
    pub fn refresh_matches(&mut self) {
        self.search.stale = false;
        self.search.matches = self.collect_matches();
        self.search.cur = match self.search.cur {
            Some(c) if !self.search.matches.is_empty() => Some(c.min(self.search.matches.len() - 1)),
            _ => None,
        };
    }

    pub fn goto_match(&mut self, i: usize) {
        let Some(m) = self.search.matches.get(i) else { return };
        let page = m.page;
        self.search.cur = Some(i);
        self.view.page = page;
        self.view.scroll_to = Some(page);
        self.search.scroll = true;
    }

    pub fn next_match(&mut self) {
        let n = self.search.matches.len();
        if n == 0 {
            return;
        }
        let i = self.search.cur.map(|c| (c + 1) % n).unwrap_or(0);
        self.goto_match(i);
    }

    pub fn prev_match(&mut self) {
        let n = self.search.matches.len();
        if n == 0 {
            return;
        }
        let i = self.search.cur.map(|c| (c + n - 1) % n).unwrap_or(n - 1);
        self.goto_match(i);
    }

    pub fn clear_search(&mut self) {
        self.search.matches.clear();
        self.search.cur = None;
    }

    // ------------------------------------------------------------ replace

    fn find_ci(hay: &str, needle: &str, case: bool, from: usize) -> Option<(usize, usize)> {
        if needle.is_empty() {
            return None;
        }
        if case {
            hay[from..].find(needle).map(|p| (from + p, from + p + needle.len()))
        } else {
            let (h, n) = (hay.to_lowercase(), needle.to_lowercase());
            // lowercasing can change the length in bytes: work on character indices
            if h.len() != hay.len() {
                return None;
            }
            h[from..].find(&n).map(|p| (from + p, from + p + n.len()))
        }
    }

    fn is_word_boundary(s: &str, a: usize, b: usize) -> bool {
        let before = s[..a].chars().next_back().map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false);
        let after = s[b..].chars().next().map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false);
        !before && !after
    }

    /// Replaces the occurrences of `query` in `text` (all of them or only the `nth` one).
    pub fn replace_in_text(text: &str, query: &str, with: &str, case: bool, word: bool, nth: Option<usize>) -> (String, usize) {
        let mut out = String::new();
        let mut pos = 0;
        let mut count = 0;
        let mut ordinal = 0;
        while let Some((a, b)) = Self::find_ci(text, query, case, pos) {
            if word && !Self::is_word_boundary(text, a, b) {
                out.push_str(&text[pos..b]);
                pos = b;
                continue;
            }
            out.push_str(&text[pos..a]);
            if nth.map(|n| n == ordinal).unwrap_or(true) {
                out.push_str(with);
                count += 1;
            } else {
                out.push_str(&text[a..b]);
            }
            ordinal += 1;
            pos = b;
        }
        out.push_str(&text[pos..]);
        (out, count)
    }

    /// Replaces everywhere in the document; returns the number of replacements.
    pub fn replace_all(&mut self, with: &str, mem: u32) -> Result<usize, String> {
        let (q, case, word) = (self.search.query.clone(), self.search.case, self.search.word);
        if q.trim().is_empty() {
            return Ok(0);
        }
        let mut total = 0;
        // targets: (page, rect, current text)
        let mut targets: Vec<(usize, R4, String)> = vec![];
        for p in 0..self.page_count() {
            let Some(info) = self.info(p) else { continue };
            for o in info.objects.iter().filter(|o| o.kind == ObjKind::Text) {
                let t = o.text().to_string();
                let (_, n) = Self::replace_in_text(&t, &q, with, case, word, None);
                if n > 0 {
                    targets.push((p, o.rect, t));
                }
            }
        }
        for (page, rect, text) in targets {
            let Some(info) = self.info(page) else { continue };
            let Some(o) = info
                .objects
                .iter()
                .find(|o| o.kind == ObjKind::Text && o.text() == text && (o.rect[0] - rect[0]).abs() < 1.0 && (o.rect[3] - rect[3]).abs() < 1.0)
                .cloned()
            else {
                continue;
            };
            let Some((_, style, _)) = self.read_text_style(page, o.idx) else { continue };
            let (new_text, n) = Self::replace_in_text(&text, &q, with, case, word, None);
            let st: TextStyle = style;
            self.apply_text(page, o.idx, &new_text, &st, mem)?;
            total += n;
        }
        self.search.stale = true;
        Ok(total)
    }

    /// Replaces the current occurrence of the search.
    pub fn replace_current(&mut self, with: &str, mem: u32) -> Result<bool, String> {
        let Some(cur) = self.search.cur else { return Ok(false) };
        let Some(hit) = self.search.matches.get(cur).cloned() else { return Ok(false) };
        let (q, case, word) = (self.search.query.clone(), self.search.case, self.search.word);
        let page = hit.page;
        let (cx, cy) = r4_center(&hit.rects[0]);
        let Some(info) = self.info(page) else { return Ok(false) };
        let Some(o) = info.objects.iter().find(|o| o.kind == ObjKind::Text && r4_contains(&o.rect, cx, cy)).cloned() else {
            return Err("This text is not editable (block not found)".into());
        };
        // rank of this occurrence in the block: among the page's occurrences located in the block
        let mut in_block: Vec<(usize, &SearchHit)> = self
            .search
            .matches
            .iter()
            .enumerate()
            .filter(|(_, m)| m.page == page && r4_contains(&o.rect, r4_center(&m.rects[0]).0, r4_center(&m.rects[0]).1))
            .collect();
        in_block.sort_by(|a, b| {
            let (ra, rb) = (&a.1.rects[0], &b.1.rects[0]);
            rb[3].partial_cmp(&ra[3]).unwrap_or(std::cmp::Ordering::Equal).then(ra[0].partial_cmp(&rb[0]).unwrap_or(std::cmp::Ordering::Equal))
        });
        let nth = in_block.iter().position(|(i, _)| *i == cur);
        let Some((_, style, _)) = self.read_text_style(page, o.idx) else { return Ok(false) };
        let (new_text, n) = Self::replace_in_text(o.text(), &q, with, case, word, nth);
        if n == 0 {
            return Ok(false);
        }
        self.apply_text(page, o.idx, &new_text, &style, mem)?;
        self.search.stale = true;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tab::Tab;

    fn tab(lines: &[&str]) -> Tab {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        for text in lines {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
            let font = doc.fonts_mut().helvetica();
            page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), *text, font, PdfPoints::new(14.0)).unwrap();
            page.regenerate_content().unwrap();
        }
        Tab::new(p, doc, None, None, 1.0)
    }

    #[test]
    fn replace_in_text_variants() {
        let r = |t, q, w, case, word, nth| Tab::replace_in_text(t, q, w, case, word, nth);
        assert_eq!(r("a b a b", "a", "X", false, false, None), ("X b X b".into(), 2));
        assert_eq!(r("a b a b", "a", "X", false, false, Some(1)), ("a b X b".into(), 1));
        assert_eq!(r("Cat cat", "cat", "dog", true, false, None), ("Cat dog".into(), 1));
        assert_eq!(r("Cat cat", "cat", "dog", false, false, None), ("dog dog".into(), 2));
        assert_eq!(r("hello hellono", "hello", "hi", false, true, None), ("hi hellono".into(), 1));
        assert_eq!(r("aaa", "a", "aa", true, false, None), ("aaaaaa".into(), 3), "no infinite loop");
    }

    #[test]
    fn navigation_wraps_and_changes_page() {
        let mut t = tab(&["Hello world, hello", "nothing here", "HELLO hellono"]);
        t.search.query = "hello".into();
        assert_eq!(t.collect_matches().len(), 4);
        t.run_search();
        assert_eq!(t.search.cur, Some(0));
        t.prev_match();
        assert_eq!(t.search.cur, Some(3), "back to the last one");
        assert_eq!(t.view.page, 2);
        t.next_match();
        assert_eq!(t.search.cur, Some(0));
        assert_eq!(t.view.page, 0);
        t.search.case = true;
        assert_eq!(t.collect_matches().len(), 2);
        t.search.case = false;
        t.search.word = true;
        assert_eq!(t.collect_matches().len(), 3);
    }

    #[test]
    fn replace_all_and_current() {
        let mut t = tab(&["Hello world", "cruel world"]);
        t.search.query = "world".into();
        let n = t.replace_all("universe", 100).unwrap();
        assert_eq!(n, 2);
        assert!(t.page_text(0).contains("Hello universe"), "{:?}", t.page_text(0));
        assert!(t.page_text(1).contains("cruel universe"));
        // replace only the current occurrence
        let mut t = tab(&["a cat and a cat"]);
        t.search.query = "cat".into();
        t.run_search();
        t.next_match(); // second occurrence
        assert!(t.replace_current("dog", 100).unwrap());
        let txt = t.page_text(0);
        assert!(txt.contains("a cat and a dog"), "{txt:?}");
    }
}
