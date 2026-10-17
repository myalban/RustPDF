//! Export of items: image (PNG, JPEG, WebP, BMP, TIFF), SVG of vector shapes, image extraction.

use crate::edit::{members_of, processed_image, union_rect};
use crate::geom::*;
use crate::pathdata::{PathData, Seg, read_path};
use crate::pdfx;
use crate::tab::Tab;
use image::DynamicImage;
use pdfium_render::prelude::*;
use std::path::Path;

fn err<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

impl Tab {
    /// Renders a page (optionally reduced to some objects) and cuts out `clip` (page space).
    /// Transparent background if `transparent`. `dpi` = output resolution.
    pub fn render_isolated(&self, page: usize, keep: Option<&[usize]>, clip: R4, transparent: bool, dpi: f32) -> Result<DynamicImage, String> {
        let r: Result<DynamicImage, PdfiumError> = (|| {
            let mut nd = self.pdfium.create_new_pdf()?;
            nd.pages_mut().copy_page_from_document(&self.doc, page as i32, 0)?;
            let mut pg = nd.pages().get(0)?;
            pg.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
            if let Some(keep) = keep {
                {
                    let objs = pg.objects_mut();
                    let n = objs.len() as usize;
                    for idx in (0..n).rev() {
                        if !keep.contains(&idx) {
                            let o = objs.get(idx)?;
                            objs.remove_object(o)?;
                        }
                    }
                }
                pg.regenerate_content()?;
            }
            let geom = pdfx::page_geom(&pg);
            // pixels per point, limited to 7000 px per side
            let s = (dpi / 72.0).min(7000.0 / geom.w.max(geom.h).max(1.0));
            let bg = if transparent { PdfColor::new(255, 255, 255, 0) } else { PdfColor::new(255, 255, 255, 255) };
            let cfg = PdfRenderConfig::new()
                .set_target_width((geom.w * s).max(1.0) as i32)
                .set_clear_color(bg)
                .render_annotations(false)
                .render_form_data(false);
            let full = pg.render_with_config(&cfg)?.as_image()?.to_rgba8();
            let (iw, ih) = (full.width(), full.height());
            let v = geom.rect_to_view(&clip);
            let x0 = ((v[0] * s).floor().max(0.0) as u32).min(iw.saturating_sub(1));
            let y0 = ((v[1] * s).floor().max(0.0) as u32).min(ih.saturating_sub(1));
            let x1 = ((v[2] * s).ceil().max(0.0) as u32).clamp(x0 + 1, iw);
            let y1 = ((v[3] * s).ceil().max(0.0) as u32).clamp(y0 + 1, ih);
            Ok(DynamicImage::ImageRgba8(image::imageops::crop_imm(&full, x0, y0, x1 - x0, y1 - y0).to_image()))
        })();
        r.map_err(err)
    }

    /// Image of an area: if `isolate`, only the items entirely inside the area are kept (transparent background).
    pub fn export_region_image(&mut self, page: usize, r: R4, isolate: bool, dpi: f32) -> Result<DynamicImage, String> {
        let info = self.info(page).ok_or("page not found")?;
        let page_area = info.geom.w * info.geom.h;
        let keep: Option<Vec<usize>> = if isolate {
            Some(
                info.objects
                    .iter()
                    .filter(|o| r4_inside(&o.rect, &r, 0.5) && r4_area(&o.rect) < page_area * 0.8)
                    .flat_map(|o| o.members.clone())
                    .collect(),
            )
        } else {
            None
        };
        self.render_isolated(page, keep.as_deref(), r, isolate, dpi)
    }

    /// Image of the selected items (transparent background).
    pub fn export_items_image(&mut self, page: usize, items: &[usize], dpi: f32) -> Result<DynamicImage, String> {
        let info = self.info(page).ok_or("page not found")?;
        let m = members_of(&info, items);
        let r = union_rect(&info, items).ok_or("no item")?;
        self.render_isolated(page, Some(&m), r4_expand(&r, 0.5), true, dpi)
    }

    /// Original image of an image object (pixels of the document).
    pub fn export_original_image(&mut self, page: usize, idx: usize) -> Result<DynamicImage, String> {
        self.read_image(page, idx)
    }

    /// All the images of a page (object index, image).
    pub fn page_images(&mut self, page: usize) -> Vec<(usize, DynamicImage)> {
        let mut out = vec![];
        let Ok(pg) = self.read_page(page) else { return out };
        for i in 0..pg.objects().len() {
            let Ok(mut o) = pg.objects().get(i) else { continue };
            if o.as_image_object().is_some() {
                if let Some(img) = processed_image(&mut o, &self.doc) {
                    out.push((i, img));
                }
            }
        }
        out
    }

    /// Extracts all the images of the pages `pages` into `dir`; returns the number of files written.
    pub fn extract_images(&mut self, pages: &[usize], dir: &Path, png: bool) -> Result<usize, String> {
        std::fs::create_dir_all(dir).map_err(err)?;
        let mut n = 0;
        for &p in pages {
            for (i, img) in self.page_images(p) {
                let ext = if png { "png" } else { "jpg" };
                let f = dir.join(format!("page{:03}_img{:02}.{ext}", p + 1, i));
                let out = if png { img } else { DynamicImage::ImageRgb8(img.to_rgb8()) };
                out.save(&f).map_err(err)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// SVG of the vector paths (and texts) of items; coordinates in points, origin at the top-left of the area.
    pub fn export_svg(&mut self, page: usize, items: &[usize]) -> Result<String, String> {
        let info = self.info(page).ok_or("page not found")?;
        let bounds = union_rect(&info, items).ok_or("no item")?;
        let g = info.geom;
        let vb = g.rect_to_view(&bounds);
        let (w, h) = (vb[2] - vb[0], vb[3] - vb[1]);
        let mut body = String::new();
        let members = members_of(&info, items);
        let pg = self.read_page(page)?;
        for k in members {
            let Ok(o) = pg.objects().get(k) else { continue };
            match o.object_type() {
                PdfPageObjectType::Path => {
                    if let Some(mut d) = read_path(&o) {
                        // page → view → area frame
                        let to_view = g.to_view;
                        let aff = Affine { a: to_view.a, b: to_view.b, c: to_view.c, d: to_view.d, e: to_view.e - vb[0], f: to_view.f - vb[1] };
                        d.transform(&aff);
                        body.push_str(&path_to_svg(&d));
                    }
                }
                PdfPageObjectType::Text => {
                    if let (Some(t), Ok(m)) = (o.as_text_object(), o.matrix()) {
                        let (x, y) = g.to_view.apply(m.e(), m.f());
                        let col = o.fill_color().map(|c| format!("#{:02x}{:02x}{:02x}", c.red(), c.green(), c.blue())).unwrap_or("#000".into());
                        let (fam, bold, italic) = crate::fonts::parse_pdf_font_name(&t.font().name());
                        body.push_str(&format!(
                            "<text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{}\" font-size=\"{:.2}\" fill=\"{}\"{}{}>{}</text>\n",
                            x - vb[0],
                            y - vb[1],
                            xml_escape(&fam),
                            t.scaled_font_size().value,
                            col,
                            if bold { " font-weight=\"bold\"" } else { "" },
                            if italic { " font-style=\"italic\"" } else { "" },
                            xml_escape(&t.text())
                        ));
                    }
                }
                _ => {}
            }
        }
        if body.is_empty() {
            return Err("No vector item or text to export".into());
        }
        Ok(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.2}\" height=\"{h:.2}\" viewBox=\"0 0 {w:.2} {h:.2}\">\n{body}</svg>\n"
        ))
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Converts a path (already in SVG coordinates) into a `<path>` element.
pub fn path_to_svg(d: &PathData) -> String {
    let mut data = String::new();
    for s in &d.segs {
        match *s {
            Seg::Move(x, y) => data.push_str(&format!("M{x:.2} {y:.2} ")),
            Seg::Line(x, y) => data.push_str(&format!("L{x:.2} {y:.2} ")),
            Seg::Cubic(x1, y1, x2, y2, x, y) => data.push_str(&format!("C{x1:.2} {y1:.2} {x2:.2} {y2:.2} {x:.2} {y:.2} ")),
            Seg::Close => data.push_str("Z "),
        }
    }
    let col = |c: [u8; 4]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let fill = d.fill.map(|c| format!("fill=\"{}\" fill-opacity=\"{:.3}\"", col(c), c[3] as f32 / 255.0)).unwrap_or_else(|| "fill=\"none\"".into());
    let stroke = d
        .stroke
        .map(|c| {
            let dash = if d.dash.is_empty() { String::new() } else { format!(" stroke-dasharray=\"{}\"", d.dash.iter().map(|v| format!("{v:.2}")).collect::<Vec<_>>().join(" ")) };
            format!("stroke=\"{}\" stroke-opacity=\"{:.3}\" stroke-width=\"{:.2}\"{dash}", col(c), c[3] as f32 / 255.0, d.width)
        })
        .unwrap_or_else(|| "stroke=\"none\"".into());
    let rule = if d.even_odd { " fill-rule=\"evenodd\"" } else { "" };
    format!("<path d=\"{}\" {fill} {stroke}{rule}/>\n", data.trim_end())
}

/// Saves an image to a file; the format is deduced from the extension. JPEG/BMP: composited on a white background.
/// Opens an image chosen by the user with decoding limits (a tiny file can claim to be gigapixels large).
pub fn open_image_safe(path: &Path) -> Result<DynamicImage, String> {
    let mut reader = image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(768 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|e| e.to_string())
}

pub fn save_image_file(img: &DynamicImage, path: &Path) -> Result<(), String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let out = if matches!(ext.as_str(), "jpg" | "jpeg" | "bmp") {
        let mut rgba = img.to_rgba8();
        for p in rgba.pixels_mut() {
            let a = p[3] as u32;
            for c in 0..3 {
                p[c] = ((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).to_rgb8())
    } else {
        img.clone()
    };
    out.save(path).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ObjKind;
    use crate::pathdata::build_path;
    use crate::tab::Tab;

    fn tab() -> Tab {
        let p = pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let solid = |c: [u8; 4]| DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(20, 20, image::Rgba(c)));
        {
            let mut objs = page.objects_mut();
            objs.create_image_object(PdfPoints::new(100.0), PdfPoints::new(500.0), &solid([255, 0, 0, 255]), Some(PdfPoints::new(50.0)), Some(PdfPoints::new(50.0))).unwrap();
            objs.create_image_object(PdfPoints::new(300.0), PdfPoints::new(500.0), &solid([0, 0, 255, 255]), Some(PdfPoints::new(50.0)), Some(PdfPoints::new(50.0))).unwrap();
            let mut r = PathData::rect([100.0, 300.0, 200.0, 350.0]);
            r.fill = Some([0, 128, 0, 255]);
            r.stroke = Some([0, 0, 0, 255]);
            build_path(&doc, &mut objs, &r).unwrap();
        }
        page.regenerate_content().unwrap();
        drop(page);
        Tab::new(p, doc, None, None, 1.0)
    }

    #[test]
    fn isolated_render_keeps_only_selected_and_is_transparent() {
        let t = tab();
        let blue_zone = [290.0, 490.0, 360.0, 560.0];
        let img = t.render_isolated(0, Some(&[1]), blue_zone, true, 144.0).unwrap().to_rgba8();
        let p = img.get_pixel(img.width() / 2, img.height() / 2);
        assert!(p[2] > 200 && p[3] == 255, "opaque blue center: {p:?}");
        assert_eq!(img.get_pixel(0, 0)[3], 0, "transparent corner");
        let img = t.render_isolated(0, Some(&[0]), blue_zone, true, 144.0).unwrap().to_rgba8();
        assert!(img.pixels().all(|p| p[3] == 0), "the blue area is empty when keeping only the red one");
    }

    #[test]
    fn region_export_isolates_objects_fully_inside() {
        let mut t = tab();
        // area around the red image only
        let img = t.export_region_image(0, [90.0, 490.0, 170.0, 560.0], true, 144.0).unwrap().to_rgba8();
        let c = img.get_pixel(img.width() / 2, img.height() / 2);
        assert!(c[0] > 200 && c[2] < 60, "red: {c:?}");
        // same area, without isolation: opaque white background
        let img = t.export_region_image(0, [90.0, 490.0, 170.0, 560.0], false, 144.0).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0)[3], 255);
    }

    #[test]
    fn svg_contains_path_and_valid_viewbox() {
        let mut t = tab();
        let info = t.info(0).unwrap();
        let rect = info.objects.iter().find(|o| o.kind == ObjKind::Path).unwrap().clone();
        let svg = t.export_svg(0, &[rect.idx]).unwrap();
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("<path d=\"M"), "{svg}");
        assert!(svg.contains("fill=\"#008000\""), "{svg}");
        assert!(svg.contains("viewBox=\"0 0 "));
        // image only: no vector items
        let img_idx = info.objects.iter().find(|o| o.kind == ObjKind::Image).unwrap().idx;
        assert!(t.export_svg(0, &[img_idx]).is_err());
    }

    #[test]
    fn extract_all_images_and_save_formats() {
        let mut t = tab();
        let dir = std::env::temp_dir().join(format!("rp_imgs_{}", std::process::id()));
        let n = t.extract_images(&[0], &dir, true).unwrap();
        assert_eq!(n, 2);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let img = t.export_original_image(0, 0).unwrap();
        let (png, jpg) = (dir.join("a.png"), dir.join("a.jpg"));
        save_image_file(&img, &png).unwrap();
        save_image_file(&img, &jpg).unwrap();
        assert!(png.exists() && jpg.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
