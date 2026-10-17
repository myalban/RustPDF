//! Command dispatch and handling of the events raised by the view.

use crate::app::App;
use crate::dialogs::Dialog;
use crate::edit::{AlignKind, Distribute, ZOrder};
use crate::geom::*;
use crate::i18n::t;
use crate::pathdata::PathData;
use crate::prefs::ViewMode;
use crate::tab::FitMode;
use crate::tools::{Cmd, ShapeKind, Tool};
use crate::view::{self, Msg};
use eframe::egui;
use std::path::PathBuf;

impl App {
    /// Selected items of the active tab: (page, indices).
    pub fn sel_items(&self) -> Option<(usize, Vec<usize>)> {
        let t = self.tab_ref()?;
        if t.sel.items.is_empty() {
            None
        } else {
            Some((t.sel.page, t.sel.items.clone()))
        }
    }

    pub fn todo(&mut self, what: &str) {
        self.say(format!("{what}: {}", t("not_yet")));
    }

    /// Runs `f` on the active tab; shows any error; clears the selection if requested.
    pub fn edit<F>(&mut self, clear_sel: bool, f: F)
    where
        F: FnOnce(&mut crate::tab::Tab, u32) -> Result<(), String>,
    {
        let mem = self.mem();
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let r = f(tab, mem);
        if clear_sel {
            tab.sel.clear();
        }
        match r {
            Ok(_) => {}
            Err(e) => self.err(e),
        }
        self.text_ui.key = None;
        self.text_ui.geo_key = None;
    }

    pub fn run_cmd(&mut self, cmd: Cmd, ctx: &egui::Context) {
        let mem = self.mem();
        match cmd {
            Cmd::Open => self.pick_and_open(),
            Cmd::OpenRecent(i) => {
                if let Some(e) = self.recent.entries.get(i).cloned() {
                    self.open_path(&e.path_buf(), None);
                }
            }
            Cmd::Save => {
                self.save_active(false);
            }
            Cmd::SaveAs => {
                self.save_active(true);
            }
            Cmd::CloseTab => {
                let a = self.active;
                self.close_tab(a, false);
            }
            Cmd::NextTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + 1) % self.tabs.len();
                }
            }
            Cmd::PrevTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
                }
            }
            Cmd::Quit => {
                self.try_quit();
                if self.dialog.is_none() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Cmd::Undo => {
                if let Some(tab) = self.tab() {
                    let l = tab.undo();
                    if let Some(l) = l {
                        self.say(format!("{}: {l}", t("undone")));
                    }
                }
                self.text_ui.key = None;
            }
            Cmd::Redo => {
                if let Some(tab) = self.tab() {
                    let l = tab.redo();
                    if let Some(l) = l {
                        self.say(format!("{}: {l}", t("redone")));
                    }
                }
                self.text_ui.key = None;
            }
            Cmd::Copy | Cmd::Cut => {
                let cut = cmd == Cmd::Cut;
                // text selected with the mouse?
                let text = self.tab_ref().and_then(|tb| tb.text_sel.map(|s| (s, tb))).map(|(s, tb)| {
                    let (a, b) = s.ordered();
                    tb.text_range(s.page, a, b).0.replace("\r\n", "\n").replace('\r', "\n")
                });
                if let Some(txt) = text.filter(|s| !s.is_empty()) {
                    ctx.copy_text(txt);
                    self.say(t("text_copied"));
                } else if let Some((page, items)) = self.sel_items() {
                    let r = self.tab().map(|tb| tb.copy_items(page, &items));
                    match r {
                        Some(Ok(c)) => {
                            // also copy the text of the text blocks to the system clipboard
                            let txt: Vec<String> = c.items.iter().filter_map(|i| if let crate::edit::ClipItem::Text { text, .. } = i { Some(text.clone()) } else { None }).collect();
                            if !txt.is_empty() {
                                ctx.copy_text(txt.join("\n"));
                            }
                            self.clip = Some(c);
                            self.say(t("copied"));
                            if cut {
                                self.edit(true, |tb, mem| tb.delete_items(page, &items, mem));
                            }
                        }
                        Some(Err(e)) => self.err(e),
                        None => {}
                    }
                }
            }
            Cmd::Paste => {
                let Some(clip) = self.clip.clone() else {
                    self.run_cmd(Cmd::PasteImage, ctx);
                    return;
                };
                let (page, at) = self.paste_target();
                self.edit(true, |tb, mem| tb.paste_clip(page, &clip, at, mem).map(|_| ()));
            }
            Cmd::PasteImage => self.paste_system_image(),
            Cmd::Duplicate => {
                if let Some((page, items)) = self.sel_items() {
                    let r = self.tab().map(|tb| tb.copy_items(page, &items));
                    if let Some(Ok(c)) = r {
                        self.edit(true, |tb, mem| tb.paste_clip(page, &c, None, mem).map(|_| ()));
                    }
                }
            }
            Cmd::Delete => self.delete_selection(),
            Cmd::SelectAll => {
                if let Some(tab) = self.tab() {
                    let page = tab.view.page;
                    if let Some(info) = tab.info(page) {
                        let area = info.geom.w * info.geom.h;
                        tab.sel.clear();
                        tab.sel.page = page;
                        tab.sel.items = info.objects.iter().filter(|o| r4_area(&o.rect) < area * 0.9).map(|o| o.idx).collect();
                    }
                }
                self.text_ui.key = None;
            }
            Cmd::Find => {
                self.search_open = true;
                self.search_focus = true;
                self.side_tab_search_hint();
            }
            Cmd::Replace => {
                self.search_open = true;
                self.search_focus = true;
                self.replace_mode = true;
            }
            Cmd::FindNext => {
                if let Some(tab) = self.tab() {
                    tab.next_match();
                }
            }
            Cmd::FindPrev => {
                if let Some(tab) = self.tab() {
                    tab.prev_match();
                }
            }
            // ----------------------------------------------------- objects
            Cmd::Group => {
                if let Some((page, items)) = self.sel_items() {
                    let mut new = None;
                    self.edit(true, |tb, mem| tb.group_items(page, &items, mem).map(|i| new = Some(i)));
                    if let (Some(i), Some(tb)) = (new, self.tab()) {
                        tb.sel.page = page;
                        tb.sel.items = vec![i];
                    }
                }
            }
            Cmd::Ungroup => {
                if let Some((page, items)) = self.sel_items() {
                    self.edit(true, |tb, mem| {
                        for i in items {
                            tb.ungroup_item(page, i, mem)?;
                        }
                        Ok(())
                    });
                }
            }
            Cmd::BringFront | Cmd::SendBack | Cmd::BringForward | Cmd::SendBackward => {
                let order = match cmd {
                    Cmd::BringFront => ZOrder::Front,
                    Cmd::SendBack => ZOrder::Back,
                    Cmd::BringForward => ZOrder::Forward,
                    _ => ZOrder::Backward,
                };
                if let Some((page, items)) = self.sel_items() {
                    self.edit(true, |tb, mem| tb.z_order(page, &items, order, mem));
                }
            }
            Cmd::AlignLeft | Cmd::AlignCenter | Cmd::AlignRight | Cmd::AlignTop | Cmd::AlignMiddle | Cmd::AlignBottom => {
                let k = match cmd {
                    Cmd::AlignLeft => AlignKind::Left,
                    Cmd::AlignCenter => AlignKind::HCenter,
                    Cmd::AlignRight => AlignKind::Right,
                    Cmd::AlignTop => AlignKind::Top,
                    Cmd::AlignMiddle => AlignKind::VCenter,
                    _ => AlignKind::Bottom,
                };
                if let Some((page, items)) = self.sel_items() {
                    self.edit(false, |tb, mem| tb.align_items(page, &items, k, false, mem));
                }
            }
            Cmd::DistributeH | Cmd::DistributeV => {
                let d = if cmd == Cmd::DistributeH { Distribute::Horizontal } else { Distribute::Vertical };
                if let Some((page, items)) = self.sel_items() {
                    self.edit(false, |tb, mem| tb.distribute_items(page, &items, d, mem));
                }
            }
            Cmd::Rotate90 | Cmd::RotateMinus90 => {
                let deg = if cmd == Cmd::Rotate90 { -90.0 } else { 90.0 };
                if let Some((page, items)) = self.sel_items() {
                    self.edit(false, |tb, mem| tb.rotate_items(page, &items, deg, mem));
                }
            }
            Cmd::FlipH | Cmd::FlipV => {
                if let Some((page, items)) = self.sel_items() {
                    self.edit(false, |tb, mem| tb.flip_items(page, &items, cmd == Cmd::FlipH, mem));
                }
            }
            // ---------------------------------------------------------- view
            Cmd::ZoomIn | Cmd::ZoomOut => {
                if let Some(tab) = self.tab() {
                    let f = if cmd == Cmd::ZoomIn { 1.2 } else { 1.0 / 1.2 };
                    tab.view.fit = FitMode::None;
                    let z = tab.view.zoom * f;
                    view::set_zoom(tab, z, None);
                }
            }
            Cmd::Zoom100 => {
                if let Some(tab) = self.tab() {
                    tab.view.fit = FitMode::None;
                    view::set_zoom(tab, 1.0, None);
                }
            }
            Cmd::FitWidth => {
                if let Some(tab) = self.tab() {
                    tab.view.fit = FitMode::Width;
                    tab.view.zoom_changed_at = Some(std::time::Instant::now());
                }
            }
            Cmd::FitPage => {
                if let Some(tab) = self.tab() {
                    tab.view.fit = FitMode::Page;
                    tab.view.zoom_changed_at = Some(std::time::Instant::now());
                }
            }
            Cmd::ViewSingle => self.set_view_mode(ViewMode::Single),
            Cmd::ViewContinuous => self.set_view_mode(ViewMode::Continuous),
            Cmd::ViewTwo => self.set_view_mode(ViewMode::TwoPages),
            Cmd::ViewTwoContinuous => self.set_view_mode(ViewMode::TwoPagesContinuous),
            Cmd::ToggleCover => {
                self.prefs.cover_page = !self.prefs.cover_page;
                self.prefs.save();
            }
            Cmd::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            Cmd::Presentation => {
                self.presentation = !self.presentation;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.presentation));
                if self.presentation {
                    self.pres_prev_mode = Some(self.prefs.view_mode);
                    self.prefs.view_mode = ViewMode::Single;
                    if let Some(tab) = self.tab() {
                        tab.view.fit = FitMode::Page;
                    }
                } else if let Some(m) = self.pres_prev_mode.take() {
                    self.prefs.view_mode = m;
                    if let Some(tab) = self.tab() {
                        tab.view.fit = FitMode::Width;
                    }
                }
            }
            Cmd::ReadingMode => {
                if self.reading {
                    self.set_tool(Tool::Select);
                } else {
                    self.set_tool(Tool::Hand);
                    self.reading = true;
                    if let Some(tb) = self.tab() {
                        tb.sel.clear();
                        tb.text_sel = None;
                    }
                    self.say(t("reading_hint"));
                }
            }
            Cmd::ToggleSidePanel => self.show_side = !self.show_side,
            Cmd::ToggleProps => self.show_props = !self.show_props,
            Cmd::ToggleRulers => {
                self.prefs.show_rulers = !self.prefs.show_rulers;
                self.prefs.save();
            }
            Cmd::ToggleNight => {
                self.prefs.night_mode = !self.prefs.night_mode;
                self.prefs.save();
            }
            Cmd::ToggleBoxes => {
                self.prefs.show_boxes = !self.prefs.show_boxes;
                self.prefs.save();
            }
            Cmd::ToggleParagraphs => {
                self.paragraph_mode = !self.paragraph_mode;
                let on = self.paragraph_mode;
                for tb in &mut self.tabs {
                    tb.set_paragraph_mode(on);
                }
                self.say(if on { t("para_mode_on") } else { t("line_mode_on") });
            }
            // ------------------------------------------------- navigation
            Cmd::FirstPage => {
                if let Some(tab) = self.tab() {
                    tab.goto_page(0);
                }
            }
            Cmd::LastPage => {
                if let Some(tab) = self.tab() {
                    let n = tab.page_count();
                    tab.goto_page(n.saturating_sub(1));
                }
            }
            Cmd::NextPage | Cmd::PrevPage => {
                if let Some(tab) = self.tab() {
                    let two = false;
                    let _ = two;
                    let step = 1usize;
                    let p = if cmd == Cmd::NextPage { tab.view.page + step } else { tab.view.page.saturating_sub(step) };
                    tab.goto_page(p);
                }
            }
            Cmd::GotoPage => self.dialog = Some(Dialog::GotoPage { text: String::new() }),
            // --------------------------------------------------- tools
            Cmd::SetTool(tool) => self.set_tool(tool),
            Cmd::Preferences => self.dialog = Some(Dialog::Prefs),
            Cmd::CommandPalette => self.dialog = Some(Dialog::Palette),
            Cmd::About => self.dialog = Some(Dialog::About),
            Cmd::OpenLog => crate::sys::open_file(&crate::logx::data_dir().join("rustpdf.log")),
            other => self.run_cmd_ext(other, ctx, mem),
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.reading = false;
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.drag = crate::tab::Drag::None;
            tab.inline = None;
            tab.poly.clear();
            if !matches!(tool, Tool::Measure) {
                tab.measure.clear();
            }
            if !tool.is_markup() && tool != Tool::TextSelect {
                tab.text_sel = None;
            }
        }
    }

    pub fn set_view_mode(&mut self, m: ViewMode) {
        self.prefs.view_mode = m;
        self.prefs.save();
        if let Some(tab) = self.tab() {
            let p = tab.view.page;
            tab.view.scroll_to = Some(p);
        }
    }

    /// Page and paste point: the center of the visible area of the current page.
    fn paste_target(&mut self) -> (usize, Option<(f32, f32)>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return (0, None) };
        let page = tab.view.page;
        let at = tab.info(page).map(|info| {
            let g = info.geom;
            let (x, y) = g.pt_from_view(g.w * 0.5, g.h * 0.4);
            (x, y)
        });
        (page, at)
    }

    pub fn delete_selection(&mut self) {
        let mem = self.mem();
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if !tab.sel.items.is_empty() {
            let (page, items) = (tab.sel.page, tab.sel.items.clone());
            self.edit(true, |tb, m| tb.delete_items(page, &items, m));
        } else if tab.sel.annot.is_some() {
            self.delete_selected_annot(mem);
        } else if !tab.redactions.is_empty() {
            tab.redactions.pop();
        }
    }

    fn side_tab_search_hint(&mut self) {}

    // ---------------------------------------------------------------- messages

    pub fn handle_msgs(&mut self, msgs: Vec<Msg>, ctx: &egui::Context) {
        for m in msgs {
            self.handle_msg(m, ctx);
        }
    }

    fn handle_msg(&mut self, m: Msg, ctx: &egui::Context) {
        match m {
            Msg::Status(s) => self.say(s),
            Msg::Error(s) => self.err(s),
            Msg::OpenUri(u) => {
                if !crate::sys::open_url(&u) {
                    self.err(t("link_blocked"));
                }
            }
            Msg::Cmd(c) => self.run_cmd(c, ctx),
            Msg::SetTool(t) => self.set_tool(t),
            Msg::SelectionChanged => {
                self.text_ui.key = None;
                self.text_ui.geo_key = None;
            }
            Msg::CopyText(s) => ctx.copy_text(s),
            Msg::ContextAt { page, x, y } => self.ctx_menu = Some((page, x, y)),
            Msg::MoveSelection { page, dx, dy } => {
                if let Some(tab) = self.tabs.get(self.active) {
                    let items = tab.sel.items.clone();
                    self.edit(false, |tb, mem| tb.move_items(page, &items, dx, dy, mem));
                }
            }
            Msg::ResizeSelection { page, old, new, reflow } => {
                let Some(tab) = self.tabs.get(self.active) else { return };
                let items = tab.sel.items.clone();
                if reflow && items.len() == 1 {
                    self.edit(true, |tb, mem| tb.reflow_text(page, items[0], (new[0], new[2]), mem).map(|n| {
                        if let Some(n) = n {
                            tb.sel.page = page;
                            tb.sel.items = vec![n];
                        }
                    }));
                } else {
                    self.edit(false, |tb, mem| tb.resize_items(page, &items, old, new, mem));
                }
            }
            Msg::PathPointMoved { page, item, seg, kind, x, y } => {
                self.edit(false, |tb, mem| tb.edit_path_point(page, item, seg, kind, x, y, mem));
            }
            Msg::ShapeDone { page, kind, a, b } => self.add_shape(page, kind, a, b, &[]),
            Msg::PolygonDone { page, pts } => self.add_shape(page, ShapeKind::Polygon, [0.0; 2], [0.0; 2], &pts),
            Msg::InkDone { page, pts } => self.add_shape(page, ShapeKind::Freehand, [0.0; 2], [0.0; 2], &pts),
            Msg::CommitNewText { page, x, y, text } => {
                let st = self.style.text.clone();
                let mut new = None;
                self.edit(true, |tb, mem| tb.add_text(page, x, y, &text, &st, mem).map(|i| new = Some(i)));
                if let (Some(i), Some(tb)) = (new, self.tab()) {
                    tb.sel.page = page;
                    tb.sel.items = vec![i];
                }
                self.set_tool(Tool::Select);
            }
            Msg::CommitObjectText { page, idx, text } => {
                let mut new = None;
                self.edit(true, |tb, mem| {
                    let (_, st, _) = tb.read_text_style(page, idx).ok_or("text not found")?;
                    tb.apply_text(page, idx, &text, &st, mem).map(|n| new = n)
                });
                if let (Some(i), Some(tb)) = (new, self.tab()) {
                    tb.sel.page = page;
                    tb.sel.items = vec![i];
                }
            }
            Msg::AddImageAt { page, x, y } => self.add_image_dialog(page, x, y),
            Msg::ReplaceImage { page, idx } => self.replace_image_dialog(page, idx),
            Msg::ExportZone { page, rect } => self.export_zone(page, rect),
            other => self.handle_msg_ext(other, ctx),
        }
    }

    fn add_shape(&mut self, page: usize, kind: ShapeKind, a: [f32; 2], b: [f32; 2], pts: &[[f32; 2]]) {
        let st = &self.style;
        let stroke = [st.shape_stroke[0], st.shape_stroke[1], st.shape_stroke[2], st.shape_opacity];
        let fill = st.shape_fill.map(|f| [f[0], f[1], f[2], st.shape_opacity]);
        let mut d = match kind {
            ShapeKind::Rect => PathData::rect(r4_from_pts(a, b)),
            ShapeKind::Ellipse => PathData::ellipse(r4_from_pts(a, b)),
            ShapeKind::Line => PathData::line((a[0], a[1]), (b[0], b[1])),
            ShapeKind::Arrow => PathData::arrow((a[0], a[1]), (b[0], b[1]), (st.shape_width * 5.0).max(9.0)),
            ShapeKind::Polygon => PathData::polyline(&pts.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>(), true),
            ShapeKind::Freehand => PathData::freehand(&pts.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>()),
        };
        d.stroke = Some(stroke);
        d.width = st.shape_width;
        d.dash = st.dash_array();
        d.fill = match kind {
            ShapeKind::Arrow => Some(stroke),
            ShapeKind::Line | ShapeKind::Freehand => None,
            _ => fill,
        };
        let label = t(Tool::Shape(kind).label_key());
        let mut new = None;
        self.edit(true, |tb, mem| tb.add_path(page, &d, label, mem).map(|i| new = Some(i)));
        if let (Some(i), Some(tb)) = (new, self.tab()) {
            tb.sel.page = page;
            tb.sel.items = vec![i];
        }
        if kind != ShapeKind::Freehand {
            // stay on the tool to chain several, like drawing editors; Esc goes back to selection
        }
    }

    pub fn add_image_dialog(&mut self, page: usize, x: f32, y: f32) {
        let Some(file) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff"]).pick_file() else { return };
        match crate::export::open_image_safe(&file) {
            Ok(img) => {
                let w = 200.0f32.min(img.width() as f32 * 0.75).max(40.0);
                let mut new = None;
                self.edit(true, |tb, mem| tb.add_image(page, x, y, &img, w, mem).map(|i| new = Some(i)));
                if let (Some(i), Some(tb)) = (new, self.tab()) {
                    tb.sel.page = page;
                    tb.sel.items = vec![i];
                }
                self.set_tool(Tool::Select);
            }
            Err(e) => self.err(format!("{}: {e}", t("err_image"))),
        }
    }

    pub fn replace_image_dialog(&mut self, page: usize, idx: usize) {
        let Some(file) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff"]).pick_file() else { return };
        match crate::export::open_image_safe(&file) {
            Ok(img) => self.edit(false, |tb, mem| tb.replace_image(page, idx, &img, mem)),
            Err(e) => self.err(format!("{}: {e}", t("err_image"))),
        }
    }

    /// Pastes an image from the system clipboard in the center of the current page.
    pub fn paste_system_image(&mut self) {
        let img = arboard::Clipboard::new().ok().and_then(|mut c| c.get_image().ok());
        let Some(img) = img else {
            self.say(t("clipboard_empty"));
            return;
        };
        let rgba = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned());
        let Some(rgba) = rgba else { return };
        let dynimg = image::DynamicImage::ImageRgba8(rgba);
        let (page, at) = self.paste_target();
        let (x, y) = at.unwrap_or((100.0, 700.0));
        let w = (dynimg.width() as f32 * 0.75).clamp(40.0, 360.0);
        let h = w * dynimg.height() as f32 / dynimg.width().max(1) as f32;
        let mut new = None;
        self.edit(true, |tb, mem| tb.add_image(page, x - w / 2.0, y + h / 2.0, &dynimg, w, mem).map(|i| new = Some(i)));
        if let (Some(i), Some(tb)) = (new, self.tab()) {
            tb.sel.page = page;
            tb.sel.items = vec![i];
        }
    }

    pub fn export_zone(&mut self, page: usize, rect: R4) {
        let dpi = self.prefs.export_dpi;
        let isolate = self.export_isolate;
        let r = self.tab().map(|tb| tb.export_region_image(page, rect, isolate, dpi));
        match r {
            Some(Ok(img)) => self.save_image_dialog(img, "area.png"),
            Some(Err(e)) => self.err(e),
            None => {}
        }
    }

    pub fn save_image_dialog(&mut self, img: image::DynamicImage, name: &str) {
        let Some(mut path) = rfd::FileDialog::new()
            .set_file_name(name)
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("WebP", &["webp"])
            .add_filter("BMP", &["bmp"])
            .add_filter("TIFF", &["tif", "tiff"])
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("png");
        }
        match crate::export::save_image_file(&img, &path) {
            Ok(_) => self.say(format!("{}: {} ({}×{} px)", t("exported"), path.display(), img.width(), img.height())),
            Err(e) => self.err(format!("{}: {e}", t("err_export"))),
        }
    }

    #[allow(dead_code)]
    fn _paths(_: PathBuf) {}
}
