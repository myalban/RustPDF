//! Scan deskewing: skew estimation (projection profile) and image rotation.

use image::{GrayImage, Rgba, RgbaImage};

/// Estimates the angle (degrees, positive = content rotated counter-clockwise on screen) by which the content
/// is skewed. Rotating the image by `-angle` straightens it. Range ±10°, precision ≈ 0.05°.
pub fn estimate_skew(img: &GrayImage) -> f32 {
    // downscale for speed
    let (w, h) = (img.width(), img.height());
    let scale = (700.0 / w.max(h) as f32).min(1.0);
    let small = if scale < 1.0 { image::imageops::resize(img, (w as f32 * scale) as u32, (h as f32 * scale) as u32, image::imageops::FilterType::Triangle) } else { img.clone() };
    let (sw, sh) = (small.width() as f32, small.height() as f32);
    let mean = small.pixels().map(|p| p.0[0] as f32).sum::<f32>() / (sw * sh);
    let thr = (mean * 0.75).min(160.0);
    let pts: Vec<(f32, f32)> = small.enumerate_pixels().filter(|(_, _, p)| (p.0[0] as f32) < thr).map(|(x, y, _)| (x as f32, y as f32)).collect();
    if pts.len() < 50 {
        return 0.0;
    }
    let (cx, cy) = (sw / 2.0, sh / 2.0);
    let score = |deg: f32| -> f64 {
        let a = deg.to_radians();
        let (s, c) = a.sin_cos();
        let nb = (sh * 1.6) as usize + 8;
        let mut bins = vec![0u32; nb];
        for &(x, y) in &pts {
            let r = (y - cy) * c + (x - cx) * s + nb as f32 / 2.0;
            if r >= 0.0 && (r as usize) < nb {
                bins[r as usize] += 1;
            }
        }
        bins.iter().map(|&b| (b as f64) * (b as f64)).sum()
    };
    let mut best = (0.0f32, f64::MIN);
    let mut a = -10.0f32;
    while a <= 10.0 {
        let s = score(a);
        if s > best.1 {
            best = (a, s);
        }
        a += 0.25;
    }
    // refinement
    let mut fine = (best.0, best.1);
    let mut a = best.0 - 0.3;
    while a <= best.0 + 0.3 {
        let s = score(a);
        if s > fine.1 {
            fine = (a, s);
        }
        a += 0.05;
    }
    fine.0
}

/// Rotates the content by `deg` degrees counter-clockwise (on screen), around the center, with bilinear
/// interpolation. The image size is kept; uncovered corners are filled with `fill`.
pub fn rotate_image(img: &RgbaImage, deg: f32, fill: Rgba<u8>) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (s, c) = deg.to_radians().sin_cos();
    let mut out = RgbaImage::from_pixel(w, h, fill);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let sx = cx + dx * c - dy * s - 0.5;
            let sy = cy + dx * s + dy * c - 0.5;
            if sx < -0.5 || sy < -0.5 || sx > w as f32 - 0.5 || sy > h as f32 - 0.5 {
                continue;
            }
            let (x0, y0) = (sx.floor().max(0.0), sy.floor().max(0.0));
            let (fx, fy) = (sx - x0, sy - y0);
            let (x0, y0) = (x0 as u32, y0 as u32);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let p = |xx, yy| img.get_pixel(xx, yy).0;
            let (a, b, cc, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
            let mut o = [0u8; 4];
            for k in 0..4 {
                let top = a[k] as f32 * (1.0 - fx) + b[k] as f32 * fx;
                let bot = cc[k] as f32 * (1.0 - fx) + d[k] as f32 * fx;
                o[k] = (top * (1.0 - fy) + bot * fy).round() as u8;
            }
            out.put_pixel(x, y, Rgba(o));
        }
    }
    out
}

/// Straightens an image: returns `(image, estimated angle)`; the image is unchanged if the angle is negligible.
pub fn deskew(img: &RgbaImage) -> (RgbaImage, f32) {
    let gray = image::DynamicImage::ImageRgba8(img.clone()).to_luma8();
    let a = estimate_skew(&gray);
    if a.abs() < 0.15 {
        return (img.clone(), a);
    }
    // background: corner color (paper)
    let fill = *img.get_pixel(2.min(img.width() - 1), 2.min(img.height() - 1));
    (rotate_image(img, -a, fill), a)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// White image with "text lines": horizontal black bars of varying lengths.
    fn text_page() -> RgbaImage {
        let mut im = RgbaImage::from_pixel(700, 500, Rgba([255, 255, 255, 255]));
        for (k, y0) in (50..450).step_by(32).enumerate() {
            let len = 420 + (k as u32 * 37) % 150;
            for y in y0..y0 + 10 {
                for x in 80..80 + len {
                    // alternating "words" to look like text
                    if (x / 14) % 5 != 4 {
                        im.put_pixel(x, y, Rgba([20, 20, 20, 255]));
                    }
                }
            }
        }
        im
    }

    #[test]
    fn estimates_known_rotation() {
        let base = text_page();
        for a in [-4.0f32, -1.5, 0.0, 2.0, 5.0] {
            let rotated = rotate_image(&base, a, Rgba([255, 255, 255, 255]));
            let gray = image::DynamicImage::ImageRgba8(rotated).to_luma8();
            let est = estimate_skew(&gray);
            assert!((est - a).abs() < 0.4, "angle {a} estimated {est}");
        }
    }

    #[test]
    fn deskew_makes_lines_sharp_again() {
        let base = text_page();
        let skewed = rotate_image(&base, 3.0, Rgba([255, 255, 255, 255]));
        let (fixed, est) = deskew(&skewed);
        assert!((est - 3.0).abs() < 0.4, "{est}");
        // the sharpness of the horizontal profile (variance of the line sums) is recovered
        let sharp = |im: &RgbaImage| -> f64 {
            let rows: Vec<f64> = (0..im.height()).map(|y| (0..im.width()).filter(|&x| im.get_pixel(x, y).0[0] < 128).count() as f64).collect();
            let m = rows.iter().sum::<f64>() / rows.len() as f64;
            rows.iter().map(|r| (r - m).powi(2)).sum::<f64>()
        };
        assert!(sharp(&fixed) > sharp(&skewed) * 1.5, "net : {} → {}", sharp(&skewed), sharp(&fixed));
        // already straight image: unchanged
        let (same, e0) = deskew(&base);
        assert!(e0.abs() < 0.2);
        assert_eq!(same.as_raw(), base.as_raw());
    }

    #[test]
    fn blank_page_has_no_skew() {
        let blank = GrayImage::from_pixel(300, 300, image::Luma([255]));
        assert_eq!(estimate_skew(&blank), 0.0);
    }
}
