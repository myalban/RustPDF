//! Left side panel: thumbnails, bookmarks, attachments, layers, comments, search results.

use crate::app::{App, SideTab};
use crate::i18n::t;
use crate::icons;
use crate::props::icon_button;
use crate::render::{self, Key};
use crate::tools::Cmd;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

const THUMB_W: f32 = 120.0;

impl App {
    pub fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, icon, key) in [
                (SideTab::Pages, icons::FILES, "side_pages"),
                (SideTab::Bookmarks, icons::BOOKMARK_SIMPLE, "side_bookmarks"),
                (SideTab::Attachments, icons::PAPERCLIP, "side_attachments"),
                (SideTab::Layers, icons::STACK, "side_layers"),
                (SideTab::Comments, icons::CHAT_TEXT, "side_comments"),
                (SideTab::Search, icons::MAGNIFYING_GLASS, "side_search"),
            ] {
                if icon_button(ui, icon, t(key), self.side_tab == tab).clicked() {
                    self.side_tab = tab;
                }
            }
        });
        ui.separator();
        match self.side_tab {
            SideTab::Pages => self.thumbs_panel(ui),
            SideTab::Search => self.search_results_panel(ui),
            SideTab::Bookmarks => self.bookmarks_panel(ui),
            SideTab::Attachments => self.attachments_panel(ui),
            SideTab::Layers => self.layers_panel(ui),
            SideTab::Comments => self.comments_panel(ui),
        }
    }

    fn thumbs_panel(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get(self.active) else {
            ui.weak(t("no_document"));
            return;
        };
        let n = tab.page_count();
        // quick actions on pages
        ui.horizontal_wrapped(|ui| {
            for c in [Cmd::RotatePage, Cmd::DeletePage, Cmd::MovePageUp, Cmd::MovePageDown, Cmd::InsertBlank, Cmd::ExtractPage] {
                if icon_button(ui, c.icon_or(icons::FILE), t(c.label_key()), false).clicked() {
                    self.pending_cmds.push(c);
                }
            }
        });
        ui.separator();
        let sizes: Vec<(f32, f32)> = tab.sizes.clone();
        let cur = tab.view.page;
        let tab_id = tab.id;
        let night = self.prefs.night_mode;
        let heights: Vec<f32> = sizes.iter().map(|(w, h)| THUMB_W * h / w.max(1.0) + 22.0).collect();
        let total: f32 = heights.iter().sum::<f32>() + 4.0 * n as f32;
        let ctx = ui.ctx().clone();
        let mut goto = None;
        let mut drop_to: Option<(usize, usize)> = None;
        let mut clicked_with_mod: Option<(usize, bool, bool)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
            ui.set_min_height(total);
            let origin = ui.min_rect().min;
            let mut y = 0.0;
            for i in 0..n {
                let h = heights[i];
                let top = y;
                y += h + 4.0;
                if top + h < viewport.min.y - 100.0 || top > viewport.max.y + 100.0 {
                    continue;
                }
                let tw = THUMB_W.min(ui.available_width() - 8.0).max(60.0);
                let th = (tw * sizes[i].1 / sizes[i].0.max(1.0)).max(10.0);
                let outer = Rect::from_min_size(origin + Vec2::new((ui.available_width() - tw) * 0.5, top), Vec2::new(tw, th + 20.0));
                let r = Rect::from_min_size(outer.min, Vec2::new(tw, th));
                let resp = ui.interact(outer, egui::Id::unique(("thumb", tab_id, i)), Sense::click_and_drag());
                let ver = self.tabs[self.active].page_ver.get(i).copied().unwrap_or(0);
                let key = Key::Thumb { tab: tab_id, page: i, ver };
                let mut tex = self.cache.get(&key);
                if tex.is_none() && self.cache.may_render() {
                    if let Ok(p) = self.tabs[self.active].read_page(i) {
                        if let Some(img) = crate::pdfx::render_full(&p, (tw * ctx.pixels_per_point()) as i32, true) {
                            let (tx, bytes) = render::upload(&ctx, format!("th{tab_id}_{i}"), &img, night);
                            self.cache.insert(key, tx.clone(), bytes);
                            tex = Some(tx);
                        }
                    }
                }
                ui.painter().rect_filled(r.translate(Vec2::new(2.0, 2.0)), 0.0, Color32::from_black_alpha(40));
                match tex {
                    Some(t) => {
                        ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.painter().rect_filled(r, 0.0, Color32::WHITE);
                        self.cache.pending = true;
                    }
                }
                let selected = self.page_sel.contains(&i);
                if i == cur || selected {
                    let col = if i == cur { Color32::from_rgb(40, 120, 255) } else { Color32::from_rgb(120, 170, 255) };
                    ui.painter().rect_stroke(r.expand(2.0), 0.0, Stroke::new(3.0, col), StrokeKind::Outside);
                }
                ui.painter().text(Pos2::new(r.center().x, r.max.y + 3.0), egui::Align2::CENTER_TOP, format!("{}", i + 1), egui::FontId::proportional(12.0), ui.visuals().text_color());
                if resp.clicked() {
                    let (ctrl, shift) = ui.input(|inp| (inp.modifiers.command, inp.modifiers.shift));
                    clicked_with_mod = Some((i, ctrl, shift));
                    goto = Some(i);
                }
                if resp.drag_started() {
                    self.thumb_drag = Some(i);
                }
                if resp.hovered() && ui.input(|inp| inp.pointer.any_released()) {
                    if let Some(from) = self.thumb_drag {
                        if from != i {
                            drop_to = Some((from, i));
                        }
                    }
                }
        // drop indicator
                if self.thumb_drag.is_some() && resp.hovered() && self.thumb_drag != Some(i) {
                    ui.painter().line_segment([r.left_top() - Vec2::new(2.0, 3.0), r.right_top() - Vec2::new(-2.0, 3.0)], Stroke::new(3.0, Color32::from_rgb(255, 120, 0)));
                }
                resp.context_menu(|ui| {
                    self.page_context_menu(ui, i);
                });
            }
            if ui.input(|inp| inp.pointer.any_released()) {
                self.thumb_drag = None;
            }
        });
        if let Some((i, ctrl, shift)) = clicked_with_mod {
            if ctrl {
                if let Some(p) = self.page_sel.iter().position(|&x| x == i) {
                    self.page_sel.remove(p);
                } else {
                    self.page_sel.push(i);
                }
            } else if shift && !self.page_sel.is_empty() {
                let a = *self.page_sel.last().unwrap();
                let (lo, hi) = (a.min(i), a.max(i));
                self.page_sel = (lo..=hi).collect();
            } else {
                self.page_sel = vec![i];
            }
        }
        if let Some(p) = goto {
            if let Some(tab) = self.tab() {
                tab.goto_page(p);
            }
        }
        if let Some((from, to)) = drop_to {
            self.pending_page_move = Some((from, to));
        }
    }

    pub fn page_context_menu(&mut self, ui: &mut egui::Ui, page: usize) {
        if !self.page_sel.contains(&page) {
            self.page_sel = vec![page];
        }
        for c in [Cmd::RotatePage, Cmd::DeletePage, Cmd::MovePageUp, Cmd::MovePageDown, Cmd::InsertBlank, Cmd::InsertFromFile, Cmd::ExtractPage, Cmd::SplitDoc, Cmd::CropPages, Cmd::Deskew] {
            if ui.button(format!("{} {}", c.icon(), t(c.label_key()))).clicked() {
                self.pending_cmds.push(c);
                ui.close();
            }
        }
    }

    fn search_results_panel(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get(self.active) else {
            ui.weak(t("no_document"));
            return;
        };
        if tab.search.matches.is_empty() {
            ui.weak(if tab.search.query.trim().is_empty() { t("search_hint") } else { t("no_results") });
            return;
        }
        ui.label(RichText::new(format!("{} {}", tab.search.matches.len(), t("results"))).strong());
        let hits: Vec<(usize, usize)> = tab.search.matches.iter().enumerate().map(|(i, h)| (i, h.page)).collect();
        let cur = tab.search.cur;
        let mut goto = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 22.0, hits.len(), |ui, range| {
            for k in range {
                let (i, page) = hits[k];
                let label = format!("{}  p.{}", i + 1, page + 1);
                if ui.selectable_label(cur == Some(i), label).clicked() {
                    goto = Some(i);
                }
            }
        });
        if let Some(i) = goto {
            if let Some(tab) = self.tab() {
                tab.goto_match(i);
            }
        }
    }
}

/// Command icon, or a fallback icon.
pub trait CmdIcon {
    fn icon_or(&self, fallback: &'static str) -> &'static str;
}

impl CmdIcon for Cmd {
    fn icon_or(&self, fallback: &'static str) -> &'static str {
        let i = self.icon();
        if i.is_empty() { fallback } else { i }
    }
}
