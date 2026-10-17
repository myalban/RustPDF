//! Operations on the objects of a page: move, resize, delete, order, alignment,
//! rotation, copy/paste, groups, shapes, image retouching.

use crate::edit_text::{self, TextStyle};
use crate::fonts::{self, FontFamily};
use crate::geom::*;
use crate::model::*;
use crate::pathdata::*;
use crate::tab::Tab;
use image::DynamicImage;
use pdfium_render::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZOrder {
    Front,
    Back,
    Forward,
    Backward,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlignKind {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Distribute {
    Horizontal,
    Vertical,
}

/// Copied item (independent of the source document).
#[derive(Clone)]
pub enum ClipItem {
    Text { text: String, font: String, size: f32, color: [u8; 3], x: f32, y: f32 },
    Image { img: DynamicImage, rect: R4 },
    Path(PathData),
}

#[derive(Clone)]
pub struct Clip {
    pub items: Vec<ClipItem>,
    pub bounds: R4,
}

fn err<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

/// PDF indices (sorted, without duplicates) of all the members of the given items.
pub fn members_of(info: &PageInfo, items: &[usize]) -> Vec<usize> {
    let mut v: Vec<usize> = items.iter().filter_map(|i| info.object(*i)).flat_map(|o| o.members.clone()).collect();
    v.sort_unstable();
    v.dedup();
    v
}

pub fn union_rect(info: &PageInfo, items: &[usize]) -> Option<R4> {
    let mut it = items.iter().filter_map(|i| info.object(*i)).map(|o| o.rect);
    let first = it.next()?;
    Some(it.fold(first, |a, b| r4_union(&a, &b)))
}

impl Tab {
    fn page_info_for(&mut self, page: usize) -> Result<std::rc::Rc<PageInfo>, String> {
        self.info(page).ok_or_else(|| "page not found".to_string())
    }

    // ------------------------------------------------------ basic operations

    pub fn delete_items(&mut self, page: usize, items: &[usize], mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let mut m = members_of(&info, items);
        m.sort_unstable_by(|a, b| b.cmp(a));
        if m.is_empty() {
            return Ok(());
        }
        self.edit_page(page, "Delete", mem, |pg, _| {
            let objs = pg.objects_mut();
            for k in m {
                let o = objs.get(k)?;
                objs.remove_object(o)?;
            }
            Ok(())
        })
    }

    /// Moves the items by a vector (in page points).
    pub fn move_items(&mut self, page: usize, items: &[usize], dx: f32, dy: f32, mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let m = members_of(&info, items);
        self.edit_page(page, "Move", mem, |pg, _| {
            let objs = pg.objects_mut();
            for k in m {
                let mut o = objs.get(k)?;
                o.translate(PdfPoints::new(dx), PdfPoints::new(dy))?;
            }
            Ok(())
        })
    }

    /// Overall affine transformation (rotation, mirror…) : `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
    pub fn transform_items(&mut self, page: usize, items: &[usize], t: Affine, label: &str, mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let m = members_of(&info, items);
        self.edit_page(page, label, mem, |pg, _| {
            let objs = pg.objects_mut();
            for k in m {
                let mut o = objs.get(k)?;
                o.transform(t.a, t.b, t.c, t.d, t.e, t.f)?;
            }
            Ok(())
        })
    }

    /// Resizes: `old` → `new` (rectangles in page space).
    pub fn resize_items(&mut self, page: usize, items: &[usize], old: R4, new: R4, mem: u32) -> Result<(), String> {
        let (ow, oh) = (r4_w(&old), r4_h(&old));
        if ow < 0.01 || oh < 0.01 {
            return Err("original size is zero".into());
        }
        let sx = r4_w(&new) / ow;
        let sy = r4_h(&new) / oh;
        let t = Affine { a: sx, b: 0.0, c: 0.0, d: sy, e: new[0] - old[0] * sx, f: new[1] - old[1] * sy };
        self.transform_items(page, items, t, "Resize", mem)
    }

    pub fn rotate_items(&mut self, page: usize, items: &[usize], degrees: f32, mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let r = union_rect(&info, items).ok_or("nothing to rotate")?;
        let (cx, cy) = r4_center(&r);
        let a = degrees.to_radians();
        let (s, c) = a.sin_cos();
        let t = Affine { a: c, b: s, c: -s, d: c, e: cx - cx * c + cy * s, f: cy - cx * s - cy * c };
        self.transform_items(page, items, t, "Rotate", mem)
    }

    pub fn flip_items(&mut self, page: usize, items: &[usize], horizontal: bool, mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let r = union_rect(&info, items).ok_or("nothing to flip")?;
        let (cx, cy) = r4_center(&r);
        let t = if horizontal {
            Affine { a: -1.0, b: 0.0, c: 0.0, d: 1.0, e: 2.0 * cx, f: 0.0 }
        } else {
            Affine { a: 1.0, b: 0.0, c: 0.0, d: -1.0, e: 0.0, f: 2.0 * cy }
        };
        self.transform_items(page, items, t, "Flip", mem)
    }

    /// Colors and stroke of the items (paths: fill, outline, width; text: color).
    pub fn set_style_items(
        &mut self,
        page: usize,
        items: &[usize],
        fill: Option<[u8; 4]>,
        stroke: Option<[u8; 4]>,
        width: Option<f32>,
        mem: u32,
    ) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let m = members_of(&info, items);
        self.edit_page(page, "Style", mem, |pg, _| {
            let objs = pg.objects_mut();
            for k in m {
                let mut o = objs.get(k)?;
                if let Some(f) = fill {
                    o.set_fill_color(PdfColor::new(f[0], f[1], f[2], f[3]))?;
                }
                if let Some(s) = stroke {
                    o.set_stroke_color(PdfColor::new(s[0], s[1], s[2], s[3]))?;
                }
                if let Some(w) = width {
                    o.set_stroke_width(PdfPoints::new(w))?;
                }
            }
            Ok(())
        })
    }

    pub fn set_dash_items(&mut self, page: usize, items: &[usize], dash: &[f32], mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let m = members_of(&info, items);
        let arr: Vec<PdfPoints> = dash.iter().map(|v| PdfPoints::new(*v)).collect();
        self.edit_page(page, "Stroke style", mem, |pg, _| {
            let objs = pg.objects_mut();
            for k in m {
                let mut o = objs.get(k)?;
                if o.as_path_object().is_some() {
                    o.set_dash_array(&arr, PdfPoints::ZERO)?;
                }
            }
            Ok(())
        })
    }

    /// Stacking order. Returns the new PDF indices of the moved items.
    pub fn z_order(&mut self, page: usize, items: &[usize], order: ZOrder, mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let mut m = members_of(&info, items);
        if m.is_empty() {
            return Ok(());
        }
        // PDFium rewrites every object in the original content stream: merge the streams first.
        self.snapshot("Stacking order", mem);
        let merged = self.doc.save_to_bytes().map_err(err).and_then(|b| crate::pdfedit::merge_page_streams(&b, page));
        match merged {
            Ok(b) => {
                if let Err(e) = self.reload(b) {
                    self.undo.pop();
                    return Err(e);
                }
            }
            Err(e) => {
                self.undo.pop();
                return Err(e);
            }
        }
        self.edit_page_impl(page, "Stacking order", mem, false, |pg, _| {
            let objs = pg.objects_mut();
            let n = objs.len() as usize;
            match order {
                ZOrder::Front => {
                    // from highest to lowest to keep the relative order
                    m.sort_unstable_by(|a, b| b.cmp(a));
                    let mut taken = vec![];
                    for k in m {
                        let o = objs.get(k)?;
                        taken.push(objs.remove_object(o)?);
                    }
                    taken.reverse();
                    for o in taken {
                        objs.add_object(o)?;
                    }
                }
                ZOrder::Back => {
                    m.sort_unstable_by(|a, b| b.cmp(a));
                    let mut taken = vec![];
                    for k in m {
                        let o = objs.get(k)?;
                        taken.push(objs.remove_object(o)?);
                    }
                    // taken goes from highest to lowest; insert the lowest first
                    for (i, o) in taken.into_iter().rev().enumerate() {
                        objs.insert_object_at_index(i, o)?;
                    }
                }
                ZOrder::Forward | ZOrder::Backward => {
                    let fwd = order == ZOrder::Forward;
                    let mut idxs = m.clone();
                    idxs.sort_unstable();
                    if fwd {
                        idxs.reverse();
                    }
                    for k in idxs {
                        let target = if fwd { k + 1 } else { k.wrapping_sub(1) };
                        if target >= n || (!fwd && k == 0) || m.contains(&target) {
                            continue;
                        }
                        let o = objs.get(k)?;
                        let o = objs.remove_object(o)?;
                        objs.insert_object_at_index(target, o)?;
                    }
                }
            }
            Ok(())
        })
    }

    /// Aligns the items with each other (or on the page if there is only one / `to_page`).
    pub fn align_items(&mut self, page: usize, items: &[usize], kind: AlignKind, to_page: bool, mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let page_rect = {
            // whole page in page space
            let g = info.geom;
            g.rect_from_view(&[0.0, 0.0, g.w, g.h])
        };
        let target = if to_page || items.len() < 2 { page_rect } else { union_rect(&info, items).ok_or("no item")? };
        let mut moves: Vec<(Vec<usize>, f32, f32)> = vec![];
        for i in items {
            let Some(o) = info.object(*i) else { continue };
            let r = o.rect;
            let (dx, dy) = match kind {
                AlignKind::Left => (target[0] - r[0], 0.0),
                AlignKind::Right => (target[2] - r[2], 0.0),
                AlignKind::HCenter => ((target[0] + target[2]) * 0.5 - (r[0] + r[2]) * 0.5, 0.0),
                AlignKind::Top => (0.0, target[3] - r[3]),
                AlignKind::Bottom => (0.0, target[1] - r[1]),
                AlignKind::VCenter => (0.0, (target[1] + target[3]) * 0.5 - (r[1] + r[3]) * 0.5),
            };
            moves.push((o.members.clone(), dx, dy));
        }
        self.apply_moves(page, moves, "Align", mem)
    }

    /// Distributes the items with equal gaps.
    pub fn distribute_items(&mut self, page: usize, items: &[usize], d: Distribute, mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let mut objs: Vec<&ObjInfo> = items.iter().filter_map(|i| info.object(*i)).collect();
        if objs.len() < 3 {
            return Err("Select at least three items".into());
        }
        let key = |o: &ObjInfo| if d == Distribute::Horizontal { o.rect[0] } else { o.rect[1] };
        objs.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
        let (lo, hi, size): (fn(&R4) -> f32, fn(&R4) -> f32, fn(&R4) -> f32) = if d == Distribute::Horizontal {
            (|r| r[0], |r| r[2], |r| r[2] - r[0])
        } else {
            (|r| r[1], |r| r[3], |r| r[3] - r[1])
        };
        let first_lo = lo(&objs[0].rect);
        let last_hi = hi(&objs.last().unwrap().rect);
        let total: f32 = objs.iter().map(|o| size(&o.rect)).sum();
        let gap = (last_hi - first_lo - total) / (objs.len() as f32 - 1.0);
        let mut pos = first_lo;
        let mut moves = vec![];
        for o in &objs {
            let delta = pos - lo(&o.rect);
            moves.push(if d == Distribute::Horizontal { (o.members.clone(), delta, 0.0) } else { (o.members.clone(), 0.0, delta) });
            pos += size(&o.rect) + gap;
        }
        self.apply_moves(page, moves, "Distribute", mem)
    }

    fn apply_moves(&mut self, page: usize, moves: Vec<(Vec<usize>, f32, f32)>, label: &str, mem: u32) -> Result<(), String> {
        self.edit_page(page, label, mem, |pg, _| {
            let objs = pg.objects_mut();
            for (members, dx, dy) in moves {
                for k in members {
                    let mut o = objs.get(k)?;
                    o.translate(PdfPoints::new(dx), PdfPoints::new(dy))?;
                }
            }
            Ok(())
        })
    }

    // ---------------------------------------------------- copy / paste

    /// Copies items (text, images, paths) independently of the document.
    pub fn copy_items(&mut self, page: usize, items: &[usize]) -> Result<Clip, String> {
        let info = self.page_info_for(page)?;
        let m = members_of(&info, items);
        let bounds = union_rect(&info, items).ok_or("nothing to copy")?;
        let pg = self.read_page(page)?;
        let mut out = vec![];
        for k in m {
            let mut o = pg.objects().get(k).map_err(err)?;
            if let Some(it) = read_clip_item(&mut o, &self.doc) {
                out.push(it);
            }
        }
        if out.is_empty() {
            return Err("These items cannot be copied".into());
        }
        Ok(Clip { items: out, bounds })
    }

    /// Pastes items: centered on `at` (page space) or offset by 12 pt.
    /// Returns the index of the first object created.
    pub fn paste_clip(&mut self, page: usize, clip: &Clip, at: Option<(f32, f32)>, mem: u32) -> Result<usize, String> {
        let (cx, cy) = r4_center(&clip.bounds);
        let (dx, dy) = match at {
            Some((x, y)) => (x - cx, y - cy),
            None => (12.0, -12.0),
        };
        let items = clip.items.clone();
        let mut first = 0usize;
        self.edit_page(page, "Paste", mem, |pg, doc| {
            let mut objs = pg.objects_mut();
            first = objs.len() as usize;
            for it in &items {
                create_clip_item(doc, &mut objs, it, dx, dy)?;
            }
            Ok(())
        })?;
        Ok(first)
    }

    // ------------------------------------------------------------ groups

    /// Groups items into a single "form" object; returns its index.
    pub fn group_items(&mut self, page: usize, items: &[usize], mem: u32) -> Result<usize, String> {
        let info = self.page_info_for(page)?;
        let m = members_of(&info, items);
        if m.len() < 2 {
            return Err("Select at least two objects".into());
        }
        let mut new_idx = 0usize;
        self.edit_page(page, "Group", mem, |pg, doc| {
            let form: PdfPageObject<'static> = {
                let objects = {
                    let objs = pg.objects();
                    m.iter().map(|k| objs.get(*k)).collect::<Result<Vec<_>, _>>()?
                };
                let pg_ref: &PdfPage = &*pg;
                let mut group = PdfPageGroupObject::from_vec(pg_ref, objects)?;
                let form = group.copy_into_x_object_form_object(shorten_doc(doc))?;
                // SAFETY: the pdfium-render lifetimes are mere markers (PhantomData);
                // the form belongs to the document, which outlives this function.
                unsafe { std::mem::transmute(form) }
            };
            {
                let objs = pg.objects_mut();
                let mut desc = m.clone();
                desc.sort_unstable_by(|a, b| b.cmp(a));
                for k in desc {
                    let o = objs.get(k)?;
                    objs.remove_object(o)?;
                }
            }
            // PDFium only writes the new form object if the content was regenerated between the two steps.
            pg.regenerate_content()?;
            let objs = pg.objects_mut();
            objs.add_object(form)?;
            new_idx = objs.len() as usize - 1;
            Ok(())
        })?;
        Ok(new_idx)
    }

    /// Ungroups a form object by recreating its items on the page. Returns the first index created.
    pub fn ungroup_item(&mut self, page: usize, item: usize, mem: u32) -> Result<usize, String> {
        let info = self.page_info_for(page)?;
        let o = info.object(item).ok_or("item not found")?;
        if o.kind != ObjKind::Form {
            return Err("Not a group".into());
        }
        let k = o.idx;
        let mut first = 0usize;
        self.edit_page(page, "Ungroup", mem, |pg, doc| {
            let mut clips: Vec<ClipItem> = vec![];
            {
                let form = pg.objects().get(k)?;
                let fm = form.matrix()?;
                let aff = Affine { a: fm.a(), b: fm.b(), c: fm.c(), d: fm.d(), e: fm.e(), f: fm.f() };
                if let Some(f) = form.as_x_object_form_object() {
                    for mut child in f.iter() {
                        if let Some(mut it) = read_clip_item(&mut child, doc) {
                            transform_clip_item(&mut it, &aff);
                            clips.push(it);
                        }
                    }
                }
            }
            let mut objs = pg.objects_mut();
            let o = objs.get(k)?;
            objs.remove_object(o)?;
            first = objs.len() as usize;
            for it in &clips {
                create_clip_item(doc, &mut objs, it, 0.0, 0.0)?;
            }
            Ok(())
        })?;
        Ok(first)
    }

    // -------------------------------------------------------------- shapes

    /// Adds a path (shape) to the page; returns its index.
    pub fn add_path(&mut self, page: usize, data: &PathData, label: &str, mem: u32) -> Result<usize, String> {
        let mut idx = 0usize;
        self.edit_page(page, label, mem, |pg, doc| {
            let mut objs = pg.objects_mut();
            build_path(doc, &mut objs, data)?;
            idx = objs.len() as usize - 1;
            Ok(())
        })?;
        Ok(idx)
    }

    /// Changes a point of a path (anchor or control handle) by recreating the path at the same stacking position.
    pub fn edit_path_point(&mut self, page: usize, idx: usize, seg: usize, kind: PtKind, x: f32, y: f32, mem: u32) -> Result<(), String> {
        self.edit_page(page, "Edit path", mem, |pg, doc| {
            let mut objs = pg.objects_mut();
            let o = objs.get(idx)?;
            let mut d = read_path(&o).ok_or(PdfiumError::PdfiumFunctionReturnValueIndicatedFailure)?;
            d.set_point(seg, kind, x, y);
            let old = objs.get(idx)?;
            objs.remove_object(old)?;
            build_path(doc, &mut objs, &d)?;
            // put the new object (last) back at its original place
            let last = objs.len() as usize - 1;
            if idx < last {
                let o = objs.get(last)?;
                let o = objs.remove_object(o)?;
                objs.insert_object_at_index(idx, o)?;
            }
            Ok(())
        })
    }

    /// Reads the vector data of a path (page coordinates).
    pub fn read_path_data(&mut self, page: usize, idx: usize) -> Option<PathData> {
        let pg = self.read_page(page).ok()?;
        let o = pg.objects().get(idx).ok()?;
        read_path(&o)
    }

    // -------------------------------------------------------------- text / images

    /// Adds a text (one or several lines) at the given position (origin, baseline); returns the index.
    pub fn add_text(&mut self, page: usize, x: f32, y: f32, text: &str, st: &TextStyle, mem: u32) -> Result<usize, String> {
        let st = st.clone();
        let text = text.to_string();
        let mut idx = 0usize;
        self.edit_page(page, "Add text", mem, |pg, doc| {
            let plan = edit_text::plan_font(&st.family, st.bold, st.italic, "", false, false);
            let tok = match &plan {
                edit_text::FontPlan::Builtin(b, bo, it) => builtin_token(doc, *b, *bo, *it),
                edit_text::FontPlan::System(bytes) => doc.fonts_mut().load_true_type_from_bytes(bytes, true).unwrap_or_else(|_| doc.fonts_mut().helvetica()),
                edit_text::FontPlan::Keep => doc.fonts_mut().helvetica(),
            };
            let objs = pg.objects_mut();
            idx = objs.len() as usize;
            let col = PdfColor::new(st.color[0], st.color[1], st.color[2], 255);
            for (k, line) in text.split('\n').enumerate() {
                if line.is_empty() {
                    continue;
                }
                let base = y - k as f32 * st.size * 1.2;
                let mut o = objs.create_text_object(PdfPoints::new(x), PdfPoints::new(base), line, tok, PdfPoints::new(st.size))?;
                o.set_fill_color(col)?;
            }
            Ok(())
        })?;
        Ok(idx)
    }

    /// Adds an image whose top-left corner is at (x, y); width in points, height proportional.
    pub fn add_image(&mut self, page: usize, x: f32, y: f32, img: &DynamicImage, width: f32, mem: u32) -> Result<usize, String> {
        let h = width * img.height() as f32 / img.width().max(1) as f32;
        let mut idx = 0usize;
        self.edit_page(page, "Add image", mem, |pg, _| {
            let objs = pg.objects_mut();
            objs.create_image_object(PdfPoints::new(x), PdfPoints::new(y - h), img, Some(PdfPoints::new(width)), Some(PdfPoints::new(h)))?;
            idx = objs.len() as usize - 1;
            Ok(())
        })?;
        Ok(idx)
    }

    /// Image of an image object (processed: masks and transformations applied).
    pub fn read_image(&mut self, page: usize, idx: usize) -> Result<DynamicImage, String> {
        let pg = self.read_page(page)?;
        let mut o = pg.objects().get(idx).map_err(err)?;
        processed_image(&mut o, &self.doc).ok_or_else(|| "not an image".to_string())
    }

    /// Replaces the pixels of an image while keeping its position and size.
    pub fn replace_image(&mut self, page: usize, idx: usize, img: &DynamicImage, mem: u32) -> Result<(), String> {
        self.edit_page(page, "Replace image", mem, |pg, _| {
            let objs = pg.objects_mut();
            let mut o = objs.get(idx)?;
            if let Some(io) = o.as_image_object_mut() {
                io.set_image(img)?;
            }
            Ok(())
        })
    }

    /// Crops an image (fractions 0..1 of the left/top/right/bottom edge kept).
    pub fn crop_image(&mut self, page: usize, idx: usize, fr: [f32; 4], mem: u32) -> Result<(), String> {
        let info = self.page_info_for(page)?;
        let old = info.object(idx).ok_or("image not found")?.rect;
        let img = self.read_image(page, idx)?;
        let (w, h) = (img.width() as f32, img.height() as f32);
        let x0 = (fr[0].clamp(0.0, 1.0) * w) as u32;
        let y0 = (fr[1].clamp(0.0, 1.0) * h) as u32;
        let x1 = ((fr[2].clamp(0.0, 1.0) * w) as u32).max(x0 + 1);
        let y1 = ((fr[3].clamp(0.0, 1.0) * h) as u32).max(y0 + 1);
        let cropped = img.crop_imm(x0, y0, x1 - x0, y1 - y0);
        // resulting rectangle in page space: the image Y axis is inverted
        let (ow, oh) = (r4_w(&old), r4_h(&old));
        let new = [old[0] + fr[0] * ow, old[3] - fr[3] * oh, old[0] + fr[2] * ow, old[3] - fr[1] * oh];
        self.edit_page(page, "Crop image", mem, |pg, _| {
            let objs = pg.objects_mut();
            let mut o = objs.get(idx)?;
            if let Some(io) = o.as_image_object_mut() {
                io.set_image(&cropped)?;
            }
            let (sx, sy) = (r4_w(&new) / ow, r4_h(&new) / oh);
            o.transform(sx, 0.0, 0.0, sy, new[0] - old[0] * sx, new[1] - old[1] * sy)?;
            Ok(())
        })
    }

    /// Image adjustments: brightness (-100..100), contrast (-100..100), grayscale, rotation by multiples of 90°.
    pub fn adjust_image(&mut self, page: usize, idx: usize, brightness: i32, contrast: f32, gray: bool, mem: u32) -> Result<(), String> {
        let img = self.read_image(page, idx)?;
        let mut out = img;
        if brightness != 0 {
            out = out.brighten(brightness);
        }
        if contrast != 0.0 {
            out = out.adjust_contrast(contrast);
        }
        if gray {
            out = DynamicImage::ImageLuma8(out.to_luma8());
            out = DynamicImage::ImageRgba8(out.to_rgba8());
        }
        self.replace_image(page, idx, &out, mem)
    }

    /// Reduces the resolution of an image to make the file lighter (maximum width in pixels).
    pub fn downscale_image(&mut self, page: usize, idx: usize, max_px: u32, mem: u32) -> Result<(), String> {
        let img = self.read_image(page, idx)?;
        if img.width() <= max_px && img.height() <= max_px {
            return Ok(());
        }
        let out = img.resize(max_px, max_px, image::imageops::FilterType::Lanczos3);
        self.replace_image(page, idx, &out, mem)
    }
}

// ----------------------------------------------------------- clip: reading / creation

/// Reads the image of an image object. PDFium (`FPDFImageObj_GetRenderedBitmap`) can leave the object's
/// matrix altered: it is always restored so the document is never modified by reading it.
pub fn processed_image(o: &mut PdfPageObject, doc: &PdfDocument<'static>) -> Option<DynamicImage> {
    let before = o.matrix().ok();
    let img = o.as_image_object().and_then(|io| io.get_processed_image(doc).or_else(|_| io.get_raw_image()).ok());
    if let Some(m) = before {
        restore_matrix(o, &m);
    }
    img
}

fn read_clip_item(o: &mut PdfPageObject, doc: &PdfDocument<'static>) -> Option<ClipItem> {
    match o.object_type() {
        PdfPageObjectType::Text => {
            let t = o.as_text_object()?;
            let text = t.text();
            if text.trim().is_empty() {
                return None;
            }
            let m = o.matrix().ok()?;
            let color = o.fill_color().map(|c| [c.red(), c.green(), c.blue()]).unwrap_or([0, 0, 0]);
            Some(ClipItem::Text { text, font: t.font().name(), size: t.scaled_font_size().value, color, x: m.e(), y: m.f() })
        }
        PdfPageObjectType::Image => {
            let img = processed_image(o, doc)?;
            let b = o.bounds().ok()?.to_rect();
            Some(ClipItem::Image { img, rect: [b.left().value, b.bottom().value, b.right().value, b.top().value] })
        }
        PdfPageObjectType::Path => read_path(o).map(ClipItem::Path),
        _ => None,
    }
}

fn transform_clip_item(it: &mut ClipItem, a: &Affine) {
    match it {
        ClipItem::Text { x, y, size, .. } => {
            let (nx, ny) = a.apply(*x, *y);
            *x = nx;
            *y = ny;
            *size *= (a.a * a.d - a.b * a.c).abs().sqrt();
        }
        ClipItem::Image { rect, .. } => {
            let b = a.bbox(rect);
            *rect = b;
        }
        ClipItem::Path(p) => {
            p.transform(a);
            let s = (a.a * a.d - a.b * a.c).abs().sqrt();
            p.width *= s;
        }
    }
}

fn builtin_token(doc: &mut PdfDocument<'static>, b: fonts::Builtin, bold: bool, italic: bool) -> PdfFontToken {
    use fonts::Builtin::*;
    let f = doc.fonts_mut();
    match (b, bold, italic) {
        (Helvetica, false, false) => f.helvetica(),
        (Helvetica, true, false) => f.helvetica_bold(),
        (Helvetica, false, true) => f.helvetica_oblique(),
        (Helvetica, true, true) => f.helvetica_bold_oblique(),
        (Times, false, false) => f.times_roman(),
        (Times, true, false) => f.times_bold(),
        (Times, false, true) => f.times_italic(),
        (Times, true, true) => f.times_bold_italic(),
        (Courier, false, false) => f.courier(),
        (Courier, true, false) => f.courier_bold(),
        (Courier, false, true) => f.courier_oblique(),
        (Courier, true, true) => f.courier_bold_oblique(),
    }
}

fn create_clip_item(doc: &mut PdfDocument<'static>, objs: &mut PdfPageObjects<'static>, it: &ClipItem, dx: f32, dy: f32) -> Result<(), PdfiumError> {
    match it {
        ClipItem::Text { text, font, size, color, x, y } => {
            let (fam, bold, italic) = fonts::parse_pdf_font_name(font);
            let plan = edit_text::plan_font(&FontFamily::Original, bold, italic, font, false, false);
            let tok = match plan {
                edit_text::FontPlan::Builtin(b, bo, i) => builtin_token(doc, b, bo, i),
                edit_text::FontPlan::System(bytes) => doc.fonts_mut().load_true_type_from_bytes(&bytes, true).unwrap_or_else(|_| doc.fonts_mut().helvetica()),
                edit_text::FontPlan::Keep => doc.fonts_mut().helvetica(),
            };
            let _ = fam;
            let mut o = objs.create_text_object(PdfPoints::new(x + dx), PdfPoints::new(y + dy), text, tok, PdfPoints::new(*size))?;
            o.set_fill_color(PdfColor::new(color[0], color[1], color[2], 255))?;
        }
        ClipItem::Image { img, rect } => {
            objs.create_image_object(
                PdfPoints::new(rect[0] + dx),
                PdfPoints::new(rect[1] + dy),
                img,
                Some(PdfPoints::new(r4_w(rect))),
                Some(PdfPoints::new(r4_h(rect))),
            )?;
        }
        ClipItem::Path(p) => {
            let mut p = p.clone();
            p.transform(&Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: dx, f: dy });
            build_path(doc, objs, &p)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tab::Tab;

    fn solid(c: [u8; 4]) -> DynamicImage {
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(40, 40, image::Rgba(c)))
    }

    /// Page with: a text, a red image, a blue rectangle, a green rectangle.
    fn fixture() -> Tab {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let font = doc.fonts_mut().helvetica();
        {
            let mut objs = page.objects_mut();
            objs.create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), "Title", font, PdfPoints::new(20.0)).unwrap();
            objs.create_image_object(PdfPoints::new(100.0), PdfPoints::new(500.0), &solid([255, 0, 0, 255]), Some(PdfPoints::new(80.0)), Some(PdfPoints::new(80.0))).unwrap();
            let mut b = PathData::rect([300.0, 500.0, 360.0, 560.0]);
            b.fill = Some([0, 0, 255, 255]);
            b.stroke = None;
            build_path(&doc, &mut objs, &b).unwrap();
            let mut g = PathData::rect([320.0, 520.0, 400.0, 580.0]);
            g.fill = Some([0, 200, 0, 255]);
            g.stroke = None;
            build_path(&doc, &mut objs, &g).unwrap();
        }
        page.regenerate_content().unwrap();
        drop(page);
        Tab::new(p, doc, None, None, 1.0)
    }

    fn find(t: &mut Tab, kind: ObjKind, nth: usize) -> ObjInfo {
        let info = t.info(0).unwrap();
        info.objects.iter().filter(|o| o.kind == kind).nth(nth).cloned().expect("object")
    }

    #[test]
    fn move_and_resize() {
        let mut t = fixture();
        let img = find(&mut t, ObjKind::Image, 0);
        t.move_items(0, &[img.idx], 10.0, -20.0, 100).unwrap();
        let moved = find(&mut t, ObjKind::Image, 0);
        assert!((moved.rect[0] - img.rect[0] - 10.0).abs() < 0.2 && (moved.rect[1] - img.rect[1] + 20.0).abs() < 0.2);
        let new = [moved.rect[0], moved.rect[1], moved.rect[0] + 160.0, moved.rect[1] + 40.0];
        t.resize_items(0, &[moved.idx], moved.rect, new, 100).unwrap();
        let r = find(&mut t, ObjKind::Image, 0).rect;
        assert!((r[2] - r[0] - 160.0).abs() < 0.3 && (r[3] - r[1] - 40.0).abs() < 0.3, "{r:?}");
        assert!((r[0] - new[0]).abs() < 0.3 && (r[1] - new[1]).abs() < 0.3);
    }

    #[test]
    fn delete_several_items_keeps_the_others() {
        let mut t = fixture();
        let img = find(&mut t, ObjKind::Image, 0);
        let txt = find(&mut t, ObjKind::Text, 0);
        t.delete_items(0, &[img.idx, txt.idx], 100).unwrap();
        let info = t.info(0).unwrap();
        assert_eq!(info.objects.len(), 2, "the two rectangles remain");
        assert!(info.objects.iter().all(|o| o.kind == ObjKind::Path));
    }

    #[test]
    fn z_order_front_and_back() {
        let mut t = fixture();
        let blue = find(&mut t, ObjKind::Path, 0);
        let n = t.info(0).unwrap().objects.len();
        t.z_order(0, &[blue.idx], ZOrder::Front, 100).unwrap();
        let info = t.info(0).unwrap();
        let top = info.objects.last().unwrap();
        assert_eq!(top.rect, blue.rect, "the blue rectangle is now in front");
        assert_eq!(info.objects.len(), n);
        t.z_order(0, &[top.idx], ZOrder::Back, 100).unwrap();
        let info = t.info(0).unwrap();
        assert_eq!(info.objects[0].rect, blue.rect, "then in the background");
        let first = info.objects[0].idx;
        t.z_order(0, &[first], ZOrder::Forward, 100).unwrap();
        let info = t.info(0).unwrap();
        assert_eq!(info.objects[1].rect, blue.rect, "moved forward one step");
    }

    #[test]
    fn align_and_distribute() {
        let mut t = fixture();
        let a = find(&mut t, ObjKind::Path, 0);
        let b = find(&mut t, ObjKind::Path, 1);
        t.align_items(0, &[a.idx, b.idx], AlignKind::Left, false, 100).unwrap();
        let (ra, rb) = (find(&mut t, ObjKind::Path, 0).rect, find(&mut t, ObjKind::Path, 1).rect);
        assert!((ra[0] - rb[0]).abs() < 0.2, "{ra:?} {rb:?}");
        t.align_items(0, &[a.idx], AlignKind::HCenter, true, 100).unwrap();
        let ra = find(&mut t, ObjKind::Path, 0).rect;
        assert!(((ra[0] + ra[2]) * 0.5 - 297.5).abs() < 0.5, "centered on the A4 page: {ra:?}");
        // distribute three items
        let img = find(&mut t, ObjKind::Image, 0);
        let info = t.info(0).unwrap();
        let ids: Vec<usize> = info.objects.iter().filter(|o| o.kind != ObjKind::Text).map(|o| o.idx).collect();
        assert_eq!(ids.len(), 3);
        t.move_items(0, &[img.idx], -50.0, 0.0, 100).unwrap();
        t.distribute_items(0, &ids, Distribute::Horizontal, 100).unwrap();
        let info = t.info(0).unwrap();
        let mut xs: Vec<R4> = info.objects.iter().filter(|o| o.kind != ObjKind::Text).map(|o| o.rect).collect();
        xs.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
        let g1 = xs[1][0] - xs[0][2];
        let g2 = xs[2][0] - xs[1][2];
        assert!((g1 - g2).abs() < 0.5, "equal gaps: {g1} vs {g2}");
    }

    #[test]
    fn rotate_and_flip() {
        let mut t = fixture();
        let a = find(&mut t, ObjKind::Path, 0); // 60x60
        let img = find(&mut t, ObjKind::Image, 0); // 80x80
        // 60x60 rectangle rotated by 45° → box ≈ 84.8
        t.rotate_items(0, &[a.idx], 45.0, 100).unwrap();
        let r = find(&mut t, ObjKind::Path, 0).rect;
        assert!(((r[2] - r[0]) - 84.85).abs() < 1.5, "{r:?}");
        // horizontal mirror of an image: same box
        t.flip_items(0, &[img.idx], true, 100).unwrap();
        let r2 = find(&mut t, ObjKind::Image, 0).rect;
        assert!((r2[0] - img.rect[0]).abs() < 0.5 && (r2[2] - img.rect[2]).abs() < 0.5);
    }

    #[test]
    fn copy_paste_across_documents() {
        let mut src = fixture();
        let info = src.info(0).unwrap();
        let ids: Vec<usize> = info.objects.iter().map(|o| o.idx).collect();
        let clip = src.copy_items(0, &ids).unwrap();
        assert_eq!(clip.items.len(), 4);
        // another empty document
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let mut dst = Tab::new(p, doc, None, None, 1.0);
        let first = dst.paste_clip(0, &clip, Some((300.0, 400.0)), 100).unwrap();
        assert_eq!(first, 0);
        let info = dst.info(0).unwrap();
        assert_eq!(info.objects.len(), 4);
        let u = info.objects.iter().map(|o| o.rect).fold(info.objects[0].rect, |a, b| r4_union(&a, &b));
        let c = r4_center(&u);
        assert!((c.0 - 300.0).abs() < 2.0 && (c.1 - 400.0).abs() < 2.0, "centered on the paste point: {c:?}");
        assert!(info.objects.iter().any(|o| o.kind == ObjKind::Text && o.text() == "Title"));
        assert!(info.objects.iter().any(|o| o.kind == ObjKind::Image));
    }

    #[test]
    fn group_then_ungroup() {
        let mut t = fixture();
        let blue = find(&mut t, ObjKind::Path, 0);
        let green = find(&mut t, ObjKind::Path, 1);
        let g = t.group_items(0, &[blue.idx, green.idx], 100).unwrap();
        let info = t.info(0).unwrap();
        let form = info.object(g).expect("group");
        assert_eq!(form.kind, ObjKind::Form, "a single group-type item");
        assert_eq!(info.objects.iter().filter(|o| o.kind == ObjKind::Path).count(), 0);
        let u = r4_union(&blue.rect, &green.rect);
        assert!((form.rect[0] - u[0]).abs() < 1.0 && (form.rect[2] - u[2]).abs() < 1.0, "{:?} vs {u:?}", form.rect);
        // move the group as a whole
        t.move_items(0, &[g], 30.0, 0.0, 100).unwrap();
        let moved = t.info(0).unwrap().object(g).unwrap().rect;
        assert!((moved[0] - u[0] - 30.0).abs() < 1.0);
        { let pg = t.doc.pages().get(0).unwrap(); println!("page objects after grouping: {:?}", (0..pg.objects().len()).map(|i| format!("{:?}{:?}", pg.objects().get(i).unwrap().object_type(), pg.objects().get(i).unwrap().bounds().map(|b| b.to_rect().left().value))).collect::<Vec<_>>()); }
        // ungroup
        t.ungroup_item(0, g, 100).unwrap();
        let info = t.info(0).unwrap();
        assert_eq!(info.objects.iter().filter(|o| o.kind == ObjKind::Path).count(), 2, "two paths recreated");
        assert_eq!(info.objects.iter().filter(|o| o.kind == ObjKind::Form).count(), 0);
        let paths: Vec<R4> = info.objects.iter().filter(|o| o.kind == ObjKind::Path).map(|o| o.rect).collect();
        let uu = r4_union(&paths[0], &paths[1]);
        assert!((uu[0] - (u[0] + 30.0)).abs() < 1.5, "position kept after ungrouping: {uu:?}");
    }

    #[test]
    fn add_shape_and_edit_point() {
        let mut t = fixture();
        let mut d = PathData::rect([100.0, 100.0, 200.0, 150.0]);
        d.fill = Some([255, 255, 0, 255]);
        let idx = t.add_path(0, &d, "Rectangle", 100).unwrap();
        let before = t.read_path_data(0, idx).unwrap();
        assert_eq!(before.bounds(), [100.0, 100.0, 200.0, 150.0]);
        // move the top-right corner (segment 2)
        t.edit_path_point(0, idx, 2, PtKind::Anchor, 260.0, 190.0, 100).unwrap();
        let after = t.read_path_data(0, idx).unwrap();
        assert_eq!(after.bounds(), [100.0, 100.0, 260.0, 190.0]);
        assert_eq!(after.fill, Some([255, 255, 0, 255]));
    }

    #[test]
    fn image_crop_and_adjust() {
        let mut t = fixture();
        let img = find(&mut t, ObjKind::Image, 0);
        t.crop_image(0, img.idx, [0.25, 0.0, 0.75, 0.5], 100).unwrap();
        let r = find(&mut t, ObjKind::Image, 0).rect;
        assert!(((r[2] - r[0]) - 40.0).abs() < 0.5 && ((r[3] - r[1]) - 40.0).abs() < 0.5, "{r:?}");
        assert!((r[0] - (img.rect[0] + 20.0)).abs() < 0.5, "{r:?}");
        assert!((r[3] - img.rect[3]).abs() < 0.5, "the top is kept: {r:?}");
        let idx = find(&mut t, ObjKind::Image, 0).idx;
        t.adjust_image(0, idx, 0, 0.0, true, 100).unwrap();
        let back = t.read_image(0, idx).unwrap().to_rgba8();
        let px = back.get_pixel(5, 5);
        assert!(px[0] == px[1] && px[1] == px[2], "grayscale: {px:?}");
    }

    #[test]
    fn add_text_and_style() {
        let mut t = fixture();
        let st = TextStyle { size: 16.0, color: [255, 0, 0], ..TextStyle::default() };
        let idx = t.add_text(0, 72.0, 300.0, "Line 1\nLine 2", &st, 100).unwrap();
        let info = t.info(0).unwrap();
        let o = info.object(idx).expect("text added");
        assert!(o.text().contains("Line 1"));
        assert_eq!(o.block.as_ref().unwrap().first_run().color, [255, 0, 0]);
        t.set_style_items(0, &[idx], Some([0, 0, 255, 255]), None, None, 100).unwrap();
        let o = t.info(0).unwrap().object(idx).cloned().unwrap();
        assert_eq!(o.block.as_ref().unwrap().first_run().color, [0, 0, 255]);
    }
}

/// Shortens the marker lifetime of a `PdfDocument` (group API of pdfium-render).
/// SAFETY: the lifetime parameter of `PdfDocument` is a `PhantomData`; there is no real reference.
fn shorten_doc<'a, 'b>(d: &'a mut PdfDocument<'static>) -> &'a mut PdfDocument<'b> {
    unsafe { std::mem::transmute(d) }
}





fn aff_of(m: &PdfMatrix) -> Affine {
    Affine { a: m.a(), b: m.b(), c: m.c(), d: m.d(), e: m.e(), f: m.f() }
}

/// Brings an object's matrix back exactly to `target` (if it differs): the crate composes
/// matrices instead of replacing them, so the transformation `target ∘ current⁻¹` is applied.
pub fn restore_matrix(o: &mut PdfPageObject, target: &PdfMatrix) {
    let Ok(now) = o.matrix() else { return };
    let (n, t) = (aff_of(&now), aff_of(target));
    let close = |a: f32, b: f32| (a - b).abs() < 1e-3 * (1.0 + b.abs());
    if close(n.a, t.a) && close(n.b, t.b) && close(n.c, t.c) && close(n.d, t.d) && close(n.e, t.e) && close(n.f, t.f) {
        return;
    }
    if let Some(inv) = n.inverse() {
        let d = inv.then(&t);
        let _ = o.transform(d.a, d.b, d.c, d.d, d.e, d.f);
    }
}






#[cfg(test)]
mod perf {
    use super::*;
    use crate::model::ObjKind;
    use crate::tab::Tab;
    use std::time::Instant;

    /// Manual measurement on a large document (ignored by default): `RUSTPDF_PERF=path cargo test --release perf_move -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn perf_move_image() {
        let Ok(path) = std::env::var("RUSTPDF_PERF") else { return };
        let t = Instant::now();
        let doc = crate::pdfx::open_path(crate::pdfx::test_pdfium(), std::path::Path::new(&path), None).ok().unwrap();
        let mut tab = Tab::new(crate::pdfx::test_pdfium(), doc, None, None, 1.0);
        println!("opening {} ms", t.elapsed().as_millis());
        let page: usize = std::env::var("RUSTPDF_PAGE").ok().and_then(|p| p.parse().ok()).unwrap_or(29);
        let t = Instant::now();
        let info = tab.info(page).unwrap();
        println!("page analysis {} ms ({} objects)", t.elapsed().as_millis(), info.objects.len());
        let t = Instant::now();
        let pg = tab.read_page(page).unwrap();
        println!("read_page {} ms", t.elapsed().as_millis());
        let t = Instant::now();
        let n = pg.objects().len();
        println!("objects().len {} ms ({n})", t.elapsed().as_millis());
        let t = Instant::now();
        let mut k = 0;
        for o in pg.objects().iter() { if o.bounds().is_ok() { k += 1; } }
        println!("bounds loop {} ms ({k})", t.elapsed().as_millis());
        let t = Instant::now();
        let _ = crate::pdfx::scan_page(&pg, page, 0, true);
        println!("scan_page alone {} ms", t.elapsed().as_millis());
        drop(pg);
        let img = info.objects.iter().find(|o| o.kind == ObjKind::Image).expect("image").idx;
        let t = Instant::now();
        let bytes = tab.doc.save_to_bytes().unwrap();
        println!("save_to_bytes {} ms ({} Mo)", t.elapsed().as_millis(), bytes.len() / 1048576);
        let t = Instant::now();
        tab.prepare_snapshot();
        println!("prepare_snapshot (idle) {} ms", t.elapsed().as_millis());
        let t = Instant::now();
        tab.move_items(page, &[img], 10.0, -10.0, 512).unwrap();
        println!("move_items total {} ms", t.elapsed().as_millis());
        let t = Instant::now();
        let _ = tab.info(page);
        println!("re-analysis {} ms", t.elapsed().as_millis());
    }
}
