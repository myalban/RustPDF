//! Central area: pages (previews + tiles), overlays, selection and mouse interactions.

use crate::edit_text::TextStyle;
use crate::geom::*;
use crate::layout::{self, Layout, PageSlot};
use crate::model::*;
use crate::pathdata::{PathData, PtKind, read_path};
use crate::prefs::{Prefs, Unit, ViewMode};
use crate::render::{self, Key, RenderCache};
use crate::tab::{Drag, FitMode, Inline, InlineKind, Tab};
use crate::textsel::TextSel;
use crate::tools::{Cmd, ShapeKind, Tool, ToolStyle};
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use pdfium_render::prelude::PdfPageObjectsCommon;
use std::rc::Rc;

/// Logical pixels per PDF point at 100 % zoom (96 dpi / 72 pt).
pub const PT_TO_PX: f32 = 96.0 / 72.0;
const HANDLE: f32 = 9.0;
const TILE: i32 = 1536;

/// Events raised to the application (dialogs, file pickers, annotations…).
#[derive(Clone, Debug)]
pub enum Msg {
    Status(String),
    Error(String),
    OpenUri(String),
    Cmd(Cmd),
    SetTool(Tool),
    ExportZone { page: usize, rect: R4 },
    AddImageAt { page: usize, x: f32, y: f32 },
    Markup { page: usize, tool: Tool, rects: Vec<R4> },
    NoteAt { page: usize, x: f32, y: f32 },
    StampAt { page: usize, x: f32, y: f32 },
    RedactZone { page: usize, rect: R4 },
    LinkZone { page: usize, rect: R4 },
    FormZone { tool: Tool, page: usize, rect: R4 },
    SignAt { page: usize, x: f32, y: f32 },
    ReplaceImage { page: usize, idx: usize },
    ListPopup { page: usize, field: usize },
    ChooseOption { page: usize, field: usize, option: usize },
    ToggleField { page: usize, field: usize },
    /// The selection changed: reload the fields of the properties panel.
    SelectionChanged,
    CopyText(String),
    /// Text to create at the end of the inline editing of a new text.
    CommitNewText { page: usize, x: f32, y: f32, text: String },
    CommitFieldText { page: usize, field: usize, text: String },
    CommitObjectText { page: usize, idx: usize, text: String },
    CommitNoteText { page: usize, annot: usize, text: String },
    ShapeDone { page: usize, kind: ShapeKind, a: [f32; 2], b: [f32; 2] },
    PolygonDone { page: usize, pts: Vec<[f32; 2]> },
    InkDone { page: usize, pts: Vec<[f32; 2]> },
    MoveSelection { page: usize, dx: f32, dy: f32 },
    ResizeSelection { page: usize, old: R4, new: R4, reflow: bool },
    PathPointMoved { page: usize, item: usize, seg: usize, kind: PtKind, x: f32, y: f32 },
    ContextAt { page: usize, x: f32, y: f32 },
}

pub struct ViewEnv<'a> {
    pub ctx: &'a egui::Context,
    pub prefs: &'a Prefs,
    pub tool: Tool,
    pub cache: &'a mut RenderCache,
    pub msgs: &'a mut Vec<Msg>,
    pub style: &'a ToolStyle,
    pub has_clip: bool,
}

/// A page visible on screen, with its coordinate conversions.
struct PageCtx {
    page: usize,
    /// Rectangle of the page on screen.
    rect: Rect,
    /// Logical pixels per point.
    s: f32,
    info: Rc<PageInfo>,
}

impl PageCtx {
    fn geom(&self) -> &PageGeom {
        &self.info.geom
    }
    /// Page rectangle (PDF space) → screen rectangle.
    fn to_screen(&self, r: &R4) -> Rect {
        let v = self.geom().rect_to_view(r);
        Rect::from_min_max(
            Pos2::new(self.rect.min.x + v[0] * self.s, self.rect.min.y + v[1] * self.s),
            Pos2::new(self.rect.min.x + v[2] * self.s, self.rect.min.y + v[3] * self.s),
        )
    }
    fn pt_to_screen(&self, x: f32, y: f32) -> Pos2 {
        let (vx, vy) = self.geom().to_view.apply(x, y);
        Pos2::new(self.rect.min.x + vx * self.s, self.rect.min.y + vy * self.s)
    }
    /// Screen → PDF page space.
    fn to_page(&self, p: Pos2) -> (f32, f32) {
        self.geom().from_view.apply((p.x - self.rect.min.x) / self.s, (p.y - self.rect.min.y) / self.s)
    }
    /// Screen rectangle → PDF page rectangle.
    fn rect_to_page(&self, r: &Rect) -> R4 {
        let v = [(r.min.x - self.rect.min.x) / self.s, (r.min.y - self.rect.min.y) / self.s, (r.max.x - self.rect.min.x) / self.s, (r.max.y - self.rect.min.y) / self.s];
        self.geom().rect_from_view(&v)
    }
    /// Screen move (pixels) → page move (points).
    fn delta_to_page(&self, d: Vec2) -> (f32, f32) {
        self.geom().from_view.apply_vec(d.x / self.s, d.y / self.s)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    None,
    Field(usize),
    Obj(usize),
    Annot(usize),
}

fn hit_test(info: &PageInfo, x: f32, y: f32, slop: f32) -> Hit {
    for (i, f) in info.fields.iter().enumerate() {
        if r4_contains(&r4_expand(&f.rect, slop), x, y) {
            return Hit::Field(i);
        }
    }
    let page_area = info.geom.w * info.geom.h;
    // "object" annotations (notes, drawings…) above the content, text markups below
    for a in info.annots.iter().rev() {
        if !matches!(a.kind, AnnotKind::Highlight | AnnotKind::Underline | AnnotKind::StrikeOut | AnnotKind::Squiggly) && r4_contains(&r4_expand(&a.rect, slop), x, y) {
            return Hit::Annot(a.idx);
        }
    }
    // text, images and groups (the topmost in the stacking order first)
    if let Some(o) = info
        .objects
        .iter()
        .rev()
        .filter(|o| matches!(o.kind, ObjKind::Text | ObjKind::Image | ObjKind::Form | ObjKind::Shading | ObjKind::Other))
        .filter(|o| r4_area(&o.rect) < page_area * 0.9)
        .find(|o| r4_contains(&r4_expand(&o.rect, slop), x, y))
    {
        return Hit::Obj(o.idx);
    }
    // small paths: the smallest first
    if let Some(o) = info
        .objects
        .iter()
        .filter(|o| o.kind == ObjKind::Path && r4_contains(&r4_expand(&o.rect, slop), x, y))
        .filter(|o| r4_area(&o.rect) < page_area * 0.8)
        .min_by(|a, b| r4_area(&a.rect).partial_cmp(&r4_area(&b.rect)).unwrap_or(std::cmp::Ordering::Equal))
    {
        return Hit::Obj(o.idx);
    }
    for a in info.annots.iter().rev() {
        if r4_contains(&r4_expand(&a.rect, slop), x, y) {
            return Hit::Annot(a.idx);
        }
    }
    Hit::None
}

pub fn px_scale(zoom: f32) -> f32 {
    PT_TO_PX * zoom
}

/// Zoom that makes the current page fit in width or entirely in the visible area.
pub fn fit_zoom(tab: &Tab, mode: FitMode, avail: Vec2, two: bool) -> f32 {
    let (pw, ph) = tab.sizes.get(tab.view.page).copied().unwrap_or((595.0, 842.0));
    let cols = if two { 2.0 } else { 1.0 };
    let w = (avail.x - 2.0 * layout::PAGE_MARGIN - (cols - 1.0) * layout::PAGE_GAP).max(50.0) / (pw * cols);
    let h = (avail.y - 2.0 * layout::PAGE_MARGIN).max(50.0) / ph;
    let z = match mode {
        FitMode::Width => w,
        FitMode::Page => w.min(h),
        FitMode::None => return tab.view.zoom,
    };
    (z / PT_TO_PX).clamp(0.1, 8.0)
}

pub fn set_zoom(tab: &mut Tab, z: f32, pivot: Option<Pos2>) {
    let z = z.clamp(0.1, 8.0);
    if (z - tab.view.zoom).abs() > 0.0005 {
        // keep the point under the cursor fixed: offset ← content · ratio + (pivot − origin)
        if let Some(p) = pivot {
            tab.view.zoom_pivot = Some(p);
        }
        tab.view.zoom = z;
        tab.view.zoom_changed_at = Some(std::time::Instant::now());
    }
}

/// Shows the document; returns the screen rectangle of the central area.
pub fn show(ui: &mut egui::Ui, tab: &mut Tab, env: &mut ViewEnv) -> Rect {
    let ctx = env.ctx.clone();
    let ppp = ctx.pixels_per_point();
    let two = matches!(env.prefs.view_mode, ViewMode::TwoPages | ViewMode::TwoPagesContinuous);
    let avail = ui.available_size();
    tab.view.avail = avail;

    // Automatic fit: follows the window size.
    if tab.view.fit != FitMode::None {
        let z = fit_zoom(tab, tab.view.fit, avail, two);
        if (z - tab.view.zoom).abs() > 0.0005 {
            tab.view.zoom = z;
        }
    }

    let s = px_scale(tab.view.zoom);
    let lay = layout::compute(&tab.sizes, env.prefs.view_mode, env.prefs.cover_page, s, avail, tab.view.page);
    let settled = tab.view.zoom_changed_at.map(|t| t.elapsed() > std::time::Duration::from_millis(150)).unwrap_or(true);
    if !settled {
        ctx.request_repaint_after(std::time::Duration::from_millis(60));
    } else {
        tab.view.zoom_changed_at = None;
    }

    // Scroll offset to impose (page jump, centered zoom).
    let mut area = egui::ScrollArea::both().auto_shrink([false, false]).id_salt(("doc", tab.id));
    let outer = ui.max_rect();
    let mut force: Option<Vec2> = tab.view.set_offset.take();
    if let Some(pivot) = tab.view.zoom_pivot.take() {
        // content under the pivot before the zoom → same position after
        let old_s = tab.view.pivot_old_scale.unwrap_or(s);
        let rel = pivot - outer.min;
        let content = (tab.view.offset + rel) / old_s;
        force = Some(content * s - rel);
    }
    tab.view.pivot_old_scale = Some(s);
    if let Some(p) = tab.view.scroll_to.take() {
        if let Some(slot) = lay.slot(p) {
            force = Some(Vec2::new(tab.view.offset.x, (slot.rect.min.y - layout::PAGE_MARGIN * 0.5).max(0.0)));
        } else {
            // page missing from the layout (single page mode): scroll back to the top
            force = Some(Vec2::new(tab.view.offset.x, 0.0));
        }
    }
    if let Some(f) = force {
        area = area.scroll_offset(Vec2::new(f.x.max(0.0), f.y.max(0.0)));
    }

    let mut visible_pages: Vec<usize> = vec![];
    let out = area.show_viewport(ui, |ui, viewport| {
        ui.set_min_size(lay.size);
        let origin = ui.min_rect().min;
        tab.view.origin_screen = origin;
        for slot in &lay.slots {
            if !slot.rect.intersects(viewport.expand(200.0)) {
                continue;
            }
            visible_pages.push(slot.page);
            draw_one_page(ui, tab, env, slot, origin, s, ppp, viewport, settled);
        }
        draw_overlays_global(ui, tab, env, &lay, origin, s);
    });
    tab.view.offset = out.state.clamped_offset();
    // current page = the most visible one
    if matches!(env.prefs.view_mode, ViewMode::Continuous | ViewMode::TwoPagesContinuous) {
        if let Some(p) = layout::page_at(&lay, out.state.clamped_offset().y, avail.y) {
            if p != tab.view.page && tab.view.scroll_to.is_none() {
                tab.view.page = p;
            }
        }
    }
    tab.view.visible = visible_pages;
    out.inner_rect
}

#[allow(clippy::too_many_arguments)]
fn draw_one_page(ui: &mut egui::Ui, tab: &mut Tab, env: &mut ViewEnv, slot: &PageSlot, origin: Pos2, s: f32, ppp: f32, viewport: Rect, settled: bool) {
    let page = slot.page;
    let rect = slot.rect.translate(origin.to_vec2());
    let Some(info) = tab.info(page) else { return };
    let pc = PageCtx { page, rect, s, info };
    let painter = ui.painter_at(ui.clip_rect());

    // shadow + white background
    painter.rect_filled(rect.translate(Vec2::new(3.0, 3.0)), 0.0, Color32::from_black_alpha(45));
    painter.rect_filled(rect, 0.0, if env.prefs.night_mode { Color32::BLACK } else { Color32::WHITE });

    draw_textures(ui, tab, env, &pc, ppp, viewport, origin, settled);

    // page number (small marker at the bottom right)
    if s < 0.6 {
        painter.text(rect.right_bottom() - Vec2::new(6.0, 4.0), egui::Align2::RIGHT_BOTTOM, format!("{}", page + 1), egui::FontId::proportional(11.0), Color32::from_gray(120));
    }

    // the page is queried first: the handles (added afterwards by the overlays) keep priority
    interact_page(ui, tab, env, &pc);
    draw_page_overlays(ui, tab, env, &pc);
    draw_inline_editor(ui, tab, env, &pc);
}

/// Low-resolution preview + high-resolution tiles of the visible areas.
fn draw_textures(ui: &mut egui::Ui, tab: &mut Tab, env: &mut ViewEnv, pc: &PageCtx, ppp: f32, viewport: Rect, origin: Pos2, settled: bool) {
    let painter = ui.painter_at(ui.clip_rect());
    let ver = tab.page_ver[pc.page];
    let night = env.prefs.night_mode;
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    let (wpt, hpt) = (pc.geom().w, pc.geom().h);

    // 1. preview
    let pkey = Key::Preview { tab: tab.id, page: pc.page, ver, night };
    let mut preview = env.cache.get(&pkey);
    let mut page_obj = None;
    if preview.is_none() && env.cache.may_render() {
        if let Ok(p) = tab.read_page(pc.page) {
            let w = (wpt * PT_TO_PX).clamp(80.0, 520.0) as i32;
            if let Some(img) = crate::pdfx::render_full(&p, w, true) {
                let (tex, bytes) = render::upload(&env.ctx, format!("pv{}_{}", tab.id, pc.page), &img, night);
                env.cache.insert(pkey, tex.clone(), bytes);
                preview = Some(tex);
            }
            page_obj = Some(p);
        }
    }
    match &preview {
        Some(t) => {
            painter.image(t.id(), pc.rect, uv, Color32::WHITE);
        }
        None => env.cache.pending = true,
    }

    // 2. tiles (only when the zoom is stable)
    if !settled {
        return;
    }
    let k = pc.s * ppp; // pixels per point
    let full_w = (wpt * k).round().max(1.0) as i32;
    let full_h = (hpt * k).round().max(1.0) as i32;
    // visible area, in page pixels
    let vis_screen = pc.rect.intersect(viewport.translate(origin.to_vec2()).expand(40.0));
    if !vis_screen.is_positive() {
        return;
    }
    let to_px = |p: Pos2| ((p.x - pc.rect.min.x) * ppp, (p.y - pc.rect.min.y) * ppp);
    let (x0, y0) = to_px(vis_screen.min);
    let (x1, y1) = to_px(vis_screen.max);
    let tx0 = (x0.max(0.0) as i32 / TILE).max(0);
    let ty0 = (y0.max(0.0) as i32 / TILE).max(0);
    let tx1 = ((x1.min(full_w as f32) as i32 - 1).max(0)) / TILE;
    let ty1 = ((y1.min(full_h as f32) as i32 - 1).max(0)) / TILE;
    let sx = pc.rect.width() / full_w as f32;
    let sy = pc.rect.height() / full_h as f32;
    let scale_bits = k.to_bits();
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            let (px, py) = (tx * TILE, ty * TILE);
            let (tw, th) = (TILE.min(full_w - px), TILE.min(full_h - py));
            if tw <= 0 || th <= 0 {
                continue;
            }
            let key = Key::Tile { tab: tab.id, page: pc.page, ver, scale: scale_bits, tx: tx as u16, ty: ty as u16, night };
            let mut tex = env.cache.get(&key);
            if tex.is_none() && env.cache.may_render() {
                if page_obj.is_none() {
                    page_obj = tab.read_page(pc.page).ok();
                }
                if let Some(p) = &page_obj {
                    if let Some(img) = crate::pdfx::render_tile(p, full_w, full_h, px, py, tw, th, true) {
                        let (t, bytes) = render::upload(&env.ctx, format!("t{}_{}_{}_{}", tab.id, pc.page, tx, ty), &img, night);
                        env.cache.insert(key, t.clone(), bytes);
                        tex = Some(t);
                    }
                }
            }
            match tex {
                Some(t) => {
                    let r = Rect::from_min_size(
                        Pos2::new(pc.rect.min.x + px as f32 * sx, pc.rect.min.y + py as f32 * sy),
                        Vec2::new(tw as f32 * sx, th as f32 * sy),
                    );
                    painter.image(t.id(), r, uv, Color32::WHITE);
                }
                None => env.cache.pending = true,
            }
        }
    }
}

fn draw_page_overlays(ui: &mut egui::Ui, tab: &mut Tab, env: &mut ViewEnv, pc: &PageCtx) {
    let painter = ui.painter_at(ui.clip_rect());
    let page = pc.page;

    // search results
    for (mi, m) in tab.search.matches.iter().enumerate().filter(|(_, m)| m.page == page) {
        let cur = Some(mi) == tab.search.cur;
        let fill = if cur { Color32::from_rgba_unmultiplied(255, 140, 0, 120) } else { Color32::from_rgba_unmultiplied(255, 230, 0, 100) };
        for r in &m.rects {
            let sr = pc.to_screen(r).expand(1.5);
            painter.rect_filled(sr, 2.0, fill);
            if cur {
                painter.rect_stroke(sr, 2.0, Stroke::new(2.0, Color32::from_rgb(255, 120, 0)), StrokeKind::Outside);
            }
        }
    }
    if tab.search.scroll && !tab.view.scroll_to.is_some() {
        if let Some(m) = tab.search.cur.and_then(|c| tab.search.matches.get(c)).filter(|m| m.page == page) {
            if let Some(r) = m.rects.first() {
                ui.scroll_to_rect(pc.to_screen(r).expand(90.0), Some(egui::Align::Center));
                tab.search.scroll = false;
            }
        }
    }

    // text selection
    if let Some(ts) = tab.text_sel.filter(|t| t.page == page) {
        let (a, b) = ts.ordered();
        let (_, rects) = tab.text_range(page, a, b);
        for r in &rects {
            painter.rect_filled(pc.to_screen(r), 0.0, Color32::from_rgba_unmultiplied(60, 130, 255, 90));
        }
    }

    let sel_tool = matches!(env.tool, Tool::Select | Tool::Hand);
    // object frames
    if env.prefs.show_boxes && env.tool == Tool::Select {
        for o in &pc.info.objects {
            let col = match o.kind {
                ObjKind::Text => Color32::from_rgba_unmultiplied(40, 120, 255, 70),
                ObjKind::Image => Color32::from_rgba_unmultiplied(40, 200, 90, 110),
                ObjKind::Form => Color32::from_rgba_unmultiplied(170, 80, 220, 110),
                _ => continue,
            };
            if !tab.sel.contains(page, o.idx) {
                painter.rect_stroke(pc.to_screen(&o.rect), 0.0, Stroke::new(1.0, col), StrokeKind::Inside);
            }
        }
    }
    // form fields
    if env.prefs.show_fields && sel_tool {
        for (fi, f) in pc.info.fields.iter().enumerate() {
            let strong = tab.sel.field == Some(fi) && tab.sel.page == page;
            painter.rect_stroke(pc.to_screen(&f.rect), 0.0, Stroke::new(if strong { 2.0 } else { 1.0 }, Color32::from_rgb(200, 90, 255)), StrokeKind::Outside);
        }
    }
    // selected annotations / outlines
    for a in &pc.info.annots {
        let selected = tab.sel.annot == Some(a.idx) && tab.sel.page == page;
        if selected {
            painter.rect_stroke(pc.to_screen(&a.rect).expand(2.0), 0.0, Stroke::new(2.0, Color32::from_rgb(255, 120, 0)), StrokeKind::Outside);
        }
    }
    // redaction areas pending
    for (p, r) in tab.redactions.iter().filter(|(p, _)| *p == page) {
        let _ = p;
        let sr = pc.to_screen(r);
        painter.rect_filled(sr, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 90));
        painter.rect_stroke(sr, 0.0, Stroke::new(1.5, Color32::from_rgb(220, 30, 30)), StrokeKind::Outside);
    }
    // measurement in progress
    if !tab.measure.is_empty() && tab.measure_page == page {
        let pts: Vec<Pos2> = tab.measure.iter().map(|p| pc.pt_to_screen(p[0], p[1])).collect();
        painter.add(egui::Shape::line(pts.clone(), Stroke::new(2.0, Color32::from_rgb(0, 160, 220))));
        for p in pts {
            painter.circle_filled(p, 3.5, Color32::from_rgb(0, 160, 220));
        }
    }
    // polygon in progress
    if !tab.poly.is_empty() && tab.measure_page == page {
        let pts: Vec<Pos2> = tab.poly.iter().map(|p| pc.pt_to_screen(p[0], p[1])).collect();
        painter.add(egui::Shape::line(pts.clone(), Stroke::new(2.0, Color32::from_rgb(220, 30, 30))));
        for p in pts {
            painter.circle_filled(p, 3.5, Color32::from_rgb(220, 30, 30));
        }
    }

    // object selection: frame + handles
    if tab.sel.page == page && !tab.sel.items.is_empty() && env.tool == Tool::Select {
        if let Some(r) = crate::edit::union_rect(&pc.info, &tab.sel.items) {
            let mut sr = pc.to_screen(&r);
            match &tab.drag {
                Drag::Move { page: p, accum } if *p == page => sr = sr.translate(*accum),
                Drag::Resize { page: p, cur, .. } if *p == page => sr = *cur,
                _ => {}
            }
            painter.rect_stroke(sr, 0.0, Stroke::new(2.0, Color32::from_rgb(255, 120, 0)), StrokeKind::Outside);
        }
    }

    // gestures in progress: area, shape, marquee, freehand stroke
    match &tab.drag {
        Drag::Zone { page: p, a, b } | Drag::Shape { page: p, a, b } if *p == page => {
            let sr = Rect::from_two_pos(pc.pt_to_screen(a[0], a[1]), pc.pt_to_screen(b[0], b[1]));
            match (&tab.drag, env.tool) {
                (Drag::Shape { .. }, Tool::Shape(kind)) => draw_shape_preview(&painter, kind, pc.pt_to_screen(a[0], a[1]), pc.pt_to_screen(b[0], b[1]), env.style),
                _ => {
                    let col = if env.tool == Tool::Redact { Color32::from_rgb(220, 30, 30) } else { Color32::from_rgb(40, 120, 255) };
                    painter.rect_filled(sr, 0.0, Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 40));
                    painter.rect_stroke(sr, 0.0, Stroke::new(1.5, col), StrokeKind::Outside);
                }
            }
        }
        Drag::Marquee { page: p, start, cur } if *p == page => {
            let sr = Rect::from_two_pos(*start, *cur);
            painter.rect_filled(sr, 0.0, Color32::from_rgba_unmultiplied(40, 120, 255, 30));
            painter.rect_stroke(sr, 0.0, Stroke::new(1.0, Color32::from_rgb(40, 120, 255)), StrokeKind::Outside);
        }
        Drag::Ink { page: p, pts } if *p == page => {
            let v: Vec<Pos2> = pts.iter().map(|q| pc.pt_to_screen(q[0], q[1])).collect();
            let c = env.style.shape_stroke;
            painter.add(egui::Shape::line(v, Stroke::new((env.style.shape_width * pc.s).max(1.0), Color32::from_rgb(c[0], c[1], c[2]))));
        }
        _ => {}
    }

    // handles (except text being edited)
    if tab.sel.page == page && !tab.sel.items.is_empty() && env.tool == Tool::Select && tab.inline.is_none() {
        draw_handles(ui, tab, env, pc);
    }
    // points of a selected vector path (a single path)
    if tab.sel.page == page && tab.sel.items.len() == 1 && env.tool == Tool::Select && tab.inline.is_none() {
        if let Some(o) = pc.info.object(tab.sel.items[0]) {
            if o.kind == ObjKind::Path && ui.input(|i| i.modifiers.alt) {
                draw_path_points(ui, tab, env, pc, o.idx);
            }
        }
    }
}

fn draw_shape_preview(painter: &egui::Painter, kind: ShapeKind, a: Pos2, b: Pos2, st: &ToolStyle) {
    let c = st.shape_stroke;
    let stroke = Stroke::new((st.shape_width).max(1.0), Color32::from_rgb(c[0], c[1], c[2]));
    let fill = st.shape_fill.map(|f| Color32::from_rgba_unmultiplied(f[0], f[1], f[2], st.shape_opacity)).unwrap_or(Color32::TRANSPARENT);
    let r = Rect::from_two_pos(a, b);
    match kind {
        ShapeKind::Rect => {
            painter.rect_filled(r, 0.0, fill);
            painter.rect_stroke(r, 0.0, stroke, StrokeKind::Middle);
        }
        ShapeKind::Ellipse => {
            painter.add(egui::Shape::ellipse_filled(r.center(), r.size() * 0.5, fill));
            painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() * 0.5, stroke));
        }
        ShapeKind::Line | ShapeKind::Arrow => {
            painter.line_segment([a, b], stroke);
        }
        _ => {}
    }
}

/// Anchor and control points of a path (Alt held): drag to move them.
fn draw_path_points(ui: &mut egui::Ui, tab: &mut Tab, env: &mut ViewEnv, pc: &PageCtx, idx: usize) {
    let Some(data) = ({
        let pg = tab.read_page(pc.page).ok();
        pg.and_then(|p| p.objects().get(idx).ok().and_then(|o| read_path(&o)))
    }) else {
        return;
    };
    let painter = ui.painter_at(ui.clip_rect());
    for (n, p) in data.points().iter().enumerate() {
        let sp = pc.pt_to_screen(p.x, p.y);
        let r = Rect::from_center_size(sp, Vec2::splat(9.0));
        let resp = ui.interact(r.expand(3.0), egui::Id::unique(("pp", pc.page, idx, n)), Sense::drag());
        let (fill, rad) = if p.kind == PtKind::Anchor { (Color32::WHITE, 4.5) } else { (Color32::from_rgb(255, 200, 0), 3.5) };
        painter.circle_filled(sp, rad, fill);
        painter.circle_stroke(sp, rad, Stroke::new(1.5, Color32::from_rgb(40, 120, 255)));
        if resp.drag_started() {
            tab.drag = Drag::PathPoint { page: pc.page, item: idx, seg: p.seg, kind: p.kind, pos: [p.x, p.y] };
        }
        if resp.dragged() {
            if let (Drag::PathPoint { pos, seg, kind, .. }, Some(m)) = (&mut tab.drag, resp.interact_pointer_pos()) {
                if *seg == p.seg && *kind == p.kind {
                    let (x, y) = pc.to_page(m);
                    *pos = [x, y];
                }
            }
        }
        if resp.drag_stopped() {
            if let Drag::PathPoint { page, item, seg, kind, pos } = tab.drag.clone() {
                tab.drag = Drag::None;
                env.msgs.push(Msg::PathPointMoved { page, item, seg, kind, x: pos[0], y: pos[1] });
            }
        }
    }
    let _ = env;
}

fn draw_handles(ui: &mut egui::Ui, tab: &mut Tab, env: &mut ViewEnv, pc: &PageCtx) {
    let page = pc.page;
    let Some(union) = crate::edit::union_rect(&pc.info, &tab.sel.items) else { return };
    let base = match &tab.drag {
        Drag::Resize { page: p, cur, .. } if *p == page => *cur,
        _ => pc.to_screen(&union),
    };
    let painter = ui.painter_at(ui.clip_rect());
    const HS: [(i8, i8); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    // a single text block: only the side handles are used to re-justify
    for (hi, (hx, hy)) in HS.iter().enumerate() {
        let cx = match hx {
            -1 => base.min.x,
            0 => base.center().x,
            _ => base.max.x,
        };
        let cy = match hy {
            -1 => base.min.y,
            0 => base.center().y,
            _ => base.max.y,
        };
        let hr = Rect::from_center_size(Pos2::new(cx, cy), Vec2::splat(HANDLE));
        let resp = ui.interact(hr.expand(3.0), egui::Id::unique(("rh", page, hi)), Sense::drag());
        painter.rect_filled(hr, 1.0, Color32::WHITE);
        painter.rect_stroke(hr, 1.0, Stroke::new(1.5, Color32::from_rgb(255, 120, 0)), StrokeKind::Inside);
        if resp.hovered() || resp.dragged() {
            env.ctx.set_cursor_icon(cursor_for(*hx, *hy));
        }
        if resp.drag_started() {
            let r = pc.to_screen(&union);
            tab.drag = Drag::Resize { page, hx: *hx, hy: *hy, orig: r, cur: r };
        }
        if resp.dragged() {
            let keep_ratio = ui.input(|i| i.modifiers.command);
            if let (Drag::Resize { hx, hy, orig, cur, .. }, Some(p)) = (&mut tab.drag, resp.interact_pointer_pos()) {
                let mut n = *orig;
                if *hx == -1 {
                    n.min.x = p.x.min(n.max.x - 4.0);
                } else if *hx == 1 {
                    n.max.x = p.x.max(n.min.x + 4.0);
                }
                if *hy == -1 {
                    n.min.y = p.y.min(n.max.y - 4.0);
                } else if *hy == 1 {
                    n.max.y = p.y.max(n.min.y + 4.0);
                }
                // Ctrl held: proportions kept
                if keep_ratio {
                    let (ow, oh) = (orig.width().max(1.0), orig.height().max(1.0));
                    let kx = n.width() / ow;
                    let ky = n.height() / oh;
                    let k = match (*hx, *hy) {
                        (0, _) => ky,
                        (_, 0) => kx,
                        _ => kx.max(ky),
                    }
                    .max(0.02);
                    let (nw, nh) = (ow * k, oh * k);
                    match *hx {
                        -1 => {
                            n.min.x = orig.max.x - nw;
                            n.max.x = orig.max.x;
                        }
                        1 => {
                            n.min.x = orig.min.x;
                            n.max.x = orig.min.x + nw;
                        }
                        _ => {
                            n.min.x = orig.center().x - nw / 2.0;
                            n.max.x = orig.center().x + nw / 2.0;
                        }
                    }
                    match *hy {
                        -1 => {
                            n.min.y = orig.max.y - nh;
                            n.max.y = orig.max.y;
                        }
                        1 => {
                            n.min.y = orig.min.y;
                            n.max.y = orig.min.y + nh;
                        }
                        _ => {
                            n.min.y = orig.center().y - nh / 2.0;
                            n.max.y = orig.center().y + nh / 2.0;
                        }
                    }
                }
                *cur = n;
            }
        }
        if resp.drag_stopped() {
            if let Drag::Resize { hx, hy, orig, cur, .. } = tab.drag.clone() {
                tab.drag = Drag::None;
                let old = pc.rect_to_page(&orig);
                let new = pc.rect_to_page(&cur);
                // a single paragraph, side handle: re-justify rather than stretch
                let single_text = tab.sel.items.len() == 1
                    && pc.info.object(tab.sel.items[0]).map(|o| o.kind == ObjKind::Text && o.block.as_ref().map(|b| b.lines.len() > 1).unwrap_or(false)).unwrap_or(false);
                let reflow = single_text && hy == 0 && hx != 0;
                env.msgs.push(Msg::ResizeSelection { page, old, new, reflow });
            }
        }
    }
}

fn cursor_for(hx: i8, hy: i8) -> CursorIcon {
    match (hx, hy) {
        (-1, -1) | (1, 1) => CursorIcon::ResizeNwSe,
        (-1, 1) | (1, -1) => CursorIcon::ResizeNeSw,
        (0, _) => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeHorizontal,
    }
}

// --------------------------------------------------------------- interactions

fn interact_page(ui: &mut egui::Ui, tab: &mut Tab, env: &mut ViewEnv, pc: &PageCtx) {
    let page = pc.page;
    let resp = ui.interact(pc.rect.intersect(ui.clip_rect()), egui::Id::unique(("page", tab.id, page)), Sense::click_and_drag());
    let shift = ui.input(|i| i.modifiers.shift);
    let ctrl = ui.input(|i| i.modifiers.command);
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let pp = pointer.map(|p| pc.to_page(p));
    let slop = 3.0 / pc.s;

    // right click: remember the targeted item for the context menu
    if resp.secondary_clicked() {
        if let Some((x, y)) = pp {
            tab.view.page = page;
            if env.tool == Tool::Select {
                match hit_test(&pc.info, x, y, slop) {
                    Hit::Obj(i) => {
                        if !tab.sel.contains(page, i) {
                            tab.sel.clear();
                            tab.sel.page = page;
                            tab.sel.items = vec![i];
                            env.msgs.push(Msg::SelectionChanged);
                        }
                    }
                    Hit::None => {}
                    _ => {}
                }
            }
            env.msgs.push(Msg::ContextAt { page, x, y });
        }
    }

    // cursor and hovered link
    if let Some((x, y)) = pp {
        if resp.hovered() {
            let over_link = pc.info.links.iter().find(|l| r4_contains(&l.rect, x, y));
            match env.tool {
                Tool::Hand => {
                    env.ctx.set_cursor_icon(if matches!(tab.drag, Drag::Pan) { CursorIcon::Grabbing } else { CursorIcon::Grab });
                    if over_link.is_some() {
                        env.ctx.set_cursor_icon(CursorIcon::PointingHand);
                    }
                }
                Tool::Select => {
                    match hit_test(&pc.info, x, y, slop) {
                        Hit::Field(_) => env.ctx.set_cursor_icon(CursorIcon::Text),
                        Hit::Obj(_) if !matches!(tab.drag, Drag::Resize { .. }) => env.ctx.set_cursor_icon(CursorIcon::Move),
                        _ => {}
                    }
                    if ctrl && over_link.is_some() {
                        env.ctx.set_cursor_icon(CursorIcon::PointingHand);
                    }
                }
                Tool::TextSelect | Tool::Highlight | Tool::Underline | Tool::StrikeOut | Tool::Squiggly => env.ctx.set_cursor_icon(CursorIcon::Text),
                Tool::Measure | Tool::ExportZone | Tool::Redact | Tool::Link | Tool::Shape(_) | Tool::FormText | Tool::FormCheck | Tool::FormRadio | Tool::FormCombo => {
                    env.ctx.set_cursor_icon(CursorIcon::Crosshair)
                }
                _ => {}
            }
        }
    }

    // Follow a link: Hand tool, or Ctrl+click with the Selection tool
    if resp.clicked() && (env.tool == Tool::Hand || (env.tool == Tool::Select && ctrl)) {
        if let Some((x, y)) = pp {
            if let Some(l) = pc.info.links.iter().find(|l| r4_contains(&l.rect, x, y)) {
                match &l.target {
                    LinkTarget::Uri(u) => env.msgs.push(Msg::OpenUri(u.clone())),
                    LinkTarget::Page(p) => tab.goto_page(*p),
                    LinkTarget::None => {}
                }
                return;
            }
        }
    }

    match env.tool {
        Tool::Hand => {
            if resp.drag_started() {
                tab.drag = Drag::Pan;
            }
            if resp.dragged() {
                tab.view.set_offset = Some(tab.view.offset - resp.drag_delta());
            }
            if resp.drag_stopped() {
                tab.drag = Drag::None;
            }
        }
        Tool::Select => interact_select(ui, tab, env, pc, &resp, pp, shift, slop),
        Tool::TextSelect | Tool::Highlight | Tool::Underline | Tool::StrikeOut | Tool::Squiggly => interact_text_select(tab, env, pc, &resp, pp),
        Tool::Measure => {
            if resp.clicked() {
                if let Some((x, y)) = pp {
                    if tab.measure_page != page || tab.measure.len() >= 64 {
                        tab.measure.clear();
                    }
                    tab.measure_page = page;
                    tab.measure.push([x, y]);
                }
            }
            if resp.double_clicked() {
                tab.measure_page = page;
            }
        }
        Tool::ExportZone | Tool::Redact | Tool::Link | Tool::FormText | Tool::FormCheck | Tool::FormRadio | Tool::FormCombo => interact_zone(tab, env, pc, &resp, pp),
        Tool::Shape(ShapeKind::Polygon) => {
            if resp.clicked() {
                if let Some((x, y)) = pp {
                    tab.measure_page = page;
                    tab.poly.push([x, y]);
                }
            }
            if resp.double_clicked() && tab.poly.len() >= 3 {
                let pts = std::mem::take(&mut tab.poly);
                env.msgs.push(Msg::PolygonDone { page, pts });
            }
        }
        Tool::Shape(ShapeKind::Freehand) => {
            if resp.drag_started() {
                if let Some((x, y)) = pp {
                    tab.drag = Drag::Ink { page, pts: vec![[x, y]] };
                }
            }
            if resp.dragged() {
                if let (Drag::Ink { pts, .. }, Some((x, y))) = (&mut tab.drag, pp) {
                    if pts.last().map(|l| (l[0] - x).hypot(l[1] - y) > 0.6).unwrap_or(true) {
                        pts.push([x, y]);
                    }
                }
            }
            if resp.drag_stopped() {
                if let Drag::Ink { page, pts } = std::mem::take(&mut tab.drag) {
                    if pts.len() >= 2 {
                        env.msgs.push(Msg::InkDone { page, pts });
                    }
                }
            }
        }
        Tool::Shape(kind) => {
            if resp.drag_started() {
                if let Some((x, y)) = pp {
                    tab.drag = Drag::Shape { page, a: [x, y], b: [x, y] };
                }
            }
            if resp.dragged() {
                if let (Drag::Shape { b, .. }, Some((x, y))) = (&mut tab.drag, pp) {
                    *b = [x, y];
                }
            }
            if resp.drag_stopped() {
                if let Drag::Shape { page, a, b } = std::mem::take(&mut tab.drag) {
                    if (a[0] - b[0]).hypot(a[1] - b[1]) > 3.0 {
                        env.msgs.push(Msg::ShapeDone { page, kind, a, b });
                    }
                }
            }
        }
        Tool::AddText => {
            if resp.clicked() {
                if let Some((x, y)) = pp {
                    tab.inline = Some(Inline { page, kind: InlineKind::NewText { x, y }, text: String::new(), focus: true });
                }
            }
        }
        Tool::AddImage => {
            if resp.clicked() {
                if let Some((x, y)) = pp {
                    env.msgs.push(Msg::AddImageAt { page, x, y });
                }
            }
        }
        Tool::Note => {
            if resp.clicked() {
                if let Some((x, y)) = pp {
                    env.msgs.push(Msg::NoteAt { page, x, y });
                }
            }
        }
        Tool::Stamp => {
            if resp.clicked() {
                if let Some((x, y)) = pp {
                    env.msgs.push(Msg::StampAt { page, x, y });
                }
            }
        }
        Tool::Signature => {
            if resp.clicked() {
                if let Some((x, y)) = pp {
                    env.msgs.push(Msg::SignAt { page, x, y });
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn interact_select(ui: &mut egui::Ui, tab: &mut Tab, env: &mut ViewEnv, pc: &PageCtx, resp: &egui::Response, pp: Option<(f32, f32)>, shift: bool, slop: f32) {
    let page = pc.page;
    if matches!(tab.drag, Drag::Resize { .. } | Drag::PathPoint { .. }) {
        return;
    }
    let hit = pp.map(|(x, y)| hit_test(&pc.info, x, y, slop)).unwrap_or(Hit::None);

    if resp.clicked() || resp.drag_started() {
        // a click outside the inline editor validates it
        match hit {
            Hit::Field(fi) => {
                tab.sel.clear();
                tab.sel.page = page;
                tab.sel.field = Some(fi);
                env.msgs.push(Msg::SelectionChanged);
                if resp.clicked() {
                    let f = &pc.info.fields[fi];
                    if !f.read_only {
                        match f.kind {
                            FieldKind::Check | FieldKind::Radio => env.msgs.push(Msg::ToggleField { page, field: fi }),
                            FieldKind::Text => {
                                tab.inline = Some(Inline { page, kind: InlineKind::Field(fi), text: f.value.clone(), focus: true });
                            }
                            FieldKind::Combo | FieldKind::List => env.msgs.push(Msg::ListPopup { page, field: fi }),
                            _ => {}
                        }
                    }
                }
            }
            Hit::Obj(i) => {
                let already = tab.sel.contains(page, i);
                if shift {
                    if tab.sel.page != page {
                        tab.sel.clear();
                    }
                    tab.sel.page = page;
                    if already {
                        tab.sel.items.retain(|&x| x != i);
                    } else {
                        tab.sel.items.push(i);
                    }
                    tab.sel.field = None;
                    tab.sel.annot = None;
                    env.msgs.push(Msg::SelectionChanged);
                } else if !already {
                    tab.sel.clear();
                    tab.sel.page = page;
                    tab.sel.items = vec![i];
                    env.msgs.push(Msg::SelectionChanged);
                }
                tab.view.page = page;
            }
            Hit::Annot(i) => {
                tab.sel.clear();
                tab.sel.page = page;
                tab.sel.annot = Some(i);
                env.msgs.push(Msg::SelectionChanged);
            }
            Hit::None => {
                if resp.clicked() && !shift {
                    tab.sel.clear();
                    env.msgs.push(Msg::SelectionChanged);
                }
            }
        }
    }

    // drag: move the selection or draw a marquee
    if resp.drag_started() {
        let on_sel = matches!(hit, Hit::Obj(i) if tab.sel.contains(page, i));
        if on_sel && tab.inline.is_none() {
            tab.drag = Drag::Move { page, accum: Vec2::ZERO };
        } else if matches!(hit, Hit::None) {
            if let Some(p) = resp.interact_pointer_pos() {
                tab.drag = Drag::Marquee { page, start: p, cur: p };
            }
        }
    }
    if resp.dragged() {
        match &mut tab.drag {
            Drag::Move { accum, .. } => *accum += resp.drag_delta(),
            Drag::Marquee { cur, .. } => {
                if let Some(p) = resp.interact_pointer_pos() {
                    *cur = p;
                }
            }
            _ => {}
        }
    }
    if resp.drag_stopped() {
        match tab.drag.clone() {
            Drag::Move { page: p, accum } if p == page => {
                tab.drag = Drag::None;
                if accum.length() > 1.0 {
                    let (dx, dy) = pc.delta_to_page(accum);
                    env.msgs.push(Msg::MoveSelection { page, dx, dy });
                }
            }
            Drag::Marquee { page: p, start, cur } if p == page => {
                tab.drag = Drag::None;
                let r = pc.rect_to_page(&Rect::from_two_pos(start, cur));
                let page_area = pc.geom().w * pc.geom().h;
                let items: Vec<usize> = pc
                    .info
                    .objects
                    .iter()
                    .filter(|o| r4_area(&o.rect) < page_area * 0.8 && r4_intersects(&o.rect, &r))
                    .filter(|o| o.kind != ObjKind::Path || r4_inside(&o.rect, &r, 0.0))
                    .map(|o| o.idx)
                    .collect();
                if shift {
                    if tab.sel.page != page {
                        tab.sel.clear();
                    }
                    for i in items {
                        if !tab.sel.items.contains(&i) {
                            tab.sel.items.push(i);
                        }
                    }
                } else {
                    tab.sel.clear();
                    tab.sel.items = items;
                }
                tab.sel.page = page;
                env.msgs.push(Msg::SelectionChanged);
            }
            _ => {}
        }
    }

    // double-click: edit the text in place, replace the image…
    if resp.double_clicked() {
        if let Hit::Obj(i) = hit {
            if let Some(o) = pc.info.object(i) {
                match o.kind {
                    ObjKind::Text => {
                        tab.inline = Some(Inline { page, kind: InlineKind::Object(i), text: o.text().to_string(), focus: true });
                    }
                    ObjKind::Image => env.msgs.push(Msg::ReplaceImage { page, idx: i }),
                    ObjKind::Form => env.msgs.push(Msg::Cmd(Cmd::Ungroup)),
                    _ => {}
                }
            }
        }
    }
    let _ = ui;
}

fn interact_text_select(tab: &mut Tab, env: &mut ViewEnv, pc: &PageCtx, resp: &egui::Response, pp: Option<(f32, f32)>) {
    let page = pc.page;
    let tol = 6.0 / pc.s;
    if resp.drag_started() {
        if let Some((x, y)) = pp {
            if let Some(c) = tab.char_near(page, x, y, tol) {
                tab.drag = Drag::TextSel { page, anchor: c };
                tab.text_sel = Some(TextSel { page, a: c, b: c });
            } else {
                tab.drag = Drag::None;
                tab.text_sel = None;
            }
        }
    }
    if resp.dragged() {
        if let (Drag::TextSel { anchor, .. }, Some((x, y))) = (&tab.drag, pp) {
            let anchor = *anchor;
            if let Some(c) = tab.char_near(page, x, y, tol * 3.0) {
                tab.text_sel = Some(TextSel { page, a: anchor, b: c });
            }
        }
    }
    if resp.drag_stopped() {
        tab.drag = Drag::None;
        finish_text_selection(tab, env, page);
    }
    if resp.double_clicked() {
        if let Some((x, y)) = pp {
            if let Some(c) = tab.char_near(page, x, y, tol) {
                if let Some((a, b)) = tab.word_at(page, c) {
                    tab.text_sel = Some(TextSel { page, a, b });
                    finish_text_selection(tab, env, page);
                }
            }
        }
    }
    if resp.triple_clicked() {
        if let Some((x, y)) = pp {
            if let Some(c) = tab.char_near(page, x, y, tol) {
                if let Some((a, b)) = tab.line_at(page, c) {
                    tab.text_sel = Some(TextSel { page, a, b });
                    finish_text_selection(tab, env, page);
                }
            }
        }
    }
    if resp.clicked() && !resp.double_clicked() && !resp.triple_clicked() {
        tab.text_sel = None;
    }
}

/// End of a text selection: for the markup tools, creates the highlight immediately.
fn finish_text_selection(tab: &mut Tab, env: &mut ViewEnv, page: usize) {
    let Some(ts) = tab.text_sel else { return };
    if env.tool.is_markup() {
        let (a, b) = ts.ordered();
        let (_, rects) = tab.text_range(page, a, b);
        if !rects.is_empty() {
            env.msgs.push(Msg::Markup { page, tool: env.tool, rects });
            tab.text_sel = None;
        }
    }
}

fn interact_zone(tab: &mut Tab, env: &mut ViewEnv, pc: &PageCtx, resp: &egui::Response, pp: Option<(f32, f32)>) {
    let page = pc.page;
    if resp.drag_started() {
        if let Some((x, y)) = pp {
            tab.drag = Drag::Zone { page, a: [x, y], b: [x, y] };
        }
    }
    if resp.dragged() {
        if let (Drag::Zone { b, .. }, Some((x, y))) = (&mut tab.drag, pp) {
            *b = [x, y];
        }
    }
    if resp.drag_stopped() {
        if let Drag::Zone { page: p, a, b } = std::mem::take(&mut tab.drag) {
            let r = r4_from_pts(a, b);
            if p == page && r4_w(&r) > 2.0 && r4_h(&r) > 2.0 {
                match env.tool {
                    Tool::ExportZone => env.msgs.push(Msg::ExportZone { page, rect: r }),
                    Tool::Redact => env.msgs.push(Msg::RedactZone { page, rect: r }),
                    Tool::Link => env.msgs.push(Msg::LinkZone { page, rect: r }),
                    t => env.msgs.push(Msg::FormZone { tool: t, page, rect: r }),
                }
            }
        }
    }
    // a simple click on a checkbox/radio creates a field of default size
    if resp.clicked() && matches!(env.tool, Tool::FormCheck | Tool::FormRadio) {
        if let Some((x, y)) = pp {
            env.msgs.push(Msg::FormZone { tool: env.tool, page, rect: [x - 7.0, y - 7.0, x + 7.0, y + 7.0] });
        }
    }
}

// ------------------------------------------------------------ inline editor

fn draw_inline_editor(ui: &mut egui::Ui, tab: &mut Tab, env: &mut ViewEnv, pc: &PageCtx) {
    let Some(inl) = tab.inline.clone() else { return };
    if inl.page != pc.page {
        return;
    }
    let (rect_page, font_pt, multi): (R4, f32, bool) = match &inl.kind {
        InlineKind::Object(i) => match pc.info.object(*i) {
            Some(o) => (o.rect, o.block.as_ref().map(|b| b.first_run().size).unwrap_or(12.0), o.block.as_ref().map(|b| b.lines.len() > 1).unwrap_or(false)),
            None => {
                tab.inline = None;
                return;
            }
        },
        InlineKind::Field(fi) => match pc.info.fields.get(*fi) {
            Some(f) => (f.rect, 12.0, false),
            None => {
                tab.inline = None;
                return;
            }
        },
        InlineKind::NewText { x, y } => ([*x, *y - 14.0, *x + 160.0, *y + 4.0], env.style.text.size, false),
        InlineKind::Note(ai) => match pc.info.annots.iter().find(|a| a.idx == *ai) {
            Some(a) => (a.rect, 12.0, true),
            None => {
                tab.inline = None;
                return;
            }
        },
    };
    let r = pc.to_screen(&rect_page);
    let mut commit = false;
    let mut cancel = false;
    let mut nav: i32 = 0;
    let mut text = inl.text.clone();
    let focus = inl.focus;
    let id = egui::Id::unique(("inline", tab.id));
    egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(r.min - Vec2::new(4.0, 4.0)).show(&env.ctx.clone(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            // Tab / Shift+Tab: next / previous form field
            if matches!(inl.kind, InlineKind::Field(_)) && !focus {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab)) {
                    nav = -1;
                } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)) {
                    nav = 1;
                }
            }
            let fs = (font_pt * pc.s).clamp(11.0, 64.0);
            let w = (r.width() + 80.0).max(180.0);
            let te = if multi || matches!(inl.kind, InlineKind::Note(_)) {
                egui::TextEdit::multiline(&mut text).desired_width(w).desired_rows(3).font(egui::FontId::proportional(fs))
            } else {
                egui::TextEdit::singleline(&mut text).desired_width(w).font(egui::FontId::proportional(fs))
            };
            let er = ui.add(te);
            if focus {
                er.request_focus();
            }
            let (esc, enter, cmd) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter), i.modifiers.command));
            if er.has_focus() {
                if esc {
                    cancel = true;
                } else if enter && (cmd || !(multi || matches!(inl.kind, InlineKind::Note(_)))) {
                    commit = true;
                }
            }
            if er.lost_focus() && !esc && !commit && !cancel {
                commit = true;
            }
            if nav != 0 {
                commit = true;
            }
            ui.small(crate::i18n::t("inline_hint"));
        });
    });
    if let Some(i) = tab.inline.as_mut() {
        i.text = text.clone();
        i.focus = false;
    }
    if cancel {
        tab.inline = None;
    } else if commit {
        let kind = inl.kind.clone();
        tab.inline = None;
        match kind {
            InlineKind::Object(idx) => env.msgs.push(Msg::CommitObjectText { page: pc.page, idx, text }),
            InlineKind::Field(field) => {
                env.msgs.push(Msg::CommitFieldText { page: pc.page, field, text });
                if nav != 0 {
                    // text fields in reading order; loop at the end of the page
                    let order: Vec<usize> = crate::forms::field_nav_order(&pc.info).into_iter().filter(|&i| pc.info.fields[i].kind == crate::model::FieldKind::Text && !pc.info.fields[i].read_only).collect();
                    if let Some(pos) = order.iter().position(|&i| i == field) {
                        let next = order[(pos as i32 + nav).rem_euclid(order.len() as i32) as usize];
                        let value = pc.info.fields[next].value.clone();
                        tab.sel.page = pc.page;
                        tab.sel.field = Some(next);
                        tab.inline = Some(Inline { page: pc.page, kind: InlineKind::Field(next), text: value, focus: true });
                    }
                }
            }
            InlineKind::NewText { x, y } => {
                if !text.trim().is_empty() {
                    env.msgs.push(Msg::CommitNewText { page: pc.page, x, y, text });
                }
            }
            InlineKind::Note(annot) => env.msgs.push(Msg::CommitNoteText { page: pc.page, annot, text }),
        }
    }
    let _ = ui;
}

/// Elements drawn above all the pages (nothing for now: reserved for guides).
fn draw_overlays_global(_ui: &mut egui::Ui, _tab: &mut Tab, _env: &mut ViewEnv, _lay: &Layout, _origin: Pos2, _s: f32) {}

/// Screen rectangle → text of the dimensions (for the status bar).
pub fn fmt_len(unit: Unit, pt: f32) -> String {
    unit.format(pt)
}

#[allow(dead_code)]
fn _unused(_: TextStyle, _: PathData) {}
