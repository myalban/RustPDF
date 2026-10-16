use image::{DynamicImage, Rgba, RgbaImage};
use pdfium_render::prelude::*;

fn pdfium() -> Pdfium {
    Pdfium::new(Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./")).unwrap())
}

fn solid(c: [u8; 4]) -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 40, Rgba(c)))
}

#[test]
fn edit_text_and_image_roundtrip() {
    let p = pdfium();
    let mut doc = p.create_new_pdf().unwrap();
    let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
    let font = doc.fonts_mut().helvetica();
    page.objects_mut()
        .create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), "Bonjour monde", font, PdfPoints::new(18.0))
        .unwrap();
    page.objects_mut()
        .create_image_object(PdfPoints::new(72.0), PdfPoints::new(500.0), &solid([255, 0, 0, 255]), Some(PdfPoints::new(100.0)), Some(PdfPoints::new(100.0)))
        .unwrap();
    page.regenerate_content().unwrap();
    drop(page);
    let bytes = doc.save_to_bytes().unwrap();
    drop(doc);

    // Réouverture + édition comme dans l'appli
    let mut doc = p.load_pdf_from_byte_vec(bytes, None).unwrap();
    let mut page = doc.pages().get(0).unwrap();
    {
        let mut objs = page.objects_mut();
        for i in 0..objs.len() {
            let mut o = objs.get(i).unwrap();
            if let Some(t) = o.as_text_object_mut() {
                assert_eq!(t.text(), "Bonjour monde");
                t.set_text("Hello world").unwrap();
            } else if let Some(im) = o.as_image_object_mut() {
                im.set_image(&solid([0, 0, 255, 255])).unwrap();
            }
        }
    }
    page.regenerate_content().unwrap();
    drop(page);
    let bytes = doc.save_to_bytes().unwrap();

    let doc = p.load_pdf_from_byte_vec(bytes, None).unwrap();
    let page = doc.pages().get(0).unwrap();
    let txt = page.text().unwrap().all();
    assert!(txt.contains("Hello world"), "texte: {txt:?}");
    let img = page.render_with_config(&PdfRenderConfig::new().set_target_width(595)).unwrap().as_image().unwrap().to_rgba8();
    // Zone de l'image: x 72..172, y(pdf) 500..600 -> pixels y = 842-600..842-500
    let px = img.get_pixel(120, 842 - 550);
    assert!(px[2] > 200 && px[0] < 60, "pixel attendu bleu, obtenu {px:?}");
}

#[test]
fn resize_image_and_text() {
    let p = pdfium();
    let mut doc = p.create_new_pdf().unwrap();
    let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
    let font = doc.fonts_mut().helvetica();
    page.objects_mut()
        .create_image_object(PdfPoints::new(100.0), PdfPoints::new(400.0), &solid([255, 0, 0, 255]), Some(PdfPoints::new(100.0)), Some(PdfPoints::new(100.0)))
        .unwrap();
    page.objects_mut()
        .create_text_object(PdfPoints::new(100.0), PdfPoints::new(300.0), "Taille", font, PdfPoints::new(20.0))
        .unwrap();
    page.regenerate_content().unwrap();
    let bounds = |page: &PdfPage, i| page.objects().get(i).unwrap().bounds().unwrap().to_rect();
    let (ib, tb) = (bounds(&page, 0), bounds(&page, 1));
    // même calcul que App::resize_selected : x2 sur l'image et le texte, ancre en bas à gauche
    for (i, b) in [(0, ib), (1, tb)] {
        let mut objs = page.objects_mut();
        let mut o = objs.get(i).unwrap();
        o.scale(2.0, 2.0).unwrap();
        o.translate(PdfPoints::new(b.left().value - b.left().value * 2.0), PdfPoints::new(b.bottom().value - b.bottom().value * 2.0)).unwrap();
    }
    page.regenerate_content().unwrap();
    let (ia, ta) = (bounds(&page, 0), bounds(&page, 1));
    assert!((ia.width().value - 2.0 * ib.width().value).abs() < 0.5, "image {ib:?} -> {ia:?}");
    assert!((ia.left().value - ib.left().value).abs() < 0.5 && (ia.bottom().value - ib.bottom().value).abs() < 0.5);
    assert!((ta.width().value - 2.0 * tb.width().value).abs() < 0.5, "texte {tb:?} -> {ta:?}");
}

fn form_pdf() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Annots [4 0 R 6 0 R] /Resources << /Font << /Helv 7 0 R >> >> >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (nom) /Rect [100 700 300 725] /F 4 /DA (/Helv 12 Tf 0 g) /P 3 0 R /MK << /BC [0] >> >>",
        "<< /Fields [4 0 R 6 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 7 0 R >> >> /NeedAppearances true >>",
        "<< /Type /Annot /Subtype /Widget /FT /Btn /T (ok) /Rect [100 650 120 670] /F 4 /V /Off /AS /Off /P 3 0 R /MK << /BC [0] /CA (4) >> /DA (/ZaDb 0 Tf 0 g) >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = vec![];
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let x = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend(format!("{:010} 00000 n \n", o).as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, x).as_bytes());
    out
}

#[test]
fn fill_form_fields() {
    std::fs::write("sample_form.pdf", form_pdf()).unwrap();
    let p = pdfium();
    let doc = p.load_pdf_from_byte_vec(form_pdf(), None).unwrap();
    let mut page = doc.pages().get(0).unwrap();
    let mut names = vec![];
    {
        let mut anns = page.annotations_mut();
        for i in 0..anns.len() {
            let mut a = anns.get(i).unwrap();
            if let Some(f) = a.as_form_field_mut() {
                names.push(f.name().unwrap_or_default());
                if let Some(t) = f.as_text_field_mut() {
                    t.set_value("Jean Dupont").unwrap();
                } else if let Some(c) = f.as_checkbox_field_mut() {
                    c.set_checked(true).unwrap();
                }
            }
        }
    }
    assert_eq!(names, vec!["nom", "ok"]);
    drop(page);
    let bytes = doc.save_to_bytes().unwrap();
    drop(doc);
    let doc = p.load_pdf_from_byte_vec(bytes, None).unwrap();
    let page = doc.pages().get(0).unwrap();
    let mut seen_text = None;
    let mut seen_check = None;
    for a in page.annotations().iter() {
        if let Some(f) = a.as_form_field() {
            if let Some(t) = f.as_text_field() { seen_text = t.value(); }
            if let Some(c) = f.as_checkbox_field() { seen_check = c.is_checked().ok(); }
        }
    }
    assert_eq!(seen_text.as_deref(), Some("Jean Dupont"));
    assert_eq!(seen_check, Some(true));
}
