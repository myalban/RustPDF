//! Side panels: bookmarks, attachments, layers, comments; annotation properties.

use crate::app::App;
use crate::i18n::t;
use crate::model::{AnnotKind, PageInfo};
use crate::pdfedit_annot::AnnotRec;
use crate::pdfedit_doc::{Bm, Layer};
use eframe::egui::{self, Color32, RichText};

type Key = (u64, u64);

/// Data read from the file, cached as long as the document has not changed.
#[derive(Default)]
pub struct PanelCache {
    bookmarks: Option<(Key, Vec<Bm>)>,
    layers: Option<(Key, Vec<Layer>)>,
    attachments: Option<(Key, Vec<(String, usize)>)>,
    comments: Option<(Key, Vec<AnnotRec>)>,
    pub bm_rename: Option<(Vec<usize>, String)>,
    pub filter: String,
    pub reply_to: Option<(usize, usize, String)>,
    pub edit_note: Option<(usize, usize, String)>,
    pub only_page: bool,
}

enum BmAct {
    Goto(usize),
    Rename(Vec<usize>, String),
    Delete(Vec<usize>),
    AddChild(Vec<usize>),
    StartRename(Vec<usize>, String),
}

fn node_mut<'a>(nodes: &'a mut Vec<Bm>, path: &[usize]) -> Option<&'a mut Bm> {
    let (first, rest) = path.split_first()?;
    let n = nodes.get_mut(*first)?;
    if rest.is_empty() { Some(n) } else { node_mut(&mut n.children, rest) }
}

fn bm_tree(ui: &mut egui::Ui, nodes: &[Bm], path: &mut Vec<usize>, acts: &mut Vec<BmAct>, rename: &mut Option<(Vec<usize>, String)>) {
    for (i, n) in nodes.iter().enumerate() {
        path.push(i);
        let renaming = rename.as_ref().map(|(p, _)| p == path).unwrap_or(false);
        let row = |ui: &mut egui::Ui, acts: &mut Vec<BmAct>, rename: &mut Option<(Vec<usize>, String)>, path: &Vec<usize>| {
            if renaming {
                if let Some((_, text)) = rename {
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                    r.request_focus();
                    if r.lost_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            *rename = None;
                        } else {
                            acts.push(BmAct::Rename(path.clone(), text.clone()));
                        }
                    }
                }
            } else {
                if ui.link(if n.title.is_empty() { "—" } else { &n.title }).clicked() {
                    acts.push(BmAct::Goto(n.page));
                }
                ui.weak(format!("p.{}", n.page + 1));
                if ui.small_button("✎").on_hover_text(t("rename")).clicked() {
                    acts.push(BmAct::StartRename(path.clone(), n.title.clone()));
                }
                if ui.small_button("＋").on_hover_text(t("bm_add_child")).clicked() {
                    acts.push(BmAct::AddChild(path.clone()));
                }
                if ui.small_button("🗑").on_hover_text(t("delete")).clicked() {
                    acts.push(BmAct::Delete(path.clone()));
                }
            }
        };
        if n.children.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                row(ui, acts, rename, path);
            });
        } else {
            let id = ui.make_persistent_id(("bm", path.clone()));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| row(ui, acts, rename, path))
                .body(|ui| bm_tree(ui, &n.children, path, acts, rename));
        }
        path.pop();
    }
}

impl App {
    fn key(&self) -> Option<Key> {
        self.tab_ref().map(|t| (t.id, t.doc_ver))
    }

    pub fn bookmarks_panel(&mut self, ui: &mut egui::Ui) {
        let Some(key) = self.key() else { return };
        if self.pc.bookmarks.as_ref().map(|c| c.0 != key).unwrap_or(true) {
            let v = self.tab_ref().map(|t| t.bookmarks()).unwrap_or_default();
            self.pc.bookmarks = Some((key, v));
        }
        let nodes = self.pc.bookmarks.as_ref().map(|c| c.1.clone()).unwrap_or_default();
        let cur = self.tab_ref().map(|t| t.view.page).unwrap_or(0);
        let mut acts = vec![];
        ui.horizontal(|ui| {
            if ui.button(format!("＋ {}", t("bm_add"))).clicked() {
                acts.push(BmAct::AddChild(vec![]));
            }
        });
        ui.separator();
        if nodes.is_empty() {
            ui.weak(t("bm_none"));
        }
        let mut rename = self.pc.bm_rename.take();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            bm_tree(ui, &nodes, &mut vec![], &mut acts, &mut rename);
        });
        self.pc.bm_rename = rename;
        let mut work = nodes;
        let mut changed = false;
        for a in acts {
            match a {
                BmAct::Goto(p) => {
                    if let Some(tb) = self.tab() {
                        tb.goto_page(p);
                    }
                }
                BmAct::StartRename(p, s) => self.pc.bm_rename = Some((p, s)),
                BmAct::Rename(p, s) => {
                    if let Some(n) = node_mut(&mut work, &p) {
                        n.title = s;
                        changed = true;
                    }
                    self.pc.bm_rename = None;
                }
                BmAct::Delete(p) => {
                    if let Some((last, parent)) = p.split_last() {
                        let list = if parent.is_empty() { Some(&mut work) } else { node_mut(&mut work, parent).map(|n| &mut n.children) };
                        if let Some(l) = list {
                            if *last < l.len() {
                                l.remove(*last);
                                changed = true;
                            }
                        }
                    }
                }
                BmAct::AddChild(p) => {
                    let nb = Bm { title: format!("{} {}", t("page"), cur + 1), page: cur, children: vec![] };
                    let list = if p.is_empty() { Some(&mut work) } else { node_mut(&mut work, &p).map(|n| &mut n.children) };
                    if let Some(l) = list {
                        l.push(nb);
                        changed = true;
                        let mut np = p.clone();
                        np.push(l.len() - 1);
                        let title = l.last().map(|b| b.title.clone()).unwrap_or_default();
                        self.pc.bm_rename = Some((np, title));
                    }
                }
            }
        }
        if changed {
            let w = work;
            self.edit(false, |tb, mem| tb.set_bookmarks(&w, mem));
        }
    }

    pub fn attachments_panel(&mut self, ui: &mut egui::Ui) {
        let Some(key) = self.key() else { return };
        if self.pc.attachments.as_ref().map(|c| c.0 != key).unwrap_or(true) {
            let v = self.tab_ref().map(|t| t.attachments()).unwrap_or_default();
            self.pc.attachments = Some((key, v));
        }
        let list = self.pc.attachments.as_ref().map(|c| c.1.clone()).unwrap_or_default();
        if ui.button(format!("＋ {}", t("att_add"))).clicked() {
            if let Some(f) = rfd::FileDialog::new().pick_file() {
                match std::fs::read(&f) {
                    Ok(data) => {
                        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        self.edit(false, |tb, mem| tb.add_attachment(&name, &data, mem));
                    }
                    Err(e) => self.err(e.to_string()),
                }
            }
        }
        ui.separator();
        if list.is_empty() {
            ui.weak(t("att_none"));
        }
        let (mut save, mut del) = (None, None);
        for (i, (name, size)) in list.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(name);
                ui.weak(format!("{} Ko", size / 1024 + 1));
                if ui.small_button("💾").on_hover_text(t("att_save")).clicked() {
                    save = Some(i);
                }
                if ui.small_button("🗑").on_hover_text(t("delete")).clicked() {
                    del = Some(i);
                }
            });
        }
        if let Some(i) = save {
            let r = self.tab_ref().map(|tb| tb.attachment_bytes(i));
            match r {
                Some(Ok(b)) => {
                    if let Some(p) = rfd::FileDialog::new().set_file_name(list[i].0.clone()).save_file() {
                        match std::fs::write(&p, b) {
                            Ok(_) => self.say(format!("{}: {}", t("exported"), p.display())),
                            Err(e) => self.err(e.to_string()),
                        }
                    }
                }
                Some(Err(e)) => self.err(e),
                None => {}
            }
        }
        if let Some(i) = del {
            self.edit(false, |tb, mem| tb.delete_attachment(i, mem));
        }
    }

    pub fn layers_panel(&mut self, ui: &mut egui::Ui) {
        let Some(key) = self.key() else { return };
        if self.pc.layers.as_ref().map(|c| c.0 != key).unwrap_or(true) {
            let v = self.tab_ref().map(|t| t.layers()).unwrap_or_default();
            self.pc.layers = Some((key, v));
        }
        let list = self.pc.layers.as_ref().map(|c| c.1.clone()).unwrap_or_default();
        if list.is_empty() {
            ui.weak(t("layers_none"));
            return;
        }
        let mut toggle = None;
        for l in &list {
            let mut v = l.visible;
            if ui.checkbox(&mut v, if l.name.is_empty() { "—" } else { &l.name }).changed() {
                toggle = Some((l.id, v));
            }
        }
        if let Some((id, v)) = toggle {
            self.edit(false, |tb, mem| tb.set_layer(id, v, mem));
            if let Some(id) = self.tab_ref().map(|t| t.id) { self.cache.purge_tab(id); }
        }
    }

    pub fn comments_panel(&mut self, ui: &mut egui::Ui) {
        let Some(key) = self.key() else { return };
        if self.pc.comments.as_ref().map(|c| c.0 != key).unwrap_or(true) {
            let v = self.tab_ref().and_then(|t| t.list_comments().ok()).unwrap_or_default();
            self.pc.comments = Some((key, v));
        }
        let all = self.pc.comments.as_ref().map(|c| c.1.clone()).unwrap_or_default();
        let cur = self.tab_ref().map(|t| t.view.page).unwrap_or(0);
        let author = self.prefs.author.clone();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.pc.filter).hint_text(t("comments_filter")).desired_width(150.0));
            ui.checkbox(&mut self.pc.only_page, t("comments_this_page"));
        });
        ui.separator();
        let q = self.pc.filter.to_lowercase();
        let roots: Vec<&AnnotRec> = all
            .iter()
            .filter(|a| a.irt.is_none())
            .filter(|a| !self.pc.only_page || a.page == cur)
            .filter(|a| q.is_empty() || a.contents.to_lowercase().contains(&q) || a.author.to_lowercase().contains(&q))
            .collect();
        if roots.is_empty() {
            ui.weak(t("comments_none"));
        }
        enum Act {
            Goto(usize, [f32; 4]),
            Delete(usize, usize),
            Reply(usize, usize, String),
            Save(usize, usize, String),
        }
        let mut act = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for a in roots {
                let sel_key = (a.page, a.index);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 5.0, Color32::from_rgb(a.color[0], a.color[1], a.color[2]));
                        if ui.link(RichText::new(format!("{} · p.{}", subtype_label(&a.subtype), a.page + 1)).strong()).clicked() {
                            act = Some(Act::Goto(a.page, a.rect));
                        }
                        if !a.author.is_empty() {
                            ui.weak(&a.author);
                        }
                    });
                    let editing = self.pc.edit_note.as_ref().map(|e| (e.0, e.1) == sel_key).unwrap_or(false);
                    if editing {
                        if let Some((_, _, text)) = &mut self.pc.edit_note {
                            ui.add(egui::TextEdit::multiline(text).desired_rows(2).desired_width(f32::INFINITY));
                            ui.horizontal(|ui| {
                                if ui.small_button(t("ok")).clicked() {
                                    act = Some(Act::Save(sel_key.0, sel_key.1, text.clone()));
                                }
                                if ui.small_button(t("cancel")).clicked() {
                                    act = Some(Act::Save(usize::MAX, 0, String::new()));
                                }
                            });
                        }
                    } else {
                        if a.contents.is_empty() {
                            ui.weak(t("comments_empty"));
                        } else {
                            ui.label(&a.contents);
                        }
                        // replies
                        for r in all.iter().filter(|r| r.irt.is_some() && r.irt == a.id) {
                            ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                ui.vertical(|ui| {
                                    ui.weak(format!("↳ {}", r.author));
                                    ui.label(&r.contents);
                                });
                            });
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button(t("reply")).clicked() {
                                self.pc.reply_to = Some((a.page, a.index, String::new()));
                            }
                            if ui.small_button(t("edit")).clicked() {
                                self.pc.edit_note = Some((a.page, a.index, a.contents.clone()));
                            }
                            if ui.small_button("🗑").on_hover_text(t("delete")).clicked() {
                                act = Some(Act::Delete(a.page, a.index));
                            }
                        });
                    }
                    if let Some((p, i, text)) = &mut self.pc.reply_to {
                        if (*p, *i) == sel_key {
                            ui.add(egui::TextEdit::multiline(text).desired_rows(2).hint_text(t("reply_hint")).desired_width(f32::INFINITY));
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!text.trim().is_empty(), egui::Button::new(t("send"))).clicked() {
                                    act = Some(Act::Reply(*p, *i, text.clone()));
                                }
                                if ui.small_button(t("cancel")).clicked() {
                                    act = Some(Act::Reply(usize::MAX, 0, String::new()));
                                }
                            });
                        }
                    }
                });
            }
        });
        let mem = self.mem();
        match act {
            Some(Act::Goto(p, rect)) => {
                if let Some(tb) = self.tab() {
                    tb.goto_page(p);
                    let _ = rect;
                }
            }
            Some(Act::Delete(p, i)) => self.edit(true, |tb, _| tb.delete_comment_by_file_index(p, i, mem)),
            Some(Act::Save(p, i, text)) => {
                self.pc.edit_note = None;
                if p != usize::MAX {
                    self.edit(false, |tb, _| tb.set_comment_by_file_index(p, i, &text, mem));
                }
            }
            Some(Act::Reply(p, i, text)) => {
                self.pc.reply_to = None;
                if p != usize::MAX {
                    self.edit(false, |tb, _| tb.reply_to(p, i, &text, &author, mem));
                }
            }
            None => {}
        }
    }

    /// Properties of the selected annotation (comment, color, deletion).
    pub fn annot_props(&mut self, ui: &mut egui::Ui, page: usize, ai: usize, info: &PageInfo) {
        let Some(a) = info.annots.iter().find(|a| a.idx == ai).cloned() else { return };
        ui.label(RichText::new(match a.kind {
            AnnotKind::Highlight => t("annot_highlight"),
            AnnotKind::Underline => t("annot_underline"),
            AnnotKind::StrikeOut => t("annot_strike"),
            AnnotKind::Squiggly => t("annot_squiggly"),
            AnnotKind::Note => t("annot_note"),
            _ => t("annotation"),
        }).strong());
        if !a.author.is_empty() {
            ui.weak(&a.author);
        }
        // comment text box
        let key = (page, ai);
        let buf = match &mut self.pc.edit_note {
            Some((p, i, s)) if (*p, *i) == key => s,
            _ => {
                self.pc.edit_note = Some((key.0, key.1, a.contents.clone()));
                &mut self.pc.edit_note.as_mut().unwrap().2
            }
        };
        let r = ui.add(egui::TextEdit::multiline(buf).desired_rows(4).desired_width(f32::INFINITY).hint_text(t("comment_hint")));
        let commit = r.lost_focus() && *buf != a.contents;
        let text = buf.clone();
        if commit {
            self.edit(false, |tb, mem| tb.annot_set_contents(page, ai, &text, mem));
            self.pc.edit_note = None;
        }
        if matches!(a.kind, AnnotKind::Highlight | AnnotKind::Underline | AnnotKind::StrikeOut | AnnotKind::Squiggly | AnnotKind::Note) {
            let mut c = Color32::from_rgb(a.color[0], a.color[1], a.color[2]);
            ui.horizontal(|ui| {
                ui.label(t("color"));
                if ui.color_edit_button_srgba(&mut c).changed() {
                    let alpha = if a.kind == AnnotKind::Highlight { self.style.markup_opacity } else { 255 };
                    let col = [c.r(), c.g(), c.b()];
                    self.edit(false, |tb, mem| tb.annot_set_color(page, ai, col, alpha, mem));
                }
            });
        }
        if ui.button(format!("🗑 {}", t("delete"))).clicked() {
            let mem = self.mem();
            self.delete_selected_annot(mem);
            self.pc.edit_note = None;
        }
    }

    pub fn move_page_to(&mut self, from: usize, to: usize) {
        self.edit(true, |tb, mem| tb.move_pages_to(&[from], to, mem));
    }
}

fn subtype_label(s: &str) -> &'static str {
    match s {
        "Highlight" => t("annot_highlight"),
        "Underline" => t("annot_underline"),
        "StrikeOut" => t("annot_strike"),
        "Squiggly" => t("annot_squiggly"),
        "Text" => t("annot_note"),
        "FreeText" => t("annot_freetext"),
        "Ink" => t("annot_ink"),
        "Stamp" => t("annot_stamp"),
        _ => t("annotation"),
    }
}
