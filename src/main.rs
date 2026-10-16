#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use anyhow::Result;
use eframe::egui::{self, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use pdfium_render::prelude::*;
use std::path::PathBuf;

/// Pixels logiques par point PDF à 100 % de zoom (96 dpi / 72 pt).
const PT_TO_PX: f32 = 96.0 / 72.0;
const HANDLE: f32 = 9.0;

fn load_pdfium() -> Result<&'static Pdfium> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    let candidates = [
        Pdfium::pdfium_platform_library_name_at_path(&exe_dir),
        Pdfium::pdfium_platform_library_name_at_path("./"),
    ];
    let mut last = None;
    for c in candidates {
        match Pdfium::bind_to_library(c) {
            Ok(b) => return Ok(Box::leak(Box::new(Pdfium::new(b)))),
            Err(e) => last = Some(e),
        }
    }
    match Pdfium::bind_to_system_library() {
        Ok(b) => Ok(Box::leak(Box::new(Pdfium::new(b)))),
        Err(e) => Err(anyhow::anyhow!("pdfium.dll introuvable: {:?} / {:?}", last, e)),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Select,
    ExportZone,
    AddText,
    AddImage,
}

#[derive(Clone, Copy, PartialEq)]
enum ObjKind {
    Text,
    Image,
    Path,
    Other,
}

#[derive(Clone)]
struct ObjInfo {
    /// Indice de l'objet (pour un bloc de texte : le morceau le plus à gauche)
    idx: usize,
    kind: ObjKind,
    /// left, bottom, right, top en points PDF
    rect: [f32; 4],
    /// Tous les objets PDF qui composent ce bloc (triés de gauche à droite)
    members: Vec<usize>,
    /// Texte fusionné du bloc
    text: String,
}

/// Une occurrence trouvée par la recherche (peut s'étendre sur plusieurs rectangles, ex. retour à la ligne).
#[derive(Clone)]
struct SearchHit {
    page: usize,
    rects: Vec<[f32; 4]>,
}

const MAX_RECENT: usize = 15;

fn recent_file() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())))
        .unwrap_or_default();
    base.join("RustPDF").join("recent.txt")
}

/// Liste des derniers fichiers ouverts (un chemin par ligne), les plus récents d'abord.
fn load_recent() -> Vec<PathBuf> {
    std::fs::read_to_string(recent_file())
        .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).take(MAX_RECENT).collect())
        .unwrap_or_default()
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// Un morceau de texte brut du PDF, avant regroupement en lignes.
struct Run {
    idx: usize,
    text: String,
    font: String,
    size: f32,
    color: [u8; 3],
    base: f32,
    rect: [f32; 4],
    leader: bool,
}

/// Regroupe les morceaux de texte d'une même ligne (même police, taille, couleur et ligne de base).
fn group_runs(mut runs: Vec<Run>) -> Vec<ObjInfo> {
    runs.sort_by(|a, b| a.rect[0].partial_cmp(&b.rect[0]).unwrap_or(std::cmp::Ordering::Equal));
    let mut blocks: Vec<Vec<Run>> = vec![];
    let mut brect: Vec<[f32; 4]> = vec![];
    for r in runs {
        let mut target = None;
        if !r.leader {
            for (bi, b) in blocks.iter().enumerate() {
                let f = &b[0];
                if f.leader
                    || f.font != r.font
                    || f.color != r.color
                    || (f.size - r.size).abs() > 0.3
                    || (f.base - r.base).abs() > 0.3 * r.size.max(1.0)
                {
                    continue;
                }
                let gap = r.rect[0] - brect[bi][2];
                if gap > -r.size && gap < r.size * 1.0 {
                    target = Some(bi);
                }
            }
        }
        match target {
            Some(bi) => {
                let u = &mut brect[bi];
                u[0] = u[0].min(r.rect[0]);
                u[1] = u[1].min(r.rect[1]);
                u[2] = u[2].max(r.rect[2]);
                u[3] = u[3].max(r.rect[3]);
                blocks[bi].push(r);
            }
            None => {
                brect.push(r.rect);
                blocks.push(vec![r]);
            }
        }
    }
    blocks
        .into_iter()
        .zip(brect)
        .map(|(b, rect)| {
            let mut text = String::new();
            let mut prev_right: Option<f32> = None;
            for r in &b {
                if let Some(pr) = prev_right {
                    if r.rect[0] - pr > 0.15 * r.size && !text.ends_with(' ') && !r.text.starts_with(' ') {
                        text.push(' ');
                    }
                }
                text.push_str(&r.text);
                prev_right = Some(r.rect[2]);
            }
            let members: Vec<usize> = b.iter().map(|r| r.idx).collect();
            ObjInfo { idx: members[0], kind: ObjKind::Text, rect, members, text }
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum FieldKind {
    Text,
    Check,
    Radio,
    Combo,
    List,
    Button,
    Signature,
    Other,
}

#[derive(Clone)]
struct FieldInfo {
    ann: usize,
    kind: FieldKind,
    rect: [f32; 4],
    name: String,
    value: String,
    checked: bool,
    read_only: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Inline {
    None,
    Object,
    Field,
}

#[derive(Clone, Copy, PartialEq)]
enum FontChoice {
    Original,
    Helvetica,
    HelveticaBold,
    HelveticaItalic,
    Times,
    TimesBold,
    TimesItalic,
    Courier,
    CourierBold,
}

impl FontChoice {
    const ALL: [(FontChoice, &'static str); 9] = [
        (FontChoice::Original, "Police d'origine"),
        (FontChoice::Helvetica, "Helvetica"),
        (FontChoice::HelveticaBold, "Helvetica Gras"),
        (FontChoice::HelveticaItalic, "Helvetica Italique"),
        (FontChoice::Times, "Times"),
        (FontChoice::TimesBold, "Times Gras"),
        (FontChoice::TimesItalic, "Times Italique"),
        (FontChoice::Courier, "Courier"),
        (FontChoice::CourierBold, "Courier Gras"),
    ];
    fn label(self) -> &'static str {
        Self::ALL.iter().find(|(c, _)| *c == self).map(|(_, l)| *l).unwrap_or("?")
    }
}

#[derive(Clone, Copy)]
struct Resize {
    /// (-1 gauche/bas, 0 milieu, 1 droite/haut)
    hx: i8,
    hy: i8,
    orig: [f32; 4],
    cur: [f32; 4],
}

struct App {
    pdfium: &'static Pdfium,
    doc: Option<PdfDocument<'static>>,
    path: Option<PathBuf>,
    page: usize,
    zoom: f32,
    tool: Tool,
    texture: Option<egui::TextureHandle>,
    thumbs: Vec<Option<egui::TextureHandle>>,
    objects: Vec<ObjInfo>,
    fields: Vec<FieldInfo>,
    page_size: (f32, f32),
    sel: Option<usize>,
    sel_field: Option<usize>,
    edit_text: String,
    edit_size: f32,
    edit_color: [u8; 3],
    font_choice: FontChoice,
    orig_font: String,
    field_text: String,
    inline: Inline,
    inline_focus: bool,
    new_text: String,
    needs_render: bool,
    drag_accum: Vec2,
    resize: Option<Resize>,
    undo: Vec<Vec<u8>>,
    redo: Vec<Vec<u8>>,
    dirty: bool,
    status: String,
    show_boxes: bool,
    show_fields: bool,
    zone: Option<([f32; 2], [f32; 2])>,
    export_isolate: bool,
    export_scale: f32,
    search: String,
    search_open: bool,
    search_focus: bool,
    search_case: bool,
    search_word: bool,
    search_stale: bool,
    matches: Vec<SearchHit>,
    match_cur: Option<usize>,
    scroll_to_match: bool,
    recent: Vec<PathBuf>,
    view_w: f32,
    fit_width: bool,
    zoom_changed_at: Option<std::time::Instant>,
}

impl App {
    fn new(pdfium: &'static Pdfium) -> Self {
        Self {
            pdfium,
            doc: None,
            path: None,
            page: 0,
            zoom: 1.0,
            tool: Tool::Select,
            texture: None,
            thumbs: vec![],
            objects: vec![],
            fields: vec![],
            page_size: (595.0, 842.0),
            sel: None,
            sel_field: None,
            edit_text: String::new(),
            edit_size: 12.0,
            edit_color: [0, 0, 0],
            font_choice: FontChoice::Original,
            orig_font: String::new(),
            field_text: String::new(),
            inline: Inline::None,
            inline_focus: false,
            new_text: "Nouveau texte".into(),
            needs_render: false,
            drag_accum: Vec2::ZERO,
            resize: None,
            undo: vec![],
            redo: vec![],
            dirty: false,
            status: "Ouvrez un PDF (Ctrl+O)".into(),
            show_boxes: true,
            show_fields: true,
            zone: None,
            export_isolate: true,
            export_scale: 300.0, // résolution d'export en dpi
            search: String::new(),
            search_open: false,
            search_focus: false,
            search_case: false,
            search_word: false,
            search_stale: false,
            matches: vec![],
            match_cur: None,
            scroll_to_match: false,
            recent: load_recent(),
            view_w: 800.0,
            fit_width: false,
            zoom_changed_at: None,
        }
    }

    fn page_count(&self) -> usize {
        self.doc.as_ref().map(|d| d.pages().len() as usize).unwrap_or(0)
    }

    fn open(&mut self, path: PathBuf) {
        match self.pdfium.load_pdf_from_file(&path, None) {
            Ok(d) => {
                self.doc = Some(d);
                self.add_recent(&path);
                self.path = Some(path);
                self.page = 0;
                self.clear_selection();
                self.undo.clear();
                self.redo.clear();
                self.dirty = false;
                self.matches.clear();
                self.match_cur = None;
                self.invalidate_all();
                self.fit_width = true;
                self.status = "Document ouvert".into();
            }
            Err(e) => {
                self.status = format!("Impossible d'ouvrir {} : {e:?}", path.display());
                self.remove_recent(&path);
            }
        }
    }

    // ----------------------------------------------------- fichiers récents

    fn store_recent(&self) {
        let f = recent_file();
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text: Vec<String> = self.recent.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let _ = std::fs::write(f, text.join("\n"));
    }

    fn add_recent(&mut self, path: &std::path::Path) {
        let p = std::fs::canonicalize(path).map(|c| PathBuf::from(c.to_string_lossy().trim_start_matches(r"\\?\").to_string())).unwrap_or_else(|_| path.to_path_buf());
        self.recent.retain(|r| !same_path(r, &p));
        self.recent.insert(0, p);
        self.recent.truncate(MAX_RECENT);
        self.store_recent();
    }

    fn remove_recent(&mut self, path: &std::path::Path) {
        self.recent.retain(|r| !same_path(r, path));
        self.store_recent();
    }

    fn clear_recent(&mut self) {
        self.recent.clear();
        self.store_recent();
    }

    /// Liste cliquable des fichiers récents ; renvoie celui choisi.
    fn recent_list_ui(&self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let mut chosen = None;
        for p in &self.recent {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
            let exists = p.exists();
            let r = ui.add_enabled(exists, egui::Button::new(format!("📄 {name}")));
            let r = r.on_hover_text(p.display().to_string()).on_disabled_hover_text(format!("{} (introuvable)", p.display()));
            if r.clicked() {
                chosen = Some(p.clone());
            }
        }
        chosen
    }

    fn clear_selection(&mut self) {
        self.sel = None;
        self.sel_field = None;
        self.inline = Inline::None;
        self.resize = None;
        self.drag_accum = Vec2::ZERO;
    }

    fn invalidate_all(&mut self) {
        self.needs_render = true;
        self.search_stale = true;
        self.thumbs = (0..self.page_count()).map(|_| None).collect();
    }

    /// Sauvegarde l'état courant pour l'annulation.
    fn snapshot(&mut self) {
        if let Some(d) = &self.doc {
            if let Ok(b) = d.save_to_bytes() {
                self.undo.push(b);
                if self.undo.len() > 50 {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
        }
        self.dirty = true;
    }

    fn restore(&mut self, bytes: Vec<u8>) {
        match self.pdfium.load_pdf_from_byte_vec(bytes, None) {
            Ok(d) => {
                self.doc = Some(d);
                let n = self.page_count();
                if self.page >= n {
                    self.page = n.saturating_sub(1);
                }
                self.clear_selection();
                self.dirty = true;
                self.invalidate_all();
            }
            Err(e) => self.status = format!("Restauration impossible: {e:?}"),
        }
    }

    fn do_undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            if let Some(cur) = self.doc.as_ref().and_then(|d| d.save_to_bytes().ok()) {
                self.redo.push(cur);
            }
            self.restore(prev);
        }
    }

    fn do_redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            if let Some(cur) = self.doc.as_ref().and_then(|d| d.save_to_bytes().ok()) {
                self.undo.push(cur);
            }
            self.restore(next);
        }
    }

    fn save(&mut self, as_new: bool) {
        let Some(doc) = &self.doc else { return };
        let target = if as_new || self.path.is_none() {
            rfd::FileDialog::new().add_filter("PDF", &["pdf"]).save_file()
        } else {
            self.path.clone()
        };
        if let Some(t) = target {
            match doc.save_to_file(&t) {
                Ok(_) => {
                    self.status = format!("Enregistré: {}", t.display());
                    self.add_recent(&t);
                    self.path = Some(t);
                    self.dirty = false;
                }
                Err(e) => self.status = format!("Erreur d'enregistrement: {e:?}"),
            }
        }
    }

    fn render_page(&mut self, ctx: &egui::Context) {
        self.needs_render = false;
        let Some(doc) = &self.doc else { return };
        let Ok(page) = doc.pages().get(self.page as i32) else { return };
        let w = page.width().value;
        let h = page.height().value;
        self.page_size = (w, h);
        if self.fit_width {
            self.fit_width = false;
            self.zoom = ((self.view_w - 24.0) / (w * PT_TO_PX)).clamp(0.2, 6.0);
        }
        let ppp = ctx.pixels_per_point();
        // Borne la taille de la texture : limite GPU, et ~24 Mpx maximum pour la mémoire.
        let max_side = (ctx.input(|i| i.max_texture_side) as f32).min(8192.0);
        let ratio = h / w.max(1.0);
        let mut px_w = w * PT_TO_PX * self.zoom * ppp;
        px_w = px_w.min(max_side).min(max_side / ratio.max(1.0));
        px_w = px_w.min((24_000_000.0 / ratio.max(0.01)).sqrt());
        let px_w = (px_w as i32).max(50);
        let cfg = PdfRenderConfig::new().set_target_width(px_w).render_form_data(true);
        if let Ok(bmp) = page.render_with_config(&cfg) {
            if let Ok(dimg) = bmp.as_image() {
                let img = dimg.to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                self.texture = Some(ctx.load_texture("page", ci, egui::TextureOptions::LINEAR));
            }
        }
        // Inventaire des objets de la page
        let mut list = vec![];
        let mut runs = vec![];
        for (i, obj) in page.objects().iter().enumerate() {
            let kind = match obj.object_type() {
                PdfPageObjectType::Text => ObjKind::Text,
                PdfPageObjectType::Image => ObjKind::Image,
                PdfPageObjectType::Path => ObjKind::Path,
                _ => ObjKind::Other,
            };
            let Ok(b) = obj.bounds() else { continue };
            let r = b.to_rect();
            let rect = [r.left().value, r.bottom().value, r.right().value, r.top().value];
            if kind == ObjKind::Text {
                if let Some(t) = obj.as_text_object() {
                    let text = t.text();
                    if text.trim().is_empty() {
                        continue;
                    }
                    let c = obj.fill_color().map(|c| [c.red(), c.green(), c.blue()]).unwrap_or([0, 0, 0]);
                    let base = obj.matrix().map(|m| m.f()).unwrap_or(rect[1]);
                    let tt = text.trim();
                    let leader = tt.len() >= 3 && tt.chars().all(|ch| matches!(ch, '.' | '_' | '-' | '…' | '·' | ' '));
                    runs.push(Run {
                        idx: i,
                        text,
                        font: t.font().name(),
                        size: t.scaled_font_size().value,
                        color: c,
                        base,
                        rect,
                        leader,
                    });
                }
            } else {
                list.push(ObjInfo { idx: i, kind, rect, members: vec![i], text: String::new() });
            }
        }
        list.extend(group_runs(runs));
        list.sort_by_key(|o| o.idx);
        self.objects = list;

        // Inventaire des champs de formulaire
        let mut fl = vec![];
        for (i, ann) in page.annotations().iter().enumerate() {
            let Some(f) = ann.as_form_field() else { continue };
            let Ok(b) = ann.bounds() else { continue };
            let mut info = FieldInfo {
                ann: i,
                kind: FieldKind::Other,
                rect: [b.left().value, b.bottom().value, b.right().value, b.top().value],
                name: f.name().unwrap_or_default(),
                value: String::new(),
                checked: false,
                read_only: f.is_read_only(),
            };
            match f.field_type() {
                PdfFormFieldType::Text => {
                    info.kind = FieldKind::Text;
                    info.value = f.as_text_field().and_then(|t| t.value()).unwrap_or_default();
                }
                PdfFormFieldType::Checkbox => {
                    info.kind = FieldKind::Check;
                    info.checked = f.as_checkbox_field().and_then(|c| c.is_checked().ok()).unwrap_or(false);
                }
                PdfFormFieldType::RadioButton => {
                    info.kind = FieldKind::Radio;
                    info.checked = f.as_radio_button_field().and_then(|c| c.is_checked().ok()).unwrap_or(false);
                }
                PdfFormFieldType::ComboBox => {
                    info.kind = FieldKind::Combo;
                    info.value = f.as_combo_box_field().and_then(|c| c.value()).unwrap_or_default();
                }
                PdfFormFieldType::ListBox => {
                    info.kind = FieldKind::List;
                    info.value = f.as_list_box_field().and_then(|c| c.value()).unwrap_or_default();
                }
                PdfFormFieldType::PushButton => info.kind = FieldKind::Button,
                PdfFormFieldType::Signature => info.kind = FieldKind::Signature,
                _ => {}
            }
            fl.push(info);
        }
        self.fields = fl;
    }

    fn thumb(&mut self, ctx: &egui::Context, i: usize) -> Option<egui::TextureHandle> {
        if self.thumbs.len() <= i {
            return None;
        }
        if self.thumbs[i].is_none() {
            let doc = self.doc.as_ref()?;
            let page = doc.pages().get(i as i32).ok()?;
            let bmp = page.render_with_config(&PdfRenderConfig::new().set_target_width(140)).ok()?;
            let img = bmp.as_image().ok()?.to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied(
                [img.width() as usize, img.height() as usize],
                img.as_raw(),
            );
            self.thumbs[i] = Some(ctx.load_texture(format!("t{i}"), ci, egui::TextureOptions::LINEAR));
        }
        self.thumbs[i].clone()
    }

    fn select(&mut self, idx: Option<usize>) {
        self.sel = idx;
        self.sel_field = None;
        self.inline = Inline::None;
        self.drag_accum = Vec2::ZERO;
        self.font_choice = FontChoice::Original;
        let block_text = idx.and_then(|i| self.objects.iter().find(|o| o.idx == i)).map(|o| o.text.clone());
        if let (Some(i), Some(doc)) = (idx, &self.doc) {
            if let Ok(page) = doc.pages().get(self.page as i32) {
                if let Ok(obj) = page.objects().get(i) {
                    if let Some(t) = obj.as_text_object() {
                        self.edit_text = block_text.unwrap_or_else(|| t.text());
                        self.edit_size = t.scaled_font_size().value;
                        self.orig_font = t.font().name();
                    }
                    if let Ok(c) = obj.fill_color() {
                        self.edit_color = [c.red(), c.green(), c.blue()];
                    }
                }
            }
        }
    }

    fn select_field(&mut self, fi: usize) {
        self.sel = None;
        self.sel_field = Some(fi);
        self.field_text = self.fields[fi].value.clone();
    }

    // ------------------------------------------------------------ éditions

    fn apply_text(&mut self) {
        let Some(i) = self.sel else { return };
        self.snapshot();
        let new_text = self.edit_text.clone();
        let size = self.edit_size;
        let choice = self.font_choice;
        let col = PdfColor::new(self.edit_color[0], self.edit_color[1], self.edit_color[2], 255);
        let page_idx = self.page as i32;
        let members = self.sel_members(i);
        let mut new_sel: Option<usize> = None;
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(page_idx)?;
            {
                let mut objs = page.objects_mut();
                let leader = members[0];
                if choice != FontChoice::Original {
                    // Nouvelle police : on recrée un seul objet à la position (ligne de base) du premier morceau.
                    let m = objs.get(leader)?.matrix()?;
                    let (x, y) = (PdfPoints::new(m.e()), PdfPoints::new(m.f()));
                    let mut desc = members.clone();
                    desc.sort_unstable_by(|a, b| b.cmp(a));
                    for k in desc {
                        let o = objs.get(k)?;
                        objs.remove_object(o)?;
                    }
                    let fonts = doc.fonts_mut();
                    let font = match choice {
                        FontChoice::Helvetica | FontChoice::Original => fonts.helvetica(),
                        FontChoice::HelveticaBold => fonts.helvetica_bold(),
                        FontChoice::HelveticaItalic => fonts.helvetica_oblique(),
                        FontChoice::Times => fonts.times_roman(),
                        FontChoice::TimesBold => fonts.times_bold(),
                        FontChoice::TimesItalic => fonts.times_italic(),
                        FontChoice::Courier => fonts.courier(),
                        FontChoice::CourierBold => fonts.courier_bold(),
                    };
                    let mut no = objs.create_text_object(x, y, &new_text, font, PdfPoints::new(size))?;
                    no.set_fill_color(col)?;
                    drop(no);
                    new_sel = Some((objs.len() - 1) as usize);
                } else {
                    // Police d'origine : le premier morceau porte tout le texte, les autres disparaissent.
                    let mut others: Vec<usize> = members.iter().copied().filter(|&k| k != leader).collect();
                    others.sort_unstable_by(|a, b| b.cmp(a));
                    let shift = others.iter().filter(|&&k| k < leader).count();
                    for k in others {
                        let o = objs.get(k)?;
                        objs.remove_object(o)?;
                    }
                    let li = leader - shift;
                    let mut obj = objs.get(li)?;
                    if let Some(t) = obj.as_text_object_mut() {
                        t.set_text(&new_text)?;
                        t.set_unscaled_font_size(PdfPoints::new(size)).ok();
                    }
                    obj.set_fill_color(col).ok();
                    new_sel = Some(li);
                }
            }
            page.regenerate_content()?;
            Ok(())
        })();
        self.sel = new_sel;
        self.after_edit(res);
    }

    /// Tous les objets PDF du bloc sélectionné (un seul pour une image ou une forme).
    fn sel_members(&self, i: usize) -> Vec<usize> {
        self.objects
            .iter()
            .find(|o| o.idx == i)
            .map(|o| o.members.clone())
            .unwrap_or_else(|| vec![i])
    }

    fn apply_color_only(&mut self) {
        let Some(i) = self.sel else { return };
        self.snapshot();
        let col = PdfColor::new(self.edit_color[0], self.edit_color[1], self.edit_color[2], 255);
        let p = self.page as i32;
        let members = self.sel_members(i);
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(p)?;
            {
                let objs = page.objects_mut();
                for k in members {
                    let mut obj = objs.get(k)?;
                    obj.set_fill_color(col)?;
                }
            }
            page.regenerate_content()
        })();
        self.after_edit(res);
    }

    fn delete_selected(&mut self) {
        let Some(i) = self.sel else { return };
        self.snapshot();
        let p = self.page as i32;
        let mut members = self.sel_members(i);
        members.sort_unstable_by(|a, b| b.cmp(a));
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(p)?;
            {
                let mut objs = page.objects_mut();
                for k in members {
                    let obj = objs.get(k)?;
                    objs.remove_object(obj)?;
                }
            }
            page.regenerate_content()
        })();
        self.clear_selection();
        self.after_edit(res);
    }

    fn move_selected(&mut self, dx_pt: f32, dy_pt: f32) {
        let Some(i) = self.sel else { return };
        self.snapshot();
        let p = self.page as i32;
        let members = self.sel_members(i);
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(p)?;
            {
                let objs = page.objects_mut();
                for k in members {
                    let mut obj = objs.get(k)?;
                    obj.translate(PdfPoints::new(dx_pt), PdfPoints::new(dy_pt))?;
                }
            }
            page.regenerate_content()
        })();
        self.after_edit(res);
    }

    /// Redimensionne l'objet sélectionné de `old` vers `new` (rectangles en points PDF).
    fn resize_selected(&mut self, old: [f32; 4], new: [f32; 4]) {
        let Some(i) = self.sel else { return };
        let (ow, oh) = (old[2] - old[0], old[3] - old[1]);
        if ow < 0.01 || oh < 0.01 {
            return;
        }
        let sx = (new[2] - new[0]) / ow;
        let sy = (new[3] - new[1]) / oh;
        let tx = new[0] - old[0] * sx;
        let ty = new[1] - old[1] * sy;
        self.snapshot();
        let p = self.page as i32;
        let members = self.sel_members(i);
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(p)?;
            {
                let objs = page.objects_mut();
                for k in members {
                    let mut obj = objs.get(k)?;
                    obj.scale(sx, sy)?;
                    obj.translate(PdfPoints::new(tx), PdfPoints::new(ty))?;
                }
            }
            page.regenerate_content()
        })();
        self.after_edit(res);
    }

    // -------------------------------------------------------------- export

    fn export_options_ui(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_label("Résolution")
            .selected_text(format!("{:.0} dpi", self.export_scale))
            .show_ui(ui, |ui| {
                for d in [72.0, 150.0, 300.0, 600.0] {
                    ui.selectable_value(&mut self.export_scale, d, format!("{d:.0} dpi"));
                }
            });
    }

    /// Rend la page courante (éventuellement réduite à certains objets) et découpe `clip`
    /// (left, bottom, right, top en points). Fond transparent si `transparent`.
    fn render_isolated(&self, keep: Option<&[usize]>, clip: [f32; 4], transparent: bool) -> Result<image::DynamicImage, String> {
        let doc = self.doc.as_ref().ok_or("aucun document")?;
        let r: Result<image::DynamicImage, PdfiumError> = (|| {
            let mut nd = self.pdfium.create_new_pdf()?;
            nd.pages_mut().copy_page_from_document(doc, self.page as i32, 0)?;
            let mut page = nd.pages().get(0)?;
            if let Some(keep) = keep {
                {
                    let mut objs = page.objects_mut();
                    let n = objs.len() as usize;
                    for idx in (0..n).rev() {
                        if !keep.contains(&idx) {
                            let o = objs.get(idx)?;
                            objs.remove_object(o)?;
                        }
                    }
                }
                page.regenerate_content()?;
            }
            let (w, h) = (page.width().value, page.height().value);
            // dpi -> pixels par point, borné à 7000 px de côté
            let s = (self.export_scale / 72.0).min(7000.0 / w.max(h).max(1.0));
            let bg = if transparent { PdfColor::new(255, 255, 255, 0) } else { PdfColor::new(255, 255, 255, 255) };
            let cfg = PdfRenderConfig::new()
                .set_target_width((w * s).max(1.0) as i32)
                .set_clear_color(bg)
                .render_annotations(false)
                .render_form_data(false);
            let full = page.render_with_config(&cfg)?.as_image()?.to_rgba8();
            let (iw, ih) = (full.width(), full.height());
            let x0 = ((clip[0] * s).floor().max(0.0) as u32).min(iw.saturating_sub(1));
            let y0 = (((h - clip[3]) * s).floor().max(0.0) as u32).min(ih.saturating_sub(1));
            let x1 = ((clip[2] * s).ceil().max(0.0) as u32).clamp(x0 + 1, iw);
            let y1 = (((h - clip[1]) * s).ceil().max(0.0) as u32).clamp(y0 + 1, ih);
            Ok(image::DynamicImage::ImageRgba8(image::imageops::crop_imm(&full, x0, y0, x1 - x0, y1 - y0).to_image()))
        })();
        r.map_err(|e| format!("{e:?}"))
    }

    fn save_image(&mut self, img: image::DynamicImage, name: &str) {
        let Some(mut path) = rfd::FileDialog::new()
            .set_file_name(name)
            .add_filter("PNG (transparence)", &["png"])
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
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let out = if matches!(ext.as_str(), "jpg" | "jpeg" | "bmp") {
            // Formats sans transparence : on compose sur fond blanc.
            let mut rgba = img.to_rgba8();
            for p in rgba.pixels_mut() {
                let a = p[3] as u32;
                for c in 0..3 {
                    p[c] = ((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
                }
                p[3] = 255;
            }
            image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(rgba).to_rgb8())
        } else {
            img
        };
        self.status = match out.save(&path) {
            Ok(_) => format!("Exporté : {} ({}×{} px)", path.display(), out.width(), out.height()),
            Err(e) => format!("Export impossible : {e}"),
        };
    }

    fn export_selected(&mut self) {
        let Some(i) = self.sel else { return };
        let Some(o) = self.objects.iter().find(|o| o.idx == i).cloned() else { return };
        let pad = 0.5;
        let clip = [o.rect[0] - pad, o.rect[1] - pad, o.rect[2] + pad, o.rect[3] + pad];
        match self.render_isolated(Some(&o.members), clip, true) {
            Ok(img) => self.save_image(img, "element.png"),
            Err(e) => self.status = format!("Export impossible : {e}"),
        }
    }

    fn export_region(&mut self, r: [f32; 4]) {
        let page_area = self.page_size.0 * self.page_size.1;
        let keep: Option<Vec<usize>> = if self.export_isolate {
            let tol = 0.5;
            Some(
                self.objects
                    .iter()
                    .filter(|o| {
                        o.rect[0] >= r[0] - tol && o.rect[1] >= r[1] - tol && o.rect[2] <= r[2] + tol && o.rect[3] <= r[3] + tol
                    })
                    .filter(|o| (o.rect[2] - o.rect[0]) * (o.rect[3] - o.rect[1]) < page_area * 0.8)
                    .flat_map(|o| o.members.clone())
                    .collect(),
            )
        } else {
            None
        };
        match self.render_isolated(keep.as_deref(), r, self.export_isolate) {
            Ok(img) => self.save_image(img, "zone.png"),
            Err(e) => self.status = format!("Export impossible : {e}"),
        }
    }

    fn export_original_image(&mut self) {
        let Some(i) = self.sel else { return };
        let img = (|| -> Result<image::DynamicImage, String> {
            let doc = self.doc.as_ref().ok_or("aucun document")?;
            let page = doc.pages().get(self.page as i32).map_err(|e| format!("{e:?}"))?;
            let obj = page.objects().get(i).map_err(|e| format!("{e:?}"))?;
            let io = obj.as_image_object().ok_or("ce n'est pas une image")?;
            io.get_processed_image(doc)
                .or_else(|_| io.get_raw_image())
                .map_err(|e| format!("{e:?}"))
        })();
        match img {
            Ok(img) => self.save_image(img, "image.png"),
            Err(e) => self.status = format!("Export impossible : {e}"),
        }
    }

    fn replace_image(&mut self) {
        let Some(i) = self.sel else { return };
        let Some(file) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "gif", "webp"])
            .pick_file()
        else {
            return;
        };
        let img = match image::open(&file) {
            Ok(i) => i,
            Err(e) => {
                self.status = format!("Image illisible: {e}");
                return;
            }
        };
        self.snapshot();
        let p = self.page as i32;
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(p)?;
            {
                let objs = page.objects_mut();
                let mut obj = objs.get(i)?;
                if let Some(io) = obj.as_image_object_mut() {
                    // Conserve la position/taille: on remplace les pixels, la matrice reste.
                    io.set_image(&img)?;
                }
            }
            page.regenerate_content()
        })();
        self.after_edit(res);
    }

    fn add_text_at(&mut self, x: f32, y: f32) {
        self.snapshot();
        let p = self.page as i32;
        let txt = self.new_text.clone();
        let size = self.edit_size.max(6.0);
        let col = PdfColor::new(self.edit_color[0], self.edit_color[1], self.edit_color[2], 255);
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(p)?;
            let font = doc.fonts_mut().helvetica();
            let mut o = page
                .objects_mut()
                .create_text_object(PdfPoints::new(x), PdfPoints::new(y), &txt, font, PdfPoints::new(size))?;
            o.set_fill_color(col)?;
            drop(o);
            page.regenerate_content()
        })();
        self.after_edit(res);
    }

    fn add_image_at(&mut self, x: f32, y: f32) {
        let Some(file) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "gif", "webp"])
            .pick_file()
        else {
            return;
        };
        let Ok(img) = image::open(&file) else {
            self.status = "Image illisible".into();
            return;
        };
        self.snapshot();
        let p = self.page as i32;
        let w = 200.0_f32;
        let h = w * img.height() as f32 / img.width().max(1) as f32;
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(p)?;
            page.objects_mut().create_image_object(
                PdfPoints::new(x),
                PdfPoints::new(y - h),
                &img,
                Some(PdfPoints::new(w)),
                Some(PdfPoints::new(h)),
            )?;
            page.regenerate_content()
        })();
        self.after_edit(res);
    }

    fn after_edit(&mut self, res: Result<(), PdfiumError>) {
        match res {
            Ok(_) => self.status = "Modification appliquée".into(),
            Err(e) => self.status = format!("Échec: {e:?}"),
        }
        self.invalidate_all();
    }

    // ---------------------------------------------------------- formulaires

    fn set_field_text(&mut self, fi: usize, value: String) {
        let Some(info) = self.fields.get(fi).cloned() else { return };
        self.snapshot();
        let p = self.page as i32;
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(p)?;
            let anns = page.annotations_mut();
            let mut a = anns.get(info.ann as _)?;
            if let Some(f) = a.as_form_field_mut() {
                if let Some(t) = f.as_text_field_mut() {
                    t.set_value(&value)?;
                }
            }
            Ok(())
        })();
        self.after_edit(res);
    }

    fn toggle_field(&mut self, fi: usize) {
        let Some(info) = self.fields.get(fi).cloned() else { return };
        self.snapshot();
        let p = self.page as i32;
        let res: Result<(), PdfiumError> = (|| {
            let doc = self.doc.as_mut().unwrap();
            let mut page = doc.pages().get(p)?;
            let anns = page.annotations_mut();
            let mut a = anns.get(info.ann as _)?;
            if let Some(f) = a.as_form_field_mut() {
                if let Some(c) = f.as_checkbox_field_mut() {
                    c.set_checked(!info.checked)?;
                } else if let Some(r) = f.as_radio_button_field_mut() {
                    r.set_checked()?;
                }
            }
            Ok(())
        })();
        self.after_edit(res);
    }

    // ------------------------------------------------------ opérations pages

    /// Reconstruit le document avec l'ordre de pages donné.
    fn rebuild_with_order(&mut self, order: &[i32]) {
        self.snapshot();
        let Ok(mut new_doc) = self.pdfium.create_new_pdf() else { return };
        let Some(old) = self.doc.take() else { return };
        for (dest, src) in order.iter().enumerate() {
            let _ = new_doc.pages_mut().copy_page_from_document(&old, *src, dest as i32);
        }
        self.doc = Some(new_doc);
        self.dirty = true;
        self.clear_selection();
        self.invalidate_all();
    }

    fn delete_page(&mut self) {
        let n = self.page_count();
        if n <= 1 {
            return;
        }
        let order: Vec<i32> = (0..n as i32).filter(|&i| i as usize != self.page).collect();
        self.rebuild_with_order(&order);
        self.page = self.page.min(n - 2);
    }

    fn move_page(&mut self, delta: i32) {
        let n = self.page_count() as i32;
        let to = self.page as i32 + delta;
        if to < 0 || to >= n {
            return;
        }
        let mut order: Vec<i32> = (0..n).collect();
        order.swap(self.page, to as usize);
        self.rebuild_with_order(&order);
        self.page = to as usize;
    }

    fn rotate_page(&mut self) {
        self.snapshot();
        let p = self.page as i32;
        if let Some(doc) = &self.doc {
            if let Ok(mut page) = doc.pages().get(p) {
                let next = match page.rotation() {
                    Ok(PdfPageRenderRotation::None) => PdfPageRenderRotation::Degrees90,
                    Ok(PdfPageRenderRotation::Degrees90) => PdfPageRenderRotation::Degrees180,
                    Ok(PdfPageRenderRotation::Degrees180) => PdfPageRenderRotation::Degrees270,
                    _ => PdfPageRenderRotation::None,
                };
                page.set_rotation(next);
            }
        }
        self.clear_selection();
        self.invalidate_all();
    }

    fn insert_blank_page(&mut self) {
        self.snapshot();
        let at = (self.page + 1) as i32;
        let size = PdfPagePaperSize::a4();
        if let Some(doc) = &mut self.doc {
            let _ = doc.pages_mut().create_page_at_index(size, at);
        }
        self.page += 1;
        self.clear_selection();
        self.invalidate_all();
    }

    fn merge_with(&mut self) {
        let Some(f) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() else { return };
        let Ok(other) = self.pdfium.load_pdf_from_file(&f, None) else {
            self.status = "PDF à fusionner illisible".into();
            return;
        };
        self.snapshot();
        if let Some(doc) = &mut self.doc {
            match doc.pages_mut().append(&other) {
                Ok(_) => self.status = "Documents fusionnés".into(),
                Err(e) => self.status = format!("Fusion impossible: {e:?}"),
            }
        }
        self.invalidate_all();
    }

    fn extract_current_page(&mut self) {
        let Some(doc) = &self.doc else { return };
        let Some(target) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).save_file() else { return };
        if let Ok(mut nd) = self.pdfium.create_new_pdf() {
            if nd.pages_mut().copy_page_from_document(doc, self.page as i32, 0).is_ok() {
                self.status = match nd.save_to_file(&target) {
                    Ok(_) => "Page extraite".into(),
                    Err(e) => format!("Erreur: {e:?}"),
                };
            }
        }
    }

    // ------------------------------------------------------------ recherche

    /// Cherche toutes les occurrences dans tout le document.
    fn collect_matches(&self) -> Vec<SearchHit> {
        let q = self.search.trim();
        let mut hits = vec![];
        let Some(doc) = &self.doc else { return hits };
        if q.is_empty() {
            return hits;
        }
        let opts = PdfSearchOptions::new().match_case(self.search_case).match_whole_word(self.search_word);
        for pi in 0..self.page_count() {
            let Ok(page) = doc.pages().get(pi as i32) else { continue };
            let Ok(text) = page.text() else { continue };
            let Ok(search) = text.search(q, &opts) else { continue };
            for segs in search.iter(PdfSearchDirection::SearchForward) {
                let rects: Vec<[f32; 4]> = segs
                    .iter()
                    .map(|s| {
                        let b = s.bounds();
                        [b.left().value, b.bottom().value, b.right().value, b.top().value]
                    })
                    .collect();
                if !rects.is_empty() {
                    hits.push(SearchHit { page: pi, rects });
                }
            }
        }
        hits
    }

    /// Relance la recherche (texte modifié) et va à la première occurrence à partir de la page courante.
    fn run_search(&mut self) {
        self.search_stale = false;
        self.matches = self.collect_matches();
        if self.matches.is_empty() {
            self.match_cur = None;
            self.status = if self.search.trim().is_empty() { String::new() } else { "Aucun résultat".into() };
            return;
        }
        let start = self.matches.iter().position(|m| m.page >= self.page).unwrap_or(0);
        self.goto_match(start);
    }

    /// Recalcule les occurrences après une modification du document, sans bouger la vue.
    fn refresh_matches(&mut self) {
        self.search_stale = false;
        self.matches = self.collect_matches();
        self.match_cur = match self.match_cur {
            Some(c) if !self.matches.is_empty() => Some(c.min(self.matches.len() - 1)),
            _ => None,
        };
    }

    fn goto_match(&mut self, i: usize) {
        let Some(m) = self.matches.get(i) else { return };
        let page = m.page;
        self.match_cur = Some(i);
        if page != self.page {
            self.page = page;
            self.clear_selection();
            self.needs_render = true;
        }
        self.scroll_to_match = true;
        self.status = format!("Occurrence {} sur {} (page {})", i + 1, self.matches.len(), page + 1);
    }

    fn next_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        let i = self.match_cur.map(|c| (c + 1) % n).unwrap_or(0);
        self.goto_match(i);
    }

    fn prev_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        let i = self.match_cur.map(|c| (c + n - 1) % n).unwrap_or(n - 1);
        self.goto_match(i);
    }

    fn close_search(&mut self) {
        self.search_open = false;
        self.matches.clear();
        self.match_cur = None;
    }

    fn set_zoom(&mut self, z: f32) {
        let z = z.clamp(0.2, 6.0);
        if (z - self.zoom).abs() > 0.001 {
            self.zoom = z;
            // Le rendu net est différé : pendant la molette on étire l'ancienne image.
            self.zoom_changed_at = Some(std::time::Instant::now());
        }
    }
}

fn contains(r: &[f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3]
}

fn cursor_for(hx: i8, hy: i8) -> CursorIcon {
    match (hx, hy) {
        (-1, -1) | (1, 1) => CursorIcon::ResizeNeSw,
        (-1, 1) | (1, -1) => CursorIcon::ResizeNwSe,
        (0, _) => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeHorizontal,
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();

        // Raccourcis
        let (open, save, undo, redo, del, esc, zoom_delta) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::O),
                i.modifiers.command && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::Z),
                i.modifiers.command && i.key_pressed(egui::Key::Y),
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::Escape),
                i.zoom_delta(),
            )
        });
        if open {
            if let Some(f) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                self.open(f);
            }
        }
        // Glisser-déposer d'un PDF sur la fenêtre
        if let Some(p) = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| { let p = f.path(); if p.as_os_str().is_empty() { None } else { Some(p.to_path_buf()) } })) {
            self.open(p);
        }
        // Recherche : Ctrl+F ouvre la barre, F3 / Maj+F3 naviguent
        let (find, f3, shift) = ctx.input(|i| {
            (i.modifiers.command && i.key_pressed(egui::Key::F), i.key_pressed(egui::Key::F3), i.modifiers.shift)
        });
        if find && self.doc.is_some() {
            self.search_open = true;
            self.search_focus = true;
            if !self.search.trim().is_empty() {
                self.run_search();
            }
        }
        if f3 && self.search_open {
            if shift {
                self.prev_match();
            } else {
                self.next_match();
            }
        }
        if self.search_stale {
            if self.search_open {
                self.refresh_matches();
            } else {
                self.search_stale = false;
            }
        }
        if save {
            self.save(false);
        }
        if undo && !ctx.egui_wants_keyboard_input() {
            self.do_undo();
        }
        if redo && !ctx.egui_wants_keyboard_input() {
            self.do_redo();
        }
        if del && self.sel.is_some() && !ctx.egui_wants_keyboard_input() {
            self.delete_selected();
        }
        if esc && self.inline == Inline::None {
            self.sel = None;
            self.sel_field = None;
        }
        // Zoom : Ctrl + molette (ou pincement)
        if (zoom_delta - 1.0).abs() > 0.0001 && self.doc.is_some() {
            self.set_zoom(self.zoom * zoom_delta);
        }

        if let Some(t) = self.zoom_changed_at {
            if t.elapsed() > std::time::Duration::from_millis(150) {
                self.zoom_changed_at = None;
                self.needs_render = true;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
            }
        }
        if self.needs_render {
            self.render_page(ctx);
            if let Some(s) = self.sel {
                if !self.objects.iter().any(|o| o.idx == s) {
                    self.sel = None;
                }
            }
            if let Some(f) = self.sel_field {
                if f >= self.fields.len() {
                    self.sel_field = None;
                }
            }
        }

        let has_doc = self.doc.is_some();
        let n = self.page_count();

        egui::Panel::top("menu").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("📂 Ouvrir").clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                        self.open(f);
                    }
                }
                let mut open_recent = None;
                ui.menu_button("🕘 Récents", |ui| {
                    if self.recent.is_empty() {
                        ui.label("Aucun fichier récent");
                    } else {
                        if let Some(p) = self.recent_list_ui(ui) {
                            open_recent = Some(p);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("🗑 Effacer la liste").clicked() {
                            self.clear_recent();
                            ui.close();
                        }
                    }
                });
                if let Some(p) = open_recent {
                    self.open(p);
                }
                ui.add_enabled_ui(has_doc, |ui| {
                    if ui.button("💾 Enregistrer").clicked() {
                        self.save(false);
                    }
                    if ui.button("Enregistrer sous…").clicked() {
                        self.save(true);
                    }
                    ui.separator();
                    if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("↩ Annuler")).clicked() {
                        self.do_undo();
                    }
                    if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("↪ Rétablir")).clicked() {
                        self.do_redo();
                    }
                    ui.separator();
                    ui.selectable_value(&mut self.tool, Tool::Select, "🖱 Sélection / édition");
                    ui.selectable_value(&mut self.tool, Tool::AddText, "T Ajouter texte");
                    ui.selectable_value(&mut self.tool, Tool::AddImage, "🖼 Ajouter image");
                    ui.selectable_value(&mut self.tool, Tool::ExportZone, "✂ Exporter zone");
                    ui.separator();
                    if ui.button("−").clicked() {
                        self.set_zoom(self.zoom / 1.2);
                    }
                    ui.label(format!("{:.0} %", self.zoom * 100.0));
                    if ui.button("+").clicked() {
                        self.set_zoom(self.zoom * 1.2);
                    }
                    if ui.button("↔ Largeur").clicked() {
                        self.fit_width = true;
                        self.needs_render = true;
                    }
                    ui.separator();
                    ui.checkbox(&mut self.show_boxes, "Cadres objets");
                    ui.checkbox(&mut self.show_fields, "Champs");
                    ui.separator();
                    if ui.selectable_label(self.search_open, "🔍 Rechercher").on_hover_text("Ctrl+F").clicked() {
                        if self.search_open {
                            self.close_search();
                        } else {
                            self.search_open = true;
                            self.search_focus = true;
                            if !self.search.trim().is_empty() {
                                self.run_search();
                            }
                        }
                    }
                });
            });
        });

        if self.search_open {
            egui::Panel::top("searchbar").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Rechercher dans le document")
                            .desired_width(260.0),
                    );
                    if self.search_focus {
                        r.request_focus();
                        self.search_focus = false;
                    }
                    let mut rerun = r.changed();
                    let (enter, shift, esc) = ctx.input(|i| {
                        (i.key_pressed(egui::Key::Enter), i.modifiers.shift, i.key_pressed(egui::Key::Escape))
                    });
                    if r.lost_focus() && enter {
                        if shift {
                            self.prev_match();
                        } else {
                            self.next_match();
                        }
                        r.request_focus();
                    }
                    if ui.button("▲").on_hover_text("Précédent (Maj+Entrée / Maj+F3)").clicked() {
                        self.prev_match();
                    }
                    if ui.button("▼").on_hover_text("Suivant (Entrée / F3)").clicked() {
                        self.next_match();
                    }
                    if ui.toggle_value(&mut self.search_case, "Aa").on_hover_text("Respecter la casse").changed() {
                        rerun = true;
                    }
                    if ui.toggle_value(&mut self.search_word, "ab").on_hover_text("Mot entier").changed() {
                        rerun = true;
                    }
                    if self.search.trim().is_empty() {
                        ui.label("");
                    } else if self.matches.is_empty() {
                        ui.colored_label(Color32::from_rgb(220, 70, 70), "Aucun résultat");
                    } else {
                        ui.label(format!("{} / {}", self.match_cur.map(|c| c + 1).unwrap_or(0), self.matches.len()));
                    }
                    let close = ui.button("✕").on_hover_text("Fermer (Échap)").clicked() || (r.lost_focus() && esc);
                    if close {
                        self.close_search();
                    } else if rerun {
                        self.run_search();
                    }
                });
            });
        }

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if has_doc {
                        ui.label(format!("Page {}/{}{}", self.page + 1, n, if self.dirty { "  ● modifié" } else { "" }));
                    }
                });
            });
        });

        // Miniatures + opérations de pages
        egui::Panel::left("thumbs").default_size(170.0).show(ui, |ui| {
            ui.add_enabled_ui(has_doc, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("⟳ Pivoter").clicked() {
                        self.rotate_page();
                    }
                    if ui.small_button("🗑 Suppr.").clicked() {
                        self.delete_page();
                    }
                    if ui.small_button("▲").clicked() {
                        self.move_page(-1);
                    }
                    if ui.small_button("▼").clicked() {
                        self.move_page(1);
                    }
                    if ui.small_button("＋ Vierge").clicked() {
                        self.insert_blank_page();
                    }
                    if ui.small_button("Fusionner…").clicked() {
                        self.merge_with();
                    }
                    if ui.small_button("Extraire…").clicked() {
                        self.extract_current_page();
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for i in 0..n {
                    if let Some(t) = self.thumb(ctx, i) {
                        let sz = t.size_vec2();
                        let (rect, resp) = ui.allocate_exact_size(sz, Sense::click());
                        ui.painter().image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                        if i == self.page {
                            ui.painter().rect_stroke(rect, 0.0, Stroke::new(3.0, Color32::from_rgb(40, 120, 255)), egui::StrokeKind::Outside);
                        }
                        ui.label(format!("{}", i + 1));
                        if resp.clicked() {
                            self.page = i;
                            self.clear_selection();
                            self.needs_render = true;
                        }
                    }
                }
            });
        });

        // Panneau de propriétés
        egui::Panel::right("props").default_size(270.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Propriétés");
                ui.separator();
                let sel_kind = self.sel.and_then(|s| self.objects.iter().find(|o| o.idx == s)).map(|o| o.kind);
                match (self.tool, sel_kind) {
                    (Tool::AddText, _) => {
                        ui.label("Cliquez sur la page pour placer le texte.");
                        ui.text_edit_multiline(&mut self.new_text);
                        ui.add(egui::DragValue::new(&mut self.edit_size).range(4.0..=200.0).prefix("Taille "));
                        ui.horizontal(|ui| {
                            ui.label("Couleur");
                            ui.color_edit_button_srgb(&mut self.edit_color);
                        });
                    }
                    (Tool::AddImage, _) => {
                        ui.label("Cliquez sur la page pour placer une image.");
                    }
                    (Tool::ExportZone, _) => {
                        ui.label("Tracez un rectangle sur la page : la zone sera exportée en image.");
                        ui.checkbox(&mut self.export_isolate, "Fond transparent (seuls les éléments entièrement dans la zone)");
                        self.export_options_ui(ui);
                    }
                    (_, Some(ObjKind::Text)) => {
                        ui.label("Texte sélectionné");
                        ui.small(format!("Police : {}", self.orig_font));
                        ui.text_edit_multiline(&mut self.edit_text);
                        ui.add(egui::DragValue::new(&mut self.edit_size).range(4.0..=400.0).prefix("Taille "));
                        ui.horizontal(|ui| {
                            ui.label("Couleur");
                            ui.color_edit_button_srgb(&mut self.edit_color);
                        });
                        egui::ComboBox::from_label("Police")
                            .selected_text(self.font_choice.label())
                            .show_ui(ui, |ui| {
                                for (c, l) in FontChoice::ALL {
                                    ui.selectable_value(&mut self.font_choice, c, l);
                                }
                            });
                        ui.small("Si la police d'origine est un sous-ensemble, des caractères peuvent manquer : choisissez une autre police.");
                        ui.horizontal(|ui| {
                            if ui.button("✔ Appliquer").clicked() {
                                self.apply_text();
                            }
                            if ui.button("✎ Sur la page").clicked() {
                                self.inline = Inline::Object;
                                self.inline_focus = true;
                            }
                        });
                        ui.small("Astuce : double-cliquez sur le texte pour l'éditer directement.");
                    }
                    (_, Some(ObjKind::Image)) => {
                        ui.label("Image sélectionnée");
                        if ui.button("🖼 Remplacer l'image…").clicked() {
                            self.replace_image();
                        }
                        if ui.button("📤 Exporter l'image d'origine…").clicked() {
                            self.export_original_image();
                        }
                        if ui.button("📤 Exporter tel qu'affiché…").clicked() {
                            self.export_selected();
                        }
                        self.export_options_ui(ui);
                        ui.small("Tirez sur les poignées pour redimensionner.");
                    }
                    (_, Some(_)) => {
                        ui.label("Objet graphique sélectionné");
                        ui.horizontal(|ui| {
                            ui.label("Couleur");
                            if ui.color_edit_button_srgb(&mut self.edit_color).changed() {
                                self.apply_color_only();
                            }
                        });
                        if ui.button("📤 Exporter en image…").clicked() {
                            self.export_selected();
                        }
                        self.export_options_ui(ui);
                        ui.small("Un logo est souvent composé de plusieurs formes : utilisez l'outil « Exporter zone » pour le prendre en entier.");
                    }
                    _ => {
                        if let Some(fi) = self.sel_field.filter(|_| self.tool == Tool::Select) {
                            let f = self.fields[fi].clone();
                            ui.label(format!("Champ : {}", f.name));
                            match f.kind {
                                FieldKind::Text => {
                                    ui.text_edit_multiline(&mut self.field_text);
                                    if ui.button("✔ Valider").clicked() {
                                        let v = self.field_text.clone();
                                        self.set_field_text(fi, v);
                                    }
                                }
                                FieldKind::Check | FieldKind::Radio => {
                                    let mut c = f.checked;
                                    if ui.checkbox(&mut c, "Coché").clicked() {
                                        self.toggle_field(fi);
                                    }
                                }
                                FieldKind::Combo | FieldKind::List => {
                                    ui.label(format!("Valeur : {}", f.value));
                                    ui.small("La modification des listes déroulantes n'est pas encore prise en charge.");
                                }
                                _ => {
                                    ui.small("Ce type de champ n'est pas modifiable.");
                                }
                            }
                        } else {
                            ui.label("Cliquez sur un texte, une image ou un champ de formulaire. Glissez pour déplacer, tirez les poignées pour redimensionner, Suppr pour effacer.");
                        }
                    }
                }
                if self.sel.is_some() && self.tool == Tool::Select {
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("🗑 Supprimer").clicked() {
                            self.delete_selected();
                        }
                        for (lbl, dx, dy) in [("←", -1.0, 0.0), ("→", 1.0, 0.0), ("↑", 0.0, 1.0), ("↓", 0.0, -1.0)] {
                            if ui.small_button(lbl).clicked() {
                                self.move_selected(dx, dy);
                            }
                        }
                    });
                }

                if !self.fields.is_empty() {
                    ui.separator();
                    ui.label(format!("Formulaire — {} champ(s) sur cette page", self.fields.len()));
                    let mut pick = None;
                    let mut toggle = None;
                    for (fi, f) in self.fields.iter().enumerate() {
                        ui.horizontal(|ui| match f.kind {
                            FieldKind::Check | FieldKind::Radio => {
                                let mut c = f.checked;
                                if ui.checkbox(&mut c, &f.name).clicked() {
                                    toggle = Some(fi);
                                }
                            }
                            _ => {
                                let label = if f.value.is_empty() { f.name.clone() } else { format!("{} : {}", f.name, f.value) };
                                if ui.selectable_label(self.sel_field == Some(fi), label).clicked() {
                                    pick = Some(fi);
                                }
                            }
                        });
                    }
                    if let Some(fi) = toggle {
                        self.toggle_field(fi);
                    }
                    if let Some(fi) = pick {
                        self.select_field(fi);
                    }
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.view_w = ui.available_width();
            egui::ScrollArea::both().show(ui, |ui| {
                let Some(tex) = self.texture.clone() else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.heading("RustPDF");
                        ui.label("Ctrl+O pour ouvrir un PDF, ou glissez un fichier ici.");
                        ui.add_space(8.0);
                        if ui.button("📂 Ouvrir un PDF…").clicked() {
                            if let Some(f) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                                self.open(f);
                            }
                        }
                        if !self.recent.is_empty() {
                            ui.add_space(16.0);
                            ui.label("Fichiers récents");
                            if let Some(p) = self.recent_list_ui(ui) {
                                self.open(p);
                            }
                        }
                    });
                    return;
                };
                let s = PT_TO_PX * self.zoom;
                let ph = self.page_size.1;
                let size = Vec2::new(self.page_size.0 * s, ph * s);
                let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                ui.painter().image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);

                let to_screen = |r: &[f32; 4]| {
                    Rect::from_min_max(
                        Pos2::new(rect.min.x + r[0] * s, rect.min.y + (ph - r[3]) * s),
                        Pos2::new(rect.min.x + r[2] * s, rect.min.y + (ph - r[1]) * s),
                    )
                };
                let pos_to_pt = |p: Pos2| ((p.x - rect.min.x) / s, ph - (p.y - rect.min.y) / s);

                let in_select = self.tool == Tool::Select;

                // Surlignage des résultats de recherche (occurrence courante en orange)
                for (mi, m) in self.matches.iter().enumerate().filter(|(_, m)| m.page == self.page) {
                    let cur = Some(mi) == self.match_cur;
                    let fill = if cur {
                        Color32::from_rgba_unmultiplied(255, 140, 0, 120)
                    } else {
                        Color32::from_rgba_unmultiplied(255, 230, 0, 100)
                    };
                    for r in &m.rects {
                        let sr = to_screen(r).expand(1.5);
                        ui.painter().rect_filled(sr, 2.0, fill);
                        if cur {
                            ui.painter().rect_stroke(sr, 2.0, Stroke::new(2.0, Color32::from_rgb(255, 120, 0)), egui::StrokeKind::Outside);
                        }
                    }
                }
                if self.scroll_to_match && !self.needs_render {
                    self.scroll_to_match = false;
                    if let Some(m) = self.match_cur.and_then(|c| self.matches.get(c)).filter(|m| m.page == self.page) {
                        if let Some(r) = m.rects.first() {
                            ui.scroll_to_rect(to_screen(r).expand(90.0), Some(egui::Align::Center));
                        }
                    }
                }

                // Cadres des objets et des champs
                if in_select && self.show_boxes {
                    for o in &self.objects {
                        let col = match o.kind {
                            ObjKind::Text => Color32::from_rgba_unmultiplied(40, 120, 255, 90),
                            ObjKind::Image => Color32::from_rgba_unmultiplied(40, 200, 90, 120),
                            _ => continue,
                        };
                        if Some(o.idx) != self.sel {
                            ui.painter().rect_stroke(to_screen(&o.rect), 0.0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
                        }
                    }
                }
                if in_select && self.show_fields {
                    for (fi, f) in self.fields.iter().enumerate() {
                        let r = to_screen(&f.rect);
                        let strong = self.sel_field == Some(fi);
                        ui.painter().rect_stroke(
                            r,
                            0.0,
                            Stroke::new(if strong { 2.0 } else { 1.0 }, Color32::from_rgb(200, 90, 255)),
                            egui::StrokeKind::Outside,
                        );
                    }
                }

                // Rectangle courant de la sélection (avec aperçu du déplacement / redimensionnement)
                let sel_obj = self.sel.and_then(|s| self.objects.iter().find(|o| o.idx == s)).cloned();
                let mut sel_screen: Option<Rect> = None;
                if let Some(o) = &sel_obj {
                    let base = self.resize.map(|r| r.cur).unwrap_or(o.rect);
                    let r = to_screen(&base).translate(self.drag_accum);
                    sel_screen = Some(r);
                    ui.painter().rect_stroke(r, 0.0, Stroke::new(2.0, Color32::from_rgb(255, 120, 0)), egui::StrokeKind::Outside);
                }

                // Gestion de la sélection / déplacement (outil Sélection)
                let mut start_inline_obj = false;
                if in_select && self.resize.is_none() {
                    if resp.clicked() || resp.drag_started() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            let (px, py) = pos_to_pt(p);
                            let field_hit = self.fields.iter().position(|f| contains(&f.rect, px, py));
                            if let Some(fi) = field_hit {
                                if resp.clicked() && !self.fields[fi].read_only {
                                    match self.fields[fi].kind {
                                        FieldKind::Check | FieldKind::Radio => self.toggle_field(fi),
                                        FieldKind::Text => {
                                            self.select_field(fi);
                                            self.inline = Inline::Field;
                                            self.inline_focus = true;
                                        }
                                        _ => self.select_field(fi),
                                    }
                                }
                            } else {
                                let page_area = self.page_size.0 * self.page_size.1;
                                let hit = self
                                    .objects
                                    .iter()
                                    .rev()
                                    .filter(|o| matches!(o.kind, ObjKind::Text | ObjKind::Image))
                                    .find(|o| contains(&o.rect, px, py))
                                    .or_else(|| {
                                        self.objects
                                            .iter()
                                            .filter(|o| o.kind == ObjKind::Path && contains(&o.rect, px, py))
                                            .filter(|o| (o.rect[2] - o.rect[0]) * (o.rect[3] - o.rect[1]) < page_area * 0.8)
                                            .min_by(|a, b| {
                                                let aa = (a.rect[2] - a.rect[0]) * (a.rect[3] - a.rect[1]);
                                                let bb = (b.rect[2] - b.rect[0]) * (b.rect[3] - b.rect[1]);
                                                aa.partial_cmp(&bb).unwrap()
                                            })
                                    })
                                    .map(|o| o.idx);
                                if hit != self.sel {
                                    self.select(hit);
                                } else if hit.is_none() {
                                    self.sel_field = None;
                                }
                            }
                        }
                    }
                    if resp.double_clicked() && self.sel.is_some() {
                        if let Some(o) = &sel_obj {
                            if o.kind == ObjKind::Text {
                                start_inline_obj = true;
                            }
                        }
                    }
                    if resp.dragged() && self.sel.is_some() && self.inline == Inline::None {
                        self.drag_accum += resp.drag_delta();
                    }
                    if resp.drag_stopped() && self.sel.is_some() {
                        let d = self.drag_accum;
                        self.drag_accum = Vec2::ZERO;
                        if d.length() > 1.0 {
                            self.move_selected(d.x / s, -d.y / s);
                        }
                    }
                }
                if start_inline_obj {
                    self.inline = Inline::Object;
                    self.inline_focus = true;
                }

                // Poignées de redimensionnement
                if in_select && self.inline == Inline::None {
                    if let (Some(o), Some(r)) = (&sel_obj, sel_screen) {
                        let handles: [(i8, i8); 8] = [(-1, 1), (0, 1), (1, 1), (-1, 0), (1, 0), (-1, -1), (0, -1), (1, -1)];
                        let mut finish: Option<Resize> = None;
                        for (hi, (hx, hy)) in handles.iter().enumerate() {
                            let cx = match hx {
                                -1 => r.min.x,
                                0 => r.center().x,
                                _ => r.max.x,
                            };
                            let cy = match hy {
                                1 => r.min.y,
                                0 => r.center().y,
                                _ => r.max.y,
                            };
                            let hr = Rect::from_center_size(Pos2::new(cx, cy), Vec2::splat(HANDLE));
                            let hresp = ui.interact(hr.expand(3.0), egui::Id::new(("rh", hi)), Sense::drag());
                            ui.painter().rect_filled(hr, 1.0, Color32::WHITE);
                            ui.painter().rect_stroke(hr, 1.0, Stroke::new(1.5, Color32::from_rgb(255, 120, 0)), egui::StrokeKind::Inside);
                            if hresp.hovered() || hresp.dragged() {
                                ctx.set_cursor_icon(cursor_for(*hx, *hy));
                            }
                            if hresp.drag_started() {
                                self.resize = Some(Resize { hx: *hx, hy: *hy, orig: o.rect, cur: o.rect });
                            }
                            if hresp.dragged() {
                                if let (Some(rs), Some(p)) = (self.resize.as_mut(), hresp.interact_pointer_pos()) {
                                    let (px, py) = pos_to_pt(p);
                                    let mut n = rs.orig;
                                    if rs.hx == -1 {
                                        n[0] = px.min(n[2] - 2.0);
                                    } else if rs.hx == 1 {
                                        n[2] = px.max(n[0] + 2.0);
                                    }
                                    if rs.hy == -1 {
                                        n[1] = py.min(n[3] - 2.0);
                                    } else if rs.hy == 1 {
                                        n[3] = py.max(n[1] + 2.0);
                                    }
                                    // Coins : proportions conservées
                                    if rs.hx != 0 && rs.hy != 0 {
                                        let (ow, oh) = (rs.orig[2] - rs.orig[0], rs.orig[3] - rs.orig[1]);
                                        let k = ((n[2] - n[0]) / ow).max((n[3] - n[1]) / oh).max(0.02);
                                        let (nw, nh) = (ow * k, oh * k);
                                        if rs.hx == -1 {
                                            n[0] = rs.orig[2] - nw;
                                            n[2] = rs.orig[2];
                                        } else {
                                            n[0] = rs.orig[0];
                                            n[2] = rs.orig[0] + nw;
                                        }
                                        if rs.hy == -1 {
                                            n[1] = rs.orig[3] - nh;
                                            n[3] = rs.orig[3];
                                        } else {
                                            n[1] = rs.orig[1];
                                            n[3] = rs.orig[1] + nh;
                                        }
                                    }
                                    rs.cur = n;
                                }
                            }
                            if hresp.drag_stopped() {
                                finish = self.resize;
                            }
                        }
                        if let Some(rs) = finish {
                            self.resize = None;
                            self.resize_selected(rs.orig, rs.cur);
                        }
                    }
                }

                // Éditeur de texte directement sur la page
                if self.inline != Inline::None {
                    let target = match self.inline {
                        Inline::Object => sel_obj.as_ref().map(|o| o.rect),
                        Inline::Field => self.sel_field.and_then(|f| self.fields.get(f)).map(|f| f.rect),
                        Inline::None => None,
                    };
                    if let Some(tr) = target {
                        let r = to_screen(&tr);
                        let fsize = if self.inline == Inline::Object { self.edit_size } else { 12.0 };
                        let mut commit = false;
                        let mut cancel = false;
                        let focus = self.inline_focus;
                        let is_obj = self.inline == Inline::Object;
                        egui::Area::new(egui::Id::new("inline_editor"))
                            .order(egui::Order::Foreground)
                            .fixed_pos(r.min - Vec2::new(4.0, 4.0))
                            .show(ctx, |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    let buf = if is_obj { &mut self.edit_text } else { &mut self.field_text };
                                    let te = egui::TextEdit::singleline(buf)
                                        .desired_width((r.width() + 60.0).max(160.0))
                                        .font(FontId::proportional((fsize * s).clamp(11.0, 60.0)));
                                    let er = ui.add(te);
                                    if focus {
                                        er.request_focus();
                                    }
                                    if er.lost_focus() {
                                        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                                            cancel = true;
                                        } else {
                                            commit = true;
                                        }
                                    }
                                    ui.small("Entrée : valider · Échap : annuler");
                                });
                            });
                        self.inline_focus = false;
                        if cancel {
                            self.inline = Inline::None;
                        } else if commit {
                            let which = self.inline;
                            self.inline = Inline::None;
                            match which {
                                Inline::Object => self.apply_text(),
                                Inline::Field => {
                                    if let Some(fi) = self.sel_field {
                                        let v = self.field_text.clone();
                                        self.set_field_text(fi, v);
                                    }
                                }
                                Inline::None => {}
                            }
                        }
                    } else {
                        self.inline = Inline::None;
                    }
                }

                // Outils d'ajout
                match self.tool {
                    Tool::AddText => {
                        if resp.clicked() {
                            if let Some(p) = resp.interact_pointer_pos() {
                                let (x, y) = pos_to_pt(p);
                                self.add_text_at(x, y);
                                self.tool = Tool::Select;
                            }
                        }
                    }
                    Tool::AddImage => {
                        if resp.clicked() {
                            if let Some(p) = resp.interact_pointer_pos() {
                                let (x, y) = pos_to_pt(p);
                                self.add_image_at(x, y);
                                self.tool = Tool::Select;
                            }
                        }
                    }
                    Tool::ExportZone => {
                        ctx.set_cursor_icon(CursorIcon::Crosshair);
                        if resp.drag_started() {
                            if let Some(p) = resp.interact_pointer_pos() {
                                let (x, y) = pos_to_pt(p);
                                self.zone = Some(([x, y], [x, y]));
                            }
                        }
                        if resp.dragged() {
                            if let (Some(z), Some(p)) = (self.zone.as_mut(), resp.interact_pointer_pos()) {
                                let (x, y) = pos_to_pt(p);
                                z.1 = [x, y];
                            }
                        }
                        if let Some((a, b)) = self.zone {
                            let r = [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])];
                            let sr = to_screen(&r);
                            ui.painter().rect_filled(sr, 0.0, Color32::from_rgba_unmultiplied(40, 120, 255, 40));
                            ui.painter().rect_stroke(sr, 0.0, Stroke::new(1.5, Color32::from_rgb(40, 120, 255)), egui::StrokeKind::Outside);
                            if resp.drag_stopped() {
                                self.zone = None;
                                if r[2] - r[0] > 2.0 && r[3] - r[1] > 2.0 {
                                    self.export_region(r);
                                }
                            }
                        }
                    }
                    Tool::Select => {}
                }
            });
        });
    }
}

fn main() -> eframe::Result<()> {
    let pdfium = match load_pdfium() {
        Ok(p) => p,
        Err(e) => {
            rfd::MessageDialog::new()
                .set_title("RustPDF")
                .set_description(format!("{e}"))
                .show();
            std::process::exit(1);
        }
    };
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 860.0]).with_title("RustPDF"),
        ..Default::default()
    };
    eframe::run_native(
        "RustPDF",
        opts,
        Box::new(move |_cc| {
            let mut app = App::new(pdfium);
            if let Some(p) = arg {
                app.open(p);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(idx: usize, text: &str, x0: f32, x1: f32, color: [u8; 3], leader: bool) -> Run {
        Run { idx, text: text.into(), font: "Calibri".into(), size: 14.0, color, base: 700.0, rect: [x0, 698.0, x1, 714.0], leader }
    }

    #[test]
    fn export_isolates_selected_objects() {
        let pdfium = load_pdfium().unwrap();
        let mut doc = pdfium.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let solid = |c: [u8; 4]| image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(20, 20, image::Rgba(c)));
        page.objects_mut().create_image_object(PdfPoints::new(100.0), PdfPoints::new(500.0), &solid([255, 0, 0, 255]), Some(PdfPoints::new(50.0)), Some(PdfPoints::new(50.0))).unwrap();
        page.objects_mut().create_image_object(PdfPoints::new(300.0), PdfPoints::new(500.0), &solid([0, 0, 255, 255]), Some(PdfPoints::new(50.0)), Some(PdfPoints::new(50.0))).unwrap();
        page.regenerate_content().unwrap();
        drop(page);
        let mut app = App::new(pdfium);
        app.doc = Some(doc);
        app.export_scale = 144.0;
        let blue_zone = [290.0, 490.0, 360.0, 560.0];
        let img = app.render_isolated(Some(&[1]), blue_zone, true).unwrap().to_rgba8();
        let p = img.get_pixel(img.width() / 2, img.height() / 2);
        assert!(p[2] > 200 && p[3] == 255, "centre bleu opaque, obtenu {p:?}");
        let corner = img.get_pixel(0, 0);
        assert_eq!(corner[3], 0, "coin transparent, obtenu {corner:?}");
        // On garde seulement le rouge, mais on découpe la zone bleue : plus rien
        let img = app.render_isolated(Some(&[0]), blue_zone, true).unwrap().to_rgba8();
        assert!(img.pixels().all(|p| p[3] == 0), "la zone bleue doit etre vide");
    }

    #[test]
    fn search_counts_cases_and_words() {
        let pdfium = load_pdfium().unwrap();
        let mut doc = pdfium.create_new_pdf().unwrap();
        for (i, t) in ["Bonjour monde, bonjour", "rien ici", "BONJOUR bonjourno"].iter().enumerate() {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
            let font = doc.fonts_mut().helvetica();
            page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), *t, font, PdfPoints::new(14.0)).unwrap();
            page.regenerate_content().unwrap();
            let _ = i;
        }
        let mut app = App::new(pdfium);
        app.doc = Some(doc);
        app.search = "bonjour".into();
        let m = app.collect_matches();
        assert_eq!(m.len(), 4, "insensible a la casse: 2 + 0 + 2");
        assert_eq!(m.iter().filter(|h| h.page == 1).count(), 0);
        assert!(m[0].rects[0][2] > m[0].rects[0][0]);
        app.search_case = true;
        assert_eq!(app.collect_matches().len(), 2, "casse: bonjour + bonjourno");
        app.search_case = false;
        app.search_word = true;
        assert_eq!(app.collect_matches().len(), 3, "mot entier: exclut bonjourno");
        // navigation circulaire
        app.search_word = false;
        app.run_search();
        assert_eq!(app.match_cur, Some(0));
        app.prev_match();
        assert_eq!(app.match_cur, Some(3));
        app.next_match();
        assert_eq!(app.match_cur, Some(0));
        assert_eq!(app.page, 0);
        app.goto_match(3);
        assert_eq!(app.page, 2);
    }

    #[test]
    fn recent_files_are_deduped_ordered_and_persisted() {
        let dir = std::env::temp_dir().join(format!("rustpdf_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("APPDATA", &dir) };
        let pdfium = load_pdfium().unwrap();
        let mut app = App::new(pdfium);
        app.recent.clear();
        let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        app.add_recent(&a);
        app.add_recent(&b);
        app.add_recent(&a);
        assert_eq!(app.recent.len(), 2);
        assert!(app.recent[0].ends_with("a.pdf") && app.recent[1].ends_with("b.pdf"));
        let reloaded = load_recent();
        assert_eq!(reloaded, app.recent);
        for i in 0..20 {
            let p = dir.join(format!("f{i}.pdf"));
            std::fs::write(&p, b"x").unwrap();
            app.add_recent(&p);
        }
        assert_eq!(app.recent.len(), MAX_RECENT);
        app.remove_recent(&app.recent[0].clone());
        assert_eq!(app.recent.len(), MAX_RECENT - 1);
        app.clear_recent();
        assert!(load_recent().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn words_of_a_line_become_one_block() {
        let runs = vec![
            run(0, "COMMENT", 50.0, 110.0, [0, 0, 0], false),
            run(1, "ABORDER", 114.0, 175.0, [0, 0, 0], false),
            run(2, "CE", 179.0, 195.0, [0, 0, 0], false),
            run(3, "LIVRE ?", 199.0, 240.0, [0, 0, 0], false),
            run(4, "...............", 244.0, 480.0, [0, 0, 0], true),
            run(5, "11", 500.0, 515.0, [0, 0, 0], false),
            run(6, "Titre", 50.0, 90.0, [255, 60, 0], false), // autre couleur -> autre bloc
        ];
        let b = group_runs(runs);
        let texts: Vec<_> = b.iter().map(|o| o.text.as_str()).collect();
        assert!(texts.contains(&"COMMENT ABORDER CE LIVRE ?"), "{texts:?}");
        assert!(texts.contains(&"11") && texts.contains(&"Titre"), "{texts:?}");
        assert_eq!(b.len(), 4, "{texts:?}");
        let main = b.iter().find(|o| o.members.len() == 4).unwrap();
        assert_eq!(main.members, vec![0, 1, 2, 3]);
    }
}
