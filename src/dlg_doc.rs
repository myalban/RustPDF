//! Dialogs: document (properties, security, cleaning, signatures, fields, links, export…).

use crate::app::App;
use crate::dlg_common::*;
use crate::i18n::t;
use crate::pageops::parse_ranges;
use crate::pdfedit_doc::LinkDest;
use crate::pdfedit_forms::NewField;
use crate::model::FieldKind;
use crate::signature_pad::{self, SigStrokes};
use crate::tools::Tool;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

fn ok_cancel(ui: &mut egui::Ui, ok_label: &str, enabled: bool) -> (bool, bool) {
    let (mut ok, mut cancel) = (false, false);
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

fn pw(ui: &mut egui::Ui, label: &str, v: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(v).password(true).desired_width(180.0));
    });
}

impl App {
    /// Document-related dialogs; returns `true` while the box must stay open.
    pub fn show_dlg_doc(&mut self, ctx: &egui::Context, d: &mut Dlg2) -> bool {
        match d {
            Dlg2::Info(x) => self.ui_info(ctx, x),
            Dlg2::Security(x) => self.ui_security(ctx, x),
            Dlg2::Clean(x) => self.ui_clean(ctx, x),
            Dlg2::Sign(x) => self.ui_sign(ctx, x),
            Dlg2::Verify(x) => self.ui_verify(ctx, x),
            Dlg2::NewField(x) => self.ui_new_field(ctx, x),
            Dlg2::Link(x) => self.ui_link(ctx, x),
            Dlg2::Signature(x) => self.ui_signature(ctx, x),
            Dlg2::Export(x) => self.ui_export(ctx, x),
            Dlg2::Choice(x) => self.ui_choice(ctx, x),
            Dlg2::Updates(msg) => {
                let mut keep = true;
                let mut open_url = None;
                modal(t("cmd_check_updates")).show(ctx, |ui| {
                    ui.label(msg.as_str());
                    if let Some(u) = msg.lines().find(|l| l.starts_with("http")) {
                        if ui.link(u).clicked() {
                            open_url = Some(u.to_string());
                        }
                    }
                    if ui.button(t("close")).clicked() {
                        keep = false;
                    }
                });
                if let Some(u) = open_url {
                    crate::sys::open_url(&u);
                }
                keep
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------------- properties

    fn ui_info(&mut self, ctx: &egui::Context, x: &mut InfoD) -> bool {
        let mut keep = true;
        let mut apply = false;
        modal(t("cmd_properties")).show(ctx, |ui| {
            egui::Grid::new("info").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                for (l, v) in [("prop_title", &mut x.info.title), ("prop_author", &mut x.info.author), ("prop_subject", &mut x.info.subject), ("prop_keywords", &mut x.info.keywords), ("prop_creator", &mut x.info.creator), ("prop_producer", &mut x.info.producer)] {
                    ui.label(t(l));
                    ui.add(egui::TextEdit::singleline(v).desired_width(280.0));
                    ui.end_row();
                }
                ui.label(t("prop_created"));
                ui.label(&x.info.created);
                ui.end_row();
                ui.label(t("prop_modified"));
                ui.label(&x.info.modified);
                ui.end_row();
            });
            let (ok, cancel) = ok_cancel(ui, t("apply"), true);
            apply = ok;
            keep = !(ok || cancel);
        });
        if apply {
            let info = x.info.clone();
            self.edit(false, |tb, mem| tb.set_doc_info(&info, mem));
        }
        keep
    }

    // ------------------------------------------------------------------- security

    fn ui_security(&mut self, ctx: &egui::Context, x: &mut SecD) -> bool {
        let mut keep = true;
        let mut apply = false;
        let mut remove = false;
        modal(t("cmd_security")).show(ctx, |ui| {
            if let Some(e) = &x.existing {
                ui.colored_label(Color32::from_rgb(200, 140, 30), format!("{} (R{}{})", t("sec_already"), e.revision, if e.aes { ", AES" } else { "" }));
            }
            pw(ui, t("sec_open_password"), &mut x.sec.user_password);
            pw(ui, t("sec_owner_password"), &mut x.sec.owner_password);
            pw(ui, t("sec_confirm"), &mut x.confirm);
            ui.separator();
            ui.label(RichText::new(t("sec_permissions")).strong());
            ui.checkbox(&mut x.sec.allow_print, t("perm_print"));
            ui.checkbox(&mut x.sec.allow_print_hq, t("perm_print_hq"));
            ui.checkbox(&mut x.sec.allow_copy, t("perm_copy"));
            ui.checkbox(&mut x.sec.allow_modify, t("perm_modify"));
            ui.checkbox(&mut x.sec.allow_annotate, t("perm_annotate"));
            ui.checkbox(&mut x.sec.allow_fill, t("perm_fill"));
            ui.checkbox(&mut x.sec.allow_assemble, t("perm_assemble"));
            let restricted = !(x.sec.allow_print && x.sec.allow_print_hq && x.sec.allow_copy && x.sec.allow_modify && x.sec.allow_annotate && x.sec.allow_fill && x.sec.allow_assemble);
            let mismatch = x.sec.user_password != x.confirm;
            let need_owner = restricted && x.sec.owner_password.is_empty();
            if mismatch {
                ui.colored_label(Color32::from_rgb(220, 70, 70), t("sec_mismatch"));
            }
            if need_owner {
                ui.colored_label(Color32::from_rgb(220, 70, 70), t("sec_need_owner"));
            }
            ui.small(t("sec_applies_on_save"));
            let (ok, cancel) = ok_cancel(ui, t("apply"), !mismatch && !need_owner);
            apply = ok;
            if ui.button(t("sec_remove")).clicked() {
                remove = true;
            }
            keep = !(ok || cancel || remove);
        });
        if apply || remove {
            let s = if remove || !x.sec.is_active() { None } else { Some(x.sec.clone()) };
            if let Some(tb) = self.tab() {
                tb.set_security(s.clone());
                tb.dirty = true;
            }
            self.say(if s.is_some() { t("sec_set") } else { t("sec_cleared") });
        }
        keep
    }

    // ------------------------------------------------------------------- cleaning

    fn ui_clean(&mut self, ctx: &egui::Context, x: &mut CleanD) -> bool {
        let mut keep = true;
        let mut run = false;
        modal(t("cmd_clean_metadata")).show(ctx, |ui| {
            if let Some(r) = &x.report {
                ui.label(RichText::new(t("clean_done")).strong());
                if r.is_empty() {
                    ui.label(t("clean_nothing"));
                }
                for l in r {
                    ui.label(format!("• {l}"));
                }
                if ui.button(t("close")).clicked() {
                    keep = false;
                }
            } else {
                let o = &mut x.opts;
                ui.checkbox(&mut o.info, t("clean_info"));
                ui.checkbox(&mut o.xmp, t("clean_xmp"));
                ui.checkbox(&mut o.comments, t("clean_comments"));
                ui.checkbox(&mut o.attachments, t("clean_attachments"));
                ui.checkbox(&mut o.bookmarks, t("clean_bookmarks"));
                ui.checkbox(&mut o.javascript, t("clean_js"));
                ui.checkbox(&mut o.thumbnails, t("clean_thumbs"));
                let (ok, cancel) = ok_cancel(ui, t("clean"), true);
                run = ok;
                if cancel {
                    keep = false;
                }
            }
        });
        if run {
            let o = x.opts.clone();
            let mem = self.mem();
            let r = self.tab().map(|tb| tb.clean_metadata(&o, mem));
            match r {
                Some(Ok(rep)) => x.report = Some(rep),
                Some(Err(e)) => {
                    self.err(e);
                    keep = false;
                }
                None => keep = false,
            }
        }
        keep
    }

    // ------------------------------------------------------------------ digital signature

    pub fn dlg_sign(&mut self) {
        let n = self.tab_ref().map(|t| t.page_count()).unwrap_or(1);
        let _ = n;
        let page = self.tab_ref().map(|t| t.view.page).unwrap_or(0);
        self.open_dlg2(Dlg2::Sign(Box::new(SignD {
            identity: None,
            p12_path: None,
            password: String::new(),
            error: String::new(),
            opts: crate::sign::SignOptions::default(),
            visible: false,
            page,
            corner: 1,
            creating: None,
            generating: None,
        })));
    }

    fn ui_sign(&mut self, ctx: &egui::Context, x: &mut Box<SignD>) -> bool {
        let mut keep = true;
        let mut do_sign = false;
        // result of the background generation
        if let Some(rx) = &x.generating {
            match rx.try_recv() {
                Ok(Ok(id)) => {
                    x.identity = Some(id);
                    x.generating = None;
                    x.creating = None;
                }
                Ok(Err(e)) => {
                    x.error = e;
                    x.generating = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(_) => x.generating = None,
            }
        }
        let author = self.prefs.author.clone();
        modal(t("cmd_sign")).show(ctx, |ui| {
            ui.set_min_width(380.0);
            if x.generating.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t("sign_generating"));
                });
                return;
            }
            if let Some(id) = &x.identity {
                ui.label(RichText::new(id.common_name()).strong());
                ui.small(format!("{} · {} {}", id.issuer(), t("sign_expires"), id.not_after()));
                if id.is_self_signed() {
                    ui.small(t("sign_self_signed_note"));
                }
                if ui.small_button(t("sign_change_cert")).clicked() {
                    x.identity = None;
                }
            } else if let Some(c) = &mut x.creating {
                ui.label(RichText::new(t("sign_create")).strong());
                egui::Grid::new("cid").num_columns(2).show(ui, |ui| {
                    ui.label(t("sign_name"));
                    ui.text_edit_singleline(&mut c.name);
                    ui.end_row();
                    ui.label(t("sign_org"));
                    ui.text_edit_singleline(&mut c.org);
                    ui.end_row();
                    ui.label(t("sign_email"));
                    ui.text_edit_singleline(&mut c.email);
                    ui.end_row();
                    ui.label(t("sign_years"));
                    ui.add(egui::DragValue::new(&mut c.years).range(1..=30));
                    ui.end_row();
                    ui.label(t("sign_p12_password"));
                    ui.add(egui::TextEdit::singleline(&mut c.password).password(true));
                    ui.end_row();
                });
                if c.password.is_empty() {
                    ui.small(t("sign_password_needed"));
                }
                let mut start = None;
                let mut back = false;
                ui.horizontal(|ui| {
                    if ui.add_enabled(!c.name.trim().is_empty() && !c.password.is_empty(), egui::Button::new(t("sign_generate"))).clicked() {
                        start = Some((c.name.clone(), c.org.clone(), c.email.clone(), c.years, c.password.clone()));
                    }
                    if ui.button(t("back")).clicked() {
                        back = true;
                    }
                });
                if let Some((name, org, email, years, pass)) = start {
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let r = crate::sign::Identity::create_self_signed(&name, &org, &email, years, 2048);
                        if let Ok(id) = &r {
                            // password-protected backup copy in the application folder (the name is reduced to safe characters)
                            if !pass.is_empty() {
                                if let Ok(p12) = id.to_pkcs12(&pass) {
                                    let safe: String = name.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect();
                                    let file = crate::logx::data_dir().join(format!("certificate_{safe}.p12"));
                                    if std::fs::write(&file, p12).is_ok() {
                                        crate::sys::restrict_to_owner(&file);
                                    }
                                }
                            }
                        }
                        let _ = tx.send(r);
                    });
                    x.generating = Some(rx);
                }
                if back {
                    x.creating = None;
                }
            } else {
                ui.label(t("sign_choose_cert"));
                ui.horizontal(|ui| {
                    if ui.button(t("sign_open_p12")).clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("PKCS#12", &["pfx", "p12"]).pick_file() {
                            x.p12_path = Some(f);
                        }
                    }
                    if let Some(p) = &x.p12_path {
                        ui.label(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    }
                });
                if x.p12_path.is_some() {
                    pw(ui, t("sign_p12_password"), &mut x.password);
                    if ui.button(t("sign_load")).clicked() {
                        let r = std::fs::read(x.p12_path.as_ref().unwrap()).map_err(|e| e.to_string()).and_then(|b| crate::sign::Identity::from_pkcs12(&b, &x.password));
                        match r {
                            Ok(id) => {
                                x.identity = Some(id);
                                x.error.clear();
                            }
                            Err(e) => x.error = e,
                        }
                    }
                }
                ui.separator();
                if ui.button(t("sign_create_self")).clicked() {
                    x.creating = Some(CreateId { name: author.clone(), org: String::new(), email: String::new(), years: 5, password: String::new() });
                }
            }
            if !x.error.is_empty() {
                ui.colored_label(Color32::from_rgb(220, 70, 70), &x.error);
            }
            if x.identity.is_some() {
                ui.separator();
                egui::Grid::new("sopt").num_columns(2).show(ui, |ui| {
                    ui.label(t("sign_reason"));
                    ui.text_edit_singleline(&mut x.opts.reason);
                    ui.end_row();
                    ui.label(t("sign_location"));
                    ui.text_edit_singleline(&mut x.opts.location);
                    ui.end_row();
                    ui.label(t("sign_contact"));
                    ui.text_edit_singleline(&mut x.opts.contact);
                    ui.end_row();
                });
                ui.checkbox(&mut x.visible, t("sign_visible"));
                if x.visible {
                    egui::ComboBox::from_label(t("sign_corner")).selected_text(t(["corner_bl", "corner_br", "corner_tl", "corner_tr"][x.corner])).show_ui(ui, |ui| {
                        for (i, k) in ["corner_bl", "corner_br", "corner_tl", "corner_tr"].iter().enumerate() {
                            ui.selectable_value(&mut x.corner, i, t(k));
                        }
                    });
                }
                ui.small(t("sign_saves_copy"));
            }
            let (ok, cancel) = ok_cancel(ui, t("sign_do"), x.identity.is_some());
            do_sign = ok;
            if ok || cancel {
                keep = false;
            }
        });
        if do_sign {
            if let Some(id) = x.identity.take() {
                let mut opts = x.opts.clone();
                if x.visible {
                    let (pw_, ph_) = self.tab_ref().and_then(|tb| tb.sizes.get(x.page).copied()).unwrap_or((595.0, 842.0));
                    let (w, h, m) = (190.0, 55.0, 30.0);
                    let (x0, y0) = match x.corner {
                        0 => (m, m),
                        1 => (pw_ - w - m, m),
                        2 => (m, ph_ - h - m),
                        _ => (pw_ - w - m, ph_ - h - m),
                    };
                    opts.visible = Some((x.page, [x0, y0, x0 + w, y0 + h]));
                }
                let name = self.tab_ref().map(|t| t.title.trim_end_matches(".pdf").to_string()).unwrap_or_default();
                if let Some(mut path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name(format!("{name}_signed.pdf")).save_file() {
                    if path.extension().is_none() {
                        path.set_extension("pdf");
                    }
                    let bytes = self.tab_ref().and_then(|tb| tb.doc.save_to_bytes().ok());
                    match bytes.ok_or_else(|| "unreadable document".to_string()).and_then(|b| crate::sign::sign_pdf(&b, &id, &opts)) {
                        Ok(out) => match std::fs::write(&path, out) {
                            Ok(_) => {
                                self.say(format!("{}: {}", t("sign_done"), path.display()));
                                self.open_path(&path, None);
                            }
                            Err(e) => self.err(e.to_string()),
                        },
                        Err(e) => self.err(e),
                    }
                }
            }
        }
        keep
    }

    fn ui_verify(&mut self, ctx: &egui::Context, reports: &mut Vec<crate::sign::SigReport>) -> bool {
        let mut keep = true;
        modal(t("cmd_verify")).show(ctx, |ui| {
            ui.set_min_width(420.0);
            if reports.is_empty() {
                ui.label(t("verify_none"));
            }
            for r in reports.iter() {
                let (icon, col) = if r.is_valid() { ("✔", Color32::from_rgb(40, 160, 70)) } else { ("✖", Color32::from_rgb(220, 70, 70)) };
                ui.horizontal(|ui| {
                    ui.colored_label(col, icon);
                    ui.label(RichText::new(if r.signer.is_empty() { &r.field } else { &r.signer }).strong());
                });
                ui.small(format!("{} · {}", r.subject, r.signing_time));
                if !r.reason.is_empty() {
                    ui.small(format!("{}: {}", t("sign_reason"), r.reason));
                }
                ui.small(format!(
                    "{} {} · {} {} · {} {}",
                    t("verify_digest"),
                    if r.digest_ok { "✔" } else { "✖" },
                    t("verify_signature"),
                    if r.signature_ok { "✔" } else { "✖" },
                    t("verify_whole"),
                    if r.covers_whole_file { "✔" } else { "✖" }
                ));
                if r.self_signed {
                    ui.small(t("verify_self_signed"));
                }
                if !r.cert_valid_now {
                    ui.small(t("verify_cert_expired"));
                }
                if let Some(p) = &r.problem {
                    ui.colored_label(Color32::from_rgb(220, 70, 70), p);
                }
                ui.separator();
            }
            if ui.button(t("close")).clicked() {
                keep = false;
            }
        });
        keep
    }

    // ------------------------------------------------------------------- new field

    fn ui_new_field(&mut self, ctx: &egui::Context, x: &mut NewFieldD) -> bool {
        let mut keep = true;
        let mut apply = false;
        modal(t("new_field")).show(ctx, |ui| {
            egui::Grid::new("nf").num_columns(2).show(ui, |ui| {
                ui.label(t("field_name"));
                ui.text_edit_singleline(&mut x.name);
                ui.end_row();
                if matches!(x.tool, Tool::FormCombo) {
                    ui.label(t("field_options"));
                    ui.add(egui::TextEdit::multiline(&mut x.options).desired_rows(4));
                    ui.end_row();
                }
                if matches!(x.tool, Tool::FormRadio) {
                    ui.label(t("field_export_value"));
                    ui.text_edit_singleline(&mut x.value);
                    ui.end_row();
                }
                if matches!(x.tool, Tool::FormText) {
                    ui.label(t("field_default"));
                    ui.text_edit_singleline(&mut x.value);
                    ui.end_row();
                }
            });
            if matches!(x.tool, Tool::FormText) {
                ui.checkbox(&mut x.multiline, t("field_multiline"));
            }
            ui.checkbox(&mut x.read_only, t("field_readonly"));
            ui.checkbox(&mut x.required, t("field_required"));
            let (ok, cancel) = ok_cancel(ui, t("create"), !x.name.trim().is_empty());
            apply = ok;
            keep = !(ok || cancel);
        });
        if apply {
            let kind = match x.tool {
                Tool::FormCheck => FieldKind::Check,
                Tool::FormRadio => FieldKind::Radio,
                Tool::FormCombo => FieldKind::Combo,
                _ => FieldKind::Text,
            };
            let options: Vec<String> = if kind == FieldKind::Combo { x.options.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect() } else if kind == FieldKind::Radio { vec![x.value.clone()] } else { vec![] };
            let nf = NewField { page: x.page, kind, rect: x.rect, name: x.name.trim().to_string(), options, value: if kind == FieldKind::Text { x.value.clone() } else { String::new() }, read_only: x.read_only, required: x.required, multiline: x.multiline };
            self.edit(false, |tb, mem| tb.create_field(&nf, mem));
            self.set_tool(Tool::Select);
        }
        keep
    }

    // ------------------------------------------------------------------------- link

    fn ui_link(&mut self, ctx: &egui::Context, x: &mut LinkD) -> bool {
        let mut keep = true;
        let mut apply = false;
        let n = self.tab_ref().map(|t| t.page_count()).unwrap_or(1);
        modal(t("cmd_link")).show(ctx, |ui| {
            ui.radio_value(&mut x.use_url, false, t("link_to_page"));
            ui.radio_value(&mut x.use_url, true, t("link_to_url"));
            if x.use_url {
                ui.add(egui::TextEdit::singleline(&mut x.url).desired_width(300.0));
            } else {
                ui.horizontal(|ui| {
                    ui.label(t("page"));
                    ui.add(egui::DragValue::new(&mut x.target_page).range(1..=n));
                });
            }
            let (ok, cancel) = ok_cancel(ui, t("create"), !x.use_url || x.url.trim().len() > 8);
            apply = ok;
            keep = !(ok || cancel);
        });
        if apply {
            let dest = if x.use_url { LinkDest::Uri(x.url.trim().to_string()) } else { LinkDest::Page(x.target_page.saturating_sub(1)) };
            let (page, rect) = (x.page, x.rect);
            self.edit(false, |tb, mem| tb.add_link(page, rect, &dest, mem));
            self.set_tool(Tool::Select);
        }
        keep
    }

    // -------------------------------------------------------------- list box

    fn ui_choice(&mut self, ctx: &egui::Context, x: &mut ChoiceD) -> bool {
        let mut keep = true;
        let mut chosen = None;
        egui::Window::new(t("choose_option")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for (i, (label, sel)) in x.options.iter().enumerate() {
                    if ui.selectable_label(*sel, label).clicked() {
                        chosen = Some(i);
                    }
                }
            });
            if ui.button(t("cancel")).clicked() {
                keep = false;
            }
        });
        if let Some(i) = chosen {
            let (p, f) = (x.page, x.field);
            self.edit(false, |tb, mem| tb.choose_option(p, f, i, mem));
            keep = false;
        }
        keep
    }

    // -------------------------------------------------------------------------- export

    fn ui_export(&mut self, ctx: &egui::Context, x: &mut ExportD) -> bool {
        let mut keep = true;
        let mut apply = false;
        let n = self.tab_ref().map(|t| t.page_count()).unwrap_or(0);
        modal(t("cmd_export")).show(ctx, |ui| {
            for (i, k) in ["export_pages_images", "export_all_images", "export_text"].iter().enumerate() {
                ui.radio_value(&mut x.kind, i, t(k));
            }
            let mut ok_pages = true;
            if x.kind != 2 {
                ui.horizontal(|ui| {
                    ui.label(t("pages_to_process"));
                    ui.add(egui::TextEdit::singleline(&mut x.pages).desired_width(140.0));
                });
                ok_pages = parse_ranges(&x.pages, n).is_ok();
            }
            if x.kind == 0 {
                ui.add(egui::Slider::new(&mut x.dpi, 36.0..=600.0).text("dpi"));
            }
            if x.kind <= 1 {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut x.png, true, "PNG");
                    ui.radio_value(&mut x.png, false, "JPEG");
                });
            }
            let (ok, cancel) = ok_cancel(ui, t("export"), ok_pages);
            apply = ok;
            keep = !(ok || cancel);
        });
        if apply {
            self.run_export(x);
        }
        keep
    }

    fn run_export(&mut self, x: &ExportD) {
        let n = self.tab_ref().map(|t| t.page_count()).unwrap_or(0);
        let base = self.tab_ref().map(|t| t.title.trim_end_matches(".pdf").to_string()).unwrap_or_default();
        if x.kind == 2 {
            let Some(mut p) = rfd::FileDialog::new().add_filter("Text", &["txt"]).set_file_name(format!("{base}.txt")).save_file() else { return };
            if p.extension().is_none() {
                p.set_extension("txt");
            }
            let text: String = self.tab_ref().map(|tb| (0..n).map(|i| format!("--- {} {} ---\r\n{}\r\n", t("page"), i + 1, tb.page_text(i))).collect()).unwrap_or_default();
            match std::fs::write(&p, text) {
                Ok(_) => self.say(format!("{}: {}", t("exported"), p.display())),
                Err(e) => self.err(e.to_string()),
            }
            return;
        }
        let Ok(pages) = parse_ranges(&x.pages, n) else { return };
        let Some(dir) = rfd::FileDialog::new().pick_folder() else { return };
        let ext = if x.png { "png" } else { "jpg" };
        if x.kind == 1 {
            let r = self.tab().map(|tb| tb.extract_images(&pages, &dir, x.png));
            match r {
                Some(Ok(k)) => self.say(format!("{k} {} → {}", t("images_extracted"), dir.display())),
                Some(Err(e)) => self.err(e),
                None => {}
            }
            return;
        }
        let mut done = 0;
        for &p in &pages {
            let r = self.tab_ref().and_then(|tb| tb.sizes.get(p).copied().map(|s| (tb, s))).map(|(tb, (w, h))| tb.render_isolated(p, None, [0.0, 0.0, w, h], false, x.dpi));
            if let Some(Ok(img)) = r {
                let f = dir.join(format!("{base}_p{:03}.{ext}", p + 1));
                if crate::export::save_image_file(&img, &f).is_ok() {
                    done += 1;
                }
            }
        }
        self.say(format!("{done} {} → {}", t("images_exported"), dir.display()));
    }

    // ------------------------------------------------------------- handwritten signature

    pub fn open_signature_dialog(&mut self) {
        let fonts = signature_pad::available_script_fonts();
        self.open_dlg2(Dlg2::Signature(Box::new(SigD {
            tab: 0,
            strokes: vec![],
            name: self.prefs.author.clone(),
            save: true,
            saved: signature_pad::load_saved(),
            text: self.prefs.author.clone(),
            font: fonts.first().cloned().unwrap_or_default(),
            color: [10, 20, 120],
        })));
    }

    fn ui_signature(&mut self, ctx: &egui::Context, x: &mut Box<SigD>) -> bool {
        let mut keep = true;
        let mut payload: Option<SigPayload> = None;
        let fonts = signature_pad::available_script_fonts();
        modal(t("cmd_signature")).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (i, k) in ["sig_draw", "sig_saved", "sig_type", "sig_image"].iter().enumerate() {
                    ui.selectable_value(&mut x.tab, i, t(k));
                }
            });
            ui.separator();
            match x.tab {
                0 => {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(420.0, 150.0), Sense::click_and_drag());
                    ui.painter().rect_filled(rect, 4.0, Color32::WHITE);
                    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Inside);
                    let base = rect.max.y - 40.0;
                    ui.painter().line_segment([Pos2::new(rect.min.x + 12.0, base), Pos2::new(rect.max.x - 12.0, base)], Stroke::new(1.0, Color32::from_gray(200)));
                    if resp.drag_started() {
                        x.strokes.push(vec![]);
                    }
                    if resp.dragged() {
                        if let (Some(p), Some(s)) = (resp.interact_pointer_pos(), x.strokes.last_mut()) {
                            if s.last().map(|l| (l[0] - p.x).abs() + (l[1] - p.y).abs() > 1.5).unwrap_or(true) {
                                s.push([p.x, p.y]);
                            }
                        }
                    }
                    for s in &x.strokes {
                        let pts: Vec<Pos2> = s.iter().map(|p| Pos2::new(p[0], p[1])).collect();
                        if pts.len() >= 2 {
                            ui.painter().add(egui::Shape::line(pts, Stroke::new(2.2, Color32::from_rgb(10, 20, 120))));
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button(t("sig_clear")).clicked() {
                            x.strokes.clear();
                        }
                        ui.checkbox(&mut x.save, t("sig_save_for_later"));
                        if x.save {
                            ui.add(egui::TextEdit::singleline(&mut x.name).desired_width(110.0));
                        }
                    });
                    let ok = x.strokes.iter().map(|s| s.len()).sum::<usize>() >= 4;
                    if ui.add_enabled(ok, egui::Button::new(RichText::new(t("sig_use")).strong())).clicked() {
                        let simp: Vec<Vec<[f32; 2]>> = x.strokes.iter().map(|s| signature_pad::simplify_stroke(s, 0.8)).collect();
                        if let Some(s) = SigStrokes::from_screen(&x.name, &simp) {
                            if x.save {
                                x.saved.push(s.clone());
                                signature_pad::save_all(&x.saved);
                            }
                            payload = Some(SigPayload::Strokes(s));
                        }
                    }
                }
                1 => {
                    if x.saved.is_empty() {
                        ui.weak(t("sig_none_saved"));
                    }
                    let mut del = None;
                    for (i, s) in x.saved.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(Vec2::new(110.0, 40.0), Sense::hover());
                            ui.painter().rect_filled(rect, 3.0, Color32::WHITE);
                            for st in &s.strokes {
                                let (w, h) = (100.0f32.min(34.0 * s.aspect), 34.0f32.min(100.0 / s.aspect.max(0.1)));
                                let pts: Vec<Pos2> = st.iter().map(|p| Pos2::new(rect.min.x + 5.0 + p[0] * w, rect.min.y + 3.0 + p[1] * h)).collect();
                                ui.painter().add(egui::Shape::line(pts, Stroke::new(1.4, Color32::from_rgb(10, 20, 120))));
                            }
                            if ui.button(if s.name.is_empty() { t("sig_use") } else { &s.name }).clicked() {
                                payload = Some(SigPayload::Strokes(s.clone()));
                            }
                            if ui.small_button("🗑").clicked() {
                                del = Some(i);
                            }
                        });
                    }
                    if let Some(i) = del {
                        x.saved.remove(i);
                        signature_pad::save_all(&x.saved);
                    }
                }
                2 => {
                    ui.add(egui::TextEdit::singleline(&mut x.text).desired_width(300.0));
                    if fonts.is_empty() {
                        ui.small(t("sig_no_script_font"));
                    } else {
                        egui::ComboBox::from_label(t("font")).selected_text(x.font.clone()).show_ui(ui, |ui| {
                            for f in &fonts {
                                ui.selectable_value(&mut x.font, f.clone(), f);
                            }
                        });
                    }
                    if ui.add_enabled(!x.text.trim().is_empty(), egui::Button::new(RichText::new(t("sig_use")).strong())).clicked() {
                        let font = if x.font.is_empty() { "Arial".to_string() } else { x.font.clone() };
                        payload = Some(SigPayload::Typed { text: x.text.clone(), font });
                    }
                }
                _ => {
                    if ui.button(t("choose_image")).clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"]).pick_file() {
                            if let Ok(img) = crate::export::open_image_safe(&f) {
                                payload = Some(SigPayload::Image(img));
                            }
                        }
                    }
                }
            }
            if ui.button(t("cancel")).clicked() {
                keep = false;
            }
        });
        if let Some(p) = payload {
            self.sig_payload = Some(p);
            self.set_tool(Tool::Signature);
            self.say(t("sig_click_to_place"));
            keep = false;
        }
        let _ = Rect::NOTHING;
        keep
    }
}
