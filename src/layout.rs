//! Layout of the pages in the scroll area: single page, continuous, two pages.

use crate::prefs::ViewMode;
use eframe::egui::{Pos2, Rect, Vec2};

pub const PAGE_GAP: f32 = 14.0;
pub const PAGE_MARGIN: f32 = 18.0;

#[derive(Clone, Debug)]
pub struct PageSlot {
    pub page: usize,
    /// Rectangle of the page in the scrolling content (logical pixels).
    pub rect: Rect,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub slots: Vec<PageSlot>,
    /// Total size of the content.
    pub size: Vec2,
}

impl Layout {
    pub fn slot(&self, page: usize) -> Option<&PageSlot> {
        self.slots.iter().find(|s| s.page == page)
    }
}

/// Grouping of the pages into "spreads": one page, or two (with an optional cover).
pub fn spreads(n: usize, two: bool, cover: bool) -> Vec<Vec<usize>> {
    let mut out = vec![];
    if !two {
        return (0..n).map(|i| vec![i]).collect();
    }
    let mut i = 0;
    if cover && n > 0 {
        out.push(vec![0]);
        i = 1;
    }
    while i < n {
        if i + 1 < n {
            out.push(vec![i, i + 1]);
            i += 2;
        } else {
            out.push(vec![i]);
            i += 1;
        }
    }
    out
}

/// Computes the position of each page.
/// `s` = logical pixels per PDF point; `viewport` = size of the visible area;
/// `current` = current page (for the non-continuous modes).
pub fn compute(sizes: &[(f32, f32)], mode: ViewMode, cover: bool, s: f32, viewport: Vec2, current: usize) -> Layout {
    let n = sizes.len();
    if n == 0 {
        return Layout::default();
    }
    let two = matches!(mode, ViewMode::TwoPages | ViewMode::TwoPagesContinuous);
    let continuous = matches!(mode, ViewMode::Continuous | ViewMode::TwoPagesContinuous);
    let all = spreads(n, two, cover);
    let shown: Vec<&Vec<usize>> = if continuous {
        all.iter().collect()
    } else {
        let cur = current.min(n - 1);
        all.iter().filter(|sp| sp.contains(&cur)).collect()
    };
    // Width needed = widest spread
    let spread_w = |sp: &Vec<usize>| -> f32 {
        sp.iter().map(|&p| sizes[p].0 * s).sum::<f32>() + PAGE_GAP * (sp.len().saturating_sub(1)) as f32
    };
    let spread_h = |sp: &Vec<usize>| -> f32 { sp.iter().map(|&p| sizes[p].1 * s).fold(0.0, f32::max) };
    let max_w = shown.iter().map(|sp| spread_w(sp)).fold(0.0, f32::max);
    let total_w = (max_w + 2.0 * PAGE_MARGIN).max(viewport.x);
    let mut y = PAGE_MARGIN;
    let mut slots = vec![];
    for sp in &shown {
        let w = spread_w(sp);
        let h = spread_h(sp);
        let mut x = (total_w - w) * 0.5;
        for &p in sp.iter() {
            let (pw, ph) = (sizes[p].0 * s, sizes[p].1 * s);
            // pages of the same spread vertically centered
            let py = y + (h - ph) * 0.5;
            slots.push(PageSlot { page: p, rect: Rect::from_min_size(Pos2::new(x, py), Vec2::new(pw, ph)) });
            x += pw + PAGE_GAP;
        }
        y += h + PAGE_GAP;
    }
    let total_h = (y - PAGE_GAP + PAGE_MARGIN).max(viewport.y);
    Layout { slots, size: Vec2::new(total_w, total_h) }
}

/// Most visible page for a given scroll offset.
pub fn page_at(layout: &Layout, offset_y: f32, view_h: f32) -> Option<usize> {
    if layout.slots.is_empty() {
        return None;
    }
    let center = offset_y + view_h * 0.4;
    let mut best = layout.slots[0].page;
    let mut best_d = f32::MAX;
    for s in &layout.slots {
        let d = if center >= s.rect.min.y && center <= s.rect.max.y {
            0.0
        } else {
            (center - s.rect.center().y).abs()
        };
        if d < best_d {
            best_d = d;
            best = s.page;
        }
    }
    Some(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(n: usize) -> Vec<(f32, f32)> {
        vec![(100.0, 200.0); n]
    }

    #[test]
    fn spreads_with_and_without_cover() {
        assert_eq!(spreads(5, true, false), vec![vec![0, 1], vec![2, 3], vec![4]]);
        assert_eq!(spreads(5, true, true), vec![vec![0], vec![1, 2], vec![3, 4]]);
        assert_eq!(spreads(3, false, false), vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn continuous_stacks_pages_and_centers() {
        let l = compute(&sizes(3), ViewMode::Continuous, false, 1.0, Vec2::new(500.0, 300.0), 0);
        assert_eq!(l.slots.len(), 3);
        let (a, b) = (&l.slots[0].rect, &l.slots[1].rect);
        assert!((b.min.y - a.max.y - PAGE_GAP).abs() < 1e-3);
        assert!((a.center().x - 250.0).abs() < 1e-3, "centered in the visible area");
        assert!(l.size.y > 3.0 * 200.0);
    }

    #[test]
    fn single_mode_shows_only_current() {
        let l = compute(&sizes(10), ViewMode::Single, false, 1.0, Vec2::new(500.0, 300.0), 4);
        assert_eq!(l.slots.len(), 1);
        assert_eq!(l.slots[0].page, 4);
    }

    #[test]
    fn two_pages_side_by_side() {
        let l = compute(&sizes(4), ViewMode::TwoPagesContinuous, false, 1.0, Vec2::new(600.0, 300.0), 0);
        assert_eq!(l.slots.len(), 4);
        assert!((l.slots[0].rect.min.y - l.slots[1].rect.min.y).abs() < 1e-3);
        assert!(l.slots[1].rect.min.x > l.slots[0].rect.max.x);
        let nc = compute(&sizes(4), ViewMode::TwoPages, false, 1.0, Vec2::new(600.0, 300.0), 3);
        assert_eq!(nc.slots.iter().map(|s| s.page).collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn page_at_follows_scroll() {
        let l = compute(&sizes(5), ViewMode::Continuous, false, 1.0, Vec2::new(500.0, 300.0), 0);
        assert_eq!(page_at(&l, 0.0, 300.0), Some(0));
        let y2 = l.slots[3].rect.min.y;
        assert_eq!(page_at(&l, y2, 300.0), Some(3));
    }
}
