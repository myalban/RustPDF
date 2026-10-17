//! Advanced commands and events (annotations, forms, pages, security, signatures…).

use crate::app::{App, SideTab};
use crate::dlg_common::*;
use crate::edit_text::TextStyle;
use crate::fonts::FontFamily;
use crate::i18n::t;
use crate::pdfedit_doc::CleanOpts;
use crate::tools::{Cmd, Tool};
use crate::view::Msg;
use eframe::egui;

impl App {
    pub fn run_cmd_ext(&mut self, cmd: Cmd, _ctx: &egui::Context, mem: u32) {
        let cur = self.tab_ref().map(|t| t.view.page).unwrap_or(0);
        let n = self.tab_ref().map(|t| t.page_count()).unwrap_or(0);
        if self.tab_ref().is_none() && !matches!(cmd, Cmd::CheckUpdates) {
            return;
        }
        match cmd {
            Cmd::RotatePage => {
                let p = self.selected_pages();
                self.edit(false, |tb, mem| tb.rotate_page_list(&p, 90, mem));
                self.page_sel.clear();
            }
            Cmd::DeletePage => {
                let p = self.selected_pages();
                if p.len() >= n {
                    self.err(t("err_last_page"));
                } else {
                    self.edit(true, |tb, mem| tb.delete_page_list(&p, mem));
                    self.page_sel.clear();
                }
            }
            Cmd::MovePageUp | Cmd::MovePageDown => {
                let p = self.selected_pages();
                let first = *p.first().unwrap_or(&cur);
                let last = *p.last().unwrap_or(&cur);
                let to = if cmd == Cmd::MovePageUp { first.saturating_sub(1) } else { (last + 2).min(n) };
                self.edit(true, |tb, mem| tb.move_pages_to(&p, to, mem));
                let new_first = if cmd == Cmd::MovePageUp { first.saturating_sub(1) } else { (first + 1).min(n.saturating_sub(p.len())) };
                self.page_sel = (new_first..new_first + p.len()).collect();
            }
            Cmd::InsertBlank => self.dlg_insert_blank(),
            Cmd::InsertFromFile => self.dlg_insert_file(),
            Cmd::ExtractPage => self.dlg_extract(),
            Cmd::SplitDoc => self.dlg_split(),
            Cmd::MergeFiles => self.dlg_merge(),
            Cmd::CropPages => self.dlg_crop(),
            Cmd::PageSize => self.dlg_page_size(),
            Cmd::PageNumbers => self.dlg_stamp(StampMode::Numbers),
            Cmd::HeaderFooter => self.dlg_stamp(StampMode::HeaderFooter),
            Cmd::Watermark => self.dlg_watermark(),
            Cmd::RotatePages => self.dlg_pages_action(false),
            Cmd::DeletePages => self.dlg_pages_action(true),
            Cmd::Deskew => {
                let p: Vec<usize> = if self.page_sel.is_empty() { (0..n).collect() } else { self.selected_pages() };
                let r = self.tab().map(|tb| tb.deskew_pages(&p, mem));
                match r {
                    Some(Ok((k, avg))) => self.say(format!("{k} {} ({avg:.2}°)", t("pages_deskewed"))),
                    Some(Err(e)) => self.err(e),
                    None => {}
                }
            }
            Cmd::Properties => {
                let info = self.tab_ref().map(|t| t.doc_info()).unwrap_or_default();
                self.open_dlg2(Dlg2::Info(InfoD { info }));
            }
            Cmd::Security => {
                let existing = self.tab_ref().and_then(|tb| tb.doc.save_to_bytes().ok()).and_then(|b| crate::pdfedit_doc::read_security(&b));
                let sec = self.tab_ref().and_then(|t| t.security.clone()).unwrap_or_default();
                self.open_dlg2(Dlg2::Security(SecD { sec, confirm: String::new(), existing }));
            }
            Cmd::CleanMetadata => self.open_dlg2(Dlg2::Clean(CleanD { opts: CleanOpts::default(), report: None })),
            Cmd::Attachments => {
                self.show_side = true;
                self.side_tab = SideTab::Attachments;
            }
            Cmd::Bookmarks => {
                self.show_side = true;
                self.side_tab = SideTab::Bookmarks;
            }
            Cmd::Sign => self.dlg_sign(),
            Cmd::VerifySignatures => {
                let r = self.tab_ref().and_then(|tb| tb.doc.save_to_bytes().ok()).map(|b| crate::sign::verify_pdf(&b));
                self.open_dlg2(Dlg2::Verify(r.unwrap_or_default()));
            }
            Cmd::FlattenForm => {
                let r = self.tab().map(|tb| tb.flatten_form(mem));
                match r {
                    Some(Ok(k)) => self.say(format!("{k} {}", t("fields_flattened"))),
                    Some(Err(e)) => self.err(e),
                    None => {}
                }
                self.text_ui.key = None;
            }
            Cmd::ApplyRedactions => {
                let r = self.tab().map(|tb| tb.apply_redactions(mem));
                match r {
                    Some(Ok(k)) => self.say(format!("{k} {}", t("redactions_applied"))),
                    Some(Err(e)) => self.err(e),
                    None => {}
                }
            }
            Cmd::Export => {
                let dpi = self.prefs.export_dpi;
                self.open_dlg2(Dlg2::Export(ExportD { kind: 0, pages: format!("1-{n}"), dpi, png: true }));
            }
            Cmd::CheckUpdates => self.check_updates(),
            Cmd::RepairDoc => {
                let bytes = self.tab_ref().and_then(|tb| tb.doc.save_to_bytes().ok());
                match bytes.and_then(|b| crate::pdfx::repair_bytes(&b)) {
                    Some(fixed) => match crate::pdfx::open_bytes(self.pdfium, fixed, None) {
                        Ok(doc) => {
                            let (zoom, fit) = self.initial_zoom();
                            let mut tab = crate::tab::Tab::new(self.pdfium, doc, None, None, zoom);
                            tab.view.fit = fit;
                            tab.title = t("repaired_doc").to_string();
                            tab.dirty = true;
                            self.tabs.push(tab);
                            self.active = self.tabs.len() - 1;
                            self.say(t("repaired_ok"));
                        }
                        Err(_) => self.err(t("err_repair")),
                    },
                    None => self.err(t("err_repair")),
                }
            }
            other => self.todo(t(other.label_key())),
        }
    }

    pub fn handle_msg_ext(&mut self, m: Msg, _ctx: &egui::Context) {
        let mem = self.mem();
        let author = self.prefs.author.clone();
        match m {
            Msg::Markup { page, tool, rects } => {
                let Some(kind) = crate::annots::markup_kind(tool) else { return };
                let (c, a) = (self.style.markup_color, if tool == Tool::Highlight { self.style.markup_opacity } else { 255 });
                self.edit(false, |tb, mem| tb.add_markup(page, kind, &rects, c, a, &author, mem));
                if let Some(tb) = self.tab() {
                    tb.text_sel = None;
                }
            }
            Msg::NoteAt { page, x, y } => {
                let text = self.style.note_text.clone();
                self.edit(false, |tb, mem| tb.add_note(page, x, y, &text, &author, mem));
                // select the new note to type its text
                if let Some(tb) = self.tab() {
                    if let Some(info) = tb.info(page) {
                        if let Some(a) = info.annots.last() {
                            tb.sel.clear();
                            tb.sel.page = page;
                            tb.sel.annot = Some(a.idx);
                        }
                    }
                }
                self.set_tool(Tool::Select);
            }
            Msg::StampAt { page, x, y } => {
                let (txt, col) = (self.style.stamp_text.clone(), self.style.stamp_color);
                self.edit(true, |tb, mem| tb.add_stamp(page, x, y, &txt, col, mem).map(|_| ()));
            }
            Msg::RedactZone { page, rect } => {
                if let Some(tb) = self.tab() {
                    tb.redactions.push((page, rect));
                }
            }
            Msg::LinkZone { page, rect } => {
                self.open_dlg2(Dlg2::Link(LinkD { page, rect, use_url: false, url: "https://".into(), target_page: 1 }));
            }
            Msg::FormZone { tool, page, rect } => {
                let base = match tool {
                    Tool::FormCheck => "case",
                    Tool::FormRadio => "radio",
                    Tool::FormCombo => "list",
                    _ => "field",
                };
                let names = self.tab_ref().map(|t| t.field_names()).unwrap_or_default();
                let mut i = 1;
                while names.iter().any(|n| *n == format!("{base}{i}")) {
                    i += 1;
                }
                self.open_dlg2(Dlg2::NewField(NewFieldD { tool, page, rect, name: format!("{base}{i}"), options: "Option 1\nOption 2\nOption 3".into(), value: String::new(), read_only: false, required: false, multiline: false }));
            }
            Msg::SignAt { page, x, y } => self.place_signature(page, x, y),
            Msg::ListPopup { page, field } => {
                let opts = self.tab().and_then(|tb| tb.info(page)).and_then(|i| i.fields.get(field).map(|f| f.options.clone()));
                if let Some(options) = opts {
                    self.open_dlg2(Dlg2::Choice(ChoiceD { page, field, options }));
                }
            }
            Msg::ChooseOption { page, field, option } => self.edit(false, |tb, mem| tb.choose_option(page, field, option, mem)),
            Msg::ToggleField { page, field } => self.edit(false, |tb, mem| tb.toggle_field(page, field, mem)),
            Msg::CommitFieldText { page, field, text } => self.edit(false, |tb, mem| tb.set_field_text(page, field, &text, mem)),
            Msg::CommitNoteText { page, annot, text } => self.edit(false, |tb, mem| tb.annot_set_contents(page, annot, &text, mem)),
            Msg::MoveSelection { .. } | Msg::SelectionChanged => {}
            _ => {}
        }
        let _ = mem;
    }

    /// Deletes the selected annotation.
    pub fn delete_selected_annot(&mut self, mem: u32) {
        let Some(tb) = self.tab() else { return };
        let (page, ann) = (tb.sel.page, tb.sel.annot);
        let Some(ai) = ann else { return };
        let r = tb.annot_delete(page, ai, mem);
        tb.sel.clear();
        if let Err(e) = r {
            self.err(e);
        }
    }

    /// Places the chosen handwritten signature where the user clicked.
    fn place_signature(&mut self, page: usize, x: f32, y: f32) {
        let Some(payload) = self.sig_payload.clone() else {
            self.open_signature_dialog();
            return;
        };
        let color = [10, 20, 120];
        let mut new = None;
        match payload {
            SigPayload::Strokes(s) => {
                let w = 150.0f32;
                let h = (w / s.aspect.max(0.2)).min(120.0);
                let rect = [x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0];
                let d = s.to_path(rect, color, 1.6);
                self.edit(true, |tb, mem| tb.add_path(page, &d, "Signature", mem).map(|i| new = Some(i)));
            }
            SigPayload::Typed { text, font } => {
                let st = TextStyle { family: FontFamily::System(font), size: 30.0, color, ..TextStyle::default() };
                self.edit(true, |tb, mem| tb.add_text(page, x - 60.0, y, &text, &st, mem).map(|i| new = Some(i)));
            }
            SigPayload::Image(img) => {
                self.edit(true, |tb, mem| tb.add_image(page, x, y, &img, 140.0, mem).map(|i| new = Some(i)));
            }
        }
        if let (Some(i), Some(tb)) = (new, self.tab()) {
            tb.sel.page = page;
            tb.sel.items = vec![i];
        }
        self.set_tool(Tool::Select);
    }

    fn check_updates(&mut self) {
        // a newer release was already found by the automatic check
        if let Some((_, msg)) = self.update_found.clone() {
            self.open_dlg2(Dlg2::Updates(msg));
            return;
        }
        let url = crate::updater::effective_url(&self.prefs.update_url).to_string();
        let msg = crate::updater::describe(&crate::updater::check_release(&url));
        self.open_dlg2(Dlg2::Updates(msg));
    }
}
