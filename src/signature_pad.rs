//! Handwritten signatures: saved vector strokes, typed signature or image.

use crate::geom::R4;
use crate::logx;
use crate::pathdata::{PathData, Seg, simplify};
use serde::{Deserialize, Serialize};

/// A drawn signature: strokes in normalized coordinates (x ∈ [0,1], y ∈ [0,1] downwards), W/H ratio.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SigStrokes {
    pub name: String,
    pub strokes: Vec<Vec<[f32; 2]>>,
    /// width / height of the bounding box
    pub aspect: f32,
}

impl SigStrokes {
    /// Normalizes strokes entered on screen (any units).
    pub fn from_screen(name: &str, strokes: &[Vec<[f32; 2]>]) -> Option<SigStrokes> {
        let pts: Vec<[f32; 2]> = strokes.iter().flatten().copied().collect();
        if pts.len() < 2 {
            return None;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &pts {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        let (w, h) = ((x1 - x0).max(1e-3), (y1 - y0).max(1e-3));
        let norm: Vec<Vec<[f32; 2]>> = strokes.iter().filter(|s| s.len() >= 2).map(|s| s.iter().map(|p| [(p[0] - x0) / w, (p[1] - y0) / h]).collect()).collect();
        Some(SigStrokes { name: name.to_string(), strokes: norm, aspect: w / h })
    }

    /// Vector path placed in `rect` (page space, y upwards), stroke width `width`.
    pub fn to_path(&self, rect: R4, color: [u8; 3], width: f32) -> PathData {
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        let mut segs: Vec<Seg> = vec![];
        for s in &self.strokes {
            let pts: Vec<(f32, f32)> = s.iter().map(|p| (rect[0] + p[0] * w, rect[3] - p[1] * h)).collect();
            let one = PathData::freehand(&pts);
            segs.extend(one.segs);
        }
        let mut d = PathData::new(segs);
        d.stroke = Some([color[0], color[1], color[2], 255]);
        d.width = width;
        d.fill = None;
        d
    }
}

fn store_path() -> std::path::PathBuf {
    logx::data_dir().join("signatures.json")
}

pub fn load_saved() -> Vec<SigStrokes> {
    std::fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_all(list: &[SigStrokes]) {
    if let Ok(s) = serde_json::to_string(list) {
        let _ = std::fs::write(store_path(), s);
    }
}

/// Reduces the number of points of a stroke (tolerance in screen units).
pub fn simplify_stroke(s: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    let v: Vec<(f32, f32)> = s.iter().map(|p| (p[0], p[1])).collect();
    simplify(&v, eps).into_iter().map(|p| [p.0, p.1]).collect()
}

/// Candidate script fonts for a typed signature (the first ones present on the system are offered).
pub const SCRIPT_FONTS: &[&str] = &["Segoe Script", "Snell Roundhand", "Apple Chancery", "Zapfino", "URW Chancery L", "Z003", "Purisa", "Kalam", "Lucida Handwriting", "Ink Free", "Brush Script MT", "Segoe Print", "Mistral", "Freestyle Script", "Edwardian Script ITC", "Monotype Corsiva", "Comic Sans MS"];

pub fn available_script_fonts() -> Vec<String> {
    let cat = crate::fonts::catalog();
    SCRIPT_FONTS.iter().filter(|f| cat.has_family(f)).map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_normalize_and_scale_into_a_rect() {
        let strokes = vec![vec![[100.0, 50.0], [150.0, 80.0], [200.0, 50.0]], vec![[100.0, 90.0], [200.0, 90.0]]];
        let s = SigStrokes::from_screen("me", &strokes).unwrap();
        assert!((s.aspect - 100.0 / 40.0).abs() < 1e-3);
        let all: Vec<[f32; 2]> = s.strokes.iter().flatten().copied().collect();
        assert!(all.iter().all(|p| (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1])));
        let d = s.to_path([100.0, 100.0, 200.0, 140.0], [0, 0, 128], 1.5);
        let b = d.bounds();
        assert!((b[0] - 100.0).abs() < 0.5 && (b[2] - 200.0).abs() < 0.5, "{b:?}");
        assert!((b[1] - 100.0).abs() < 0.5 && (b[3] - 140.0).abs() < 0.5, "{b:?}");
        assert_eq!(d.segs.iter().filter(|s| matches!(s, Seg::Move(..))).count(), 2, "one sub-path per stroke");
        assert!(SigStrokes::from_screen("x", &[vec![[1.0, 1.0]]]).is_none());
    }

    #[test]
    fn json_roundtrip() {
        let s = SigStrokes::from_screen("a", &[vec![[0.0, 0.0], [10.0, 5.0]]]).unwrap();
        let j = serde_json::to_string(&vec![s.clone()]).unwrap();
        let back: Vec<SigStrokes> = serde_json::from_str(&j).unwrap();
        assert_eq!(back, vec![s]);
    }

    #[test]
    fn simplify_keeps_endpoints() {
        let s: Vec<[f32; 2]> = (0..30).map(|i| [i as f32, 0.2 * (i % 2) as f32]).collect();
        let r = simplify_stroke(&s, 1.0);
        assert_eq!(r.first(), s.first());
        assert_eq!(r.last(), s.last());
        assert!(r.len() < s.len());
    }
}
