//! Properties panel (on the right): depends on the selection.

use crate::app::App;
use crate::edit_text::TextStyle;
use crate::fonts::{self, Builtin, FontFamily};
use crate::i18n::t;
use crate::icons;
use crate::model::*;
use crate::prefs::Unit;
use crate::textgroup::Align;
use crate::tools::{Cmd, ShapeKind, Tool};
use eframe::egui::{self, Color32, RichText};

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Icon button with a tooltip.
pub fn icon_button(ui: &mut egui::Ui, icon: &str, tip: &str, selected: bool) -> egui::Response {
    let b = egui::Button::new(RichText::new(icon).size(16.0)).selected(selected).min_size(egui::vec2(26.0, 24.0));
    ui.add(b).on_hover_text(tip)
}

fn commit(r: &egui::Response) -> bool {
    r.drag_stopped() || (r.lost_focus() && !r.ctx.input(|i| i.key_pressed(egui::Key::Escape)))
}

impl App {
    pub fn props_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.heading(t("properties"));
            ui.separator();
            let Some(tab) = self.tabs.get_mut(self.active) else {
                ui.weak(t("no_document"));
                return;
            };
            let (page, items, field, annot) = (tab.sel.page, tab.sel.items.clone(), tab.sel.field, tab.sel.annot);
            let tool = self.tool;
            // add / draw tools: tool settings
            if !matches!(tool, Tool::Select | Tool::Hand | Tool::TextSelect) {
                self.tool_options(ui, tool);
                if items.is_empty() && field.is_none() && annot.is_none() {
                    return;
                }
                ui.separator();
            }
            let Some(info) = self.tabs.get_mut(self.active).and_then(|tb| tb.info(page)) else { return };
            if let Some(fi) = field {
                self.field_props(ui, page, fi, &info);
                return;
            }
            if let Some(ai) = annot {
                self.annot_props(ui, page, ai, &info);
                return;
            }
            if items.is_empty() {
                self.page_props(ui);
                return;
            }
            let kinds: Vec<ObjKind> = items.iter().filter_map(|i| info.object(*i)).map(|o| o.kind).collect();
            let single = items.len() == 1;
            if single && kinds.first() == Some(&ObjKind::Text) {
                self.text_props(ui, page, items[0]);
            } else if single && kinds.first() == Some(&ObjKind::Image) {
                self.image_props(ui, page, items[0]);
            } else if single && kinds.first() == Some(&ObjKind::Path) {
                self.path_props(ui, page, items[0]);
            } else if single && kinds.first() == Some(&ObjKind::Form) {
                ui.label(t("group_selected"));
                if ui.button(format!("{} {}", icons::SELECTION_SLASH, t("cmd_ungroup"))).clicked() {
                    self.pending_cmds.push(Cmd::Ungroup);
                }
            } else {
                ui.label(format!("{} {}", items.len(), t("items_selected")));
                if kinds.iter().any(|k| *k == ObjKind::Path) {
                    self.multi_style(ui, page, &items);
                }
                if ui.button(format!("{} {}", icons::SELECTION_PLUS, t("cmd_group"))).clicked() {
                    self.pending_cmds.push(Cmd::Group);
                }
            }
            ui.separator();
            self.arrange_props(ui, page, &items, &info);
            ui.separator();
            self.geometry_props(ui, page, &items, &info);
        });
    }

    // ----------------------------------------------------------------- text

    fn load_text_ui(&mut self, page: usize, idx: usize) {
        let id = self.tabs[self.active].id;
        if self.text_ui.key == Some((id, page, idx)) {
            return;
        }
        let tab = &mut self.tabs[self.active];
        if let Some((text, style, orig_font)) = tab.read_text_style(page, idx) {
            self.text_ui.text = text;
            self.text_ui.style = Some(style);
            self.text_ui.orig_font = orig_font;
            self.text_ui.key = Some((id, page, idx));
        }
    }

    fn apply_text_ui(&mut self, page: usize, idx: usize) {
        let Some(st) = self.text_ui.style.clone() else { return };
        let text = self.text_ui.text.clone();
        let mem = self.mem();
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        match tab.apply_text(page, idx, &text, &st, mem) {
            Ok(new) => {
                if let Some(n) = new {
                    tab.sel.page = page;
                    tab.sel.items = vec![n];
                }
                self.text_ui.key = None;
            }
            Err(e) => self.err(e),
        }
    }

    fn text_props(&mut self, ui: &mut egui::Ui, page: usize, idx: usize) {
        self.load_text_ui(page, idx);
        let Some(mut st) = self.text_ui.style.clone() else { return };
        let mut apply = false;
        ui.label(RichText::new(t("text_selected")).strong());
        ui.small(format!("{} {}", t("font_in_doc"), self.text_ui.orig_font));

        // font
        let before = st.family.clone();
        font_combo(ui, &mut st.family, &mut self.font_filter);
        if st.family != before {
            apply = true;
        }
        ui.horizontal(|ui| {
            let r = ui.add(egui::DragValue::new(&mut st.size).range(2.0..=500.0).speed(0.2).suffix(" pt"));
            if commit(&r) && r.changed() || (r.lost_focus() && st.size != self.text_ui.style.as_ref().map(|s| s.size).unwrap_or(st.size)) {
                apply = true;
            }
            let mut color = rgb(st.color);
            if ui.color_edit_button_srgba(&mut color).changed() {
                st.color = [color.r(), color.g(), color.b()];
                apply = true;
            }
        });
        ui.horizontal(|ui| {
            for (icon, on, tip) in [(icons::TEXT_B, &mut st.bold, "bold"), (icons::TEXT_ITALIC, &mut st.italic, "italic"), (icons::TEXT_UNDERLINE, &mut st.underline, "underline"), (icons::TEXT_STRIKETHROUGH, &mut st.strike, "strike")] {
                if icon_button(ui, icon, t(tip), *on).clicked() {
                    *on = !*on;
                    apply = true;
                }
            }
            ui.separator();
            for (icon, a, tip) in [(icons::TEXT_ALIGN_LEFT, Align::Left, "align_left"), (icons::TEXT_ALIGN_CENTER, Align::Center, "align_center"), (icons::TEXT_ALIGN_RIGHT, Align::Right, "align_right"), (icons::TEXT_ALIGN_JUSTIFY, Align::Justify, "align_justify")] {
                if icon_button(ui, icon, t(tip), st.align == a).clicked() {
                    st.align = a;
                    apply = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(t("letter_spacing"));
            let r = ui.add(egui::DragValue::new(&mut st.letter_spacing).range(-5.0..=40.0).speed(0.05).suffix(" pt"));
            if commit(&r) && r.changed() {
                apply = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(t("line_spacing"));
            let r = ui.add(egui::DragValue::new(&mut st.line_spacing).range(0.0..=5.0).speed(0.02).custom_formatter(|v, _| if v < 0.01 { t("auto").to_string() } else { format!("×{v:.2}") }));
            if commit(&r) && r.changed() {
                apply = true;
            }
        });
        ui.add_space(4.0);
        ui.label(t("text_content"));
        let r = ui.add(egui::TextEdit::multiline(&mut self.text_ui.text).desired_rows(4).desired_width(f32::INFINITY));
        let ctrl_enter = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
        ui.horizontal(|ui| {
            if ui.button(format!("{} {}", icons::CHECK, t("apply"))).clicked() || ctrl_enter {
                apply = true;
            }
            if ui.button(format!("{} {}", icons::PENCIL_SIMPLE, t("edit_on_page"))).clicked() {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let text = self.text_ui.text.clone();
                    tab.inline = Some(crate::tab::Inline { page, kind: crate::tab::InlineKind::Object(idx), text, focus: true });
                }
            }
        });
        ui.small(t("tip_double_click_text"));
        self.text_ui.style = Some(st);
        if apply {
            self.apply_text_ui(page, idx);
        }
    }

    // ------------------------------------------------------------------ image

    fn image_props(&mut self, ui: &mut egui::Ui, page: usize, idx: usize) {
        ui.label(RichText::new(t("image_selected")).strong());
        let dpi = self.prefs.export_dpi;
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{} {}", icons::IMAGE, t("replace_image"))).clicked() {
                self.pending_msgs.push(crate::view::Msg::ReplaceImage { page, idx });
            }
            if ui.button(format!("{} {}", icons::EXPORT, t("export_original"))).clicked() {
                let r = self.tab().map(|tb| tb.export_original_image(page, idx));
                match r {
                    Some(Ok(img)) => self.save_image_dialog(img, "image.png"),
                    Some(Err(e)) => self.err(e),
                    None => {}
                }
            }
            if ui.button(format!("{} {}", icons::EXPORT, t("export_as_shown"))).clicked() {
                let r = self.tab().map(|tb| tb.export_items_image(page, &[idx], dpi));
                match r {
                    Some(Ok(img)) => self.save_image_dialog(img, "element.png"),
                    Some(Err(e)) => self.err(e),
                    None => {}
                }
            }
        });
        ui.separator();
        ui.collapsing(t("crop_image"), |ui| {
            let c = &mut self.img_crop;
            ui.add(egui::Slider::new(&mut c[0], 0.0..=0.95).text(t("crop_left")));
            ui.add(egui::Slider::new(&mut c[1], 0.0..=0.95).text(t("crop_top")));
            ui.add(egui::Slider::new(&mut c[2], 0.05..=1.0).text(t("crop_right")));
            ui.add(egui::Slider::new(&mut c[3], 0.05..=1.0).text(t("crop_bottom")));
            if ui.button(t("apply")).clicked() {
                let fr = self.img_crop;
                self.img_crop = [0.0, 0.0, 1.0, 1.0];
                self.edit_pub(true, move |tb, mem| tb.crop_image(page, idx, fr, mem));
            }
        });
        ui.collapsing(t("adjust_image"), |ui| {
            ui.add(egui::Slider::new(&mut self.img_adj.0, -100..=100).text(t("brightness")));
            ui.add(egui::Slider::new(&mut self.img_adj.1, -100.0..=100.0).text(t("contrast")));
            ui.checkbox(&mut self.img_adj.2, t("grayscale"));
            if ui.button(t("apply")).clicked() {
                let (b, c, g) = self.img_adj;
                self.img_adj = (0, 0.0, false);
                self.edit_pub(false, move |tb, mem| tb.adjust_image(page, idx, b, c, g, mem));
            }
        });
        ui.collapsing(t("reduce_resolution"), |ui| {
            ui.add(egui::DragValue::new(&mut self.img_max_px).range(64..=8000).suffix(" px"));
            if ui.button(t("apply")).clicked() {
                let m = self.img_max_px;
                self.edit_pub(false, move |tb, mem| tb.downscale_image(page, idx, m, mem));
            }
        });
    }

    // ------------------------------------------------------------------ path

    fn path_props(&mut self, ui: &mut egui::Ui, page: usize, idx: usize) {
        ui.label(RichText::new(t("shape_selected")).strong());
        let data = self.tabs.get_mut(self.active).and_then(|tb| tb.read_path_data(page, idx));
        let Some(d) = data else { return };
        let mut stroke = d.stroke.unwrap_or([0, 0, 0, 255]);
        let mut fill = d.fill.unwrap_or([255, 255, 255, 255]);
        let (mut has_stroke, mut has_fill) = (d.stroke.is_some(), d.fill.is_some());
        let mut width = d.width;
        let mut changed_stroke = false;
        let mut changed_fill = false;
        let mut changed_width = false;
        ui.horizontal(|ui| {
            if ui.checkbox(&mut has_stroke, t("stroke")).changed() {
                changed_stroke = true;
            }
            let mut c = Color32::from_rgb(stroke[0], stroke[1], stroke[2]);
            if ui.color_edit_button_srgba(&mut c).changed() {
                stroke = [c.r(), c.g(), c.b(), stroke[3]];
                has_stroke = true;
                changed_stroke = true;
            }
            let r = ui.add(egui::DragValue::new(&mut width).range(0.1..=100.0).speed(0.1).suffix(" pt"));
            if commit(&r) && r.changed() {
                changed_width = true;
            }
        });
        ui.horizontal(|ui| {
            if ui.checkbox(&mut has_fill, t("fill")).changed() {
                changed_fill = true;
            }
            let mut c = Color32::from_rgb(fill[0], fill[1], fill[2]);
            if ui.color_edit_button_srgba(&mut c).changed() {
                fill = [c.r(), c.g(), c.b(), fill[3]];
                has_fill = true;
                changed_fill = true;
            }
        });
        let mut op = (stroke[3].max(fill[3]) as f32 / 255.0 * 100.0).round();
        let r = ui.add(egui::Slider::new(&mut op, 5.0..=100.0).suffix(" %").text(t("opacity")));
        let changed_op = commit(&r) && r.changed();
        let mut dash_sel = if d.dash.is_empty() { 0 } else if d.dash.len() >= 2 && d.dash[0] > d.dash[1] * 1.5 { 1 } else { 2 };
        let before_dash = dash_sel;
        ui.horizontal(|ui| {
            ui.label(t("dash"));
            egui::ComboBox::from_id_salt("dash").selected_text(t(["dash_solid", "dash_dashed", "dash_dotted"][dash_sel])).show_ui(ui, |ui| {
                ui.selectable_value(&mut dash_sel, 0, t("dash_solid"));
                ui.selectable_value(&mut dash_sel, 1, t("dash_dashed"));
                ui.selectable_value(&mut dash_sel, 2, t("dash_dotted"));
            });
        });
        let items = vec![idx];
        if changed_stroke || changed_fill || changed_width || changed_op {
            let alpha = (op / 100.0 * 255.0).round() as u8;
            let fill_arg = if has_fill && (changed_fill || changed_op) { Some([fill[0], fill[1], fill[2], alpha]) } else { None };
            let stroke_arg = if has_stroke && (changed_stroke || changed_op) { Some([stroke[0], stroke[1], stroke[2], alpha]) } else { None };
            let w = if changed_width { Some(width) } else { None };
            // remove the outline / fill: recreate the path with the new configuration
            if (changed_stroke && !has_stroke && d.stroke.is_some()) || (changed_fill && !has_fill && d.fill.is_some()) {
                let mut nd = d.clone();
                if !has_stroke {
                    nd.stroke = None;
                }
                if !has_fill {
                    nd.fill = None;
                }
                self.edit_pub(true, move |tb, mem| {
                    tb.delete_items(page, &[idx], mem)?;
                    tb.add_path(page, &nd, "Style", mem).map(|_| ())
                });
            } else if (changed_stroke && has_stroke && d.stroke.is_none()) || (changed_fill && has_fill && d.fill.is_none()) {
                let mut nd = d.clone();
                if has_stroke {
                    nd.stroke = Some([stroke[0], stroke[1], stroke[2], alpha]);
                }
                if has_fill {
                    nd.fill = Some([fill[0], fill[1], fill[2], alpha]);
                }
                self.edit_pub(true, move |tb, mem| {
                    tb.delete_items(page, &[idx], mem)?;
                    tb.add_path(page, &nd, "Style", mem).map(|_| ())
                });
            } else {
                let it = items.clone();
                self.edit_pub(false, move |tb, mem| tb.set_style_items(page, &it, fill_arg, stroke_arg, w, mem));
            }
        }
        if dash_sel != before_dash {
            let dash: Vec<f32> = match dash_sel {
                1 => vec![width * 4.0, width * 2.0],
                2 => vec![width, width * 2.0],
                _ => vec![],
            };
            let it = items.clone();
            self.edit_pub(false, move |tb, mem| tb.set_dash_items(page, &it, &dash, mem));
        }
        ui.small(t("tip_path_points"));
        if ui.button(format!("{} SVG", icons::EXPORT)).clicked() {
            self.export_svg_dialog(page, &items);
        }
    }

    fn multi_style(&mut self, ui: &mut egui::Ui, page: usize, items: &[usize]) {
        ui.horizontal(|ui| {
            ui.label(t("stroke"));
            let mut c = rgb(self.style.shape_stroke);
            if ui.color_edit_button_srgba(&mut c).changed() {
                let col = [c.r(), c.g(), c.b(), 255];
                let it = items.to_vec();
                self.edit_pub(false, move |tb, mem| tb.set_style_items(page, &it, None, Some(col), None, mem));
            }
            ui.label(t("fill"));
            let mut f = rgb(self.style.shape_fill.unwrap_or([255, 255, 255]));
            if ui.color_edit_button_srgba(&mut f).changed() {
                let col = [f.r(), f.g(), f.b(), 255];
                let it = items.to_vec();
                self.edit_pub(false, move |tb, mem| tb.set_style_items(page, &it, Some(col), None, None, mem));
            }
        });
    }

    // ------------------------------------------------------------- organisation

    fn arrange_props(&mut self, ui: &mut egui::Ui, _page: usize, items: &[usize], _info: &PageInfo) {
        ui.label(RichText::new(t("arrange")).strong());
        ui.horizontal_wrapped(|ui| {
            for c in [Cmd::BringFront, Cmd::BringForward, Cmd::SendBackward, Cmd::SendBack] {
                if icon_button(ui, c.icon(), t(c.label_key()), false).clicked() {
                    self.pending_cmds.push(c);
                }
            }
            ui.separator();
            for c in [Cmd::Rotate90, Cmd::RotateMinus90, Cmd::FlipH, Cmd::FlipV] {
                if icon_button(ui, c.icon(), t(c.label_key()), false).clicked() {
                    self.pending_cmds.push(c);
                }
            }
        });
        if items.len() >= 2 {
            ui.horizontal_wrapped(|ui| {
                for c in [Cmd::AlignLeft, Cmd::AlignCenter, Cmd::AlignRight, Cmd::AlignTop, Cmd::AlignMiddle, Cmd::AlignBottom] {
                    if icon_button(ui, c.icon(), t(c.label_key()), false).clicked() {
                        self.pending_cmds.push(c);
                    }
                }
            });
            if items.len() >= 3 {
                ui.horizontal(|ui| {
                    if ui.button(t("cmd_distribute_h")).clicked() {
                        self.pending_cmds.push(Cmd::DistributeH);
                    }
                    if ui.button(t("cmd_distribute_v")).clicked() {
                        self.pending_cmds.push(Cmd::DistributeV);
                    }
                });
            }
        }
        ui.horizontal(|ui| {
            for c in [Cmd::Copy, Cmd::Duplicate, Cmd::Delete] {
                if icon_button(ui, c.icon(), t(c.label_key()), false).clicked() {
                    self.pending_cmds.push(c);
                }
            }
        });
    }

    /// Position and size in the chosen units (origin: top-left corner of the page).
    fn geometry_props(&mut self, ui: &mut egui::Ui, page: usize, items: &[usize], info: &PageInfo) {
        let Some(r) = crate::edit::union_rect(info, items) else { return };
        let unit = self.prefs.units;
        let v = info.geom.rect_to_view(&r);
        let cur = [unit.from_pt(v[0]), unit.from_pt(v[1]), unit.from_pt(v[2] - v[0]), unit.from_pt(v[3] - v[1])];
        let id = self.tabs[self.active].id;
        let key = (id, page, items.to_vec());
        if self.text_ui.geo_key.as_ref() != Some(&key) {
            self.text_ui.geo = cur;
            self.text_ui.geo_key = Some(key);
        }
        let mut g = self.text_ui.geo;
        let mut done = false;
        ui.label(RichText::new(t("position_size")).strong());
        egui::Grid::new("geo").num_columns(4).spacing([6.0, 4.0]).show(ui, |ui| {
            for (label, k) in [("X", 0usize), ("Y", 1)] {
                ui.label(label);
                let rr = ui.add(egui::DragValue::new(&mut g[k]).speed(0.1).suffix(format!(" {}", unit.label())));
                if commit(&rr) && rr.changed() {
                    done = true;
                }
            }
            ui.end_row();
            for (label, k) in [("L", 2usize), ("H", 3)] {
                ui.label(label);
                let rr = ui.add(egui::DragValue::new(&mut g[k]).speed(0.1).range(0.1..=10000.0).suffix(format!(" {}", unit.label())));
                if commit(&rr) && rr.changed() {
                    done = true;
                }
            }
            ui.end_row();
        });
        self.text_ui.geo = g;
        if done {
            let nv = [unit.to_pt(g[0]), unit.to_pt(g[1]), unit.to_pt(g[0] + g[2]), unit.to_pt(g[1] + g[3])];
            let new = info.geom.rect_from_view(&nv);
            let it = items.to_vec();
            let same_size = ((g[2] - cur[2]).abs() < 1e-3) && ((g[3] - cur[3]).abs() < 1e-3);
            if same_size {
                let (dx, dy) = (new[0] - r[0], new[1] - r[1]);
                self.edit_pub(false, move |tb, mem| tb.move_items(page, &it, dx, dy, mem));
            } else {
                self.edit_pub(false, move |tb, mem| tb.resize_items(page, &it, r, new, mem));
            }
        }
    }

    // ------------------------------------------------------------- fields / page

    fn field_props(&mut self, ui: &mut egui::Ui, page: usize, fi: usize, info: &PageInfo) {
        let Some(f) = info.fields.get(fi).cloned() else { return };
        ui.label(RichText::new(format!("{}: {}", t("form_field"), f.name)).strong());
        if f.read_only {
            ui.weak(t("read_only"));
        }
        match f.kind {
            FieldKind::Text => {
                if self.field_buf.0 != (page, fi) {
                    self.field_buf = ((page, fi), f.value.clone());
                }
                ui.add(egui::TextEdit::multiline(&mut self.field_buf.1).desired_width(f32::INFINITY).desired_rows(3));
                if ui.button(format!("{} {}", icons::CHECK, t("apply"))).clicked() {
                    let text = self.field_buf.1.clone();
                    self.pending_msgs.push(crate::view::Msg::CommitFieldText { page, field: fi, text });
                }
            }
            FieldKind::Check | FieldKind::Radio => {
                let mut c = f.checked;
                if ui.checkbox(&mut c, t("checked")).clicked() {
                    self.pending_msgs.push(crate::view::Msg::ToggleField { page, field: fi });
                }
            }
            FieldKind::Combo | FieldKind::List => {
                ui.label(format!("{}: {}", t("value"), f.value));
                for (i, (label, set)) in f.options.iter().enumerate() {
                    if ui.selectable_label(*set, label).clicked() {
                        self.pending_msgs.push(crate::view::Msg::ChooseOption { page, field: fi, option: i });
                    }
                }
            }
            _ => {
                ui.weak(t("field_not_editable"));
            }
        }
    }

    fn page_props(&mut self, ui: &mut egui::Ui) {
        let unit = self.prefs.units;
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let p = tab.view.page;
        ui.label(RichText::new(format!("{} {}/{}", t("page"), p + 1, tab.page_count())).strong());
        if let Some((w, h)) = tab.sizes.get(p).copied() {
            ui.label(format!("{} × {}", unit.format(w), unit.format(h)));
        }
        ui.separator();
        ui.label(RichText::new(t("document")).strong());
        if let Some(path) = &tab.path {
            ui.small(path.display().to_string());
        }
        ui.label(format!("{} {}", tab.page_count(), t("pages")));
        ui.label(format!("PDF {:?}", tab.doc.version()).replace("PdfDocumentVersion::", ""));
        ui.add_space(6.0);
        ui.weak(t("tip_select"));
        let _ = fonts::Builtin::Helvetica;
    }

    /// Settings of the drawing / annotation tools (color, width, etc.).
    fn tool_options(&mut self, ui: &mut egui::Ui, tool: Tool) {
        ui.label(RichText::new(t(tool.label_key())).strong());
        let st = &mut self.style;
        match tool {
            Tool::Shape(kind) => {
                ui.horizontal(|ui| {
                    ui.label(t("stroke"));
                    let mut c = rgb(st.shape_stroke);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        st.shape_stroke = [c.r(), c.g(), c.b()];
                    }
                    ui.add(egui::DragValue::new(&mut st.shape_width).range(0.2..=60.0).speed(0.1).suffix(" pt"));
                });
                if !matches!(kind, ShapeKind::Line | ShapeKind::Arrow | ShapeKind::Freehand) {
                    ui.horizontal(|ui| {
                        let mut has = st.shape_fill.is_some();
                        if ui.checkbox(&mut has, t("fill")).changed() {
                            st.shape_fill = if has { Some([255, 235, 150]) } else { None };
                        }
                        if let Some(f) = st.shape_fill.as_mut() {
                            let mut c = rgb(*f);
                            if ui.color_edit_button_srgba(&mut c).changed() {
                                *f = [c.r(), c.g(), c.b()];
                            }
                        }
                    });
                }
                let mut op = st.shape_opacity as f32 / 255.0 * 100.0;
                if ui.add(egui::Slider::new(&mut op, 5.0..=100.0).suffix(" %").text(t("opacity"))).changed() {
                    st.shape_opacity = (op / 100.0 * 255.0).round() as u8;
                }
                egui::ComboBox::from_id_salt("dash_tool").selected_text(t(["dash_solid", "dash_dashed", "dash_dotted"][st.shape_dash as usize % 3])).show_ui(ui, |ui| {
                    ui.selectable_value(&mut st.shape_dash, 0, t("dash_solid"));
                    ui.selectable_value(&mut st.shape_dash, 1, t("dash_dashed"));
                    ui.selectable_value(&mut st.shape_dash, 2, t("dash_dotted"));
                });
                if kind == ShapeKind::Polygon {
                    ui.small(t("tip_polygon"));
                }
            }
            Tool::AddText => {
                font_combo(ui, &mut st.text.family, &mut self.font_filter);
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut st.text.size).range(2.0..=300.0).suffix(" pt"));
                    let mut c = rgb(st.text.color);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        st.text.color = [c.r(), c.g(), c.b()];
                    }
                    for (icon, on) in [(icons::TEXT_B, &mut st.text.bold), (icons::TEXT_ITALIC, &mut st.text.italic)] {
                        if icon_button(ui, icon, "", *on).clicked() {
                            *on = !*on;
                        }
                    }
                });
                ui.small(t("tip_add_text"));
            }
            Tool::Highlight | Tool::Underline | Tool::StrikeOut | Tool::Squiggly => {
                ui.horizontal(|ui| {
                    let mut c = rgb(st.markup_color);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        st.markup_color = [c.r(), c.g(), c.b()];
                    }
                    for col in [[255, 235, 59], [120, 220, 90], [255, 150, 200], [120, 190, 255], [255, 140, 60]] {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                        ui.painter().rect_filled(r, 3.0, rgb(col));
                        if resp.clicked() {
                            st.markup_color = col;
                        }
                    }
                });
                ui.small(t("tip_markup"));
            }
            Tool::Stamp => {
                ui.text_edit_singleline(&mut st.stamp_text);
                let mut c = rgb(st.stamp_color);
                if ui.color_edit_button_srgba(&mut c).changed() {
                    st.stamp_color = [c.r(), c.g(), c.b()];
                }
                ui.horizontal_wrapped(|ui| {
                    for s in ["APPROVED", "REJECTED", "CONFIDENTIAL", "DRAFT", "FINAL", "FOR REVIEW"] {
                        if ui.small_button(s).clicked() {
                            st.stamp_text = s.into();
                        }
                    }
                });
            }
            Tool::Note => {
                ui.small(t("tip_note"));
            }
            Tool::ExportZone => {
                ui.checkbox(&mut self.export_isolate, t("export_isolate"));
                ui.horizontal(|ui| {
                    ui.label(t("pref_export_dpi"));
                    egui::ComboBox::from_id_salt("dpi_tool").selected_text(format!("{:.0} dpi", self.prefs.export_dpi)).show_ui(ui, |ui| {
                        for d in [72.0, 150.0, 300.0, 600.0] {
                            ui.selectable_value(&mut self.prefs.export_dpi, d, format!("{d:.0} dpi"));
                        }
                    });
                });
                ui.small(t("tip_export_zone"));
            }
            Tool::Measure => {
                let unit = self.prefs.units;
                ui.small(t("tip_measure"));
                if let Some(tab) = self.tabs.get(self.active) {
                    let pts = &tab.measure;
                    if pts.len() >= 2 {
                        let mut len = 0.0;
                        for w in pts.windows(2) {
                            len += ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
                        }
                        ui.label(format!("{}: {}", t("distance"), unit.format(len)));
                        if pts.len() >= 3 {
                            let mut area = 0.0;
                            for i in 0..pts.len() {
                                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                                area += a[0] * b[1] - b[0] * a[1];
                            }
                            let area = area.abs() / 2.0;
                            let u2 = unit.pt_per_unit() * unit.pt_per_unit();
                            ui.label(format!("{}: {:.2} {}²", t("area"), area / u2, unit.label()));
                            let per = len + ((pts[0][0] - pts[pts.len() - 1][0]).powi(2) + (pts[0][1] - pts[pts.len() - 1][1]).powi(2)).sqrt();
                            ui.label(format!("{}: {}", t("perimeter"), unit.format(per)));
                        }
                    }
                }
                if ui.button(t("clear")).clicked() {
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        tab.measure.clear();
                    }
                }
            }
            Tool::Redact => {
                ui.small(t("tip_redact"));
                let n = self.tabs.get(self.active).map(|tb| tb.redactions.len()).unwrap_or(0);
                ui.label(format!("{n} {}", t("redactions_pending")));
                ui.horizontal(|ui| {
                    if ui.add_enabled(n > 0, egui::Button::new(t("cmd_apply_redactions"))).clicked() {
                        self.pending_cmds.push(Cmd::ApplyRedactions);
                    }
                    if ui.add_enabled(n > 0, egui::Button::new(t("clear"))).clicked() {
                        if let Some(tab) = self.tabs.get_mut(self.active) {
                            tab.redactions.clear();
                        }
                    }
                });
            }
            Tool::Link => {
                ui.small(t("tip_link"));
            }
            Tool::Signature => {
                ui.small(t("tip_signature"));
            }
            Tool::FormText | Tool::FormCheck | Tool::FormRadio | Tool::FormCombo => {
                ui.text_edit_singleline(&mut self.new_field_name);
                ui.small(t("tip_form_create"));
            }
            _ => {}
        }
    }
}

/// Font picker: document fonts, standard fonts and system fonts (filterable).
pub fn font_combo(ui: &mut egui::Ui, family: &mut FontFamily, filter: &mut String) {
    egui::ComboBox::from_id_salt("font_family").width(ui.available_width() - 8.0).selected_text(family.label()).show_ui(ui, |ui| {
        ui.selectable_value(family, FontFamily::Original, FontFamily::Original.label());
        ui.separator();
        for (b, n) in [(Builtin::Helvetica, "Helvetica"), (Builtin::Times, "Times"), (Builtin::Courier, "Courier")] {
            ui.selectable_value(family, FontFamily::Builtin(b), n);
        }
        ui.separator();
        ui.add(egui::TextEdit::singleline(filter).hint_text(t("filter_fonts")).desired_width(180.0));
        let f = filter.to_lowercase();
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for name in fonts::catalog().families.iter().filter(|n| f.is_empty() || n.to_lowercase().contains(&f)) {
                let sel = matches!(family, FontFamily::System(s) if s == name);
                if ui.selectable_label(sel, name).clicked() {
                    *family = FontFamily::System(name.clone());
                }
            }
        });
    });
}

#[allow(dead_code)]
fn _unused(_: TextStyle, _: Unit) {}
