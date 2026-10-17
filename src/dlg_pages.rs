//! Dialogs: page operations (insert, extract, split, merge, crop, size, numbers, watermark…).

use crate::app::App;
use crate::dlg_common::*;
use crate::edit_text::TextStyle;
use crate::fonts::{Builtin, FontFamily};
use crate::geom::*;
use crate::i18n::t;
use crate::pageops::{HAlign, StampItem, StampSpec, VAlign, format_ranges, parse_ranges};
use crate::pdfedit_doc::{WatermarkKind, WatermarkSpec};
use crate::prefs::Unit;
use eframe::egui::{self, Color32, RichText};
use std::path::PathBuf;

fn err_label(ui: &mut egui::Ui, e: &str) {
    ui.colored_label(Color32::from_rgb(220, 70, 70), e);
}

fn units_drag(ui: &mut egui::Ui, v: &mut f32, unit: Unit, max: f32) {
    let mut x = unit.from_pt(*v);
    if ui.add(egui::DragValue::new(&mut x).range(0.0..=unit.from_pt(max)).speed(0.2).suffix(format!(" {}", unit.label()))).changed() {
        *v = unit.to_pt(x);
    }
}

fn pages_field(ui: &mut egui::Ui, text: &mut String, n: usize, cur: usize) -> Option<Vec<usize>> {
    ui.horizontal(|ui| {
        ui.label(t("pages_to_process"));
        ui.add(egui::TextEdit::singleline(text).desired_width(150.0));
        if ui.small_button(t("current_page")).clicked() {
            *text = format!("{}", cur + 1);
        }
        if ui.small_button(t("all_pages")).clicked() {
            *text = format!("1-{n}");
        }
    });
    match parse_ranges(text, n) {
        Ok(v) => Some(v),
        Err(e) => {
            err_label(ui, &e);
            None
        }
    }
}

impl App {
    fn cur_page(&self) -> usize {
        self.tab_ref().map(|t| t.view.page).unwrap_or(0)
    }

    fn n_pages(&self) -> usize {
        self.tab_ref().map(|t| t.page_count()).unwrap_or(0)
    }

    /// Pages of the thumbnail selection, or the current page.
    pub fn selected_pages(&self) -> Vec<usize> {
        if self.page_sel.is_empty() { vec![self.cur_page()] } else { let mut v = self.page_sel.clone(); v.sort_unstable(); v }
    }

    pub fn open_dlg2(&mut self, d: Dlg2) {
        self.dlg2 = Some(d);
    }

    pub fn dlg_insert_blank(&mut self) {
        self.open_dlg2(Dlg2::InsertBlank(InsertBlank { size: usize::MAX, landscape: false, custom: [595.0, 842.0], count: 1, position: 0 }));
    }

    pub fn dlg_insert_file(&mut self) {
        let Some(f) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() else { return };
        match std::fs::read(&f) {
            Ok(bytes) => {
                let n = crate::pdfx::open_bytes(self.pdfium, bytes.clone(), None).map(|d| d.pages().len() as usize);
                match n {
                    Ok(n) => self.open_dlg2(Dlg2::InsertFile(InsertFile { path: f, bytes, n_other: n, pages: format!("1-{n}"), position: 0, page_no: 1 })),
                    Err(_) => self.err(t("err_open")),
                }
            }
            Err(e) => self.err(format!("{}: {e}", t("err_open"))),
        }
    }

    pub fn dlg_extract(&mut self) {
        let p = self.selected_pages();
        self.open_dlg2(Dlg2::Extract(Extract { pages: format_ranges(&p), remove_after: false }));
    }

    pub fn dlg_split(&mut self) {
        let n = self.n_pages();
        self.open_dlg2(Dlg2::Split(Split { mode: 0, every: n.max(2) / 2, ranges: "1-2\n3-".into(), max_mb: 5.0 }));
    }

    pub fn dlg_merge(&mut self) {
        let Some(files) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_files() else { return };
        self.open_dlg2(Dlg2::Merge(Merge { files, include_current: true, new_document: true }));
    }

    pub fn dlg_crop(&mut self) {
        let p = self.selected_pages();
        self.open_dlg2(Dlg2::Crop(Crop { pages: format_ranges(&p), margins: [0.0; 4] }));
    }

    pub fn dlg_page_size(&mut self) {
        let n = self.n_pages();
        self.open_dlg2(Dlg2::PageSize(PageSizeD { pages: format!("1-{n}"), preset: 1, landscape: false, custom: [595.0, 842.0], fit: true }));
    }

    pub fn dlg_stamp(&mut self, mode: StampMode) {
        let n = self.n_pages();
        self.open_dlg2(Dlg2::Stamp(StampD {
            mode,
            pages: format!("1-{n}"),
            format: "Page {n} of {N}".into(),
            pos: (HAlign::Center, VAlign::Bottom),
            header: [String::new(), "{title}".into(), String::new()],
            footer: [String::new(), "Page {n} / {N}".into(), "{date}".into()],
            family: 0,
            size: 10.0,
            bold: false,
            italic: false,
            color: [60, 60, 60],
            margin: 28.0,
            start: 1,
            skip_first: false,
        }));
    }

    pub fn dlg_watermark(&mut self) {
        let n = self.n_pages();
        self.open_dlg2(Dlg2::Watermark(WmD { text_mode: true, text: "CONFIDENTIAL".into(), image: None, pages: format!("1-{n}"), size: 72.0, color: [200, 0, 0], opacity: 0.25, rotation: 45.0, behind: false }));
    }

    pub fn dlg_pages_action(&mut self, delete: bool) {
        let p = self.selected_pages();
        self.open_dlg2(Dlg2::Pages(PagesAction { delete, pages: format_ranges(&p), angle: 90 }));
    }

    /// Shows the operation dialog currently open.
    pub fn show_dlg2(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.dlg2.take() else { return };
        let keep = match &mut d {
            Dlg2::InsertBlank(x) => self.ui_insert_blank(ctx, x),
            Dlg2::InsertFile(x) => self.ui_insert_file(ctx, x),
            Dlg2::Extract(x) => self.ui_extract(ctx, x),
            Dlg2::Split(x) => self.ui_split(ctx, x),
            Dlg2::Merge(x) => self.ui_merge(ctx, x),
            Dlg2::Crop(x) => self.ui_crop(ctx, x),
            Dlg2::PageSize(x) => self.ui_page_size(ctx, x),
            Dlg2::Stamp(x) => self.ui_stamp(ctx, x),
            Dlg2::Watermark(x) => self.ui_watermark(ctx, x),
            Dlg2::Pages(x) => self.ui_pages_action(ctx, x),
            other => self.show_dlg_doc(ctx, other),
        };
        if keep && self.dlg2.is_none() {
            self.dlg2 = Some(d);
        }
    }

    fn ok_cancel(ui: &mut egui::Ui, ok_label: &str, enabled: bool) -> (bool, bool) {
        let mut ok = false;
        let mut cancel = false;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(enabled, egui::Button::new(RichText::new(ok_label).strong())).clicked() {
                ok = true;
            }
            if ui.button(t("cancel")).clicked() {
                cancel = true;
            }
        });
        (ok, cancel)
    }

    fn report(&mut self, r: Result<(), String>, ok_msg: &str) {
        match r {
            Ok(_) => self.say(ok_msg.to_string()),
            Err(e) => self.err(e),
        }
        self.page_sel.clear();
        self.text_ui.key = None;
    }

    // ------------------------------------------------------------ insert a blank page

    fn ui_insert_blank(&mut self, ctx: &egui::Context, x: &mut InsertBlank) -> bool {
        let mut keep = true;
        let unit = self.prefs.units;
        let cur = self.cur_page();
        let n = self.n_pages();
        let cur_size = self.tab_ref().and_then(|t| t.sizes.get(cur).copied()).unwrap_or((595.0, 842.0));
        let mut apply = false;
        modal(t("cmd_insert_blank")).show(ctx, |ui| {
            egui::ComboBox::from_label(t("page_size")).selected_text(if x.size == usize::MAX { t("same_as_current").to_string() } else if x.size == usize::MAX - 1 { t("custom").to_string() } else { PAGE_SIZES[x.size].0.to_string() }).show_ui(ui, |ui| {
                ui.selectable_value(&mut x.size, usize::MAX, t("same_as_current"));
                for (i, (nm, _, _)) in PAGE_SIZES.iter().enumerate() {
                    ui.selectable_value(&mut x.size, i, *nm);
                }
                ui.selectable_value(&mut x.size, usize::MAX - 1, t("custom"));
            });
            if x.size == usize::MAX - 1 {
                ui.horizontal(|ui| {
                    units_drag(ui, &mut x.custom[0], unit, 5000.0);
                    ui.label("×");
                    units_drag(ui, &mut x.custom[1], unit, 5000.0);
                });
            }
            if x.size < PAGE_SIZES.len() {
                ui.checkbox(&mut x.landscape, t("landscape"));
            }
            ui.horizontal(|ui| {
                ui.label(t("count"));
                ui.add(egui::DragValue::new(&mut x.count).range(1..=100));
            });
            egui::ComboBox::from_label(t("position")).selected_text(t(["pos_after", "pos_before", "pos_start", "pos_end"][x.position])).show_ui(ui, |ui| {
                for (i, k) in ["pos_after", "pos_before", "pos_start", "pos_end"].iter().enumerate() {
                    ui.selectable_value(&mut x.position, i, t(k));
                }
            });
            let (ok, cancel) = Self::ok_cancel(ui, t("insert"), true);
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if apply {
            let (w, h) = if x.size == usize::MAX { cur_size } else if x.size == usize::MAX - 1 { (x.custom[0], x.custom[1]) } else { preset_size(x.size, x.landscape) };
            let at = match x.position {
                0 => cur + 1,
                1 => cur,
                2 => 0,
                _ => n,
            };
            let mem = self.mem();
            let count = x.count;
            let r = self.tab().map(|tb| (0..count).try_for_each(|k| tb.insert_blank_at(at + k, w, h, mem)));
            if let Some(r) = r {
                self.report(r, t("pages_inserted"));
            }
        }
        keep
    }

    // ------------------------------------------------------------ insert from a file

    fn ui_insert_file(&mut self, ctx: &egui::Context, x: &mut InsertFile) -> bool {
        let mut keep = true;
        let n = self.n_pages();
        let mut apply = false;
        modal(t("cmd_insert_from_file")).show(ctx, |ui| {
            ui.label(x.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default());
            ui.label(format!("{} {}", x.n_other, t("pages")));
            let pages = pages_field(ui, &mut x.pages, x.n_other, 0);
            egui::ComboBox::from_label(t("position")).selected_text(t(["pos_start", "pos_end", "pos_before_n", "pos_after_n"][x.position])).show_ui(ui, |ui| {
                for (i, k) in ["pos_start", "pos_end", "pos_before_n", "pos_after_n"].iter().enumerate() {
                    ui.selectable_value(&mut x.position, i, t(k));
                }
            });
            if x.position >= 2 {
                ui.horizontal(|ui| {
                    ui.label(t("page"));
                    ui.add(egui::DragValue::new(&mut x.page_no).range(1..=n.max(1)));
                });
            }
            let (ok, cancel) = Self::ok_cancel(ui, t("insert"), pages.is_some());
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if apply {
            if let Ok(pages) = parse_ranges(&x.pages, x.n_other) {
                let at = match x.position {
                    0 => 0,
                    1 => n,
                    2 => x.page_no.saturating_sub(1),
                    _ => x.page_no,
                };
                let mem = self.mem();
                let bytes = std::mem::take(&mut x.bytes);
                let r = self.tab().map(|tb| tb.insert_from_bytes(&bytes, &pages, at, mem));
                if let Some(r) = r {
                    self.report(r, t("pages_inserted"));
                }
            }
        }
        keep
    }

    // ----------------------------------------------------------------------- extract

    fn ui_extract(&mut self, ctx: &egui::Context, x: &mut Extract) -> bool {
        let mut keep = true;
        let n = self.n_pages();
        let cur = self.cur_page();
        let mut apply = false;
        modal(t("cmd_extract_page")).show(ctx, |ui| {
            let pages = pages_field(ui, &mut x.pages, n, cur);
            ui.checkbox(&mut x.remove_after, t("remove_after_extract"));
            let (ok, cancel) = Self::ok_cancel(ui, t("extract"), pages.is_some());
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if apply {
            if let Ok(pages) = parse_ranges(&x.pages, n) {
                let name = self.tab_ref().map(|t| t.title.trim_end_matches(".pdf").to_string()).unwrap_or_default();
                let Some(mut path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name(format!("{name}_extrait.pdf")).save_file() else { return false };
                if path.extension().is_none() {
                    path.set_extension("pdf");
                }
                let r = self.tab_ref().map(|tb| tb.extract_bytes(&pages));
                match r {
                    Some(Ok(b)) => match std::fs::write(&path, b) {
                        Ok(_) => {
                            self.say(format!("{}: {}", t("extracted"), path.display()));
                            if x.remove_after {
                                let mem = self.mem();
                                let r = self.tab().map(|tb| tb.delete_page_list(&pages, mem));
                                if let Some(Err(e)) = r {
                                    self.err(e);
                                }
                            }
                        }
                        Err(e) => self.err(e.to_string()),
                    },
                    Some(Err(e)) => self.err(e),
                    None => {}
                }
            }
        }
        keep
    }

    // --------------------------------------------------------------------- split

    fn ui_split(&mut self, ctx: &egui::Context, x: &mut Split) -> bool {
        let mut keep = true;
        let n = self.n_pages();
        let mut apply = false;
        modal(t("cmd_split")).show(ctx, |ui| {
            for (i, k) in ["split_every", "split_ranges", "split_size", "split_bookmarks"].iter().enumerate() {
                ui.radio_value(&mut x.mode, i, t(k));
            }
            match x.mode {
                0 => {
                    ui.horizontal(|ui| {
                        ui.label(t("pages_per_file"));
                        ui.add(egui::DragValue::new(&mut x.every).range(1..=n.max(1)));
                    });
                }
                1 => {
                    ui.label(t("split_ranges_hint"));
                    ui.add(egui::TextEdit::multiline(&mut x.ranges).desired_rows(4).desired_width(260.0));
                }
                2 => {
                    ui.horizontal(|ui| {
                        ui.label(t("max_size"));
                        ui.add(egui::DragValue::new(&mut x.max_mb).range(0.05..=500.0).suffix(" Mo"));
                    });
                }
                _ => {
                    ui.small(t("split_bookmarks_hint"));
                }
            }
            let (ok, cancel) = Self::ok_cancel(ui, t("split"), true);
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if apply {
            let Some(dir) = rfd::FileDialog::new().pick_folder() else { return false };
            let mode = if x.mode == 1 {
                let groups: Result<Vec<Vec<usize>>, String> = x.ranges.lines().filter(|l| !l.trim().is_empty()).map(|l| parse_ranges(l, n)).collect();
                match groups {
                    Ok(g) if !g.is_empty() => crate::pageops::SplitMode::Ranges(g),
                    Ok(_) => {
                        self.err(t("err_no_ranges"));
                        return false;
                    }
                    Err(e) => {
                        self.err(e);
                        return false;
                    }
                }
            } else {
                match split_mode(x) {
                    Ok(m) => m,
                    Err(e) => {
                        self.err(e);
                        return false;
                    }
                }
            };
            let base = self.tab_ref().map(|t| t.title.trim_end_matches(".pdf").to_string()).unwrap_or_default();
            let r = self.tab_ref().map(|tb| tb.split_groups(&mode).and_then(|g| tb.split_bytes(&g).map(|b| (g, b))));
            match r {
                Some(Ok((groups, parts))) => {
                    let mut written = 0;
                    for (i, (g, bytes)) in groups.iter().zip(parts).enumerate() {
                        let f = dir.join(format!("{base}_partie{:02}_p{}.pdf", i + 1, format_ranges(g).replace(", ", "_")));
                        if std::fs::write(&f, bytes).is_ok() {
                            written += 1;
                        }
                    }
                    self.say(format!("{written} {} → {}", t("files_created"), dir.display()));
                }
                Some(Err(e)) => self.err(e),
                None => {}
            }
        }
        keep
    }

    // ------------------------------------------------------------------------ merge

    fn ui_merge(&mut self, ctx: &egui::Context, x: &mut Merge) -> bool {
        let mut keep = true;
        let mut apply = false;
        modal(t("cmd_merge")).show(ctx, |ui| {
            ui.checkbox(&mut x.include_current, t("merge_include_current"));
            ui.label(t("merge_files"));
            let mut remove = None;
            let mut swap: Option<(usize, usize)> = None;
            let len = x.files.len();
            for (i, f) in x.files.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    if ui.add_enabled(i > 0, egui::Button::new("▲")).clicked() {
                        swap = Some((i, i - 1));
                    }
                    if ui.add_enabled(i + 1 < len, egui::Button::new("▼")).clicked() {
                        swap = Some((i, i + 1));
                    }
                    if ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some((a, b)) = swap {
                x.files.swap(a, b);
            }
            if let Some(i) = remove {
                x.files.remove(i);
            }
            if ui.button(t("add_files")).clicked() {
                if let Some(more) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_files() {
                    x.files.extend(more);
                }
            }
            ui.separator();
            ui.radio_value(&mut x.new_document, true, t("merge_new_doc"));
            ui.radio_value(&mut x.new_document, false, t("merge_append"));
            let (ok, cancel) = Self::ok_cancel(ui, t("merge"), !x.files.is_empty() || x.include_current);
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if apply {
            let mut all: Vec<Vec<u8>> = vec![];
            if x.include_current {
                if let Some(Ok(b)) = self.tab_ref().map(|t| t.doc.save_to_bytes()) {
                    all.push(b);
                }
            }
            for f in &x.files {
                match std::fs::read(f) {
                    Ok(b) => all.push(b),
                    Err(e) => {
                        self.err(format!("{}: {e}", f.display()));
                        return false;
                    }
                }
            }
            if all.len() < 2 && !(all.len() == 1 && !x.include_current) {
                self.err(t("err_need_two"));
                return false;
            }
            let mem = self.mem();
            if x.new_document || !x.include_current {
                match crate::pdfedit_pages::merge_documents(&all) {
                    Ok(bytes) => match crate::pdfx::open_bytes(self.pdfium, bytes, None) {
                        Ok(doc) => {
                            let (zoom, fit) = self.initial_zoom();
                            let mut tab = crate::tab::Tab::new(self.pdfium, doc, None, None, zoom);
                            tab.view.fit = fit;
                            tab.title = t("merged_doc").to_string();
                            tab.dirty = true;
                            self.tabs.push(tab);
                            self.active = self.tabs.len() - 1;
                            self.say(t("merged_ok"));
                        }
                        Err(_) => self.err(t("err_open")),
                    },
                    Err(e) => self.err(e),
                }
            } else {
                let others: Vec<Vec<u8>> = all.into_iter().skip(1).collect();
                let r = self.tab().map(|tb| tb.merge_bytes(&others, mem));
                if let Some(r) = r {
                    self.report(r, t("merged_ok"));
                }
            }
        }
        keep
    }

    // -------------------------------------------------------------------------- crop

    fn ui_crop(&mut self, ctx: &egui::Context, x: &mut Crop) -> bool {
        let mut keep = true;
        let unit = self.prefs.units;
        let n = self.n_pages();
        let cur = self.cur_page();
        let mut apply = false;
        let mut detect = false;
        modal(t("cmd_crop_pages")).show(ctx, |ui| {
            let pages = pages_field(ui, &mut x.pages, n, cur);
            egui::Grid::new("crop").num_columns(2).show(ui, |ui| {
                for (i, l) in ["margin_left", "margin_top", "margin_right", "margin_bottom"].iter().enumerate() {
                    ui.label(t(l));
                    units_drag(ui, &mut x.margins[i], unit, 1000.0);
                    ui.end_row();
                }
            });
            if ui.button(t("detect_margins")).clicked() {
                detect = true;
            }
            let (ok, cancel) = Self::ok_cancel(ui, t("apply"), pages.is_some());
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if detect {
            // margins = empty space around the content of the current page
            if let Some(info) = self.tab().and_then(|tb| tb.info(cur)) {
                if let Some(u) = info.objects.iter().map(|o| o.rect).reduce(|a, b| r4_union(&a, &b)) {
                    let g = info.geom;
                    let v = g.rect_to_view(&u);
                    x.margins = [(v[0] - 8.0).max(0.0), (v[1] - 8.0).max(0.0), (g.w - v[2] - 8.0).max(0.0), (g.h - v[3] - 8.0).max(0.0)];
                }
            }
            return true;
        }
        if apply {
            if let Ok(pages) = parse_ranges(&x.pages, n) {
                // the margins are expressed in the view (top/bottom): convert them to PDF space page by page
                let m = x.margins;
                let mem = self.mem();
                let r = self.tab().map(|tb| {
                    let mut res = Ok(());
                    for &p in &pages {
                        let rot = tb.info(p).map(|i| i.geom).map(|g| g.to_view.a.abs() < 0.5).unwrap_or(false);
                        // page rotated by 90°/270°: the margins rotate too
                        let margins = if rot { [m[3], m[0], m[1], m[2]] } else { [m[0], m[3], m[2], m[1]] };
                        if let Err(e) = tb.crop_page_list(&[p], Some(margins), None, mem) {
                            res = Err(e);
                            break;
                        }
                    }
                    res
                });
                if let Some(r) = r {
                    self.report(r, t("pages_cropped"));
                }
            }
        }
        keep
    }

    // ---------------------------------------------------------------------- page size

    fn ui_page_size(&mut self, ctx: &egui::Context, x: &mut PageSizeD) -> bool {
        let mut keep = true;
        let unit = self.prefs.units;
        let n = self.n_pages();
        let cur = self.cur_page();
        let mut apply = false;
        modal(t("cmd_page_size")).show(ctx, |ui| {
            let pages = pages_field(ui, &mut x.pages, n, cur);
            egui::ComboBox::from_label(t("page_size")).selected_text(if x.preset == usize::MAX { t("custom").to_string() } else { PAGE_SIZES[x.preset].0.to_string() }).show_ui(ui, |ui| {
                for (i, (nm, _, _)) in PAGE_SIZES.iter().enumerate() {
                    ui.selectable_value(&mut x.preset, i, *nm);
                }
                ui.selectable_value(&mut x.preset, usize::MAX, t("custom"));
            });
            if x.preset == usize::MAX {
                ui.horizontal(|ui| {
                    units_drag(ui, &mut x.custom[0], unit, 5000.0);
                    ui.label("×");
                    units_drag(ui, &mut x.custom[1], unit, 5000.0);
                });
            } else {
                ui.checkbox(&mut x.landscape, t("landscape"));
            }
            ui.checkbox(&mut x.fit, t("keep_proportions"));
            let (ok, cancel) = Self::ok_cancel(ui, t("apply"), pages.is_some());
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if apply {
            if let Ok(pages) = parse_ranges(&x.pages, n) {
                let (w, h) = if x.preset == usize::MAX { (x.custom[0], x.custom[1]) } else { preset_size(x.preset, x.landscape) };
                let mem = self.mem();
                let fit = x.fit;
                let r = self.tab().map(|tb| tb.resize_page_list(&pages, w.max(20.0), h.max(20.0), fit, mem));
                if let Some(r) = r {
                    self.report(r, t("page_size_changed"));
                }
            }
        }
        keep
    }

    // ------------------------------------------------- page numbers, headers and footers

    fn ui_stamp(&mut self, ctx: &egui::Context, x: &mut StampD) -> bool {
        let mut keep = true;
        let n = self.n_pages();
        let cur = self.cur_page();
        let mut apply = false;
        let title = if x.mode == StampMode::Numbers { t("cmd_page_numbers") } else { t("cmd_header_footer") };
        modal(title).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut x.mode, StampMode::Numbers, t("cmd_page_numbers"));
                ui.selectable_value(&mut x.mode, StampMode::HeaderFooter, t("cmd_header_footer"));
            });
            let pages = pages_field(ui, &mut x.pages, n, cur);
            match x.mode {
                StampMode::Numbers => {
                    egui::ComboBox::from_label(t("number_format")).selected_text(x.format.clone()).show_ui(ui, |ui| {
                        for f in ["{n}", "Page {n}", "Page {n} of {N}", "{n} / {N}", "- {n} -"] {
                            ui.selectable_value(&mut x.format, f.to_string(), f);
                        }
                    });
                    ui.add(egui::TextEdit::singleline(&mut x.format).hint_text("{n} {N} {date} {file}").desired_width(220.0));
                    ui.label(t("position"));
                    egui::Grid::new("stamp_pos").show(ui, |ui| {
                        for v in [VAlign::Top, VAlign::Bottom] {
                            for h in [HAlign::Left, HAlign::Center, HAlign::Right] {
                                if ui.radio(x.pos == (h, v), "").clicked() {
                                    x.pos = (h, v);
                                }
                            }
                            ui.end_row();
                        }
                    });
                }
                StampMode::HeaderFooter => {
                    ui.small(t("tokens_hint"));
                    egui::Grid::new("hf").num_columns(4).show(ui, |ui| {
                        ui.label("");
                        for l in ["align_left", "align_center", "align_right"] {
                            ui.label(t(l));
                        }
                        ui.end_row();
                        ui.label(t("header"));
                        for s in x.header.iter_mut() {
                            ui.add(egui::TextEdit::singleline(s).desired_width(110.0));
                        }
                        ui.end_row();
                        ui.label(t("footer"));
                        for s in x.footer.iter_mut() {
                            ui.add(egui::TextEdit::singleline(s).desired_width(110.0));
                        }
                        ui.end_row();
                    });
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("stamp_font").selected_text(["Helvetica", "Times", "Courier"][x.family]).show_ui(ui, |ui| {
                    for (i, nm) in ["Helvetica", "Times", "Courier"].iter().enumerate() {
                        ui.selectable_value(&mut x.family, i, *nm);
                    }
                });
                ui.add(egui::DragValue::new(&mut x.size).range(4.0..=72.0).suffix(" pt"));
                ui.toggle_value(&mut x.bold, "B");
                ui.toggle_value(&mut x.italic, "I");
                let mut c = Color32::from_rgb(x.color[0], x.color[1], x.color[2]);
                if ui.color_edit_button_srgba(&mut c).changed() {
                    x.color = [c.r(), c.g(), c.b()];
                }
            });
            ui.horizontal(|ui| {
                ui.label(t("margin"));
                ui.add(egui::DragValue::new(&mut x.margin).range(0.0..=200.0).suffix(" pt"));
                ui.label(t("start_number"));
                ui.add(egui::DragValue::new(&mut x.start));
            });
            ui.checkbox(&mut x.skip_first, t("skip_first_page"));
            let (ok, cancel) = Self::ok_cancel(ui, t("apply"), pages.is_some());
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if apply {
            if let Ok(pages) = parse_ranges(&x.pages, n) {
                let fam = [Builtin::Helvetica, Builtin::Times, Builtin::Courier][x.family];
                let style = TextStyle { family: FontFamily::Builtin(fam), bold: x.bold, italic: x.italic, size: x.size, color: x.color, ..TextStyle::default() };
                let items: Vec<StampItem> = match x.mode {
                    StampMode::Numbers => vec![StampItem { template: x.format.clone(), h: x.pos.0, v: x.pos.1 }],
                    StampMode::HeaderFooter => {
                        let mut v = vec![];
                        for (i, h) in [HAlign::Left, HAlign::Center, HAlign::Right].iter().enumerate() {
                            v.push(StampItem { template: x.header[i].clone(), h: *h, v: VAlign::Top });
                            v.push(StampItem { template: x.footer[i].clone(), h: *h, v: VAlign::Bottom });
                        }
                        v
                    }
                };
                let spec = StampSpec { items, style, margin: x.margin, start: x.start, skip_first: x.skip_first };
                let mem = self.mem();
                let r = self.tab().map(|tb| tb.stamp_pages(&pages, &spec, mem).map(|_| ()));
                if let Some(r) = r {
                    self.report(r, t("stamps_added"));
                }
            }
        }
        keep
    }

    // ------------------------------------------------------------------------- watermark

    fn ui_watermark(&mut self, ctx: &egui::Context, x: &mut WmD) -> bool {
        let mut keep = true;
        let n = self.n_pages();
        let cur = self.cur_page();
        let mut apply = false;
        modal(t("cmd_watermark")).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut x.text_mode, true, t("wm_text"));
                ui.selectable_value(&mut x.text_mode, false, t("wm_image"));
            });
            if x.text_mode {
                ui.add(egui::TextEdit::singleline(&mut x.text).desired_width(260.0));
                ui.horizontal(|ui| {
                    ui.label(t("font_size"));
                    ui.add(egui::DragValue::new(&mut x.size).range(8.0..=400.0).suffix(" pt"));
                    let mut c = Color32::from_rgb(x.color[0], x.color[1], x.color[2]);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        x.color = [c.r(), c.g(), c.b()];
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    if ui.button(t("choose_image")).clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"]).pick_file() {
                            if let Ok(img) = image::open(&f) {
                                x.image = Some((f, img));
                            }
                        }
                    }
                    if let Some((p, _)) = &x.image {
                        ui.label(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(t("width"));
                    ui.add(egui::DragValue::new(&mut x.size).range(20.0..=1000.0).suffix(" pt"));
                });
            }
            ui.add(egui::Slider::new(&mut x.opacity, 0.05..=1.0).text(t("opacity")));
            ui.add(egui::Slider::new(&mut x.rotation, -90.0..=90.0).text(t("rotation")).suffix("°"));
            ui.checkbox(&mut x.behind, t("behind_content"));
            let pages = pages_field(ui, &mut x.pages, n, cur);
            let ready = pages.is_some() && (x.text_mode && !x.text.trim().is_empty() || !x.text_mode && x.image.is_some());
            let (ok, cancel) = Self::ok_cancel(ui, t("apply"), ready);
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if apply {
            if let Ok(pages) = parse_ranges(&x.pages, n) {
                let kind = if x.text_mode { WatermarkKind::Text(x.text.clone()) } else { WatermarkKind::Image(x.image.as_ref().map(|i| i.1.clone()).unwrap_or_else(|| image::DynamicImage::new_rgba8(1, 1))) };
                let spec = WatermarkSpec { kind, pages, opacity: x.opacity, rotation_deg: x.rotation, size: x.size, color: x.color, behind: x.behind };
                let mem = self.mem();
                let r = self.tab().map(|tb| tb.add_watermark(&spec, mem));
                if let Some(r) = r {
                    self.report(r, t("watermark_added"));
                }
            }
        }
        keep
    }

    // --------------------------------------------------------------- rotate / delete

    fn ui_pages_action(&mut self, ctx: &egui::Context, x: &mut PagesAction) -> bool {
        let mut keep = true;
        let n = self.n_pages();
        let cur = self.cur_page();
        let mut apply = false;
        modal(if x.delete { t("cmd_delete_pages") } else { t("cmd_rotate_pages") }).show(ctx, |ui| {
            let pages = pages_field(ui, &mut x.pages, n, cur);
            if x.delete {
                if let Some(p) = &pages {
                    ui.label(format!("{} {}", p.len(), t("pages_will_be_deleted")));
                }
            } else {
                egui::ComboBox::from_label(t("angle")).selected_text(match x.angle {
                    90 => t("rot_cw"),
                    270 => t("rot_ccw"),
                    _ => t("rot_180"),
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut x.angle, 90, t("rot_cw"));
                    ui.selectable_value(&mut x.angle, 270, t("rot_ccw"));
                    ui.selectable_value(&mut x.angle, 180, t("rot_180"));
                });
            }
            let (ok, cancel) = Self::ok_cancel(ui, if x.delete { t("delete") } else { t("apply") }, pages.is_some());
            apply = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if apply {
            if let Ok(pages) = parse_ranges(&x.pages, n) {
                let mem = self.mem();
                let (del, ang) = (x.delete, x.angle);
                let r = self.tab().map(|tb| if del { tb.delete_page_list(&pages, mem) } else { tb.rotate_page_list(&pages, ang, mem) });
                if let Some(r) = r {
                    self.report(r, if del { t("pages_deleted") } else { t("pages_rotated") });
                }
            }
        }
        keep
    }

    #[allow(dead_code)]
    fn _unused(_: PathBuf) {}
}
