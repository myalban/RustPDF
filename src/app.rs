//! Application: main window, tabs, commands, dialogs.

use crate::dialogs::Dialog;
use crate::edit::{AlignKind, Clip, Distribute, ZOrder};
use crate::edit_text::TextStyle;
use crate::i18n::{self, t};
use crate::icons;
use crate::logx;
use crate::pdfx::{self, OpenErr};
use crate::prefs::{Lang, Prefs, Recent, ThemePref, ViewMode};
use crate::render::RenderCache;
use crate::tab::{FitMode, Tab};
use crate::tools::{Cmd, ShapeKind, Tool, ToolStyle};
use crate::view::{self, Msg, ViewEnv};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use pdfium_render::prelude::Pdfium;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SideTab {
    Pages,
    Bookmarks,
    Attachments,
    Layers,
    Comments,
    Search,
}

/// Values being edited in the properties panel (text).
#[derive(Default)]
pub struct TextUi {
    /// (tab, page, item) whose values are loaded.
    pub key: Option<(u64, usize, usize)>,
    pub text: String,
    pub style: Option<TextStyle>,
    pub orig_font: String,
    /// Numeric position/size values (chosen unit) of the selection.
    pub geo_key: Option<(u64, usize, Vec<usize>)>,
    pub geo: [f32; 4],
}

pub struct App {
    pub pdfium: &'static Pdfium,
    pub prefs: Prefs,
    pub recent: Recent,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub tool: Tool,
    pub cache: RenderCache,
    pub clip: Option<Clip>,
    pub style: ToolStyle,
    pub status: String,
    pub status_at: Instant,
    pub dialog: Option<Dialog>,
    pub side_tab: SideTab,
    pub show_side: bool,
    pub show_props: bool,
    pub search_open: bool,
    pub search_focus: bool,
    pub palette_query: String,
    pub text_ui: TextUi,
    pub presentation: bool,
    pub fullscreen: bool,
    pub applied_prefs: Option<(Lang, ThemePref)>,
    pub fonts_ready: bool,
    pub last_autosave_check: Instant,
    pub pending_close: Option<usize>,
    pub quit_requested: bool,
    pub thumb_budget_frame: u64,
    pub ctx_menu: Option<(usize, f32, f32)>,
    pub ptr_page_pos: Option<(usize, f32, f32)>,
    pub startup_files: Vec<PathBuf>,
    pub replace_mode: bool,
    pub replace_text: String,
    pub pres_prev_mode: Option<ViewMode>,
    pub paragraph_mode: bool,
    pub export_isolate: bool,
    pub title_shown: String,
    pub page_sel: Vec<usize>,
    pub thumb_drag: Option<usize>,
    pub pending_cmds: Vec<Cmd>,
    pub pending_msgs: Vec<Msg>,
    pub force_quit: bool,
    pub ctx_menu_pos: Option<egui::Pos2>,
    pub ctx_menu_age: Instant,
    pub ctx_menu_age_set: Option<(usize, f32, f32)>,
    pub pending_page_move: Option<(usize, usize)>,
    pub font_filter: String,
    pub img_crop: [f32; 4],
    pub img_adj: (i32, f32, bool),
    pub img_max_px: u32,
    pub field_buf: ((usize, usize), String),
    pub new_field_name: String,
    pub dlg2: Option<crate::dlg_common::Dlg2>,
    pub update_rx: Option<std::sync::mpsc::Receiver<crate::updater::Outcome>>,
    pub update_started: bool,
    /// Message and version of a newer release found by the automatic check.
    pub update_found: Option<(String, String)>,
    pub last_input: Instant,
    pub reading: bool,
    pub pc: crate::panels_doc::PanelCache,
    pub sig_payload: Option<crate::dlg_common::SigPayload>,
}

impl App {
    pub fn new(pdfium: &'static Pdfium, files: Vec<PathBuf>) -> App {
        let prefs = Prefs::load();
        i18n::set_lang(prefs.lang);
        let cache = RenderCache::new(prefs.cache_mb);
        let mut style = ToolStyle::default();
        style.text.size = 14.0;
        App {
            pdfium,
            cache,
            recent: Recent::load(),
            tabs: vec![],
            active: 0,
            tool: Tool::Select,
            clip: None,
            style,
            status: String::new(),
            status_at: Instant::now(),
            dialog: None,
            side_tab: SideTab::Pages,
            show_side: true,
            show_props: true,
            search_open: false,
            search_focus: false,
            palette_query: String::new(),
            text_ui: TextUi::default(),
            presentation: false,
            fullscreen: false,
            applied_prefs: None,
            fonts_ready: false,
            last_autosave_check: Instant::now(),
            pending_close: None,
            quit_requested: false,
            thumb_budget_frame: 0,
            ctx_menu: None,
            ptr_page_pos: None,
            startup_files: files,
            replace_mode: false,
            replace_text: String::new(),
            pres_prev_mode: None,
            paragraph_mode: true,
            export_isolate: true,
            title_shown: String::new(),
            page_sel: vec![],
            thumb_drag: None,
            pending_cmds: vec![],
            pending_msgs: vec![],
            force_quit: false,
            ctx_menu_pos: None,
            ctx_menu_age: Instant::now(),
            ctx_menu_age_set: None,
            pending_page_move: None,
            font_filter: String::new(),
            img_crop: [0.0, 0.0, 1.0, 1.0],
            img_adj: (0, 0.0, false),
            img_max_px: 1600,
            field_buf: ((usize::MAX, 0), String::new()),
            new_field_name: "field".into(),
            dlg2: None,
            update_rx: None,
            update_started: false,
            update_found: None,
            last_input: Instant::now(),
            reading: false,
            pc: Default::default(),
            sig_payload: None,
            prefs,
        }
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_at = Instant::now();
    }

    pub fn err(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        logx::error(&m);
        self.status = m;
        self.status_at = Instant::now();
    }

    pub fn tab(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn tab_ref(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn mem(&self) -> u32 {
        self.prefs.undo_mem_mb
    }

    // -------------------------------------------------------------- opening

    pub fn initial_zoom(&self) -> (f32, FitMode) {
        if self.prefs.default_zoom <= 0.01 {
            (1.0, FitMode::Width)
        } else {
            (self.prefs.default_zoom, FitMode::None)
        }
    }

    /// Opens a file in a new tab (or activates the existing tab).
    pub fn open_path(&mut self, path: &Path, password: Option<String>) {
        let clean = crate::prefs::clean_path(path);
        if let Some(i) = self.tabs.iter().position(|t| t.path.as_ref().map(|p| crate::prefs::same_path(p, &clean)).unwrap_or(false)) {
            self.active = i;
            self.say(t("already_open"));
            return;
        }
        match pdfx::open_path(self.pdfium, path, password.as_deref()) {
            Ok(doc) => {
                let (zoom, fit) = self.initial_zoom();
                let mut tab = Tab::new(self.pdfium, doc, Some(clean.clone()), password, zoom);
                tab.view.fit = fit;
                if self.prefs.restore_view {
                    if let Some(e) = self.recent.get(&clean) {
                        if e.page > 0 && e.page < tab.page_count() {
                            tab.view.page = e.page;
                            tab.view.scroll_to = Some(e.page);
                        }
                        if e.zoom > 0.0 {
                            tab.view.zoom = e.zoom;
                            tab.view.fit = FitMode::None;
                        }
                    }
                }
                tab.paragraphs = true;
                self.recent.add(&clean);
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                self.say(format!("{} — {}", t("opened"), clean.display()));
                logx::info(format!("opened: {}", clean.display()));
            }
            Err(OpenErr::Password) => {
                self.dialog = Some(Dialog::Password { path: path.to_path_buf(), text: String::new(), wrong: password.is_some() });
            }
            Err(OpenErr::Damaged) => {
                self.dialog = Some(Dialog::Repair { path: path.to_path_buf() });
            }
            Err(OpenErr::Missing(m)) => {
                self.recent.remove(path);
                self.err(format!("{}: {m}", t("err_missing")));
            }
            Err(OpenErr::Other(m)) => self.err(format!("{}: {m}", t("err_open"))),
        }
    }

    pub fn pick_and_open(&mut self) {
        let dir = if self.prefs.default_dir.is_empty() { None } else { Some(PathBuf::from(&self.prefs.default_dir)) };
        let mut dlg = rfd::FileDialog::new().add_filter("PDF", &["pdf"]);
        if let Some(d) = dir {
            dlg = dlg.set_directory(d);
        }
        if let Some(files) = dlg.pick_files() {
            if let Some(parent) = files.first().and_then(|f| f.parent()) {
                self.prefs.default_dir = parent.to_string_lossy().into_owned();
            }
            for f in files {
                self.open_path(&f, None);
            }
        }
    }

    pub fn close_tab(&mut self, i: usize, force: bool) {
        if i >= self.tabs.len() {
            return;
        }
        if !force && self.tabs[i].dirty {
            self.pending_close = Some(i);
            self.dialog = Some(Dialog::ConfirmClose { tab: i });
            return;
        }
        let tab = self.tabs.remove(i);
        if let Some(p) = &tab.path {
            self.recent.remember_view(p, tab.view.page, if tab.view.fit == FitMode::None { tab.view.zoom } else { 0.0 });
        }
        if let Some(f) = &tab.autosave_file {
            let _ = std::fs::remove_file(f);
        }
        self.cache.purge_tab(tab.id);
        if self.active >= self.tabs.len() && self.active > 0 {
            self.active = self.tabs.len() - 1;
        }
    }

    pub fn save_active(&mut self, as_new: bool) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        let target = if as_new || tab.path.is_none() {
            let mut d = rfd::FileDialog::new().add_filter("PDF", &["pdf"]);
            if let Some(p) = &tab.path {
                if let Some(n) = p.file_name() {
                    d = d.set_file_name(n.to_string_lossy().to_string());
                }
            }
            d.save_file()
        } else {
            tab.path.clone()
        };
        let Some(mut out) = target else { return false };
        if out.extension().is_none() {
            out.set_extension("pdf");
        }
        match tab.save_to(&out) {
            Ok(_) => {
                if let Some(f) = tab.autosave_file.take() {
                    let _ = std::fs::remove_file(f);
                }
                self.recent.add(&out);
                self.say(format!("{}: {}", t("saved"), out.display()));
                true
            }
            Err(e) => {
                self.err(format!("{}: {e}", t("err_save")));
                false
            }
        }
    }
}

#[allow(dead_code)]
fn _unused(_: Pos2, _: Rect, _: Vec2, _: Color32, _: ZOrder, _: AlignKind, _: Distribute, _: ShapeKind, _: Cmd, _: ViewMode, _: Msg, _: &ViewEnv, _: &str) {
    let _ = icons::GEAR;
    let _ = view::PT_TO_PX;
    let _ = Duration::from_secs(0);
}
