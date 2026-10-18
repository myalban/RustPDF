//! Panels and helpers still in progress (replaced batch by batch).

use crate::app::App;
use crate::i18n::t;

impl App {
    pub fn edit_pub<F>(&mut self, clear_sel: bool, f: F)
    where
        F: FnOnce(&mut crate::tab::Tab, u32) -> Result<(), String>,
    {
        self.edit(clear_sel, f)
    }


    pub fn export_svg_dialog(&mut self, page: usize, items: &[usize]) {
        let r = self.tab().map(|tb| tb.export_svg(page, items));
        match r {
            Some(Ok(svg)) => {
                if let Some(mut p) = rfd::FileDialog::new().set_file_name("element.svg").add_filter("SVG", &["svg"]).save_file() {
                    if p.extension().is_none() {
                        p.set_extension("svg");
                    }
                    match std::fs::write(&p, svg) {
                        Ok(_) => self.say(format!("{}: {}", t("exported"), p.display())),
                        Err(e) => self.err(e.to_string()),
                    }
                }
            }
            Some(Err(e)) => self.err(e),
            None => {}
        }
    }

}

impl App {
    /// Reopens a document from its autosave (unsaved state).
    pub fn restore_autosave(&mut self, orig: &std::path::Path, auto: &std::path::Path) {
        match crate::pdfx::open_path(self.pdfium, auto, None) {
            Ok(doc) => {
                let (zoom, fit) = self.initial_zoom();
                let path = if orig.exists() { Some(orig.to_path_buf()) } else { None };
                let mut tab = crate::tab::Tab::new(self.pdfium, doc, path, None, zoom);
                tab.view.fit = fit;
                tab.dirty = true;
                tab.title = format!("{} ({})", orig.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), t("recovered"));
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                crate::autosave::remove(auto);
                self.say(t("recovered_done"));
            }
            Err(e) => self.err(format!("{}: {e:?}", t("err_open"))),
        }
    }
}
