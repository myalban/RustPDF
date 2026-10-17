//! Text editing: line wrapping, alignment, justification, spacing, font, underline.

use crate::fonts::{self, Builtin, FontFamily};
use crate::tab::Tab;
use crate::textgroup::Align;
use pdfium_render::prelude::*;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub family: FontFamily,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub size: f32,
    pub color: [u8; 3],
    pub align: Align,
    /// Extra spacing between letters, in points.
    pub letter_spacing: f32,
    /// Line spacing multiplier; 0 = automatic (the document's own).
    pub line_spacing: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            family: FontFamily::Builtin(Builtin::Helvetica),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            size: 12.0,
            color: [0, 0, 0],
            align: Align::Left,
            letter_spacing: 0.0,
            line_spacing: 0.0,
        }
    }
}

/// Measures the advance of a character (abstraction to test the layout without PDFium).
pub trait Measure {
    fn char_adv(&mut self, c: char) -> f32;
    fn width(&mut self, s: &str) -> f32 {
        s.chars().map(|c| self.char_adv(c)).sum()
    }
}

/// Splits a text into lines: explicit line breaks, then automatic wrapping at `max_width`.
/// Each line comes with a "last line of the paragraph" flag.
pub fn wrap_text<M: Measure>(text: &str, max_width: Option<f32>, m: &mut M) -> Vec<(String, bool)> {
    let mut out = vec![];
    let text = text.replace('\t', "    ").replace("\r\n", "\n").replace('\r', "\n");
    for para in text.split('\n') {
        let Some(maxw) = max_width else {
            out.push((para.to_string(), true));
            continue;
        };
        let mut line = String::new();
        let mut any = false;
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && m.width(&candidate) > maxw {
                out.push((std::mem::take(&mut line), false));
                line = word.to_string();
            } else {
                line = candidate;
            }
            // Word wider than the line: break by characters.
            while m.width(&line) > maxw && line.chars().count() > 1 {
                let mut cut = String::new();
                let mut rest = String::new();
                let mut full = false;
                for c in line.chars() {
                    if !full {
                        let t = format!("{cut}{c}");
                        if m.width(&t) > maxw && !cut.is_empty() {
                            full = true;
                            rest.push(c);
                        } else {
                            cut = t;
                        }
                    } else {
                        rest.push(c);
                    }
                }
                if rest.is_empty() {
                    break;
                }
                out.push((cut, false));
                line = rest;
            }
            any = true;
        }
        let _ = any;
        out.push((line, true));
    }
    out
}

/// A piece of text to create: position (origin, baseline) and content.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub base: f32,
    pub text: String,
}

/// Geometry of a placed line (for underlines).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLine {
    pub x0: f32,
    pub x1: f32,
    pub base: f32,
}

pub struct LayoutIn<'a> {
    pub lines: &'a [(String, bool)],
    pub left: f32,
    pub width: f32,
    pub align: Align,
    pub base0: f32,
    pub pitch: f32,
    pub letter_spacing: f32,
}

/// Places the lines: alignment, justification (one object per word), letter spacing (one object per letter).
pub fn layout<M: Measure>(inp: &LayoutIn, m: &mut M) -> (Vec<Placed>, Vec<PlacedLine>) {
    let mut placed = vec![];
    let mut lines_geo = vec![];
    let ls = inp.letter_spacing;
    let text_w = |m: &mut M, s: &str| -> f32 {
        let n = s.chars().count() as f32;
        m.width(s) + ls * (n - 1.0).max(0.0)
    };
    for (k, (line, last)) in inp.lines.iter().enumerate() {
        let base = inp.base0 - k as f32 * inp.pitch;
        let w = text_w(m, line);
        let words: Vec<&str> = line.split(' ').filter(|w| !w.is_empty()).collect();
        let justify = inp.align == Align::Justify && !*last && words.len() >= 2;
        let mut segments: Vec<(f32, String)> = vec![];
        let (x0, x1);
        if justify {
            let total: f32 = words.iter().map(|w| text_w(m, w)).sum();
            let gap = ((inp.width - total) / (words.len() as f32 - 1.0)).max(0.0);
            let mut x = inp.left;
            for w in &words {
                segments.push((x, w.to_string()));
                x += text_w(m, w) + gap;
            }
            x0 = inp.left;
            x1 = inp.left + inp.width;
        } else {
            let x = match inp.align {
                Align::Center => inp.left + (inp.width - w) * 0.5,
                Align::Right => inp.left + inp.width - w,
                _ => inp.left,
            };
            if !line.is_empty() {
                segments.push((x, line.clone()));
            }
            x0 = x;
            x1 = x + w;
        }
        for (x, text) in segments {
            if ls.abs() > 1e-4 {
                let mut cx = x;
                for c in text.chars() {
                    if c != ' ' {
                        placed.push(Placed { x: cx, base, text: c.to_string() });
                    }
                    cx += m.char_adv(c) + ls;
                }
            } else {
                placed.push(Placed { x, base, text });
            }
        }
        if !line.trim().is_empty() {
            lines_geo.push(PlacedLine { x0, x1, base });
        }
    }
    (placed, lines_geo)
}

// ---------------------------------------------------------------- PDFium

/// Actual measurement of a font: advance of each character through a temporary object.
struct PdfMetrics<'b> {
    objs: &'b mut PdfPageObjects<'static>,
    font: PdfFontToken,
    size: f32,
    cache: HashMap<char, f32>,
    hh: Option<f32>,
}

impl<'b> PdfMetrics<'b> {
    fn ink_width(&mut self, s: &str) -> Option<f32> {
        let o = self
            .objs
            .create_text_object(PdfPoints::ZERO, PdfPoints::ZERO, s, self.font, PdfPoints::new(self.size))
            .ok()?;
        let w = o.bounds().ok().map(|b| b.to_rect().width().value);
        let _ = self.objs.remove_object(o);
        w
    }
}

impl<'b> Measure for PdfMetrics<'b> {
    fn char_adv(&mut self, c: char) -> f32 {
        if let Some(w) = self.cache.get(&c) {
            return *w;
        }
        // adv(c) = ink("HcH") − ink("HH"): the side bearings cancel out.
        let hh = match self.hh {
            Some(v) => v,
            None => {
                let v = self.ink_width("HH").unwrap_or(self.size);
                self.hh = Some(v);
                v
            }
        };
        let w = self
            .ink_width(&format!("H{c}H"))
            .map(|w| (w - hh).max(0.0))
            .unwrap_or(self.size * 0.5);
        self.cache.insert(c, w);
        w
    }
}

/// Font to use to rebuild a text.
pub enum FontPlan {
    /// Same font as the original object.
    Keep,
    Builtin(Builtin, bool, bool),
    System(Vec<u8>),
}

fn builtin_token(fonts: &mut PdfFonts<'static>, b: Builtin, bold: bool, italic: bool) -> PdfFontToken {
    match (b, bold, italic) {
        (Builtin::Helvetica, false, false) => fonts.helvetica(),
        (Builtin::Helvetica, true, false) => fonts.helvetica_bold(),
        (Builtin::Helvetica, false, true) => fonts.helvetica_oblique(),
        (Builtin::Helvetica, true, true) => fonts.helvetica_bold_oblique(),
        (Builtin::Times, false, false) => fonts.times_roman(),
        (Builtin::Times, true, false) => fonts.times_bold(),
        (Builtin::Times, false, true) => fonts.times_italic(),
        (Builtin::Times, true, true) => fonts.times_bold_italic(),
        (Builtin::Courier, false, false) => fonts.courier(),
        (Builtin::Courier, true, false) => fonts.courier_bold(),
        (Builtin::Courier, false, true) => fonts.courier_oblique(),
        (Builtin::Courier, true, true) => fonts.courier_bold_oblique(),
    }
}

fn builtin_for_name(name: &str) -> Option<Builtin> {
    let l = name.to_lowercase();
    if l.contains("helvetica") || l.contains("arial") {
        Some(Builtin::Helvetica)
    } else if l.contains("times") {
        Some(Builtin::Times)
    } else if l.contains("courier") {
        Some(Builtin::Courier)
    } else {
        None
    }
}

/// Chooses the replacement font: system font of the same family if possible, otherwise a standard font.
pub fn plan_font(family: &FontFamily, bold: bool, italic: bool, orig_name: &str, serif: bool, mono: bool) -> FontPlan {
    let (orig_family, _, _) = fonts::parse_pdf_font_name(orig_name);
    match family {
        FontFamily::Builtin(b) => FontPlan::Builtin(*b, bold, italic),
        FontFamily::Original | FontFamily::System(_) => {
            let fam = match family {
                FontFamily::System(s) => s.clone(),
                _ => orig_family,
            };
            if let Some(bytes) = fonts::catalog().find(&fam, bold, italic) {
                return FontPlan::System(bytes);
            }
            let b = builtin_for_name(&fam).unwrap_or(if mono {
                Builtin::Courier
            } else if serif {
                Builtin::Times
            } else {
                Builtin::Helvetica
            });
            FontPlan::Builtin(b, bold, italic)
        }
    }
}

/// Parameters for rebuilding a text block.
struct Rebuild {
    members: Vec<usize>,
    leader: usize,
    text: String,
    style: TextStyle,
    plan: FontPlan,
    wrap: bool,
    container: (f32, f32),
    orig_pitch: Option<f32>,
    orig_size: f32,
}

fn rebuild_text(page: &mut PdfPage<'static>, doc: &mut PdfDocument<'static>, rb: &Rebuild) -> Result<usize, PdfiumError> {
    let col = PdfColor::new(rb.style.color[0], rb.style.color[1], rb.style.color[2], 255);
    let size = rb.style.size;
    let mut objs = page.objects_mut();
    // origin of the first piece and original font
    let (x0, base0, keep_tok) = {
        let o = objs.get(rb.leader)?;
        let m = o.matrix()?;
        let tok = o.as_text_object().map(|t| t.font().token());
        (m.e(), m.f(), tok)
    };
    let mut desc = rb.members.clone();
    desc.sort_unstable_by(|a, b| b.cmp(a));
    for k in desc {
        let o = objs.get(k)?;
        objs.remove_object(o)?;
    }
    let tok = match &rb.plan {
        FontPlan::Keep => keep_tok.ok_or(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::Unknown))?,
        FontPlan::Builtin(b, bo, it) => builtin_token(doc.fonts_mut(), *b, *bo, *it),
        FontPlan::System(bytes) => match doc.fonts_mut().load_true_type_from_bytes(bytes, true) {
            Ok(t) => t,
            Err(_) => doc.fonts_mut().helvetica(),
        },
    };
    // layout
    let (cl, cr) = rb.container;
    let width = (cr - cl).max(1.0);
    let pitch = if rb.style.line_spacing > 0.0 {
        size * 1.2 * rb.style.line_spacing
    } else {
        rb.orig_pitch.map(|p| p * size / rb.orig_size.max(0.1)).unwrap_or(size * 1.2)
    };
    let left = if rb.style.align == Align::Left { x0 } else { cl };
    let (placed, geo) = {
        let mut met = PdfMetrics { objs: &mut objs, font: tok, size, cache: HashMap::new(), hh: None };
        let lines = wrap_text(&rb.text, if rb.wrap { Some(width) } else { None }, &mut met);
        layout(
            &LayoutIn { lines: &lines, left, width, align: rb.style.align, base0, pitch, letter_spacing: rb.style.letter_spacing },
            &mut met,
        )
    };
    let first_new = objs.len() as usize;
    for p in &placed {
        let mut o = objs.create_text_object(PdfPoints::new(p.x), PdfPoints::new(p.base), &p.text, tok, PdfPoints::new(size))?;
        o.set_fill_color(col)?;
    }
    let th = (size / 18.0).max(0.5);
    for g in &geo {
        let mut deco = |yc: f32| -> Result<(), PdfiumError> {
            let r = PdfRect::new(PdfPoints::new(yc - th / 2.0), PdfPoints::new(g.x0), PdfPoints::new(yc + th / 2.0), PdfPoints::new(g.x1));
            objs.create_path_object_rect(r, None, None, Some(col))?;
            Ok(())
        };
        if rb.style.underline {
            deco(g.base - 0.12 * size)?;
        }
        if rb.style.strike {
            deco(g.base + 0.30 * size)?;
        }
    }
    Ok(first_new)
}

impl Tab {
    /// Style state of a text block (to initialize the properties panel).
    pub fn read_text_style(&mut self, page: usize, idx: usize) -> Option<(String, TextStyle, String)> {
        let info = self.info(page)?;
        let obj = info.object(idx)?;
        let block = obj.block.as_ref()?;
        let first = block.first_run();
        let (_, bold, italic) = fonts::parse_pdf_font_name(&first.font);
        let style = TextStyle {
            family: FontFamily::Original,
            bold,
            italic,
            underline: obj.underline,
            strike: obj.strike,
            size: first.size,
            color: first.color,
            align: block.align,
            letter_spacing: 0.0,
            line_spacing: 0.0,
        };
        Some((block.text.clone(), style, first.font.clone()))
    }

    /// Applies a new text and/or style to a text block. Returns the index of the first rebuilt
    /// object (to keep the selection) when the block was recreated.
    pub fn apply_text(&mut self, page: usize, idx: usize, new_text: &str, st: &TextStyle, mem: u32) -> Result<Option<usize>, String> {
        self.apply_text_in(page, idx, new_text, st, mem, None)
    }

    /// Re-layout of a block within a given width (side handles): the text is wrapped at `container`.
    pub fn reflow_text(&mut self, page: usize, idx: usize, container: (f32, f32), mem: u32) -> Result<Option<usize>, String> {
        let (text, st, _) = self.read_text_style(page, idx).ok_or("text not found")?;
        self.apply_text_in(page, idx, &text, &st, mem, Some(container))
    }

    /// `container`: imposed column (left, right); forces the wrapping and the rebuild.
    pub fn apply_text_in(&mut self, page: usize, idx: usize, new_text: &str, st: &TextStyle, mem: u32, container: Option<(f32, f32)>) -> Result<Option<usize>, String> {
        let forced = container.is_some();
        let info = self.info(page).ok_or("page not found")?;
        let obj = info.object(idx).ok_or("item not found")?.clone();
        let block = obj.block.clone().ok_or("not a text")?;
        let first = block.first_run().clone();
        let (_, orig_bold, orig_italic) = fonts::parse_pdf_font_name(&first.font);
        let missing = fonts::is_subset_name(&first.font) && new_text.chars().any(|c| !c.is_whitespace() && !block.text.contains(c));
        let font_same = st.family == FontFamily::Original && st.bold == orig_bold && st.italic == orig_italic && !missing;
        let same_decor = st.underline == obj.underline && st.strike == obj.strike;
        let multi = block.lines.len() > 1;
        let text_same = new_text == block.text;
        let col = PdfColor::new(st.color[0], st.color[1], st.color[2], 255);
        let size_same = (st.size - first.size).abs() < 0.01;
        let members = obj.members.clone();
        let leader = obj.idx;

        // A. Only the color changes: modify the objects in place.
        if !forced && text_same && font_same && same_decor && size_same && st.letter_spacing == 0.0 && st.align == block.align && st.line_spacing == 0.0 {
            if st.color == first.color {
                return Ok(None);
            }
            self.edit_page(page, "Text color", mem, |pg, _| {
                let objs = pg.objects_mut();
                for k in &members {
                    let mut o = objs.get(*k)?;
                    o.set_fill_color(col)?;
                }
                Ok(())
            })?;
            return Ok(Some(leader));
        }

        // B. Single line, same font, no decoration: set_text in place (rendering identical to the original).
        let in_place = !forced
            && !multi
            && font_same
            && !obj.underline
            && !obj.strike
            && !st.underline
            && !st.strike
            && st.letter_spacing == 0.0
            && st.align == block.align
            && st.line_spacing == 0.0
            && !new_text.contains('\n')
            && (st.align == Align::Left || st.align == Align::Justify);
        if in_place {
            let size = st.size;
            let mut new_leader = leader;
            self.edit_page(page, "Edit text", mem, |pg, _| {
                let objs = pg.objects_mut();
                let mut others: Vec<usize> = members.iter().copied().filter(|&k| k != leader).collect();
                others.sort_unstable_by(|a, b| b.cmp(a));
                let shift = others.iter().filter(|&&k| k < leader).count();
                for k in others {
                    let o = objs.get(k)?;
                    objs.remove_object(o)?;
                }
                new_leader = leader - shift;
                let mut o = objs.get(new_leader)?;
                if let Some(t) = o.as_text_object_mut() {
                    t.set_text(new_text)?;
                    t.set_unscaled_font_size(PdfPoints::new(size)).ok();
                }
                o.set_fill_color(col).ok();
                Ok(())
            })?;
            return Ok(Some(new_leader));
        }

        // C. Full rebuild of the block.
        let (serif, mono) = {
            let p = self.doc.pages().get(page as i32).map_err(|e| format!("{e:?}"))?;
            let o = p.objects().get(leader).map_err(|e| format!("{e:?}"))?;
            match o.as_text_object() {
                Some(t) => {
                    let f = t.font();
                    (f.is_serif(), f.is_fixed_pitch())
                }
                None => (false, false),
            }
        };
        let plan = if font_same {
            FontPlan::Keep
        } else {
            plan_font(&st.family, st.bold, st.italic, &first.font, serif, mono)
        };
        let orig_pitch = if multi { Some(block.lines[0].base - block.lines[1].base) } else { None };
        let rb = Rebuild {
            members,
            leader,
            text: new_text.to_string(),
            style: st.clone(),
            plan,
            wrap: multi || forced,
            container: container.unwrap_or((block.rect[0], block.rect[2])),
            orig_pitch,
            orig_size: first.size,
        };
        let mut first_new = 0usize;
        self.edit_page(page, "Edit text", mem, |pg, doc| {
            first_new = rebuild_text(pg, doc, &rb)?;
            Ok(())
        })?;
        Ok(Some(first_new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ObjKind;

    /// Each character is 10 units wide.
    struct Fixed;
    impl Measure for Fixed {
        fn char_adv(&mut self, _c: char) -> f32 {
            10.0
        }
    }

    #[test]
    fn wraps_at_word_boundaries() {
        let l = wrap_text("aaa bbb ccc ddd", Some(75.0), &mut Fixed);
        let texts: Vec<_> = l.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(texts, vec!["aaa bbb", "ccc ddd"]);
        assert_eq!(l.iter().map(|(_, last)| *last).collect::<Vec<_>>(), vec![false, true]);
    }

    #[test]
    fn hard_newlines_and_no_wrap() {
        let l = wrap_text("ab\ncd", None, &mut Fixed);
        assert_eq!(l, vec![("ab".to_string(), true), ("cd".to_string(), true)]);
        let l = wrap_text("a b c d e f g", None, &mut Fixed);
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn long_word_is_broken() {
        let l = wrap_text("abcdefghij", Some(35.0), &mut Fixed);
        assert!(l.iter().all(|(s, _)| s.chars().count() <= 3), "{l:?}");
        assert_eq!(l.iter().map(|(s, _)| s.as_str()).collect::<String>(), "abcdefghij");
    }

    #[test]
    fn alignment_positions() {
        let lines = vec![("abcd".to_string(), true)]; // width 40
        let mk = |a| LayoutIn { lines: &lines, left: 100.0, width: 100.0, align: a, base0: 500.0, pitch: 14.0, letter_spacing: 0.0 };
        assert_eq!(layout(&mk(Align::Left), &mut Fixed).0[0].x, 100.0);
        assert_eq!(layout(&mk(Align::Center), &mut Fixed).0[0].x, 130.0);
        assert_eq!(layout(&mk(Align::Right), &mut Fixed).0[0].x, 160.0);
    }

    #[test]
    fn justify_fills_width_except_last_line() {
        let lines = vec![("aa bb cc".to_string(), false), ("dd ee".to_string(), true)];
        let inp = LayoutIn { lines: &lines, left: 0.0, width: 200.0, align: Align::Justify, base0: 100.0, pitch: 12.0, letter_spacing: 0.0 };
        let (placed, geo) = layout(&inp, &mut Fixed);
        // line 1: 3 words of 20 → 60, 2 gaps of 70
        assert_eq!(placed[0].x, 0.0);
        assert_eq!(placed[1].x, 90.0);
        assert_eq!(placed[2].x, 180.0);
        assert_eq!(geo[0].x1, 200.0);
        // last line: left-aligned, a single object
        assert_eq!(placed[3].text, "dd ee");
        assert_eq!(placed[3].base, 88.0);
    }

    #[test]
    fn letter_spacing_places_each_char() {
        let lines = vec![("ab c".to_string(), true)];
        let inp = LayoutIn { lines: &lines, left: 0.0, width: 100.0, align: Align::Left, base0: 0.0, pitch: 12.0, letter_spacing: 2.0 };
        let (placed, _) = layout(&inp, &mut Fixed);
        let xs: Vec<f32> = placed.iter().map(|p| p.x).collect();
        assert_eq!(xs, vec![0.0, 12.0, 36.0]); // a, b, (space skipped), c
    }

    // ------------------------------------------------------- with PDFium

    fn doc_tab(lines: &[(&str, f32)]) -> Tab {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let font = doc.fonts_mut().helvetica();
        for (text, y) in lines {
            page.objects_mut()
                .create_text_object(PdfPoints::new(72.0), PdfPoints::new(*y), *text, font, PdfPoints::new(12.0))
                .unwrap();
        }
        page.regenerate_content().unwrap();
        drop(page);
        Tab::new(p, doc, None, None, 1.0)
    }

    fn texts(t: &mut Tab) -> Vec<String> {
        let info = t.info(0).unwrap();
        info.objects.iter().filter(|o| o.kind == ObjKind::Text).map(|o| o.text().to_string()).collect()
    }

    #[test]
    fn in_place_edit_keeps_single_object() {
        let mut t = doc_tab(&[("Hello", 700.0)]);
        let idx = t.info(0).unwrap().objects[0].idx;
        let (txt, st, _) = t.read_text_style(0, idx).unwrap();
        assert_eq!(txt, "Hello");
        t.apply_text(0, idx, "Hi", &st, 100).unwrap();
        assert_eq!(texts(&mut t), vec!["Hi"]);
        assert_eq!(t.info(0).unwrap().objects.len(), 1);
    }

    #[test]
    fn color_only_edit_is_in_place() {
        let mut t = doc_tab(&[("Hello", 700.0)]);
        let idx = t.info(0).unwrap().objects[0].idx;
        let (txt, mut st, _) = t.read_text_style(0, idx).unwrap();
        st.color = [255, 0, 0];
        t.apply_text(0, idx, &txt, &st, 100).unwrap();
        let info = t.info(0).unwrap();
        assert_eq!(info.objects[0].block.as_ref().unwrap().first_run().color, [255, 0, 0]);
    }

    #[test]
    fn multiline_paragraph_is_rewrapped() {
        // three lines of a paragraph (right-justified at ~520)
        let mut t = doc_tab(&[
            ("Lorem ipsum dolor sit amet consectetur adipiscing elit sed do", 700.0),
            ("eiusmod tempor incididunt ut labore et dolore magna aliqua ut", 686.0),
            ("enim ad minim", 672.0),
        ]);
        let info = t.info(0).unwrap();
        let para = info.objects.iter().find(|o| o.kind == ObjKind::Text && o.block.as_ref().unwrap().lines.len() == 3).expect("3-line paragraph").clone();
        let (txt, st, _) = t.read_text_style(0, para.idx).unwrap();
        assert!(txt.starts_with("Lorem ipsum"));
        // We lengthen the text: it must wrap onto more lines, without exceeding the original width.
        let longer = format!("{txt} and still many additional words to force one more line break");
        let new_idx = t.apply_text(0, para.idx, &longer, &st, 100).unwrap().expect("rebuilt");
        let info2 = t.info(0).unwrap();
        let b = info2.object(new_idx).expect("rebuilt block found").block.as_ref().unwrap();
        assert!(b.lines.len() >= 4, "{} lines: {:?} / original width {:?}", b.lines.len(), b.lines.iter().map(|l| (l.text.clone(), l.rect[2] - l.rect[0])).collect::<Vec<_>>(), para.rect);
        let max_right = para.rect[2];
        assert!(b.rect[2] <= max_right + 1.5, "exceeds the margin: {} > {}", b.rect[2], max_right);
        assert!(b.text.contains("additional"));
    }

    #[test]
    fn style_change_creates_underline_and_is_detected() {
        let mut t = doc_tab(&[("Underline-me", 700.0)]);
        let idx = t.info(0).unwrap().objects[0].idx;
        let (txt, mut st, _) = t.read_text_style(0, idx).unwrap();
        st.underline = true;
        let n = t.apply_text(0, idx, &txt, &st, 100).unwrap().unwrap();
        let info = t.info(0).unwrap();
        let o = info.object(n).unwrap();
        assert!(o.underline, "underline detected");
        assert!(o.members.len() >= 2, "text + line");
        assert_eq!(info.objects.iter().filter(|o| o.kind == ObjKind::Path).count(), 0, "the line is attached to the block");
        // We remove it
        let (txt, mut st2, _) = t.read_text_style(0, n).unwrap();
        assert!(st2.underline);
        st2.underline = false;
        t.apply_text(0, n, &txt, &st2, 100).unwrap();
        let info = t.info(0).unwrap();
        assert!(!info.objects.iter().any(|o| o.underline));
        assert_eq!(info.objects.len(), 1);
    }

    #[test]
    fn reflow_into_a_narrower_column() {
        let mut t = doc_tab(&[("Lorem ipsum dolor sit amet consectetur adipiscing elit sed do", 700.0)]);
        let idx = t.info(0).unwrap().objects[0].idx;
        let new = t.reflow_text(0, idx, (72.0, 200.0), 100).unwrap().unwrap();
        let info = t.info(0).unwrap();
        let b = info.object(new).unwrap().block.as_ref().unwrap();
        assert!(b.lines.len() >= 3, "{} lines", b.lines.len());
        assert!(b.rect[2] <= 201.5, "does not exceed the column: {}", b.rect[2]);
        assert!(b.text.starts_with("Lorem ipsum") && b.text.ends_with("sed do"));
    }

    #[test]
    fn bold_switches_font_and_keeps_text() {
        let mut t = doc_tab(&[("Bold", 700.0)]);
        let idx = t.info(0).unwrap().objects[0].idx;
        let (txt, mut st, _) = t.read_text_style(0, idx).unwrap();
        st.bold = true;
        let n = t.apply_text(0, idx, &txt, &st, 100).unwrap().unwrap();
        let info = t.info(0).unwrap();
        let b = info.object(n).unwrap().block.as_ref().unwrap();
        assert_eq!(b.text, "Bold");
        let (_, bold, _) = fonts::parse_pdf_font_name(&b.first_run().font);
        assert!(bold, "font: {}", b.first_run().font);
    }

    #[test]
    fn alignment_and_letter_spacing_rebuild() {
        let mut t = doc_tab(&[("Centre", 700.0)]);
        let idx = t.info(0).unwrap().objects[0].idx;
        let before = t.info(0).unwrap().objects[0].rect;
        let (txt, mut st, _) = t.read_text_style(0, idx).unwrap();
        st.letter_spacing = 3.0;
        let n = t.apply_text(0, idx, &txt, &st, 100).unwrap().unwrap();
        let info = t.info(0).unwrap();
        let o = info.object(n).unwrap();
        assert!(o.rect[2] - o.rect[0] > before[2] - before[0] + 8.0, "the text stretches with the spacing");
        assert_eq!(o.text().replace(' ', ""), "Centre");
    }
}
