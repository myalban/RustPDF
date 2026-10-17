//! Vector paths: data model, PDFium reading/writing, common shapes.

use crate::geom::{Affine, R4};
use pdfium_render::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// Cubic Bézier curve: control 1, control 2, end point.
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PathData {
    pub segs: Vec<Seg>,
    /// RGBA fill (None = no fill).
    pub fill: Option<[u8; 4]>,
    /// RGBA outline (None = no outline).
    pub stroke: Option<[u8; 4]>,
    pub width: f32,
    pub dash: Vec<f32>,
    pub even_odd: bool,
}

/// Editable point type of a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtKind {
    Anchor,
    Control1,
    Control2,
}

#[derive(Clone, Copy, Debug)]
pub struct PathPoint {
    pub seg: usize,
    pub kind: PtKind,
    pub x: f32,
    pub y: f32,
}

impl PathData {
    pub fn new(segs: Vec<Seg>) -> PathData {
        PathData { segs, fill: None, stroke: Some([0, 0, 0, 255]), width: 1.0, dash: vec![], even_odd: false }
    }

    /// All the points (anchors and control handles).
    pub fn points(&self) -> Vec<PathPoint> {
        let mut v = vec![];
        for (i, s) in self.segs.iter().enumerate() {
            match *s {
                Seg::Move(x, y) | Seg::Line(x, y) => v.push(PathPoint { seg: i, kind: PtKind::Anchor, x, y }),
                Seg::Cubic(x1, y1, x2, y2, x, y) => {
                    v.push(PathPoint { seg: i, kind: PtKind::Control1, x: x1, y: y1 });
                    v.push(PathPoint { seg: i, kind: PtKind::Control2, x: x2, y: y2 });
                    v.push(PathPoint { seg: i, kind: PtKind::Anchor, x, y });
                }
                Seg::Close => {}
            }
        }
        v
    }

    pub fn set_point(&mut self, seg: usize, kind: PtKind, nx: f32, ny: f32) {
        if let Some(s) = self.segs.get_mut(seg) {
            match (s, kind) {
                (Seg::Move(x, y), PtKind::Anchor) | (Seg::Line(x, y), PtKind::Anchor) => {
                    *x = nx;
                    *y = ny;
                }
                (Seg::Cubic(_, _, _, _, x, y), PtKind::Anchor) => {
                    *x = nx;
                    *y = ny;
                }
                (Seg::Cubic(x1, y1, ..), PtKind::Control1) => {
                    *x1 = nx;
                    *y1 = ny;
                }
                (Seg::Cubic(_, _, x2, y2, ..), PtKind::Control2) => {
                    *x2 = nx;
                    *y2 = ny;
                }
                _ => {}
            }
        }
    }

    /// Bounding box of the points (including controls: upper bound of the curve).
    pub fn bounds(&self) -> R4 {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in self.points() {
            b[0] = b[0].min(p.x);
            b[1] = b[1].min(p.y);
            b[2] = b[2].max(p.x);
            b[3] = b[3].max(p.y);
        }
        b
    }

    pub fn transform(&mut self, a: &Affine) {
        let t = |x: f32, y: f32| a.apply(x, y);
        for s in &mut self.segs {
            *s = match *s {
                Seg::Move(x, y) => {
                    let (x, y) = t(x, y);
                    Seg::Move(x, y)
                }
                Seg::Line(x, y) => {
                    let (x, y) = t(x, y);
                    Seg::Line(x, y)
                }
                Seg::Cubic(x1, y1, x2, y2, x, y) => {
                    let (x1, y1) = t(x1, y1);
                    let (x2, y2) = t(x2, y2);
                    let (x, y) = t(x, y);
                    Seg::Cubic(x1, y1, x2, y2, x, y)
                }
                Seg::Close => Seg::Close,
            };
        }
    }

    // ------------------------------------------------------------- shapes

    pub fn rect(r: R4) -> PathData {
        PathData::new(vec![Seg::Move(r[0], r[1]), Seg::Line(r[2], r[1]), Seg::Line(r[2], r[3]), Seg::Line(r[0], r[3]), Seg::Close])
    }

    pub fn ellipse(r: R4) -> PathData {
        const K: f32 = 0.552_284_75;
        let (cx, cy) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
        let (rx, ry) = ((r[2] - r[0]) * 0.5, (r[3] - r[1]) * 0.5);
        PathData::new(vec![
            Seg::Move(cx + rx, cy),
            Seg::Cubic(cx + rx, cy + K * ry, cx + K * rx, cy + ry, cx, cy + ry),
            Seg::Cubic(cx - K * rx, cy + ry, cx - rx, cy + K * ry, cx - rx, cy),
            Seg::Cubic(cx - rx, cy - K * ry, cx - K * rx, cy - ry, cx, cy - ry),
            Seg::Cubic(cx + K * rx, cy - ry, cx + rx, cy - K * ry, cx + rx, cy),
            Seg::Close,
        ])
    }

    pub fn line(a: (f32, f32), b: (f32, f32)) -> PathData {
        PathData::new(vec![Seg::Move(a.0, a.1), Seg::Line(b.0, b.1)])
    }

    pub fn polyline(pts: &[(f32, f32)], closed: bool) -> PathData {
        let mut segs = vec![];
        for (i, p) in pts.iter().enumerate() {
            segs.push(if i == 0 { Seg::Move(p.0, p.1) } else { Seg::Line(p.0, p.1) });
        }
        if closed {
            segs.push(Seg::Close);
        }
        PathData::new(segs)
    }

    /// Arrow from `a` to `b`: a single path (shaft + filled head).
    pub fn arrow(a: (f32, f32), b: (f32, f32), head: f32) -> PathData {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (ux, uy) = (dx / len, dy / len);
        let h = head.min(len * 0.6);
        let base = (b.0 - ux * h, b.1 - uy * h);
        let (nx, ny) = (-uy, ux);
        let w = h * 0.45;
        let mut p = PathData::new(vec![
            Seg::Move(a.0, a.1),
            Seg::Line(base.0, base.1),
            Seg::Move(b.0, b.1),
            Seg::Line(base.0 + nx * w, base.1 + ny * w),
            Seg::Line(base.0 - nx * w, base.1 - ny * w),
            Seg::Close,
        ]);
        p.fill = p.stroke;
        p
    }

    /// Smoothed freehand path: simplification (Douglas-Peucker) then Bézier curves (Catmull-Rom).
    pub fn freehand(pts: &[(f32, f32)]) -> PathData {
        let simp = simplify(pts, 0.6);
        if simp.len() < 3 {
            return PathData::polyline(&simp, false);
        }
        let mut segs = vec![Seg::Move(simp[0].0, simp[0].1)];
        for i in 0..simp.len() - 1 {
            let p0 = if i == 0 { simp[0] } else { simp[i - 1] };
            let (p1, p2) = (simp[i], simp[i + 1]);
            let p3 = if i + 2 < simp.len() { simp[i + 2] } else { p2 };
            let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
            let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
            segs.push(Seg::Cubic(c1.0, c1.1, c2.0, c2.1, p2.0, p2.1));
        }
        PathData::new(segs)
    }

    /// Total length (polyline; curves are approximated by their chord).
    pub fn length(&self) -> f32 {
        let mut len = 0.0;
        let mut cur = (0.0, 0.0);
        let mut start = (0.0, 0.0);
        for s in &self.segs {
            let next = match *s {
                Seg::Move(x, y) => {
                    start = (x, y);
                    (x, y)
                }
                Seg::Line(x, y) | Seg::Cubic(_, _, _, _, x, y) => (x, y),
                Seg::Close => start,
            };
            if !matches!(s, Seg::Move(..)) {
                len += ((next.0 - cur.0).powi(2) + (next.1 - cur.1).powi(2)).sqrt();
            }
            cur = next;
        }
        len
    }
}

fn matrix_apply(m: &PdfMatrix, x: f32, y: f32) -> (f32, f32) {
    (m.a() * x + m.c() * y + m.e(), m.b() * x + m.d() * y + m.f())
}

/// Reads a PDFium path in page coordinates (object matrix applied).
pub fn read_path(obj: &PdfPageObject) -> Option<PathData> {
    let p = obj.as_path_object()?;
    let m = obj.matrix().ok();
    let tf = |x: f32, y: f32| match &m {
        Some(m) => matrix_apply(m, x, y),
        None => (x, y),
    };
    let mut segs: Vec<Seg> = vec![];
    let mut pending: Vec<(f32, f32)> = vec![];
    for s in p.segments().iter() {
        let (x, y) = (s.x().value, s.y().value);
        match s.segment_type() {
            PdfPathSegmentType::MoveTo => {
                let (x, y) = tf(x, y);
                segs.push(Seg::Move(x, y));
            }
            PdfPathSegmentType::LineTo => {
                let (x, y) = tf(x, y);
                segs.push(Seg::Line(x, y));
            }
            PdfPathSegmentType::BezierTo => {
                pending.push(tf(x, y));
                if pending.len() == 3 {
                    segs.push(Seg::Cubic(pending[0].0, pending[0].1, pending[1].0, pending[1].1, pending[2].0, pending[2].1));
                    pending.clear();
                }
            }
            _ => {}
        }
        if s.is_close() {
            segs.push(Seg::Close);
        }
    }
    let col = |c: Result<PdfColor, PdfiumError>| c.ok().map(|c| [c.red(), c.green(), c.blue(), c.alpha()]);
    let fill_mode = p.fill_mode().unwrap_or(PdfPathFillMode::None);
    let fill = if fill_mode == PdfPathFillMode::None { None } else { col(obj.fill_color()) };
    let stroke = if p.is_stroked().unwrap_or(false) { col(obj.stroke_color()) } else { None };
    // outline width: scaled by the matrix (isotropic approximation)
    let sw = obj.stroke_width().map(|w| w.value).unwrap_or(1.0);
    let scale = m.map(|m| ((m.a() * m.d() - m.b() * m.c()).abs()).sqrt()).unwrap_or(1.0);
    let dash = obj.dash_array().map(|d| d.iter().map(|v| v.value * scale).collect()).unwrap_or_default();
    Some(PathData { segs, fill, stroke, width: sw * scale, dash, even_odd: fill_mode == PdfPathFillMode::EvenOdd })
}

/// Creates the path object on the page.
pub fn build_path(doc: &PdfDocument<'static>, objs: &mut PdfPageObjects<'static>, d: &PathData) -> Result<PdfPageObject<'static>, PdfiumError> {
    let (sx, sy) = match d.segs.iter().find_map(|s| match s {
        Seg::Move(x, y) | Seg::Line(x, y) => Some((*x, *y)),
        _ => None,
    }) {
        Some(p) => p,
        None => return Err(PdfiumError::PdfiumFunctionReturnValueIndicatedFailure),
    };
    let c = |c: [u8; 4]| PdfColor::new(c[0], c[1], c[2], c[3]);
    let mut path = PdfPagePathObject::new(
        doc,
        PdfPoints::new(sx),
        PdfPoints::new(sy),
        d.stroke.map(c),
        d.stroke.map(|_| PdfPoints::new(d.width.max(0.01))),
        d.fill.map(c),
    )?;
    let mut first = true;
    for s in &d.segs {
        match *s {
            Seg::Move(x, y) => {
                if first {
                    first = false;
                } else {
                    path.move_to(PdfPoints::new(x), PdfPoints::new(y))?;
                }
            }
            Seg::Line(x, y) => {
                if first {
                    first = false;
                } else {
                    path.line_to(PdfPoints::new(x), PdfPoints::new(y))?;
                }
            }
            Seg::Cubic(x1, y1, x2, y2, x, y) => {
                path.bezier_to(PdfPoints::new(x), PdfPoints::new(y), PdfPoints::new(x1), PdfPoints::new(y1), PdfPoints::new(x2), PdfPoints::new(y2))?;
            }
            Seg::Close => {
                path.close_path()?;
            }
        }
    }
    let mode = match (d.fill.is_some(), d.even_odd) {
        (false, _) => PdfPathFillMode::None,
        (true, true) => PdfPathFillMode::EvenOdd,
        (true, false) => PdfPathFillMode::Winding,
    };
    path.set_fill_and_stroke_mode(mode, d.stroke.is_some())?;
    let mut obj = objs.add_path_object(path)?;
    if !d.dash.is_empty() {
        let arr: Vec<PdfPoints> = d.dash.iter().map(|v| PdfPoints::new(*v)).collect();
        obj.set_dash_array(&arr, PdfPoints::ZERO)?;
    }
    Ok(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_have_expected_bounds() {
        let r = PathData::rect([10.0, 20.0, 110.0, 70.0]);
        assert_eq!(r.bounds(), [10.0, 20.0, 110.0, 70.0]);
        let e = PathData::ellipse([0.0, 0.0, 100.0, 50.0]);
        let b = e.bounds();
        assert!((b[0] - 0.0).abs() < 0.01 && (b[2] - 100.0).abs() < 0.01 && (b[3] - 50.0).abs() < 0.01);
        assert_eq!(PathData::line((0.0, 0.0), (3.0, 4.0)).length(), 5.0);
    }

    #[test]
    fn transform_and_set_point() {
        let mut p = PathData::rect([0.0, 0.0, 10.0, 10.0]);
        p.transform(&Affine { a: 2.0, b: 0.0, c: 0.0, d: 2.0, e: 5.0, f: 5.0 });
        assert_eq!(p.bounds(), [5.0, 5.0, 25.0, 25.0]);
        p.set_point(2, PtKind::Anchor, 40.0, 40.0);
        assert_eq!(p.bounds(), [5.0, 5.0, 40.0, 40.0]);
        assert_eq!(p.points().len(), 4);
    }

    #[test]
    fn arrow_is_single_path_with_head() {
        let a = PathData::arrow((0.0, 0.0), (100.0, 0.0), 12.0);
        assert!(a.fill.is_some());
        let b = a.bounds();
        assert!((b[2] - 100.0).abs() < 0.01 && b[3] > 4.0 && b[1] < -4.0);
    }

    #[test]
    fn build_and_read_roundtrip_through_pdfium() {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let mut d = PathData::ellipse([100.0, 100.0, 200.0, 160.0]);
        d.fill = Some([255, 0, 0, 255]);
        d.stroke = Some([0, 0, 255, 255]);
        d.width = 3.0;
        d.dash = vec![4.0, 2.0];
        {
            let mut objs = page.objects_mut();
            let o = build_path(&doc, &mut objs, &d).unwrap();
            drop(o);
            let r = PathData::rect([300.0, 300.0, 400.0, 350.0]);
            let mut r = r;
            r.fill = None;
            build_path(&doc, &mut objs, &r).unwrap();
        }
        page.regenerate_content().unwrap();
        let o = page.objects().get(0).unwrap();
        let back = read_path(&o).expect("path");
        let b = back.bounds();
        assert!((b[0] - 100.0).abs() < 0.5 && (b[2] - 200.0).abs() < 0.5 && (b[1] - 100.0).abs() < 0.5 && (b[3] - 160.0).abs() < 0.5, "{b:?}");
        assert_eq!(back.fill, Some([255, 0, 0, 255]));
        assert_eq!(back.stroke, Some([0, 0, 255, 255]));
        assert!((back.width - 3.0).abs() < 0.01);
        assert_eq!(back.dash.len(), 2);
        let c = back.segs.iter().filter(|s| matches!(s, Seg::Cubic(..))).count();
        assert_eq!(c, 4, "4 Bézier curves: {:?}", back.segs);
        let o2 = page.objects().get(1).unwrap();
        let r2 = read_path(&o2).unwrap();
        assert_eq!(r2.fill, None);
        assert_eq!(r2.bounds(), [300.0, 300.0, 400.0, 350.0]);
    }

    #[test]
    fn read_applies_object_matrix() {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        {
            let mut objs = page.objects_mut();
            let r = PathData::rect([0.0, 0.0, 100.0, 50.0]);
            let mut o = build_path(&doc, &mut objs, &r).unwrap();
            o.scale(2.0, 2.0).unwrap();
            o.translate(PdfPoints::new(10.0), PdfPoints::new(20.0)).unwrap();
        }
        let o = page.objects().get(0).unwrap();
        let b = read_path(&o).unwrap().bounds();
        let ob = o.bounds().unwrap().to_rect();
        assert!((b[2] - b[0] - 200.0).abs() < 0.5, "{b:?}");
        assert!((b[0] - ob.left().value).abs() < 3.0 && (b[3] - ob.top().value).abs() < 3.0, "{b:?} vs {ob:?}");
    }
}

/// Polyline simplification (Douglas-Peucker).
pub fn simplify(pts: &[(f32, f32)], eps: f32) -> Vec<(f32, f32)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    let mut best = (0usize, 0.0f32);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = if len < 1e-6 { ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt() } else { ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len };
        if d > best.1 {
            best = (i, d);
        }
    }
    if best.1 > eps {
        let mut l = simplify(&pts[..=best.0], eps);
        let r = simplify(&pts[best.0..], eps);
        l.pop();
        l.extend(r);
        l
    } else {
        vec![a, b]
    }
}

#[cfg(test)]
mod freehand_tests {
    use super::*;

    #[test]
    fn simplify_collapses_a_straight_line() {
        let pts: Vec<(f32, f32)> = (0..50).map(|i| (i as f32, 2.0 * i as f32)).collect();
        assert_eq!(simplify(&pts, 0.5).len(), 2);
    }

    #[test]
    fn freehand_passes_through_simplified_points() {
        let pts: Vec<(f32, f32)> = (0..=40).map(|i| (i as f32 * 3.0, 20.0 * (i as f32 * 0.2).sin())).collect();
        let p = PathData::freehand(&pts);
        assert!(p.segs.iter().any(|s| matches!(s, Seg::Cubic(..))));
        let b = p.bounds();
        assert!(b[0] <= 0.5 && b[2] >= 119.0, "{b:?}");
        // the last point is kept
        let last = p.points().last().copied().unwrap();
        assert!((last.x - 120.0).abs() < 0.01);
    }
}
