//! Main window: menus, toolbar, tabs, status bar, central area.

use crate::app::App;
use crate::dialogs::Dialog;
use crate::i18n::t;
use crate::icons;
use crate::logx;
use crate::props::icon_button;
use crate::tab::FitMode;
use crate::tools::{Cmd, ShapeKind, Tool};
use crate::view::{self, ViewEnv};
use eframe::egui::{self, Color32, RichText};

impl App {
    /// Applies the language, theme and fonts (once, then on every change).
    fn setup_once(&mut self, ctx: &egui::Context) {
        if !self.fonts_ready {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("phosphor".into(), std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../assets/Phosphor.ttf"))));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push("phosphor".into());
            }
            ctx.set_fonts(fonts);
            self.fonts_ready = true;
            // recovery after a crash
            let found = crate::autosave::scan(std::process::id());
            if !found.is_empty() {
                self.dialog = Some(Dialog::Recovery { files: found });
            }
            let files = std::mem::take(&mut self.startup_files);
            for f in files {
                self.open_path(&f, None);
            }
        }
        let want = (self.prefs.lang, self.prefs.theme);
        if self.applied_prefs != Some(want) {
            ctx.set_theme(match self.prefs.theme {
                crate::prefs::ThemePref::Auto => egui::ThemePreference::System,
                crate::prefs::ThemePref::Light => egui::ThemePreference::Light,
                crate::prefs::ThemePref::Dark => egui::ThemePreference::Dark,
            });
            self.applied_prefs = Some(want);
        }
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() && !matches!(self.dialog, Some(Dialog::Palette)) {
            return;
        }
        // text being typed: leave the keys to the field
        let typing = ctx.egui_wants_keyboard_input();
        let mut cmds = vec![];
        for c in Cmd::palette_list().into_iter().chain([Cmd::NextTab, Cmd::PrevTab]) {
            if let Some(sc) = c.shortcut() {
                // while typing text, only Ctrl/Cmd commands go through (except copy/cut/paste/undo handled by the field)
                if typing && (!sc.modifiers.command || matches!(c, Cmd::Copy | Cmd::Cut | Cmd::Paste | Cmd::Undo | Cmd::Redo | Cmd::SelectAll | Cmd::Delete | Cmd::Duplicate)) {
                    continue;
                }
                if ctx.input_mut(|i| i.consume_shortcut(&sc)) {
                    cmds.push(c);
                }
            }
        }
        if !typing {
            if ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
                cmds.push(Cmd::Delete);
            }
            // arrows: move the selection (Shift = 10 pt) or change page
            let (l, r, u, d, shift) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.modifiers.shift));
            let step = if shift { 10.0 } else { 1.0 };
            let has_sel = self.tabs.get(self.active).map(|t| t.sel.has_object()).unwrap_or(false);
            if has_sel && (l || r || u || d) {
                let (vx, vy) = ((r as i32 - l as i32) as f32 * step, (d as i32 - u as i32) as f32 * step);
                if let (Some((page, items)), Some(tab)) = (self.sel_items(), self.tabs.get(self.active)) {
                    // arrows in view space (screen) → page space
                    let delta = tab.sel_view_delta(page, vx, vy);
                    let mem = self.mem();
                    if let Some(tb) = self.tabs.get_mut(self.active) {
                        if let Err(e) = tb.move_items(page, &items, delta.0, delta.1, mem) {
                            self.err(e);
                        }
                    }
                }
            } else if self.presentation || !has_sel {
                if r || d || ctx.input(|i| i.key_pressed(egui::Key::PageDown) || i.key_pressed(egui::Key::Space)) {
                    if self.presentation || self.prefs.view_mode == crate::prefs::ViewMode::Single || self.prefs.view_mode == crate::prefs::ViewMode::TwoPages {
                        cmds.push(Cmd::NextPage);
                    }
                }
                if l || u || ctx.input(|i| i.key_pressed(egui::Key::PageUp)) {
                    if self.presentation || self.prefs.view_mode == crate::prefs::ViewMode::Single || self.prefs.view_mode == crate::prefs::ViewMode::TwoPages {
                        cmds.push(Cmd::PrevPage);
                    }
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                if self.presentation {
                    cmds.push(Cmd::Presentation);
                } else if self.reading {
                    cmds.push(Cmd::ReadingMode);
                } else if self.search_open {
                    self.search_open = false;
                    if let Some(tab) = self.tab() {
                        tab.clear_search();
                    }
                } else if self.tool != Tool::Select {
                    self.set_tool(Tool::Select);
                } else if let Some(tab) = self.tab() {
                    tab.sel.clear();
                    tab.text_sel = None;
                }
            }
            // single-key tools
            for (key, tool) in [(egui::Key::V, Tool::Select), (egui::Key::H, Tool::Hand), (egui::Key::T, Tool::AddText), (egui::Key::R, Tool::Shape(ShapeKind::Rect)), (egui::Key::E, Tool::Shape(ShapeKind::Ellipse)), (egui::Key::L, Tool::Shape(ShapeKind::Line))] {
                if ctx.input(|i| i.key_pressed(key) && !i.modifiers.any()) {
                    self.set_tool(tool);
                }
            }
        }
        for c in cmds {
            self.run_cmd(c, ctx);
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        // automatic saving
        let secs = self.prefs.autosave_secs;
        if secs > 0 && self.last_autosave_check.elapsed() > std::time::Duration::from_secs(secs as u64) {
            self.last_autosave_check = std::time::Instant::now();
            for tab in &mut self.tabs {
                if tab.dirty && tab.last_edit.map(|t| t.elapsed().as_secs() >= 3).unwrap_or(true) {
                    crate::autosave::write(tab);
                }
            }
        }
        // Automatic update check: once a day, in the background.
        if !self.update_started && self.prefs.check_updates && ctx.input(|i| i.time) > 4.0 {
            self.update_started = true;
            let now = crate::logx::unix_secs();
            if now.saturating_sub(self.prefs.last_update_check) > 24 * 3600 {
                self.prefs.last_update_check = now;
                self.prefs.save();
                let url = crate::updater::effective_url(&self.prefs.update_url).to_string();
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(crate::updater::check_release(&url));
                });
                self.update_rx = Some(rx);
            }
        }
        if let Some(rx) = &self.update_rx {
            match rx.try_recv() {
                Ok(outcome) => {
                    self.update_rx = None;
                    match &outcome {
                        crate::updater::Outcome::Newer(r) => {
                            let msg = crate::updater::describe(&outcome);
                            self.say(format!("{} {}", t("update_available"), r.version));
                            self.update_found = Some((r.version.clone(), msg));
                        }
                        crate::updater::Outcome::Error(e) => logx::warn(format!("update check failed: {e}")),
                        crate::updater::Outcome::UpToDate => {}
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_secs(1)),
                Err(_) => self.update_rx = None,
            }
        }
        // While idle: prepare the undo snapshot of the active document (the next edit becomes instant).
        if ctx.input(|i| !i.events.is_empty() || i.pointer.any_down() || i.pointer.delta() != egui::Vec2::ZERO) {
            self.last_input = std::time::Instant::now();
        }
        if let Some(tb) = self.tabs.get_mut(self.active) {
            const MAX_PREP: u64 = 150 * 1024 * 1024;
            let small = tb.path.as_ref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len() < MAX_PREP).unwrap_or(true);
            if small && !tb.snapshot_ready() {
                if self.last_input.elapsed() > std::time::Duration::from_millis(500) {
                    tb.prepare_snapshot();
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(600));
                }
            }
        }
        // window title
        let title = match self.tabs.get(self.active) {
            Some(tb) => format!("{}{} — RustPDF", if tb.dirty { "● " } else { "" }, tb.title),
            None => "RustPDF".to_string(),
        };
        if title != self.title_shown {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title_shown = title;
        }
        // window closing
        if ctx.input(|i| i.viewport().close_requested()) && self.tabs.iter().any(|t| t.dirty) && !self.force_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.try_quit();
        } else if self.quit_requested && self.dialog.is_none() && !self.tabs.iter().any(|t| t.dirty) {
            self.force_quit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            // remember the zoom/page of each open document
            for tab in &self.tabs {
                if let Some(p) = &tab.path {
                    self.recent.remember_view(p, tab.view.page, if tab.view.fit == FitMode::None { tab.view.zoom } else { 0.0 });
                }
            }
            self.prefs.save();
            for tab in &self.tabs {
                if let Some(f) = &tab.autosave_file {
                    crate::autosave::remove(f);
                }
            }
        }
        // files dropped on the window
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| { let p = f.path(); if p.as_os_str().is_empty() { None } else { Some(p.to_path_buf()) } }).collect());
        for p in dropped {
            self.open_path(&p, None);
        }
    }

    pub fn menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(t("menu_file"), |ui| {
                self.menu_cmd(ui, ctx, Cmd::Open);
                ui.menu_button(format!("{} {}", icons::CLOCK_COUNTER_CLOCKWISE, t("recent")), |ui| {
                    if self.recent.entries.is_empty() {
                        ui.weak(t("no_recent"));
                    }
                    let entries = self.recent.entries.clone();
                    for (i, e) in entries.iter().enumerate() {
                        let p = e.path_buf();
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let r = ui.add_enabled(p.exists(), egui::Button::new(format!("{} {name}", icons::FILE_PDF))).on_hover_text(e.path.clone());
                        if r.clicked() {
                            self.run_cmd(Cmd::OpenRecent(i), ctx);
                            ui.close();
                        }
                    }
                    if !entries.is_empty() {
                        ui.separator();
                        if ui.button(format!("{} {}", icons::TRASH, t("clear_recent"))).clicked() {
                            self.recent.clear();
                            ui.close();
                        }
                    }
                });
                ui.separator();
                for c in [Cmd::Save, Cmd::SaveAs, Cmd::Export] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::Properties, Cmd::Security, Cmd::CleanMetadata, Cmd::RepairDoc] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::CloseTab, Cmd::Quit] {
                    self.menu_cmd(ui, ctx, c);
                }
            });
            ui.menu_button(t("menu_edit"), |ui| {
                for c in [Cmd::Undo, Cmd::Redo] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.menu_button(t("history"), |ui| {
                    let labels: Vec<String> = self.tabs.get(self.active).map(|tb| tb.undo.iter().map(|e| e.label.clone()).collect()).unwrap_or_default();
                    if labels.is_empty() {
                        ui.weak(t("history_empty"));
                    }
                    let mut goto = None;
                    for (i, l) in labels.iter().enumerate().rev() {
                        if ui.button(format!("{}. {l}", i + 1)).clicked() {
                            goto = Some(i);
                            ui.close();
                        }
                    }
                    if let (Some(i), Some(tb)) = (goto, self.tabs.get_mut(self.active)) {
                        tb.undo_to(i);
                    }
                });
                ui.separator();
                for c in [Cmd::Cut, Cmd::Copy, Cmd::Paste, Cmd::PasteImage, Cmd::Duplicate, Cmd::Delete, Cmd::SelectAll] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::Find, Cmd::FindNext, Cmd::FindPrev, Cmd::Replace] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                self.menu_cmd(ui, ctx, Cmd::Preferences);
            });
            ui.menu_button(t("menu_view"), |ui| {
                for c in [Cmd::ZoomIn, Cmd::ZoomOut, Cmd::Zoom100, Cmd::FitWidth, Cmd::FitPage] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::ViewSingle, Cmd::ViewContinuous, Cmd::ViewTwo, Cmd::ViewTwoContinuous, Cmd::ToggleCover] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::ToggleSidePanel, Cmd::ToggleProps, Cmd::ToggleRulers, Cmd::ToggleNight, Cmd::ToggleBoxes, Cmd::ToggleParagraphs] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::ReadingMode, Cmd::Fullscreen, Cmd::Presentation] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::FirstPage, Cmd::PrevPage, Cmd::NextPage, Cmd::LastPage, Cmd::GotoPage] {
                    self.menu_cmd(ui, ctx, c);
                }
            });
            ui.menu_button(t("menu_object"), |ui| {
                for c in [Cmd::Group, Cmd::Ungroup] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::BringFront, Cmd::BringForward, Cmd::SendBackward, Cmd::SendBack] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::AlignLeft, Cmd::AlignCenter, Cmd::AlignRight, Cmd::AlignTop, Cmd::AlignMiddle, Cmd::AlignBottom, Cmd::DistributeH, Cmd::DistributeV] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::Rotate90, Cmd::RotateMinus90, Cmd::FlipH, Cmd::FlipV] {
                    self.menu_cmd(ui, ctx, c);
                }
            });
            ui.menu_button(t("menu_document"), |ui| {
                for c in [Cmd::InsertBlank, Cmd::InsertFromFile, Cmd::ExtractPage, Cmd::DeletePage, Cmd::RotatePage, Cmd::MovePageUp, Cmd::MovePageDown] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::MergeFiles, Cmd::SplitDoc, Cmd::DeletePages, Cmd::RotatePages, Cmd::CropPages, Cmd::PageSize, Cmd::Deskew] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::PageNumbers, Cmd::HeaderFooter, Cmd::Watermark] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::Attachments, Cmd::Bookmarks, Cmd::FlattenForm] {
                    self.menu_cmd(ui, ctx, c);
                }
                ui.separator();
                for c in [Cmd::Sign, Cmd::VerifySignatures, Cmd::Security] {
                    self.menu_cmd(ui, ctx, c);
                }
            });
            ui.menu_button(t("menu_help"), |ui| {
                for c in [Cmd::CommandPalette, Cmd::CheckUpdates, Cmd::OpenLog, Cmd::About] {
                    self.menu_cmd(ui, ctx, c);
                }
            });
        });
    }

    fn menu_cmd(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, c: Cmd) {
        let mut b = egui::Button::new(format!("{} {}", c.icon(), t(c.label_key())));
        if let Some(sc) = c.shortcut() {
            b = b.shortcut_text(ctx.format_shortcut(&sc));
        }
        if ui.add(b).clicked() {
            self.pending_cmds.push(c);
            ui.close();
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let has = !self.tabs.is_empty();
        let tool = self.tool;
        // wide window: everything on a single line; otherwise two rows (tools, then view)
        let single = ui.available_width() > 1260.0;
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(true, |ui| {
                for c in [Cmd::Open] {
                    if icon_button(ui, c.icon(), t(c.label_key()), false).clicked() {
                        self.pending_cmds.push(c);
                    }
                }
            });
            ui.add_enabled_ui(has, |ui| {
                for c in [Cmd::Save, Cmd::Undo, Cmd::Redo] {
                    if icon_button(ui, c.icon(), t(c.label_key()), false).clicked() {
                        self.pending_cmds.push(c);
                    }
                }
                ui.separator();
                for tl in [Tool::Select, Tool::Hand, Tool::TextSelect] {
                    if icon_button(ui, tl.icon(), t(tl.label_key()), tool == tl).clicked() {
                        self.pending_cmds.push(Cmd::SetTool(tl));
                    }
                }
                ui.separator();
                for tl in [Tool::AddText, Tool::AddImage] {
                    if icon_button(ui, tl.icon(), t(tl.label_key()), tool == tl).clicked() {
                        self.pending_cmds.push(Cmd::SetTool(tl));
                    }
                }
                self.tool_menu(ui, icons::SHAPES, "tools_shapes", &[
                    Tool::Shape(ShapeKind::Rect),
                    Tool::Shape(ShapeKind::Ellipse),
                    Tool::Shape(ShapeKind::Line),
                    Tool::Shape(ShapeKind::Arrow),
                    Tool::Shape(ShapeKind::Polygon),
                    Tool::Shape(ShapeKind::Freehand),
                ]);
                self.tool_menu(ui, icons::HIGHLIGHTER, "tools_markup", &[Tool::Highlight, Tool::Underline, Tool::StrikeOut, Tool::Squiggly]);
                for tl in [Tool::Note, Tool::Stamp, Tool::Redact, Tool::Measure, Tool::Link] {
                    if icon_button(ui, tl.icon(), t(tl.label_key()), tool == tl).clicked() {
                        self.pending_cmds.push(Cmd::SetTool(tl));
                    }
                }
                self.tool_menu(ui, icons::TEXTBOX, "tools_forms", &[Tool::FormText, Tool::FormCheck, Tool::FormRadio, Tool::FormCombo]);
                for tl in [Tool::Signature, Tool::ExportZone] {
                    if icon_button(ui, tl.icon(), t(tl.label_key()), tool == tl).clicked() {
                        self.pending_cmds.push(Cmd::SetTool(tl));
                    }
                }
                if single { ui.separator(); } else { ui.end_row(); }
                for c in [Cmd::ZoomOut, Cmd::ZoomIn, Cmd::FitWidth, Cmd::FitPage] {
                    if icon_button(ui, c.icon(), t(c.label_key()), false).clicked() {
                        self.pending_cmds.push(c);
                    }
                }
                ui.menu_button(icons::COLUMNS, |ui| {
                    for c in [Cmd::ViewSingle, Cmd::ViewContinuous, Cmd::ViewTwo, Cmd::ViewTwoContinuous, Cmd::ToggleCover] {
                        if ui.button(t(c.label_key())).clicked() {
                            self.pending_cmds.push(c);
                            ui.close();
                        }
                    }
                });
                ui.separator();
                if icon_button(ui, icons::MAGNIFYING_GLASS, t("cmd_find"), self.search_open).clicked() {
                    self.pending_cmds.push(Cmd::Find);
                }
                if icon_button(ui, icons::TEXT_ALIGN_JUSTIFY, t("cmd_toggle_paragraphs"), self.paragraph_mode).clicked() {
                    self.pending_cmds.push(Cmd::ToggleParagraphs);
                }
            });
            if icon_button(ui, icons::BOOK_OPEN, t("cmd_reading_mode"), self.reading).clicked() {
                self.pending_cmds.push(Cmd::ReadingMode);
            }
            if icon_button(ui, icons::COMMAND, t("cmd_palette"), false).clicked() {
                self.pending_cmds.push(Cmd::CommandPalette);
            }
            if icon_button(ui, icons::GEAR, t("cmd_preferences"), false).clicked() {
                self.pending_cmds.push(Cmd::Preferences);
            }
        });
    }

    fn tool_menu(&mut self, ui: &mut egui::Ui, icon: &str, tip: &'static str, tools: &[Tool]) {
        let active = tools.contains(&self.tool);
        let shown = if active { self.tool.icon() } else { tools[0].icon() };
        let _ = icon;
        let resp = ui.menu_button(RichText::new(shown).size(16.0), |ui| {
            for tl in tools {
                if ui.button(format!("{} {}", tl.icon(), t(tl.label_key()))).clicked() {
                    self.pending_cmds.push(Cmd::SetTool(*tl));
                    ui.close();
                }
            }
        });
        let r = resp.response.rect;
        resp.response.on_hover_text(t(tip));
        if active {
            ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(1.5, Color32::from_rgb(40, 120, 255)), egui::StrokeKind::Outside);
        }
    }

    fn tabs_bar(&mut self, ui: &mut egui::Ui) {
        if self.tabs.is_empty() {
            return;
        }
        let mut close = None;
        let mut select = None;
        ui.horizontal(|ui| {
            for (i, tab) in self.tabs.iter().enumerate() {
                let title = format!("{}{}", if tab.dirty { "● " } else { "" }, tab.title);
                let r = ui.selectable_label(i == self.active, title).on_hover_text(tab.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default());
                if r.clicked() {
                    select = Some(i);
                }
                if r.middle_clicked() {
                    close = Some(i);
                }
                if ui.small_button(icons::X).clicked() {
                    close = Some(i);
                }
                ui.add_space(6.0);
            }
        });
        if let Some(i) = select {
            self.active = i;
        }
        if let Some(i) = close {
            self.close_tab(i, false);
        }
    }

    fn search_bar(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let mut run = false;
        let mut close = false;
        let mut next = false;
        let mut prev = false;
        let mut replace_one = false;
        let mut replace_all = false;
        ui.horizontal(|ui| {
            ui.label(icons::MAGNIFYING_GLASS);
            let r = ui.add(egui::TextEdit::singleline(&mut tab.search.query).hint_text(t("search_in_doc")).desired_width(260.0));
            if self.search_focus {
                r.request_focus();
                self.search_focus = false;
            }
            if r.changed() {
                run = true;
            }
            let (enter, shift, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.modifiers.shift, i.key_pressed(egui::Key::Escape)));
            if r.lost_focus() && enter {
                if shift {
                    prev = true;
                } else {
                    next = true;
                }
                r.request_focus();
            }
            if icon_button(ui, icons::CARET_UP, t("cmd_find_prev"), false).clicked() {
                prev = true;
            }
            if icon_button(ui, icons::CARET_DOWN, t("cmd_find_next"), false).clicked() {
                next = true;
            }
            if ui.toggle_value(&mut tab.search.case, "Aa").on_hover_text(t("match_case")).changed() {
                run = true;
            }
            if ui.toggle_value(&mut tab.search.word, "ab").on_hover_text(t("whole_word")).changed() {
                run = true;
            }
            if tab.search.query.trim().is_empty() {
                ui.label("");
            } else if tab.search.matches.is_empty() {
                ui.colored_label(Color32::from_rgb(220, 70, 70), t("no_results"));
            } else {
                ui.label(format!("{} / {}", tab.search.cur.map(|c| c + 1).unwrap_or(0), tab.search.matches.len()));
            }
            ui.toggle_value(&mut self.replace_mode, icons::SWAP).on_hover_text(t("cmd_replace"));
            if icon_button(ui, icons::X, t("close"), false).clicked() || (r.lost_focus() && esc) {
                close = true;
            }
        });
        if self.replace_mode {
            ui.horizontal(|ui| {
                ui.label(icons::SWAP);
                ui.add(egui::TextEdit::singleline(&mut self.replace_text).hint_text(t("replace_with")).desired_width(260.0));
                if ui.button(t("replace")).clicked() {
                    replace_one = true;
                }
                if ui.button(t("replace_all")).clicked() {
                    replace_all = true;
                }
            });
        }
        let mem = self.prefs.undo_mem_mb;
        let with = self.replace_text.clone();
        if close {
            self.search_open = false;
            tab.clear_search();
        } else {
            if run {
                tab.run_search();
            }
            if next {
                tab.next_match();
            }
            if prev {
                tab.prev_match();
            }
            if replace_one {
                match tab.replace_current(&with, mem) {
                    Ok(true) => {
                        tab.refresh_matches();
                        if tab.search.cur.is_none() && !tab.search.matches.is_empty() {
                            tab.goto_match(0);
                        }
                    }
                    Ok(false) => {}
                    Err(e) => self.status = e,
                }
            }
            if replace_all {
                match tab.replace_all(&with, mem) {
                    Ok(n) => {
                        tab.refresh_matches();
                        self.status = format!("{n} {}", t("replacements"));
                    }
                    Err(e) => self.status = e,
                }
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let coords = self.tabs.get(self.active).and_then(|tb| crate::rulers::pointer_coords(ui.ctx(), tb, &self.prefs));
        ui.horizontal(|ui| {
            // messages: disappear after a few seconds
            if self.status_at.elapsed().as_secs() < 12 {
                ui.label(&self.status);
            }
            if let Some((v, _)) = &self.update_found {
                ui.separator();
                if ui.link(format!("⬆ {} {v}", t("update_available"))).clicked() {
                    self.pending_cmds.push(Cmd::CheckUpdates);
                }
            }
            if let Some((_, x, y)) = coords {
                ui.separator();
                ui.weak(format!("x {}  y {}", self.prefs.units.format(x), self.prefs.units.format(y)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let n = tab.page_count();
                    // zoom
                    let mut pct = tab.view.zoom * 100.0;
                    let r = ui.add(egui::DragValue::new(&mut pct).range(10.0..=800.0).speed(1.0).suffix(" %"));
                    if r.changed() {
                        tab.view.fit = FitMode::None;
                        view::set_zoom(tab, pct / 100.0, None);
                    }
                    ui.separator();
                    // page
                    let mut p = tab.view.page + 1;
                    ui.label(format!("/ {n}"));
                    let r = ui.add(egui::DragValue::new(&mut p).range(1..=n.max(1)).speed(0.1));
                    if r.changed() {
                        tab.goto_page(p.saturating_sub(1));
                    }
                    ui.label(t("page"));
                    if tab.dirty {
                        ui.separator();
                        ui.colored_label(Color32::from_rgb(230, 150, 40), t("modified"));
                    }
                    let mem: usize = tab.undo.iter().chain(tab.redo.iter()).map(|e| e.bytes.len()).sum();
                    if mem > 0 {
                        ui.separator();
                        ui.weak(format!("{} {:.0} Mo", t("undo_mem"), mem as f32 / 1048576.0));
                    }
                }
            });
        });
    }

    fn context_menu_popup(&mut self, ctx: &egui::Context) {
        let Some((page, x, y)) = self.ctx_menu else { return };
        let mut close = false;
        let id = egui::Id::unique("page_ctx");
        let pos = ctx.pointer_latest_pos().unwrap_or_default();
        let has_sel = self.tabs.get(self.active).map(|t| !t.sel.is_empty()).unwrap_or(false);
        let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(self.ctx_menu_pos.unwrap_or(pos)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(190.0);
                let mut items: Vec<Cmd> = vec![];
                if has_sel {
                    items.extend([Cmd::Cut, Cmd::Copy, Cmd::Duplicate, Cmd::Delete]);
                    items.extend([Cmd::BringFront, Cmd::SendBack, Cmd::Group, Cmd::Ungroup]);
                    items.extend([Cmd::Rotate90, Cmd::FlipH]);
                } else {
                    items.extend([Cmd::Paste, Cmd::PasteImage, Cmd::SelectAll]);
                    items.extend([Cmd::SetTool(Tool::AddText), Cmd::SetTool(Tool::AddImage)]);
                }
                for c in items {
                    if ui.button(format!("{} {}", c.icon(), t(c.label_key()))).clicked() {
                        self.pending_cmds.push(c);
                        close = true;
                    }
                }
            });
        });
        let _ = (page, x, y);
        if self.ctx_menu_pos.is_none() {
            self.ctx_menu_pos = Some(pos);
        }
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && self.ctx_menu_age.elapsed().as_millis() > 150;
        if close || clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.ctx_menu = None;
            self.ctx_menu_pos = None;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.setup_once(&ctx);
        self.cache.begin_frame();
        self.tick(&ctx);
        self.global_shortcuts(&ctx);

        if !self.presentation {
            egui::Panel::top("menu").show(ui, |ui| {
                self.menu_bar(ui, &ctx);
            });
            egui::Panel::top("toolbar").show(ui, |ui| {
                self.toolbar(ui, &ctx);
            });
            if !self.tabs.is_empty() {
                egui::Panel::top("tabs").show(ui, |ui| {
                    self.tabs_bar(ui);
                });
            }
            if self.search_open && !self.tabs.is_empty() {
                egui::Panel::top("searchbar").show(ui, |ui| {
                    self.search_bar(ui);
                });
            }
            egui::Panel::bottom("status").show(ui, |ui| {
                self.status_bar(ui);
            });
            if self.show_side && !self.tabs.is_empty() {
                egui::Panel::left("side").default_size(190.0).min_size(120.0).show(ui, |ui| {
                    self.side_panel(ui);
                });
            }
            if self.show_props && !self.reading && !self.tabs.is_empty() {
                egui::Panel::right("props").default_size(270.0).min_size(180.0).show(ui, |ui| {
                    self.props_panel(ui);
                });
            }
        }

        let mut msgs = std::mem::take(&mut self.pending_msgs);
        egui::CentralPanel::default().show(ui, |ui| {
            if self.tabs.is_empty() {
                self.welcome(ui);
            } else {
                let tool = self.tool;
                let style = self.style.clone();
                let has_clip = self.clip.is_some();
                let prefs = self.prefs.clone();
                let tab = &mut self.tabs[self.active];
                tab.set_paragraph_mode(self.paragraph_mode);
                let mut env = ViewEnv { ctx: &ctx, prefs: &prefs, tool, cache: &mut self.cache, msgs: &mut msgs, style: &style, has_clip };
                if prefs.show_rulers && !self.presentation && !self.reading {
                    let full = ui.max_rect();
                    let t = crate::rulers::THICKNESS;
                    let inner = egui::Rect::from_min_max(egui::Pos2::new(full.min.x + t, full.min.y + t), full.max);
                    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                        view::show(ui, tab, &mut env);
                    });
                    crate::rulers::draw(ui, full, inner, tab, &prefs);
                } else {
                    view::show(ui, tab, &mut env);
                }
            }
        });
        if self.search_open || self.tabs.iter().any(|t| t.search.stale) {
            for tab in &mut self.tabs {
                if tab.search.stale && self.search_open {
                    tab.refresh_matches();
                } else if tab.search.stale {
                    tab.search.stale = false;
                }
            }
        }

        if self.ctx_menu.is_some() {
            if self.ctx_menu_age_set != self.ctx_menu {
                self.ctx_menu_age = std::time::Instant::now();
                self.ctx_menu_age_set = self.ctx_menu;
                self.ctx_menu_pos = None;
            }
            self.context_menu_popup(&ctx);
        } else {
            self.ctx_menu_age_set = None;
        }

        self.show_dialogs(&ctx);
        self.show_dlg2(&ctx);
        self.handle_msgs(msgs, &ctx);
        // commands triggered by the menus and panels
        let cmds = std::mem::take(&mut self.pending_cmds);
        for c in cmds {
            self.run_cmd(c, &ctx);
        }
        if let Some((from, to)) = self.pending_page_move.take() {
            self.move_page_to(from, to);
        }
        if self.cache.pending {
            ctx.request_repaint();
        }
    }
}

impl App {
    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            ui.label(RichText::new("RustPDF").size(36.0).strong());
            ui.label(t("welcome_text"));
            ui.add_space(10.0);
            if ui.button(RichText::new(format!("{} {}", icons::FOLDER_OPEN, t("cmd_open"))).size(16.0)).clicked() {
                self.pending_cmds.push(Cmd::Open);
            }
            if !self.recent.entries.is_empty() {
                ui.add_space(16.0);
                ui.label(RichText::new(t("recent")).strong());
                let entries = self.recent.entries.clone();
                for (i, e) in entries.iter().enumerate() {
                    let p = e.path_buf();
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let r = ui.add_enabled(p.exists(), egui::Button::new(format!("{} {name}", icons::FILE_PDF))).on_hover_text(e.path.clone());
                    if r.clicked() {
                        self.pending_cmds.push(Cmd::OpenRecent(i));
                    }
                }
            }
        });
        let _ = logx::data_dir;
    }
}
