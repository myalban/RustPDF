//! Operations on the PDF file at the object level (via `lopdf`): merging content streams, etc.

use lopdf::{Dictionary, Document, Object, Stream};

/// Merges all the content streams of a page into a single one (essential to reorder objects,
/// because PDFium rewrites each object in the stream it came from).
pub fn merge_page_streams(bytes: &[u8], page_index: usize) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| format!("lopdf read: {e}"))?;
    let pid = *doc.get_pages().get(&((page_index + 1) as u32)).ok_or("page not found")?;
    let content = doc.get_page_content(pid);
    let new_id = doc.add_object(Stream::new(Dictionary::new(), content));
    let page = doc.get_object_mut(pid).and_then(|o| o.as_dict_mut()).map_err(|e| format!("page: {e}"))?;
    page.set("Contents", Object::Reference(new_id));
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| format!("write: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_streams_keeps_page_count_and_single_stream() {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        doc.pages_mut().create_page_at_end(pdfium_render::prelude::PdfPagePaperSize::a4()).unwrap();
        let bytes = doc.save_to_bytes().unwrap();
        let merged = merge_page_streams(&bytes, 0).unwrap();
        let ld = Document::load_mem(&merged).unwrap();
        let pid = *ld.get_pages().values().next().unwrap();
        assert_eq!(ld.get_page_contents(pid).len(), 1);
    }
}
