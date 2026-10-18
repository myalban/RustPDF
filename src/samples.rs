//! Generation of demo documents (used by the tests and to try the application).

use crate::pathdata::{PathData, build_path};
use pdfium_render::prelude::*;

const LOREM: &[&str] = &[
    "RustPDF is a PDF document editor written in Rust. It lets you edit the text, replace",
    "images, add annotations, fill in forms and reorder the pages without leaving the",
    "application. This paragraph is made of several separate lines in the file: the editor",
    "groups them into a single text block that can be edited in one go, then automatically",
    "rewraps the lines to fit the width of the column, as in a word processor.",
];

pub fn build_demo(pdfium: &'static Pdfium) -> PdfDocument<'static> {
    let mut doc = pdfium.create_new_pdf().unwrap();
    let helv = doc.fonts_mut().helvetica();
    let bold = doc.fonts_mut().helvetica_bold();
    let times = doc.fonts_mut().times_roman();
    // ------------------------------------------------------------ page 1
    let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
    {
        let mut objs = page.objects_mut();
        let mut o = objs.create_text_object(PdfPoints::new(72.0), PdfPoints::new(760.0), "Demo document", bold, PdfPoints::new(26.0)).unwrap();
        o.set_fill_color(PdfColor::new(30, 60, 140, 255)).unwrap();
        for (i, l) in LOREM.iter().enumerate() {
            let w = if i == LOREM.len() - 1 { 0.0 } else { 0.0 };
            let _ = w;
            objs.create_text_object(PdfPoints::new(72.0), PdfPoints::new(712.0 - i as f32 * 16.0), *l, helv, PdfPoints::new(12.0)).unwrap();
        }
        objs.create_text_object(PdfPoints::new(72.0), PdfPoints::new(600.0), "A second text in Times font, smaller.", times, PdfPoints::new(11.0)).unwrap();
        // gradient image
        let mut img = image::RgbaImage::new(160, 100);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 255 / 160) as u8, (y * 255 / 100) as u8, 180, 255]);
        }
        objs.create_image_object(PdfPoints::new(72.0), PdfPoints::new(460.0), &image::DynamicImage::ImageRgba8(img), Some(PdfPoints::new(200.0)), Some(PdfPoints::new(125.0))).unwrap();
        let mut r = PathData::rect([300.0, 460.0, 400.0, 520.0]);
        r.fill = Some([255, 200, 60, 255]);
        r.stroke = Some([160, 90, 0, 255]);
        r.width = 2.0;
        build_path(&doc, &mut objs, &r).unwrap();
        let mut e = PathData::ellipse([420.0, 460.0, 520.0, 520.0]);
        e.fill = Some([120, 200, 140, 255]);
        e.stroke = Some([20, 100, 50, 255]);
        e.width = 2.0;
        build_path(&doc, &mut objs, &e).unwrap();
        objs.create_text_object(PdfPoints::new(72.0), PdfPoints::new(380.0), "Search for the word \"paragraph\" with Ctrl+F, or double-click here to edit.", helv, PdfPoints::new(12.0)).unwrap();
    }
    page.regenerate_content().unwrap();
    drop(page);
    // ------------------------------------------------------------ page 2: table of contents
    let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
    {
        let objs = page.objects_mut();
        let mut t = objs.create_text_object(PdfPoints::new(72.0), PdfPoints::new(760.0), "Contents", bold, PdfPoints::new(22.0)).unwrap();
        t.set_fill_color(PdfColor::new(200, 60, 20, 255)).unwrap();
        for (i, (title, pg)) in [("INTRODUCTION", 3), ("BASIC KNOWLEDGE", 14), ("EQUIPMENT FOR TRAINING", 27), ("SETS AND REPETITIONS", 28)].iter().enumerate() {
            let y = 700.0 - i as f32 * 24.0;
            // one object per word, as in a PDF exported from Word (one space gap)
            let mut x = 72.0;
            for word in title.split(' ') {
                let mut o = objs.create_text_object(PdfPoints::new(x), PdfPoints::new(y), word, helv, PdfPoints::new(13.0)).unwrap();
                let w = o.bounds().unwrap().to_rect().width().value;
                let _ = &mut o;
                x += w + 4.0;
            }
            objs.create_text_object(PdfPoints::new(300.0), PdfPoints::new(y), "..............................", helv, PdfPoints::new(13.0)).unwrap();
            objs.create_text_object(PdfPoints::new(500.0), PdfPoints::new(y), &pg.to_string(), helv, PdfPoints::new(13.0)).unwrap();
        }
    }
    page.regenerate_content().unwrap();
    drop(page);
    // ------------------------------------------------------------ page 3: empty with a title
    let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
    page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(760.0), "Page 3", bold, PdfPoints::new(18.0)).unwrap();
    page.regenerate_content().unwrap();
    drop(page);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes `samples/demo.pdf` (to try the application by hand).
    #[test]
    fn write_demo_pdf() {
        let doc = build_demo(crate::pdfx::test_pdfium());
        std::fs::create_dir_all("samples").unwrap();
        doc.save_to_file("samples/demo.pdf").unwrap();
        assert_eq!(doc.pages().len(), 3);
    }

    /// Writes `samples/demo_protected.pdf` (user password: `demo`) to try protected view with a password.
    #[test]
    fn write_protected_demo_pdf() {
        let doc = build_demo(crate::pdfx::test_pdfium());
        let bytes = doc.save_to_bytes().unwrap();
        let sec = crate::pdfedit_doc::Security { user_password: "demo".into(), owner_password: "demo".into(), ..Default::default() };
        std::fs::write("samples/demo_protected.pdf", crate::pdfedit_doc::encrypt(&bytes, &sec).unwrap()).unwrap();
    }

    #[test]
    fn demo_groups_into_expected_blocks() {
        let p = crate::pdfx::test_pdfium();
        let mut t = crate::tab::Tab::new(p, build_demo(p), None, None, 1.0);
        let info = t.info(0).unwrap();
        let para = info.objects.iter().find(|o| o.block.as_ref().map(|b| b.lines.len() == LOREM.len()).unwrap_or(false));
        assert!(para.is_some(), "the {}-line paragraph is detected as a single block", LOREM.len());
        let info2 = t.info(1).unwrap();
        let rows = info2.objects.iter().filter(|o| o.text().starts_with("BASIC KNOWLEDGE") || o.text().starts_with("INTRODUCTION")).count();
        assert_eq!(rows, 2, "each line of the table of contents stays separate");
    }
}
