//! Geometry: rectangles in PDF page space and affine page → view transformation.

/// Rectangle `[left, bottom, right, top]` in PDF points (Y axis pointing up).
pub type R4 = [f32; 4];

pub fn r4_w(r: &R4) -> f32 {
    r[2] - r[0]
}
pub fn r4_h(r: &R4) -> f32 {
    r[3] - r[1]
}
pub fn r4_area(r: &R4) -> f32 {
    r4_w(r).max(0.0) * r4_h(r).max(0.0)
}
pub fn r4_union(a: &R4, b: &R4) -> R4 {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}
pub fn r4_contains(r: &R4, x: f32, y: f32) -> bool {
    x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3]
}
/// `inner` is entirely inside `outer` (with a tolerance).
pub fn r4_inside(inner: &R4, outer: &R4, tol: f32) -> bool {
    inner[0] >= outer[0] - tol && inner[1] >= outer[1] - tol && inner[2] <= outer[2] + tol && inner[3] <= outer[3] + tol
}
pub fn r4_intersects(a: &R4, b: &R4) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}
/// Rectangle normalized from any two corners.
pub fn r4_from_pts(a: [f32; 2], b: [f32; 2]) -> R4 {
    [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
}
pub fn r4_expand(r: &R4, d: f32) -> R4 {
    [r[0] - d, r[1] - d, r[2] + d, r[3] + d]
}
pub fn r4_center(r: &R4) -> (f32, f32) {
    ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5)
}

/// Transformation affine : `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// Linear part only (for vectors / displacements).
    pub fn apply_vec(&self, dx: f32, dy: f32) -> (f32, f32) {
        (self.a * dx + self.c * dy, self.b * dx + self.d * dy)
    }

    pub fn inverse(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Affine { a, b, c, d, e: -(a * self.e + c * self.f), f: -(b * self.e + d * self.f) })
    }

    /// Composition: applies `self` then `next`.
    pub fn then(&self, next: &Affine) -> Affine {
        Affine {
            a: next.a * self.a + next.c * self.b,
            b: next.b * self.a + next.d * self.b,
            c: next.a * self.c + next.c * self.d,
            d: next.b * self.c + next.d * self.d,
            e: next.a * self.e + next.c * self.f + next.e,
            f: next.b * self.e + next.d * self.f + next.f,
        }
    }

    /// Bounding box (min x, min y, max x, max y) of a transformed rectangle.
    pub fn bbox(&self, r: &R4) -> [f32; 4] {
        let pts = [self.apply(r[0], r[1]), self.apply(r[2], r[1]), self.apply(r[2], r[3]), self.apply(r[0], r[3])];
        let mut o = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for (x, y) in pts {
            o[0] = o[0].min(x);
            o[1] = o[1].min(y);
            o[2] = o[2].max(x);
            o[3] = o[3].max(y);
        }
        o
    }
}

/// Geometry of a page: size in view space (rotation applied, origin at the top-left,
/// Y pointing down, in points) and page ↔ view transformations.
#[derive(Clone, Copy, Debug)]
pub struct PageGeom {
    pub w: f32,
    pub h: f32,
    pub to_view: Affine,
    pub from_view: Affine,
}

impl PageGeom {
    /// "Simple" geometry: unrotated page, origin (0,0), Y pointing up.
    pub fn simple(w: f32, h: f32) -> PageGeom {
        let to_view = Affine { a: 1.0, b: 0.0, c: 0.0, d: -1.0, e: 0.0, f: h };
        PageGeom { w, h, to_view, from_view: to_view.inverse().unwrap_or(Affine::IDENTITY) }
    }

    /// Page rectangle (left, bottom, right, top) → view rectangle (min x, min y, max x, max y).
    pub fn rect_to_view(&self, r: &R4) -> [f32; 4] {
        self.to_view.bbox(r)
    }

    /// View rectangle → page rectangle `[left, bottom, right, top]`.
    pub fn rect_from_view(&self, v: &[f32; 4]) -> R4 {
        self.from_view.bbox(v)
    }

    pub fn pt_from_view(&self, x: f32, y: f32) -> (f32, f32) {
        self.from_view.apply(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_inverse_roundtrip() {
        let g = PageGeom::simple(600.0, 800.0);
        let (vx, vy) = g.to_view.apply(100.0, 700.0);
        assert!((vx - 100.0).abs() < 1e-4 && (vy - 100.0).abs() < 1e-4);
        let (px, py) = g.pt_from_view(vx, vy);
        assert!((px - 100.0).abs() < 1e-3 && (py - 700.0).abs() < 1e-3);
    }

    #[test]
    fn then_composes_in_order() {
        let scale = Affine { a: 2.0, b: 0.0, c: 0.0, d: 2.0, e: 0.0, f: 0.0 };
        let shift = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 10.0, f: 0.0 };
        let (x, _) = scale.then(&shift).apply(3.0, 0.0);
        assert_eq!(x, 16.0, "first the scale then the offset");
        let inv = scale.inverse().unwrap();
        let (y, _) = scale.then(&inv).apply(7.0, 1.0);
        assert!((y - 7.0).abs() < 1e-5);
    }

    #[test]
    fn rect_helpers() {
        let a = [0.0, 0.0, 10.0, 10.0];
        assert!(r4_inside(&[1.0, 1.0, 9.0, 9.0], &a, 0.0));
        assert!(!r4_inside(&[1.0, 1.0, 11.0, 9.0], &a, 0.5));
        assert!(r4_intersects(&a, &[5.0, 5.0, 20.0, 20.0]));
        assert_eq!(r4_union(&a, &[5.0, 5.0, 20.0, 20.0]), [0.0, 0.0, 20.0, 20.0]);
        assert_eq!(r4_from_pts([5.0, 1.0], [2.0, 9.0]), [2.0, 1.0, 5.0, 9.0]);
    }
}
