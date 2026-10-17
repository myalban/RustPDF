//! Protected view inside the application: opening, closing and commands.

use crate::app::App;
use crate::i18n::t;
use crate::logx;
use crate::pdfx;
use crate::prefs::clean_path;
use crate::protected::{Action, ProtectedView};
use crate::tools::Cmd;
use std::path::Path;

impl App {
    /// Opens a document. In protected view (the default) it is parsed by the isolated worker; otherwise in the editor.
    pub fn open_path(&mut self, path: &Path, password: Option<String>) {
        if self.prefs.protected_view && self.try_open_protected(path, password.clone()) {
            return;
        }
        self.open_editable(path, password, true);
    }

    /// Returns `true` when the request was handled (opened, or refused with a message).
    fn try_open_protected(&mut self, path: &Path, password: Option<String>) -> bool {
        let clean = clean_path(path);
        if let Some(i) = self.tabs.iter().position(|t| t.path.as_ref().map(|p| crate::prefs::same_path(p, &clean)).unwrap_or(false)) {
            self.active = i;
            self.pactive = None;
            self.say(t("already_open"));
            return true;
        }
        if let Some(j) = self.pviews.iter().position(|v| crate::prefs::same_path(&v.path, &clean)) {
            self.pactive = Some(j);
            self.say(t("already_open"));
            return true;
        }
        let Some(ctx) = self.egui_ctx.clone() else { return false };
        let size = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => return false, // the editor path reports a missing file
        };
        if size > pdfx::MAX_PDF_BYTES {
            self.err(format!("{} ({} MB)", t("protected_too_large"), size / 1_048_576));
            return true;
        }
        match std::fs::read(path) {
            Ok(bytes) => {
                let mut view = ProtectedView::new(clean.clone(), bytes, password.clone().unwrap_or_default(), &ctx);
                view.password = password;
                self.pviews.push(view);
                self.pactive = Some(self.pviews.len() - 1);
                self.recent.add(&clean);
                self.say(format!("{} — {}", t("opened"), clean.display()));
                logx::info(format!("opened in protected view: {}", clean.display()));
            }
            Err(e) => self.err(format!("{}: {e}", t("err_open"))),
        }
        true
    }

    pub fn close_protected(&mut self, j: usize) {
        if j >= self.pviews.len() {
            return;
        }
        let v = self.pviews.remove(j);
        v.shutdown();
        self.pactive = match self.pactive {
            Some(a) if a == j => {
                if self.pviews.is_empty() { None } else { Some(j.min(self.pviews.len() - 1)) }
            }
            Some(a) if a > j => Some(a - 1),
            other => other,
        };
    }

    /// Per-frame work: answers from the workers.
    pub fn poll_protected(&mut self, ctx: &eframe::egui::Context) {
        let night = self.prefs.night_mode;
        for v in &mut self.pviews {
            v.poll(ctx, night);
        }
    }

    pub fn handle_protected_action(&mut self, j: usize, action: Action) {
        match action {
            Action::Close => self.close_protected(j),
            Action::OpenUri(u) => {
                if !crate::sys::open_url(&u) {
                    self.err(t("link_blocked"));
                }
            }
            Action::EnableEditing => {
                let Some(v) = self.pviews.get(j) else { return };
                let (path, pw) = (v.path.clone(), v.password.clone());
                self.close_protected(j);
                self.open_editable(&path, pw, false);
            }
            Action::Retry(pw) => {
                let Some(ctx) = self.egui_ctx.clone() else { return };
                let Some(v) = self.pviews.get(j) else { return };
                let path = v.path.clone();
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let mut nv = ProtectedView::new(path, bytes, pw.clone(), &ctx);
                        nv.password = Some(pw);
                        let old = std::mem::replace(&mut self.pviews[j], nv);
                        old.shutdown();
                    }
                    Err(e) => self.err(format!("{}: {e}", t("err_open"))),
                }
            }
        }
    }

    /// While a protected view is shown, only reading commands are available. Returns `true` when the command
    /// was handled here.
    pub fn protected_command(&mut self, cmd: &Cmd) -> bool {
        let Some(j) = self.pactive.filter(|j| *j < self.pviews.len()) else { return false };
        match cmd {
            Cmd::Open | Cmd::OpenRecent(_) | Cmd::OpenUnprotected | Cmd::Quit | Cmd::Preferences | Cmd::CommandPalette | Cmd::About | Cmd::CheckUpdates | Cmd::OpenLog | Cmd::Fullscreen => false,
            Cmd::CloseTab => {
                self.close_protected(j);
                true
            }
            Cmd::ZoomIn => {
                self.pviews[j].zoom_by(1.25);
                true
            }
            Cmd::ZoomOut => {
                self.pviews[j].zoom_by(0.8);
                true
            }
            Cmd::Zoom100 => {
                self.pviews[j].set_zoom(1.0);
                true
            }
            Cmd::FitWidth => {
                self.pviews[j].fit_width = true;
                true
            }
            Cmd::FitPage => {
                self.pviews[j].fit_page();
                true
            }
            Cmd::FirstPage => {
                self.pviews[j].goto_page(0);
                true
            }
            Cmd::LastPage => {
                self.pviews[j].goto_page(usize::MAX);
                true
            }
            Cmd::NextPage => {
                let p = self.pviews[j].page + 1;
                self.pviews[j].goto_page(p);
                true
            }
            Cmd::PrevPage => {
                let p = self.pviews[j].page.saturating_sub(1);
                self.pviews[j].goto_page(p);
                true
            }
            _ => {
                self.say(t("protected_blocked"));
                true
            }
        }
    }
}

impl App {
    /// File ▸ Open for editing: skips protected view (the user trusts the file).
    pub fn pick_and_open_editable(&mut self) {
        let mut dlg = rfd::FileDialog::new().add_filter("PDF", &["pdf"]);
        if !self.prefs.default_dir.is_empty() {
            dlg = dlg.set_directory(&self.prefs.default_dir);
        }
        if let Some(files) = dlg.pick_files() {
            for f in files {
                self.open_editable(&f, None, false);
            }
        }
    }
}
