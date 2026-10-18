//! Rulers (top and left) and pointer coordinates, in the chosen unit.

use crate::layout;
use crate::prefs::{Prefs, Unit};
use crate::tab::Tab;
use crate::view::px_scale;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke};

pub const THICKNESS: f32 = 20.0;

/// Screen rectangle of the current page.
fn current_page_rect(tab: &Tab, prefs: &Prefs, avail: egui::Vec2) -> Option<(Rect, f32)> {
    let s = px_scale(tab.view.zoom);
    let lay = layout::compute(&tab.sizes, prefs.view_mode, prefs.cover_page, s, avail, tab.view.page);
    let slot = lay.slot(tab.view.page)?;
    Some((slot.rect.translate(tab.view.origin_screen.to_vec2()), s))
}

/// Tick step (in units) giving at least `min_px` pixels between two major ticks.
pub fn nice_step(px_per_unit: f32, min_px: f32) -> f32 {
    let mut mag = 0.01f32;
    loop {
        for m in [1.0, 2.0, 5.0] {
            let step = m * mag;
            if step * px_per_unit >= min_px {
                return step;
            }
        }
        mag *= 10.0;
        if mag > 1e7 {
            return mag;
        }
    }
}

fn decimals(step: f32) -> usize {
    if step >= 1.0 { 0 } else if step >= 0.1 { 1 } else { 2 }
}

#[allow(clippy::too_many_arguments)]
fn draw_axis(ui: &egui::Ui, strip: Rect, horizontal: bool, origin_px: f32, px_per_unit: f32, ptr: Option<f32>, dark: bool) {
    let p = ui.painter_at(strip);
    let (bg, fg, tick) = if dark { (Color32::from_gray(38), Color32::from_gray(190), Color32::from_gray(130)) } else { (Color32::from_gray(238), Color32::from_gray(60), Color32::from_gray(120)) };
    p.rect_filled(strip, 0.0, bg);
    let (lo, hi) = if horizontal { (strip.min.x, strip.max.x) } else { (strip.min.y, strip.max.y) };
    let step = nice_step(px_per_unit, 60.0);
    let minor = step / 5.0;
    let first = ((lo - origin_px) / px_per_unit / minor).floor() as i64;
    let last = ((hi - origin_px) / px_per_unit / minor).ceil() as i64;
    let font = FontId::proportional(10.0);
    for i in first..=last {
        let v = i as f32 * minor;
        let pos = origin_px + v * px_per_unit;
        let major = i % 5 == 0;
        let len = if major { THICKNESS * 0.6 } else { THICKNESS * 0.3 };
        let (a, b) = if horizontal { (Pos2::new(pos, strip.max.y), Pos2::new(pos, strip.max.y - len)) } else { (Pos2::new(strip.max.x, pos), Pos2::new(strip.max.x - len, pos)) };
        p.line_segment([a, b], Stroke::new(1.0, tick));
        if major {
            let label = format!("{:.*}", decimals(step), v);
            if horizontal {
                p.text(Pos2::new(pos + 3.0, strip.min.y + 1.0), Align2::LEFT_TOP, label, font.clone(), fg);
            } else {
                // text read from bottom to top: written letter by letter horizontally, compact
                p.text(Pos2::new(strip.min.x + 2.0, pos + 2.0), Align2::LEFT_TOP, label, font.clone(), fg);
            }
        }
    }
    if let Some(m) = ptr {
        let (a, b) = if horizontal { (Pos2::new(m, strip.min.y), Pos2::new(m, strip.max.y)) } else { (Pos2::new(strip.min.x, m), Pos2::new(strip.max.x, m)) };
        p.line_segment([a, b], Stroke::new(1.0, Color32::from_rgb(230, 60, 60)));
    }
}

/// Draws the two rulers around `inner` (document area); `full` is the total area (rulers included).
pub fn draw(ui: &egui::Ui, full: Rect, inner: Rect, tab: &Tab, prefs: &Prefs) {
    let dark = ui.visuals().dark_mode;
    let Some((page, s)) = current_page_rect(tab, prefs, inner.size()) else { return };
    let unit: Unit = prefs.units;
    let px_per_unit = s * unit.pt_per_unit();
    let ptr = ui.ctx().pointer_latest_pos();
    let top = Rect::from_min_max(Pos2::new(inner.min.x, full.min.y), Pos2::new(inner.max.x, inner.min.y));
    let left = Rect::from_min_max(Pos2::new(full.min.x, inner.min.y), Pos2::new(inner.min.x, inner.max.y));
    draw_axis(ui, top, true, page.min.x, px_per_unit, ptr.map(|p| p.x), dark);
    draw_axis(ui, left, false, page.min.y, px_per_unit, ptr.map(|p| p.y), dark);
    let corner = Rect::from_min_max(full.min, Pos2::new(inner.min.x, inner.min.y));
    ui.painter().rect_filled(corner, 0.0, if dark { Color32::from_gray(30) } else { Color32::from_gray(225) });
    ui.painter().text(corner.center(), Align2::CENTER_CENTER, unit.label(), FontId::proportional(9.0), if dark { Color32::from_gray(190) } else { Color32::from_gray(60) });
}

/// Pointer position on a page: (page, x, y) in points from the top-left corner of the page.
pub fn pointer_coords(ctx: &egui::Context, tab: &Tab, prefs: &Prefs) -> Option<(usize, f32, f32)> {
    let pos = ctx.pointer_latest_pos()?;
    let s = px_scale(tab.view.zoom);
    let lay = layout::compute(&tab.sizes, prefs.view_mode, prefs.cover_page, s, tab.view.avail, tab.view.page);
    for slot in &lay.slots {
        let r = slot.rect.translate(tab.view.origin_screen.to_vec2());
        if r.contains(pos) {
            return Some((slot.page, (pos.x - r.min.x) / s, (pos.y - r.min.y) / s));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_round_and_wide_enough() {
        for ppu in [0.5f32, 1.0, 3.78, 12.0, 80.0, 1000.0] {
            let st = nice_step(ppu, 60.0);
            assert!(st * ppu >= 60.0, "{ppu}");
            let m = st / 10f32.powf(st.log10().floor());
            assert!([1.0, 2.0, 5.0, 10.0].iter().any(|k| (m - k).abs() < 0.01), "{st}");
        }
    }
}
