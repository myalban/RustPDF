//! Forms: fill, check, choose, create fields, flatten; simple calculations and formats; tab order.

use crate::model::*;
use crate::pdfedit_forms::{self, NewField};
use crate::tab::Tab;
use pdfium_render::prelude::*;
use std::collections::HashMap;

fn err<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

/// Tab order: top to bottom then left to right (in view space).
pub fn field_nav_order(info: &PageInfo) -> Vec<usize> {
    let mut v: Vec<(usize, f32, f32)> = info
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.read_only && matches!(f.kind, FieldKind::Text | FieldKind::Check | FieldKind::Radio | FieldKind::Combo | FieldKind::List))
        .map(|(i, f)| {
            let b = info.geom.rect_to_view(&f.rect);
            (i, b[1], b[0])
        })
        .collect();
    v.sort_by(|a, b| {
        let (ra, rb) = ((a.1 / 4.0).round(), (b.1 / 4.0).round());
        ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal).then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    });
    v.into_iter().map(|x| x.0).collect()
}

impl Tab {
    /// Values of all the fields (name → text): typed text, "Yes"/"No" for the checkboxes.
    pub fn all_field_values(&mut self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        for p in 0..self.page_count() {
            if let Some(info) = self.info(p) {
                for f in &info.fields {
                    let v = match f.kind {
                        FieldKind::Check | FieldKind::Radio => if f.checked { "1".to_string() } else { "0".to_string() },
                        _ => f.value.clone(),
                    };
                    m.entry(f.name.clone()).or_insert(v);
                }
            }
        }
        m
    }

    fn set_text_raw(&mut self, page: usize, ann: usize, value: &str, snap: bool, mem: u32) -> Result<(), String> {
        let value = value.to_string();
        self.edit_page_impl(page, "Fill a field", mem, snap, |pg, _| {
            let anns = pg.annotations_mut();
            let mut a = anns.get(ann as _)?;
            if let Some(f) = a.as_form_field_mut() {
                if let Some(t) = f.as_text_field_mut() {
                    t.set_value(&value)?;
                }
            }
            Ok(())
        })
    }

    /// Calculation/format rules of the document (cached for the current version).
    fn calc_rules(&mut self) -> Vec<pdfedit_forms::CalcRule> {
        if let Some((v, r)) = &self.calc {
            if *v == self.doc_ver {
                return r.clone();
            }
        }
        let rules = self.doc.save_to_bytes().map(|b| pdfedit_forms::scan_calc_rules(&b)).unwrap_or_default();
        self.calc = Some((self.doc_ver, rules.clone()));
        rules
    }

    /// Fills a text field: applies the field's format if it has one, then recomputes the dependent fields.
    pub fn set_field_text(&mut self, page: usize, field: usize, value: &str, mem: u32) -> Result<(), String> {
        let info = self.info(page).ok_or("page not found")?;
        let f = info.fields.get(field).ok_or("field not found")?.clone();
        if f.kind != FieldKind::Text {
            return Err("this field is not a text field".into());
        }
        let rules = self.calc_rules();
        let mut text = value.to_string();
        if let Some(r) = rules.iter().find(|r| r.target == f.name && r.op.is_none()) {
            if let (Some(fm), Some(n)) = (&r.format, pdfedit_forms::parse_number(value)) {
                text = pdfedit_forms::format_number(n, fm);
            }
        }
        self.set_text_raw(page, f.ann, &text, true, mem)?;
        self.recalc_fields(mem)?;
        Ok(())
    }

    /// Recomputes the calculated fields (SUM, AVERAGE, PRODUCT, MIN, MAX) from the current values.
    pub fn recalc_fields(&mut self, mem: u32) -> Result<usize, String> {
        let rules = self.calc_rules();
        if !rules.iter().any(|r| r.op.is_some()) {
            return Ok(0);
        }
        let mut changed = 0;
        for _pass in 0..3 {
            let values = self.all_field_values();
            let mut any = false;
            for r in rules.iter().filter(|r| r.op.is_some()) {
                let Some(new) = pdfedit_forms::eval_rule(r, &|n| values.get(n).cloned()) else { continue };
                if values.get(&r.target).map(|v| v.as_str()) == Some(new.as_str()) {
                    continue;
                }
                // find the target field
                let mut target: Option<(usize, usize)> = None;
                for p in 0..self.page_count() {
                    if let Some(info) = self.info(p) {
                        if let Some(f) = info.fields.iter().find(|f| f.name == r.target && f.kind == FieldKind::Text) {
                            target = Some((p, f.ann));
                            break;
                        }
                    }
                }
                if let Some((p, ann)) = target {
                    self.set_text_raw(p, ann, &new, false, mem)?;
                    changed += 1;
                    any = true;
                }
            }
            if !any {
                break;
            }
        }
        Ok(changed)
    }

    /// Checks / unchecks a box, or selects a radio button.
    pub fn toggle_field(&mut self, page: usize, field: usize, mem: u32) -> Result<(), String> {
        let info = self.info(page).ok_or("page not found")?;
        let f = info.fields.get(field).ok_or("field not found")?.clone();
        self.edit_page(page, "Check", mem, |pg, _| {
            let anns = pg.annotations_mut();
            let mut a = anns.get(f.ann as _)?;
            if let Some(fl) = a.as_form_field_mut() {
                if let Some(c) = fl.as_checkbox_field_mut() {
                    c.set_checked(!f.checked)?;
                } else if let Some(r) = fl.as_radio_button_field_mut() {
                    r.set_checked()?;
                }
            }
            Ok(())
        })?;
        self.recalc_fields(mem)?;
        Ok(())
    }

    /// Chooses an option in a dropdown list or a list box.
    pub fn choose_option(&mut self, page: usize, field: usize, option: usize, mem: u32) -> Result<(), String> {
        let info = self.info(page).ok_or("page not found")?;
        let f = info.fields.get(field).ok_or("field not found")?.clone();
        if !matches!(f.kind, FieldKind::Combo | FieldKind::List) {
            return Err("this field is not a list".into());
        }
        let bytes = self.doc.save_to_bytes().map_err(err)?;
        let out = pdfedit_forms::set_choice(&bytes, page, f.ann, option)?;
        self.replace_with_bytes("Choose an option", mem, out)
    }

    /// Creates a form field.
    pub fn create_field(&mut self, nf: &NewField, mem: u32) -> Result<(), String> {
        let bytes = self.doc.save_to_bytes().map_err(err)?;
        let out = pdfedit_forms::add_field(&bytes, nf)?;
        self.replace_with_bytes("Create a field", mem, out)
    }

    /// Field names already used in the document.
    pub fn field_names(&self) -> Vec<String> {
        self.doc.save_to_bytes().map(|b| pdfedit_forms::field_names(&b)).unwrap_or_default()
    }

    /// Flattens the form: the fields become frozen content.
    pub fn flatten_form(&mut self, mem: u32) -> Result<usize, String> {
        let mut total = 0;
        self.snapshot("Flatten form", mem);
        for p in 0..self.page_count() {
            let n = self.info(p).map(|i| i.fields.len()).unwrap_or(0);
            if n == 0 {
                continue;
            }
            let r = (|| {
                let mut pg = self.doc.pages().get(p as i32)?;
                pg.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
                pg.flatten()?;
                pg.regenerate_content()?;
                Ok::<(), PdfiumError>(())
            })();
            match r {
                Ok(_) => {
                    total += n;
                    self.page_ver[p] += 1;
                    self.infos.remove(&p);
                }
                Err(e) => {
                    self.undo.pop();
                    return Err(err(e));
                }
            }
        }
        if total == 0 {
            self.undo.pop();
        }
        self.search.stale = true;
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tab::Tab;

    fn blank() -> Tab {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        Tab::new(p, doc, None, None, 1.0)
    }

    fn nf(kind: FieldKind, name: &str, rect: [f32; 4]) -> NewField {
        NewField { page: 0, kind, rect, name: name.into(), options: vec![], value: String::new(), read_only: false, required: false, multiline: false }
    }

    #[test]
    fn create_every_kind_of_field_and_read_it_back() {
        let mut t = blank();
        t.create_field(&nf(FieldKind::Text, "name", [100.0, 700.0, 300.0, 722.0]), 100).unwrap();
        t.create_field(&nf(FieldKind::Check, "ok", [100.0, 650.0, 114.0, 664.0]), 100).unwrap();
        let mut combo = nf(FieldKind::Combo, "country", [100.0, 600.0, 250.0, 620.0]);
        combo.options = vec!["France".into(), "Belgium".into(), "Switzerland".into()];
        t.create_field(&combo, 100).unwrap();
        let mut r1 = nf(FieldKind::Radio, "choice", [100.0, 560.0, 114.0, 574.0]);
        r1.options = vec!["A".into()];
        t.create_field(&r1, 100).unwrap();
        let mut r2 = nf(FieldKind::Radio, "choice", [140.0, 560.0, 154.0, 574.0]);
        r2.options = vec!["B".into()];
        t.create_field(&r2, 100).unwrap();
        let info = t.info(0).unwrap();
        let kinds: Vec<(String, FieldKind)> = info.fields.iter().map(|f| (f.name.clone(), f.kind)).collect();
        assert!(kinds.contains(&("name".into(), FieldKind::Text)), "{kinds:?}");
        assert!(kinds.contains(&("ok".into(), FieldKind::Check)), "{kinds:?}");
        assert!(kinds.contains(&("country".into(), FieldKind::Combo)), "{kinds:?}");
        assert_eq!(kinds.iter().filter(|(n, k)| n == "choice" && *k == FieldKind::Radio).count(), 2, "{kinds:?}");
        let c = info.fields.iter().find(|f| f.name == "country").unwrap();
        assert_eq!(c.options.len(), 3);
        assert_eq!(t.field_names().len(), 4, "{:?}", t.field_names());
    }

    #[test]
    fn fill_toggle_choose() {
        let mut t = blank();
        t.create_field(&nf(FieldKind::Text, "name", [100.0, 700.0, 300.0, 722.0]), 100).unwrap();
        t.create_field(&nf(FieldKind::Check, "ok", [100.0, 650.0, 114.0, 664.0]), 100).unwrap();
        let mut combo = nf(FieldKind::Combo, "country", [100.0, 600.0, 250.0, 620.0]);
        combo.options = vec!["France".into(), "Belgium".into()];
        t.create_field(&combo, 100).unwrap();
        let idx = |t: &mut Tab, n: &str| t.info(0).unwrap().fields.iter().position(|f| f.name == n).unwrap();
        let i = idx(&mut t, "name");
        t.set_field_text(0, i, "Jean Dupont", 100).unwrap();
        assert_eq!(t.info(0).unwrap().fields[i].value, "Jean Dupont");
        let i = idx(&mut t, "ok");
        assert!(!t.info(0).unwrap().fields[i].checked);
        t.toggle_field(0, i, 100).unwrap();
        assert!(t.info(0).unwrap().fields[i].checked);
        t.toggle_field(0, i, 100).unwrap();
        assert!(!t.info(0).unwrap().fields[i].checked);
        let i = idx(&mut t, "country");
        t.choose_option(0, i, 1, 100).unwrap();
        let f = t.info(0).unwrap().fields.iter().find(|f| f.name == "country").unwrap().clone();
        assert_eq!(f.value, "Belgium", "{f:?}");
        // save / reopen: the values persist
        let bytes = t.doc.save_to_bytes().unwrap();
        let p = crate::pdfx::test_pdfium();
        let doc2 = p.load_pdf_from_byte_vec(bytes, None).unwrap();
        let mut t2 = Tab::new(p, doc2, None, None, 1.0);
        let vals = t2.all_field_values();
        assert_eq!(vals.get("name").map(|s| s.as_str()), Some("Jean Dupont"));
        assert_eq!(vals.get("country").map(|s| s.as_str()), Some("Belgium"));
    }

    #[test]
    fn tab_order_is_top_to_bottom_then_left_to_right() {
        let mut t = blank();
        t.create_field(&nf(FieldKind::Text, "bottom", [100.0, 300.0, 200.0, 320.0]), 100).unwrap();
        t.create_field(&nf(FieldKind::Text, "top_right", [300.0, 700.0, 400.0, 720.0]), 100).unwrap();
        t.create_field(&nf(FieldKind::Text, "top_left", [100.0, 700.0, 200.0, 720.0]), 100).unwrap();
        let info = t.info(0).unwrap();
        let order: Vec<String> = field_nav_order(&info).into_iter().map(|i| info.fields[i].name.clone()).collect();
        assert_eq!(order, vec!["top_left", "top_right", "bottom"]);
    }

    #[test]
    fn flatten_removes_fields() {
        let mut t = blank();
        t.create_field(&nf(FieldKind::Text, "name", [100.0, 700.0, 300.0, 722.0]), 100).unwrap();
        let i = 0;
        t.set_field_text(0, i, "Flat", 100).unwrap();
        assert_eq!(t.info(0).unwrap().fields.len(), 1);
        let n = t.flatten_form(100).unwrap();
        assert_eq!(n, 1);
        assert!(t.info(0).unwrap().fields.is_empty(), "no field left after flattening");
        // undoable
        t.undo();
        assert_eq!(t.info(0).unwrap().fields.len(), 1);
    }

    #[test]
    fn calculated_field_follows_its_sources() {
        let mut t = blank();
        for (n, y) in [("a", 700.0), ("b", 670.0), ("total", 640.0)] {
            t.create_field(&nf(FieldKind::Text, n, [100.0, y, 200.0, y + 20.0]), 100).unwrap();
        }
        // add the AFSimple_Calculate rule to the "total" field with lopdf
        let bytes = t.doc.save_to_bytes().unwrap();
        let mut ld = lopdf::Document::load_mem(&bytes).unwrap();
        let pid = *ld.get_pages().values().next().unwrap();
        let annots: Vec<lopdf::Object> = crate::pdfedit_annot::annots_array(&ld, pid);
        for o in annots {
            if let lopdf::Object::Reference(r) = o {
                let is_total = ld.get_dictionary(r).ok().and_then(|d| d.get(b"T").ok().map(crate::pdfedit_annot::text_of)).as_deref() == Some("total");
                if is_total {
                    let mut js = lopdf::Dictionary::new();
                    js.set("S", lopdf::Object::Name(b"JavaScript".to_vec()));
                    js.set("JS", lopdf::Object::string_literal("AFSimple_Calculate(\"SUM\", new Array (\"a\", \"b\"));"));
                    let mut fm = lopdf::Dictionary::new();
                    fm.set("S", lopdf::Object::Name(b"JavaScript".to_vec()));
                    fm.set("JS", lopdf::Object::string_literal("AFNumber_Format(2, 2, 0, 0, \" €\", false);"));
                    let mut aa = lopdf::Dictionary::new();
                    aa.set("C", lopdf::Object::Dictionary(js));
                    aa.set("F", lopdf::Object::Dictionary(fm));
                    ld.get_object_mut(r).unwrap().as_dict_mut().unwrap().set("AA", lopdf::Object::Dictionary(aa));
                }
            }
        }
        let mut out = vec![];
        ld.save_to(&mut out).unwrap();
        t.replace_with_bytes("rule", 100, out).unwrap();
        let rules = pdfedit_forms::scan_calc_rules(&t.doc.save_to_bytes().unwrap());
        assert!(rules.iter().any(|r| r.target == "total" && r.op.is_some() && r.format.is_some()), "{rules:?}");
        let idx = |t: &mut Tab, n: &str| t.info(0).unwrap().fields.iter().position(|f| f.name == n).unwrap();
        let ia = idx(&mut t, "a");
        let ib = idx(&mut t, "b");
        t.set_field_text(0, ia, "10", 100).unwrap();
        t.set_field_text(0, ib, "2,5", 100).unwrap();
        let it = idx(&mut t, "total");
        assert_eq!(t.info(0).unwrap().fields[it].value, "12,50 €", "sum formatted in the French style");
    }
}
