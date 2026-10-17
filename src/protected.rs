//! Protected view: shows a document that is parsed and rendered by the isolated worker process.
//!
//! Nothing here touches PDFium: pages arrive as bitmaps from a locked-down process, so a hostile PDF cannot reach the
//! rest of the application or the system. Only reading is possible; "Enable editing" opens the file in the normal,
//! in-process editor (the user's decision to trust the document).

use crate::client::Client;
use crate::i18n::t;
use crate::icons;
use crate::pdfx::Rgba;
use crate::render;
use crate::worker_proto::{LinkTarget, Req, Resp, WLink};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, TextureHandle, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MARGIN: f32 = 18.0;
const PT_TO_PX: f32 = 96.0 / 72.0;
const MAX_TEXTURES: usize = 28;
const MAX_IN_FLIGHT: usize = 3;
const WATCHDOG: Duration = Duration::from_secs(40);

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Starting,
    NeedPassword { wrong: bool },
    Ready,
    Failed(String),
}

/// What the application must do after a frame of the protected view.
#[derive(Debug, PartialEq)]
pub enum Action {
    EnableEditing,
    OpenUri(String),
    Retry(String),
    Close,
}

pub struct ProtectedView {
    pub id: u64,
    pub path: PathBuf,
    pub title: String,
    client: Client,
    pub state: State,
    pub sizes: Vec<(f32, f32)>,
    /// 1.0 = 100 %.
    pub zoom: f32,
    pub fit_width: bool,
    pub page: usize,
    pub isolation: String,
    pub scroll_to: Option<usize>,
    /// Password used to open the document (needed to reopen it for editing).
    pub password: Option<String>,
    avail: Vec2,
    textures: HashMap<(usize, u32), (TextureHandle, u64)>,
    inflight: HashMap<u32, (usize, u32)>,
    next_id: u32,
    links: HashMap<usize, Vec<WLink>>,
    links_inflight: HashMap<u32, usize>,
    last_activity: Instant,
    frame: u64,
    password_input: String,
    scroll_y: f32,
    pending_zoom_ratio: Option<f32>,
}

fn next_view_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

impl ProtectedView {
    pub fn new(path: PathBuf, data: Vec<u8>, password: String, ctx: &egui::Context) -> ProtectedView {
        let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "?".into());
        ProtectedView {
            id: next_view_id(),
            path,
            title,
            client: Client::start(data, password, ctx.clone()),
            state: State::Starting,
            sizes: vec![],
            zoom: 1.0,
            fit_width: true,
            page: 0,
            isolation: String::new(),
            scroll_to: None,
            password: None,
            avail: Vec2::new(800.0, 600.0),
            textures: HashMap::new(),
            inflight: HashMap::new(),
            next_id: 1,
            links: HashMap::new(),
            links_inflight: HashMap::new(),
            last_activity: Instant::now(),
            frame: 0,
            password_input: String::new(),
            scroll_y: 0.0,
            pending_zoom_ratio: None,
        }
    }

    pub fn zoom_by(&mut self, k: f32) {
        self.fit_width = false;
        let z = (self.zoom * k).clamp(0.25, 4.0);
        self.pending_zoom_ratio = Some(z / self.zoom);
        self.zoom = z;
    }

    pub fn set_zoom(&mut self, z: f32) {
        self.fit_width = false;
        self.pending_zoom_ratio = Some(z.clamp(0.25, 4.0) / self.zoom);
        self.zoom = z.clamp(0.25, 4.0);
    }

    /// Zoom that makes the current page fit entirely in the window.
    pub fn fit_page(&mut self) {
        if let Some((w, h)) = self.sizes.get(self.page).copied() {
            let z = ((self.avail.x - 2.0 * MARGIN - 14.0) / (w * PT_TO_PX)).min((self.avail.y - 2.0 * MARGIN) / (h * PT_TO_PX));
            self.set_zoom(z);
        }
    }

    pub fn goto_page(&mut self, p: usize) {
        if !self.sizes.is_empty() {
            self.scroll_to = Some(p.min(self.sizes.len() - 1));
        }
    }

    /// Stops the worker (the document is closed or looked at no more).
    pub fn shutdown(&self) {
        self.client.kill();
    }

    fn scale(&self) -> f32 {
        PT_TO_PX * self.zoom
    }

    /// Processes the answers received from the worker.
    pub fn poll(&mut self, ctx: &egui::Context, night: bool) {
        while let Ok(r) = self.client.rx.try_recv() {
            self.last_activity = Instant::now();
            match r {
                Resp::Ready { isolation } => self.isolation = isolation,
                Resp::Opened { sizes } => {
                    self.sizes = sizes;
                    self.state = State::Ready;
                }
                Resp::NeedPassword { wrong } => self.state = State::NeedPassword { wrong },
                Resp::Failed { msg } => {
                    crate::logx::warn(format!("protected view: {msg}"));
                    self.state = State::Failed(msg);
                    self.inflight.clear();
                }
                Resp::Rendered { id, w, h, rgba } => {
                    if let Some(key) = self.inflight.remove(&id) {
                        let img = Rgba { w: w as usize, h: h as usize, data: rgba };
                        let (tex, _) = render::upload(ctx, format!("pv{}_{}_{}", self.id, key.0, key.1), &img, night);
                        self.textures.insert(key, (tex, self.frame));
                    }
                }
                Resp::Links { id, items } => {
                    if let Some(p) = self.links_inflight.remove(&id) {
                        self.links.insert(p, items);
                    }
                }
                Resp::SelfTest { .. } => {}
                Resp::Error { id, .. } => {
                    self.inflight.remove(&id);
                    if let Some(p) = self.links_inflight.remove(&id) {
                        self.links.insert(p, vec![]);
                    }
                }
            }
        }
        // a worker that stops answering is stopped: a document cannot freeze the application
        let busy = !self.inflight.is_empty() || !self.links_inflight.is_empty() || self.state == State::Starting;
        if busy && !matches!(self.state, State::Failed(_)) && self.last_activity.elapsed() > WATCHDOG {
            self.client.kill();
            self.state = State::Failed("the isolated viewer did not answer in time".into());
            self.inflight.clear();
            self.links_inflight.clear();
        }
        if busy {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn request_render(&mut self, page: usize, bucket: u32) {
        let key = (page, bucket);
        if self.textures.contains_key(&key) || self.inflight.values().any(|k| *k == key) || self.inflight.len() >= MAX_IN_FLIGHT {
            return;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.inflight.insert(id, key);
        self.client.send(Req::Render { id, page: page as u32, width: bucket });
    }

    fn request_links(&mut self, page: usize) {
        if self.links.contains_key(&page) || self.links_inflight.values().any(|p| *p == page) || self.links_inflight.len() >= 2 {
            return;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.links_inflight.insert(id, page);
        self.client.send(Req::Links { id, page: page as u32 });
    }

    fn evict(&mut self) {
        while self.textures.len() > MAX_TEXTURES {
            let oldest = self.textures.iter().min_by_key(|(_, (_, f))| *f).map(|(k, _)| *k);
            match oldest {
                Some(k) => {
                    self.textures.remove(&k);
                }
                None => break,
            }
        }
    }

    /// Draws the banner and the document; returns what the user asked for.
    pub fn show(&mut self, ui: &mut egui::Ui, ppp: f32) -> Option<Action> {
        self.frame += 1;
        let mut action = None;
        egui::Frame::new().fill(Color32::from_rgb(40, 70, 110)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} {}", icons::LOCK, t("protected_banner"))).color(Color32::WHITE));
                if !self.isolation.is_empty() {
                    ui.label(RichText::new(format!("({})", self.isolation)).small().color(Color32::from_rgb(190, 205, 225)));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new(t("protected_enable")).strong()).on_hover_text(t("protected_enable_hint")).clicked() {
                        action = Some(Action::EnableEditing);
                    }
                });
            });
        });
        match self.state.clone() {
            State::Starting => {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.label(t("protected_starting"));
                });
            }
            State::Failed(msg) => {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.colored_label(Color32::from_rgb(220, 90, 90), RichText::new(t("protected_failed")).strong());
                    ui.label(msg);
                    ui.add_space(8.0);
                    ui.weak(t("protected_failed_hint"));
                });
            }
            State::NeedPassword { wrong } => {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(t("dlg_password")).strong());
                    if wrong {
                        ui.colored_label(Color32::from_rgb(220, 90, 90), t("wrong_password"));
                    }
                    let r = ui.add(egui::TextEdit::singleline(&mut self.password_input).password(true).desired_width(220.0));
                    let go = ui.button(t("ok")).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if go && !self.password_input.is_empty() {
                        action = Some(Action::Retry(std::mem::take(&mut self.password_input)));
                    }
                });
            }
            State::Ready => {
                if let Some(a) = self.show_pages(ui, ppp) {
                    action = Some(a);
                }
            }
        }
        action
    }

    fn show_pages(&mut self, ui: &mut egui::Ui, ppp: f32) -> Option<Action> {
        let n = self.sizes.len();
        if n == 0 {
            return None;
        }
        let avail = ui.available_size();
        self.avail = avail;
        let widest = self.sizes.iter().map(|s| s.0).fold(1.0f32, f32::max);
        if self.fit_width {
            let z = ((avail.x - 2.0 * MARGIN - 14.0) / (widest * PT_TO_PX)).clamp(0.25, 4.0);
            self.zoom = z;
        }
        let s = self.scale();
        // top of every page in the content, and the total size
        let mut tops = Vec::with_capacity(n);
        let mut y = MARGIN;
        for (_, h) in &self.sizes {
            tops.push(y);
            y += h * s + MARGIN;
        }
        let content_w = (widest * s + 2.0 * MARGIN).max(avail.x - 14.0);
        let content = Vec2::new(content_w, y);

        // zoom with Ctrl + wheel
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 1e-3 && ui.rect_contains_pointer(ui.max_rect()) {
            self.zoom_by(zoom_delta);
        }
        let mut area = egui::ScrollArea::both().auto_shrink([false, false]).id_salt(("pview", self.id));
        if let Some(r) = self.pending_zoom_ratio.take() {
            area = area.vertical_scroll_offset((self.scroll_y * r).max(0.0));
        }
        if let Some(p) = self.scroll_to.take() {
            area = area.vertical_scroll_offset((tops[p.min(n - 1)] - MARGIN * 0.5).max(0.0));
        }
        let mut action = None;
        let bucket_of = |page_w_pt: f32| -> u32 {
            let px = (page_w_pt * s * ppp).ceil() as u32;
            (px.div_ceil(128) * 128).clamp(256, 4096)
        };
        let out = area.show_viewport(ui, |ui, viewport| {
            ui.set_min_size(content);
            let origin = ui.min_rect().min;
            let first = tops.partition_point(|&t| t + 1.0 < viewport.min.y).saturating_sub(1);
            let center = (viewport.min.y + viewport.max.y) / 2.0;
            let mut visible: Vec<usize> = vec![];
            let mut cur = first;
            for p in first..n {
                if tops[p] > viewport.max.y + 300.0 {
                    break;
                }
                visible.push(p);
                if tops[p] <= center {
                    cur = p;
                }
            }
            self.page = cur;
            let painter = ui.painter_at(ui.clip_rect());
            // nearest to the center first
            visible.sort_by(|a, b| {
                let da = (tops[*a] + self.sizes[*a].1 * s / 2.0 - center).abs();
                let db = (tops[*b] + self.sizes[*b].1 * s / 2.0 - center).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            });
            for &p in &visible {
                let (w_pt, h_pt) = self.sizes[p];
                let (w, h) = (w_pt * s, h_pt * s);
                let x = (content_w - w) / 2.0;
                let rect = Rect::from_min_size(origin + Vec2::new(x, tops[p]), Vec2::new(w, h));
                painter.rect_filled(rect.translate(Vec2::new(3.0, 3.0)), 0.0, Color32::from_black_alpha(45));
                painter.rect_filled(rect, 0.0, Color32::WHITE);
                let bucket = bucket_of(w_pt);
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                if let Some((tex, last)) = self.textures.get_mut(&(p, bucket)) {
                    *last = self.frame;
                    painter.image(tex.id(), rect, uv, Color32::WHITE);
                } else {
                    // another resolution of the same page is better than a blank page while the right one is produced
                    if let Some(((_, _), (tex, last))) = self.textures.iter_mut().filter(|((pg, _), _)| *pg == p).max_by_key(|((_, b), _)| *b) {
                        *last = self.frame;
                        painter.image(tex.id(), rect, uv, Color32::WHITE);
                    }
                    self.request_render(p, bucket);
                }
                self.request_links(p);
                // links
                if let Some(links) = self.links.get(&p) {
                    let resp = ui.interact(rect, egui::Id::unique(("pvpage", self.id, p)), Sense::click());
                    if let Some(pos) = resp.hover_pos() {
                        let (px, py) = ((pos.x - rect.min.x) / s, (pos.y - rect.min.y) / s);
                        if let Some(l) = links.iter().find(|l| px >= l.rect[0] && px <= l.rect[2] && py >= l.rect[1] && py <= l.rect[3]) {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            let r = Rect::from_min_max(rect.min + Vec2::new(l.rect[0], l.rect[1]) * s, rect.min + Vec2::new(l.rect[2], l.rect[3]) * s);
                            painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, Color32::from_rgb(60, 120, 220)), egui::StrokeKind::Inside);
                            if resp.clicked() {
                                match &l.target {
                                    LinkTarget::Uri(u) => action = Some(Action::OpenUri(u.clone())),
                                    LinkTarget::Page(pg) => self.scroll_to = Some((*pg as usize).min(n - 1)),
                                }
                            }
                        }
                    }
                }
            }
        });
        self.scroll_y = out.state.clamped_offset().y;
        self.evict();
        action
    }
}

