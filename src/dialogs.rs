//! Modal dialog boxes.

use crate::app::App;
use crate::i18n::{self, t};
use crate::logx;
use crate::pdfx;
use crate::prefs::{Lang, ThemePref, Unit, ViewMode};
use crate::tab::Tab;
use crate::tools::Cmd;
use eframe::egui::{self, Align2, Key};
use std::path::PathBuf;

pub enum Dialog {
    Password { path: PathBuf, text: String, wrong: bool },
    Repair { path: PathBuf },
    ConfirmClose { tab: usize },
    GotoPage { text: String },
    Prefs,
    About,
    Palette,
    Message { title: String, body: String },
    Recovery { files: Vec<(PathBuf, PathBuf)> },
}

fn window(_ctx: &egui::Context, title: &str) -> egui::Window<'static> {
    egui::Window::new(title.to_string()).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).order(egui::Order::Foreground)
}

impl App {
    pub fn message(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.dialog = Some(Dialog::Message { title: title.into(), body: body.into() });
    }

    /// Shows the current dialog box, if any.
    pub fn show_dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.dialog.take() else { return };
        let keep = match &mut d {
            Dialog::Password { path, text, wrong } => {
                let mut keep = true;
                window(ctx, t("dlg_password")).show(ctx, |ui| {
                    ui.label(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    if *wrong {
                        ui.colored_label(egui::Color32::from_rgb(220, 70, 70), t("wrong_password"));
                    }
                    let r = ui.add(egui::TextEdit::singleline(text).password(true).desired_width(260.0));
                    r.request_focus();
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.horizontal(|ui| {
                        if ui.button(t("ok")).clicked() || enter {
                            keep = false;
                        }
                        if ui.button(t("cancel")).clicked() {
                            keep = false;
                            text.clear();
                            *wrong = false;
                            path.clear();
                        }
                    });
                });
                if !keep && !path.as_os_str().is_empty() {
                    let (p, pw) = (path.clone(), text.clone());
                    self.open_path(&p, Some(pw));
                }
                keep
            }
            Dialog::Repair { path } => {
                let mut keep = true;
                let mut do_repair = false;
                window(ctx, t("dlg_repair")).show(ctx, |ui| {
                    ui.label(t("repair_text"));
                    ui.label(path.display().to_string());
                    ui.horizontal(|ui| {
                        if ui.button(t("repair_btn")).clicked() {
                            do_repair = true;
                            keep = false;
                        }
                        if ui.button(t("cancel")).clicked() {
                            keep = false;
                        }
                    });
                });
                if do_repair {
                    let p = path.clone();
                    self.repair_and_open(&p);
                }
                keep
            }
            Dialog::ConfirmClose { tab } => {
                let mut keep = true;
                let idx = *tab;
                let name = self.tabs.get(idx).map(|t| t.title.clone()).unwrap_or_default();
                let mut action = 0; // 1 = save, 2 = don't save
                window(ctx, t("dlg_confirm_close")).show(ctx, |ui| {
                    ui.label(format!("{name} — {}", t("unsaved_changes")));
                    ui.horizontal(|ui| {
                        if ui.button(t("save")).clicked() {
                            action = 1;
                            keep = false;
                        }
                        if ui.button(t("dont_save")).clicked() {
                            action = 2;
                            keep = false;
                        }
                        if ui.button(t("cancel")).clicked() {
                            keep = false;
                            self.quit_requested = false;
                        }
                    });
                });
                if action == 1 {
                    self.active = idx.min(self.tabs.len().saturating_sub(1));
                    if self.save_active(false) {
                        self.close_tab(idx, true);
                    }
                    self.continue_quit();
                } else if action == 2 {
                    self.close_tab(idx, true);
                    self.continue_quit();
                }
                keep
            }
            Dialog::GotoPage { text } => {
                let mut keep = true;
                let mut go = false;
                let n = self.tab_ref().map(|t| t.page_count()).unwrap_or(0);
                window(ctx, t("cmd_goto_page")).show(ctx, |ui| {
                    ui.label(format!("{} (1–{n})", t("page")));
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                    r.request_focus();
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.horizontal(|ui| {
                        if ui.button(t("ok")).clicked() || enter {
                            go = true;
                            keep = false;
                        }
                        if ui.button(t("cancel")).clicked() {
                            keep = false;
                        }
                    });
                });
                if go {
                    if let Ok(p) = text.trim().parse::<usize>() {
                        if let Some(tab) = self.tab() {
                            tab.goto_page(p.saturating_sub(1));
                        }
                    }
                }
                keep
            }
            Dialog::About => {
                let mut keep = true;
                window(ctx, t("cmd_about")).show(ctx, |ui| {
                    ui.heading("RustPDF");
                    ui.label(format!("{} {}", t("version"), env!("CARGO_PKG_VERSION")));
                    ui.label(t("about_text"));
                    ui.small("egui · PDFium · lopdf · Phosphor Icons (MIT)");
                    if ui.button(t("close")).clicked() {
                        keep = false;
                    }
                });
                keep
            }
            Dialog::Message { title, body } => {
                let mut keep = true;
                window(ctx, title).show(ctx, |ui| {
                    ui.label(body.as_str());
                    if ui.button(t("ok")).clicked() {
                        keep = false;
                    }
                });
                keep
            }
            Dialog::Prefs => self.prefs_dialog(ctx),
            Dialog::Palette => self.palette_dialog(ctx),
            Dialog::Recovery { files } => {
                let mut keep = true;
                let mut restore: Option<(PathBuf, PathBuf)> = None;
                let mut discard_all = false;
                window(ctx, t("dlg_recovery")).show(ctx, |ui| {
                    ui.label(t("recovery_text"));
                    for (orig, auto) in files.iter() {
                        ui.horizontal(|ui| {
                            ui.label(orig.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                            if ui.button(t("restore")).clicked() {
                                restore = Some((orig.clone(), auto.clone()));
                            }
                        });
                    }
                    if ui.button(t("discard_all")).clicked() {
                        discard_all = true;
                        keep = false;
                    }
                });
                if let Some((orig, auto)) = restore {
                    self.restore_autosave(&orig, &auto);
                    files.retain(|(o, _)| o != &orig);
                    if files.is_empty() {
                        keep = false;
                    }
                }
                if discard_all {
                    for (_, auto) in files.iter() {
                        let _ = std::fs::remove_file(auto);
                        let _ = std::fs::remove_file(auto.with_extension("json"));
                    }
                }
                keep
            }
        };
        if keep && self.dialog.is_none() {
            self.dialog = Some(d);
        }
    }

    fn prefs_dialog(&mut self, ctx: &egui::Context) -> bool {
        let mut keep = true;
        let mut changed = false;
        let mut p = self.prefs.clone();
        egui::Window::new(t("cmd_preferences")).collapsible(false).resizable(true).default_width(460.0).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            egui::Grid::new("prefs_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(t("pref_language"));
                let auto_label = format!("{} ({})", t("auto"), Lang::system().native_name());
                egui::ComboBox::from_id_salt("pl").selected_text(if p.lang == Lang::Auto { auto_label.clone() } else { p.lang.native_name().to_string() }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.lang, Lang::Auto, auto_label);
                    for l in Lang::ALL {
                        ui.selectable_value(&mut p.lang, l, l.native_name());
                    }
                });
                ui.end_row();

                ui.label(t("pref_theme"));
                egui::ComboBox::from_id_salt("pt").selected_text(match p.theme {
                    ThemePref::Auto => t("theme_auto"),
                    ThemePref::Light => t("theme_light"),
                    ThemePref::Dark => t("theme_dark"),
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.theme, ThemePref::Auto, t("theme_auto"));
                    ui.selectable_value(&mut p.theme, ThemePref::Light, t("theme_light"));
                    ui.selectable_value(&mut p.theme, ThemePref::Dark, t("theme_dark"));
                });
                ui.end_row();

                ui.label(t("pref_night"));
                ui.checkbox(&mut p.night_mode, "");
                ui.end_row();

                ui.label(t("pref_default_zoom"));
                ui.horizontal(|ui| {
                    let mut fit = p.default_zoom <= 0.01;
                    if ui.checkbox(&mut fit, t("fit_width")).changed() {
                        p.default_zoom = if fit { 0.0 } else { 1.0 };
                    }
                    if !fit {
                        let mut pct = p.default_zoom * 100.0;
                        if ui.add(egui::DragValue::new(&mut pct).range(10.0..=800.0).suffix(" %")).changed() {
                            p.default_zoom = pct / 100.0;
                        }
                    }
                });
                ui.end_row();

                ui.label(t("pref_view_mode"));
                egui::ComboBox::from_id_salt("pv").selected_text(match p.view_mode {
                    ViewMode::Single => t("cmd_view_single"),
                    ViewMode::Continuous => t("cmd_view_continuous"),
                    ViewMode::TwoPages => t("cmd_view_two"),
                    ViewMode::TwoPagesContinuous => t("cmd_view_two_continuous"),
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.view_mode, ViewMode::Single, t("cmd_view_single"));
                    ui.selectable_value(&mut p.view_mode, ViewMode::Continuous, t("cmd_view_continuous"));
                    ui.selectable_value(&mut p.view_mode, ViewMode::TwoPages, t("cmd_view_two"));
                    ui.selectable_value(&mut p.view_mode, ViewMode::TwoPagesContinuous, t("cmd_view_two_continuous"));
                });
                ui.end_row();

                ui.label(t("pref_units"));
                egui::ComboBox::from_id_salt("pu").selected_text(p.units.label()).show_ui(ui, |ui| {
                    for u in Unit::ALL {
                        ui.selectable_value(&mut p.units, u, u.label());
                    }
                });
                ui.end_row();

                ui.label(t("pref_author"));
                ui.text_edit_singleline(&mut p.author);
                ui.end_row();

                ui.label(t("pref_export_dpi"));
                egui::ComboBox::from_id_salt("pe").selected_text(format!("{:.0} dpi", p.export_dpi)).show_ui(ui, |ui| {
                    for d in [72.0, 150.0, 300.0, 600.0] {
                        ui.selectable_value(&mut p.export_dpi, d, format!("{d:.0} dpi"));
                    }
                });
                ui.end_row();

                ui.label(t("pref_autosave"));
                ui.add(egui::DragValue::new(&mut p.autosave_secs).range(0..=3600).suffix(" s"));
                ui.end_row();

                ui.label(t("pref_cache"));
                ui.add(egui::DragValue::new(&mut p.cache_mb).range(64..=4096).suffix(" Mo"));
                ui.end_row();

                ui.label(t("pref_undo_mem"));
                ui.add(egui::DragValue::new(&mut p.undo_mem_mb).range(50..=4096).suffix(" Mo"));
                ui.end_row();

                ui.label(t("pref_restore_view"));
                ui.checkbox(&mut p.restore_view, "");
                ui.end_row();
                ui.label(t("pref_reading_default"));
                ui.checkbox(&mut p.open_in_reading_mode, "");
                ui.end_row();
                ui.label(t("pref_protected_view"));
                ui.checkbox(&mut p.protected_view, "").on_hover_text(t("protected_enable_hint"));
                ui.end_row();

                ui.label(t("pref_check_updates"));
                ui.checkbox(&mut p.check_updates, "");
                ui.end_row();

                ui.label(t("pref_update_url"));
                ui.add(egui::TextEdit::singleline(&mut p.update_url).desired_width(260.0).hint_text(crate::updater::DEFAULT_URL));
                ui.end_row();
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(t("assoc_pdf")).clicked() {
                    let r = crate::sys::associate_pdf();
                    self.status = match r {
                        Ok(_) => t("assoc_done").to_string(),
                        Err(e) => format!("{}: {e}", t("assoc_failed")),
                    };
                }
                if ui.button(t("open_data_folder")).clicked() {
                    crate::sys::open_folder(&logx::data_dir());
                }
                if ui.button(t("cmd_open_log")).clicked() {
                    crate::sys::open_file(&logx::data_dir().join("rustpdf.log"));
                }
            });
            ui.separator();
            if ui.button(t("close")).clicked() {
                keep = false;
            }
        });
        if p.lang != self.prefs.lang
            || p.theme != self.prefs.theme
            || p.night_mode != self.prefs.night_mode
            || p.view_mode != self.prefs.view_mode
            || p.cache_mb != self.prefs.cache_mb
            || p.units != self.prefs.units
            || p.default_zoom != self.prefs.default_zoom
            || p.export_dpi != self.prefs.export_dpi
            || p.autosave_secs != self.prefs.autosave_secs
            || p.undo_mem_mb != self.prefs.undo_mem_mb
            || p.restore_view != self.prefs.restore_view
            || p.open_in_reading_mode != self.prefs.open_in_reading_mode
            || p.protected_view != self.prefs.protected_view
            || p.check_updates != self.prefs.check_updates
            || p.update_url != self.prefs.update_url
            || p.author != self.prefs.author
        {
            changed = true;
        }
        if changed {
            let night_changed = p.night_mode != self.prefs.night_mode;
            self.prefs = p;
            i18n::set_lang(self.prefs.lang);
            self.cache.set_cap_mb(self.prefs.cache_mb);
            if night_changed {
                // the cached textures do not have the right polarity: they are keyed by `night`, nothing to purge.
            }
            self.prefs.save();
        }
        if !keep {
            self.prefs.save();
        }
        keep
    }

    fn palette_dialog(&mut self, ctx: &egui::Context) -> bool {
        let mut keep = true;
        let mut chosen: Option<Cmd> = None;
        let q = self.palette_query.to_lowercase();
        let cmds: Vec<Cmd> = Cmd::palette_list().into_iter().filter(|c| t(c.label_key()).to_lowercase().contains(&q)).collect();
        egui::Window::new(t("cmd_palette")).title_bar(false).collapsible(false).resizable(false).anchor(Align2::CENTER_TOP, [0.0, 80.0]).default_width(480.0).show(ctx, |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.palette_query).desired_width(f32::INFINITY).hint_text(t("palette_hint")));
            r.request_focus();
            let (esc, enter) = ui.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
            if esc {
                keep = false;
            }
            if enter {
                chosen = cmds.first().copied();
                keep = false;
            }
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for c in cmds.iter().take(60) {
                    let sc = c.shortcut().map(|s| ctx.format_shortcut(&s)).unwrap_or_default();
                    let label = format!("{} {}", c.icon(), t(c.label_key()));
                    ui.horizontal(|ui| {
                        if ui.selectable_label(false, label).clicked() {
                            chosen = Some(*c);
                            keep = false;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(sc);
                        });
                    });
                }
            });
        });
        if let Some(c) = chosen {
            self.palette_query.clear();
            self.run_cmd(c, ctx);
        }
        if !keep {
            self.palette_query.clear();
        }
        keep
    }

    pub fn repair_and_open(&mut self, path: &PathBuf) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                self.err(format!("{}: {e}", t("err_open")));
                return;
            }
        };
        match pdfx::repair_bytes(&bytes).and_then(|b| pdfx::open_bytes(self.pdfium, b, None).ok()) {
            Some(doc) => {
                let (zoom, fit) = self.initial_zoom();
                let mut tab = Tab::new(self.pdfium, doc, Some(path.clone()), None, zoom);
                tab.view.fit = fit;
                tab.title = format!("{} ({})", tab.title, t("repaired"));
                tab.dirty = true;
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                self.say(t("repair_done"));
            }
            None => self.message(t("dlg_repair"), t("repair_failed")),
        }
    }

    /// Resumes the application's close handling after a confirmation.
    pub fn continue_quit(&mut self) {
        if self.quit_requested {
            self.try_quit();
        }
    }

    pub fn try_quit(&mut self) {
        self.quit_requested = true;
        if let Some(i) = self.tabs.iter().position(|t| t.dirty) {
            self.pending_close = Some(i);
            self.active = i;
            self.dialog = Some(Dialog::ConfirmClose { tab: i });
        }
    }
}
