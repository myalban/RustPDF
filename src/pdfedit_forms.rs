//! Forms (AcroForm) written with `lopdf`: field creation, list choices, calculation/format rules.

use crate::geom::R4;
use crate::model::FieldKind;
use crate::pdfedit_annot::{annot_dict_of, annots_array, page_id, pdf_text, set_annots_array, text_of};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

fn f(v: f32) -> Object {
    Object::Real(v)
}

fn name(s: &str) -> Object {
    Object::Name(s.as_bytes().to_vec())
}

fn rect_obj(r: &R4) -> Object {
    Object::Array(vec![f(r[0]), f(r[1]), f(r[2]), f(r[3])])
}

fn save(doc: &mut Document) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| format!("write: {e}"))?;
    Ok(out)
}

/// The document's /AcroForm dictionary (created if needed, with Helvetica as the default font).
fn ensure_acroform(doc: &mut Document) -> Result<ObjectId, String> {
    let root = doc.trailer.get(b"Root").and_then(|r| r.as_reference()).map_err(|e| e.to_string())?;
    let existing = doc.get_dictionary(root).ok().and_then(|c| c.get(b"AcroForm").ok().cloned());
    let af_id = match existing {
        Some(Object::Reference(r)) => r,
        Some(Object::Dictionary(d)) => {
            let id = doc.add_object(Object::Dictionary(d));
            doc.get_object_mut(root).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("AcroForm", Object::Reference(id));
            id
        }
        _ => {
            let id = doc.add_object(Object::Dictionary(Dictionary::new()));
            doc.get_object_mut(root).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("AcroForm", Object::Reference(id));
            id
        }
    };
    // default font
    let mut font = Dictionary::new();
    font.set("Type", name("Font"));
    font.set("Subtype", name("Type1"));
    font.set("BaseFont", name("Helvetica"));
    font.set("Encoding", name("WinAnsiEncoding"));
    let have_dr = doc.get_dictionary(af_id).map(|d| d.has(b"DR")).unwrap_or(false);
    let font_id = if have_dr { None } else { Some(doc.add_object(Object::Dictionary(font))) };
    let af = doc.get_object_mut(af_id).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
    if !af.has(b"Fields") {
        af.set("Fields", Object::Array(vec![]));
    }
    if let Some(fid) = font_id {
        let mut fonts = Dictionary::new();
        fonts.set("Helv", Object::Reference(fid));
        let mut dr = Dictionary::new();
        dr.set("Font", Object::Dictionary(fonts));
        af.set("DR", Object::Dictionary(dr));
    }
    if !af.has(b"DA") {
        af.set("DA", Object::String(b"/Helv 11 Tf 0 g".to_vec(), StringFormat::Literal));
    }
    af.set("NeedAppearances", Object::Boolean(true));
    Ok(af_id)
}

fn field_array(doc: &Document, af: ObjectId) -> Vec<Object> {
    doc.get_dictionary(af).ok().and_then(|d| d.get(b"Fields").ok().and_then(|o| o.as_array().ok().cloned())).unwrap_or_default()
}

/// Full name (T) of a dictionary's field, walking up the parents.
fn full_name(doc: &Document, d: &Dictionary) -> String {
    let mut parts = vec![];
    let mut cur = d.clone();
    for _ in 0..16 {
        if let Ok(t) = cur.get(b"T") {
            parts.push(text_of(t));
        }
        match cur.get(b"Parent").ok().and_then(|p| p.as_reference().ok()).and_then(|r| doc.get_dictionary(r).ok()) {
            Some(p) => cur = p.clone(),
            None => break,
        }
    }
    parts.reverse();
    parts.join(".")
}

/// Names of all the terminal fields of the document (to avoid duplicates).
pub fn field_names(bytes: &[u8]) -> Vec<String> {
    let Ok(doc) = Document::load_mem(bytes) else { return vec![] };
    let mut out = vec![];
    for (_, pid) in doc.get_pages() {
        for o in annots_array(&doc, pid) {
            if let Some((_, d)) = annot_dict_of(&doc, &o) {
                if d.get(b"Subtype").ok().map(text_of).as_deref() == Some("Widget") {
                    let n = full_name(&doc, &d);
                    if !n.is_empty() && !out.contains(&n) {
                        out.push(n);
                    }
                }
            }
        }
    }
    out
}

fn box_stream(doc: &mut Document, w: f32, h: f32, check: bool) -> ObjectId {
    let mut content = format!("q 1 g 0.5 0.5 {:.2} {:.2} re f 0 G 1 w 0.5 0.5 {:.2} {:.2} re S Q\n", w - 1.0, h - 1.0, w - 1.0, h - 1.0);
    if check {
        content.push_str(&format!("q 0 G 1.6 w 1 J {:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l S Q\n", w * 0.2, h * 0.5, w * 0.42, h * 0.25, w * 0.8, h * 0.78));
    }
    let mut d = Dictionary::new();
    d.set("Type", name("XObject"));
    d.set("Subtype", name("Form"));
    d.set("BBox", rect_obj(&[0.0, 0.0, w, h]));
    doc.add_object(Stream::new(d, content.into_bytes()))
}

fn radio_stream(doc: &mut Document, w: f32, h: f32, on: bool) -> ObjectId {
    let (cx, cy, r) = (w / 2.0, h / 2.0, (w.min(h) / 2.0 - 0.5).max(1.0));
    let k = 0.5523 * r;
    let circle = |rr: f32, kk: f32| {
        format!(
            "{:.2} {:.2} m {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ",
            cx + rr, cy,
            cx + rr, cy + kk, cx + kk, cy + rr, cx, cy + rr,
            cx - kk, cy + rr, cx - rr, cy + kk, cx - rr, cy,
            cx - rr, cy - kk, cx - kk, cy - rr, cx, cy - rr,
            cx + kk, cy - rr, cx + rr, cy - kk, cx + rr, cy
        )
    };
    let mut content = format!("q 1 g {} f 0 G 1 w {} S Q\n", circle(r, k), circle(r, k));
    if on {
        content.push_str(&format!("q 0 g {} f Q\n", circle(r * 0.5, k * 0.5)));
    }
    let mut d = Dictionary::new();
    d.set("Type", name("XObject"));
    d.set("Subtype", name("Form"));
    d.set("BBox", rect_obj(&[0.0, 0.0, w, h]));
    doc.add_object(Stream::new(d, content.into_bytes()))
}

pub struct NewField {
    pub page: usize,
    pub kind: FieldKind,
    pub rect: R4,
    pub name: String,
    /// Options (dropdown list) or export value (radio button: first entry).
    pub options: Vec<String>,
    pub value: String,
    pub read_only: bool,
    pub required: bool,
    pub multiline: bool,
}

/// Creates a form field (text, checkbox, radio, dropdown list, list box).
pub fn add_field(bytes: &[u8], nf: &NewField) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| format!("read: {e}"))?;
    let pid = page_id(&doc, nf.page)?;
    let af = ensure_acroform(&mut doc)?;
    let (w, h) = (nf.rect[2] - nf.rect[0], nf.rect[3] - nf.rect[1]);
    let mut flags: i64 = 0;
    if nf.read_only {
        flags |= 1;
    }
    if nf.required {
        flags |= 2;
    }
    let mut d = Dictionary::new();
    d.set("Type", name("Annot"));
    d.set("Subtype", name("Widget"));
    d.set("Rect", rect_obj(&nf.rect));
    d.set("F", Object::Integer(4));
    d.set("P", Object::Reference(pid));
    let mut mk = Dictionary::new();
    mk.set("BC", Object::Array(vec![f(0.0), f(0.0), f(0.0)]));
    mk.set("BG", Object::Array(vec![f(1.0), f(1.0), f(1.0)]));
    let mut bs = Dictionary::new();
    bs.set("W", Object::Integer(1));
    bs.set("S", name("S"));
    d.set("BS", Object::Dictionary(bs));
    let mut group_for_radio: Option<ObjectId> = None;
    match nf.kind {
        FieldKind::Text => {
            if nf.multiline {
                flags |= 1 << 12;
            }
            d.set("FT", name("Tx"));
            d.set("T", pdf_text(&nf.name));
            d.set("V", pdf_text(&nf.value));
            d.set("DA", Object::String(b"/Helv 11 Tf 0 g".to_vec(), StringFormat::Literal));
            d.set("Ff", Object::Integer(flags));
            d.set("MK", Object::Dictionary(mk));
        }
        FieldKind::Check => {
            d.set("FT", name("Btn"));
            d.set("T", pdf_text(&nf.name));
            d.set("Ff", Object::Integer(flags));
            d.set("V", name("Off"));
            d.set("AS", name("Off"));
            mk.set("CA", Object::String(b"4".to_vec(), StringFormat::Literal));
            d.set("MK", Object::Dictionary(mk));
            d.set("DA", Object::String(b"/ZaDb 0 Tf 0 g".to_vec(), StringFormat::Literal));
            let (on, off) = (box_stream(&mut doc, w, h, true), box_stream(&mut doc, w, h, false));
            let mut n = Dictionary::new();
            n.set("Yes", Object::Reference(on));
            n.set("Off", Object::Reference(off));
            let mut ap = Dictionary::new();
            ap.set("N", Object::Dictionary(n));
            d.set("AP", Object::Dictionary(ap));
        }
        FieldKind::Radio => {
            let export = nf.options.first().cloned().unwrap_or_else(|| "Choix1".into());
            // existing group of the same name?
            let mut found: Option<ObjectId> = None;
            for fo in field_array(&doc, af) {
                if let Object::Reference(r) = fo {
                    if let Ok(g) = doc.get_dictionary(r) {
                        if g.get(b"T").ok().map(text_of).as_deref() == Some(nf.name.as_str()) && g.get(b"FT").ok().map(text_of).as_deref() == Some("Btn") {
                            found = Some(r);
                        }
                    }
                }
            }
            let gid = match found {
                Some(g) => g,
                None => {
                    let mut g = Dictionary::new();
                    g.set("FT", name("Btn"));
                    g.set("T", pdf_text(&nf.name));
                    g.set("Ff", Object::Integer(flags | (1 << 15) | (1 << 14)));
                    g.set("V", name("Off"));
                    g.set("Kids", Object::Array(vec![]));
                    let gid = doc.add_object(Object::Dictionary(g));
                    let mut fields = field_array(&doc, af);
                    fields.push(Object::Reference(gid));
                    doc.get_object_mut(af).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("Fields", Object::Array(fields));
                    gid
                }
            };
            group_for_radio = Some(gid);
            d.set("Parent", Object::Reference(gid));
            d.set("AS", name("Off"));
            d.set("MK", Object::Dictionary(mk));
            let (on, off) = (radio_stream(&mut doc, w, h, true), radio_stream(&mut doc, w, h, false));
            let mut n = Dictionary::new();
            n.set(export.as_bytes(), Object::Reference(on));
            n.set("Off", Object::Reference(off));
            let mut ap = Dictionary::new();
            ap.set("N", Object::Dictionary(n));
            d.set("AP", Object::Dictionary(ap));
        }
        FieldKind::Combo | FieldKind::List => {
            flags |= if nf.kind == FieldKind::Combo { 1 << 17 } else { 0 };
            d.set("FT", name("Ch"));
            d.set("T", pdf_text(&nf.name));
            d.set("Ff", Object::Integer(flags));
            d.set("Opt", Object::Array(nf.options.iter().map(|o| pdf_text(o)).collect()));
            if !nf.value.is_empty() {
                d.set("V", pdf_text(&nf.value));
            }
            d.set("DA", Object::String(b"/Helv 11 Tf 0 g".to_vec(), StringFormat::Literal));
            d.set("MK", Object::Dictionary(mk));
        }
        _ => return Err("unsupported field type".into()),
    }
    let wid = doc.add_object(Object::Dictionary(d));
    // page
    let mut arr = annots_array(&doc, pid);
    arr.push(Object::Reference(wid));
    set_annots_array(&mut doc, pid, arr)?;
    // root of the fields
    match group_for_radio {
        Some(g) => {
            let gd = doc.get_object_mut(g).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
            let mut kids = gd.get(b"Kids").ok().and_then(|k| k.as_array().ok().cloned()).unwrap_or_default();
            kids.push(Object::Reference(wid));
            gd.set("Kids", Object::Array(kids));
        }
        None => {
            let mut fields = field_array(&doc, af);
            fields.push(Object::Reference(wid));
            doc.get_object_mut(af).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("Fields", Object::Array(fields));
        }
    }
    save(&mut doc)
}

/// Dictionary carrying the /FT of a widget: itself or one of its ancestors.
fn field_owner(doc: &Document, widget_id: ObjectId) -> ObjectId {
    let mut cur = widget_id;
    for _ in 0..16 {
        let Ok(d) = doc.get_dictionary(cur) else { break };
        if d.has(b"FT") {
            return cur;
        }
        match d.get(b"Parent").ok().and_then(|p| p.as_reference().ok()) {
            Some(p) => cur = p,
            None => break,
        }
    }
    widget_id
}

/// Selects an option of a dropdown list / list box (`option` = rank in /Opt).
pub fn set_choice(bytes: &[u8], page: usize, widget_index: usize, option: usize) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| format!("read: {e}"))?;
    let pid = page_id(&doc, page)?;
    let arr = annots_array(&doc, pid);
    let Some(Object::Reference(wid)) = arr.get(widget_index).cloned() else { return Err("field not found".into()) };
    let owner = field_owner(&doc, wid);
    let opts = doc
        .get_dictionary(owner)
        .ok()
        .and_then(|d| d.get(b"Opt").ok().cloned())
        .and_then(|o| o.as_array().ok().cloned())
        .ok_or("list without options")?;
    let chosen = opts.get(option).ok_or("option does not exist")?.clone();
    let export = match chosen {
        Object::Array(a) => a.first().cloned().unwrap_or(Object::String(vec![], StringFormat::Literal)),
        other => other,
    };
    {
        let od = doc.get_object_mut(owner).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
        od.set("V", export);
        od.set("I", Object::Array(vec![Object::Integer(option as i64)]));
    }
    // force the regeneration of the widget's appearance
    if let Ok(wd) = doc.get_object_mut(wid).and_then(|o| o.as_dict_mut()) {
        wd.remove(b"AP");
    }
    ensure_acroform(&mut doc)?;
    save(&mut doc)
}

// ------------------------------------------------------------ calculations and formats

#[derive(Clone, Debug, PartialEq)]
pub enum CalcOp {
    Sum,
    Avg,
    Prd,
    Min,
    Max,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NumFormat {
    pub decimals: usize,
    /// 0 = 1,234.56 ; 1 = 1234.56 ; 2 = 1.234,56 ; 3 = 1234,56
    pub sep_style: u8,
    pub currency: String,
    pub prepend: bool,
    pub percent: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalcRule {
    pub target: String,
    pub op: Option<(CalcOp, Vec<String>)>,
    pub format: Option<NumFormat>,
}

fn js_of(doc: &Document, action: &Object) -> String {
    let d = match action {
        Object::Reference(r) => doc.get_dictionary(*r).ok().cloned(),
        Object::Dictionary(d) => Some(d.clone()),
        _ => None,
    };
    let Some(d) = d else { return String::new() };
    match d.get(b"JS") {
        Ok(Object::String(..)) => text_of(d.get(b"JS").unwrap()),
        Ok(Object::Reference(r)) => match doc.get_object(*r) {
            Ok(Object::Stream(s)) => String::from_utf8_lossy(&s.decompressed_content().unwrap_or_else(|_| s.content.clone())).into_owned(),
            Ok(o) => text_of(o),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn quoted_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut q: Option<char> = None;
    for ch in s.chars() {
        match q {
            Some(c) if ch == c => {
                out.push(std::mem::take(&mut cur));
                q = None;
            }
            Some(_) => cur.push(ch),
            None if ch == '"' || ch == '\'' => q = Some(ch),
            None => {}
        }
    }
    out
}

fn number_args(s: &str) -> Vec<String> {
    let inner = s.split_once('(').map(|x| x.1).unwrap_or("");
    let inner = inner.rsplit_once(')').map(|x| x.0).unwrap_or(inner);
    inner.split(',').map(|x| x.trim().trim_matches('"').trim_matches('\'').to_string()).collect()
}

/// Calculation (/AA /C) and format (/AA /F) rules recognized among the document's fields.
pub fn scan_calc_rules(bytes: &[u8]) -> Vec<CalcRule> {
    let Ok(doc) = Document::load_mem(bytes) else { return vec![] };
    let mut rules: Vec<CalcRule> = vec![];
    for (_, pid) in doc.get_pages() {
        for o in annots_array(&doc, pid) {
            let Some((id, d)) = annot_dict_of(&doc, &o) else { continue };
            if d.get(b"Subtype").ok().map(text_of).as_deref() != Some("Widget") {
                continue;
            }
            let owner_d = id.map(|i| field_owner(&doc, i)).and_then(|i| doc.get_dictionary(i).ok().cloned()).unwrap_or(d.clone());
            let aa = d.get(b"AA").ok().or_else(|| owner_d.get(b"AA").ok()).cloned();
            let Some(Object::Dictionary(aa)) = aa.map(|o| match o {
                Object::Reference(r) => doc.get_object(r).cloned().unwrap_or(Object::Null),
                x => x,
            }) else {
                continue;
            };
            let target = full_name(&doc, &owner_d);
            let mut rule = CalcRule { target: target.clone(), op: None, format: None };
            if let Ok(c) = aa.get(b"C") {
                let js = js_of(&doc, c);
                if let Some(pos) = js.find("AFSimple_Calculate") {
                    let rest = &js[pos..];
                    let op = if rest.contains("\"SUM\"") || rest.contains("'SUM'") {
                        Some(CalcOp::Sum)
                    } else if rest.contains("\"AVG\"") || rest.contains("'AVG'") {
                        Some(CalcOp::Avg)
                    } else if rest.contains("\"PRD\"") || rest.contains("'PRD'") {
                        Some(CalcOp::Prd)
                    } else if rest.contains("\"MIN\"") || rest.contains("'MIN'") {
                        Some(CalcOp::Min)
                    } else if rest.contains("\"MAX\"") || rest.contains("'MAX'") {
                        Some(CalcOp::Max)
                    } else {
                        None
                    };
                    let args = quoted_args(rest);
                    // the first quoted argument is the operation
                    if let Some(op) = op {
                        rule.op = Some((op, args.into_iter().skip(1).collect()));
                    }
                }
            }
            if let Ok(fm) = aa.get(b"F") {
                let js = js_of(&doc, fm);
                if js.contains("AFNumber_Format") {
                    let a = number_args(js.split("AFNumber_Format").nth(1).unwrap_or(""));
                    rule.format = Some(NumFormat {
                        decimals: a.first().and_then(|x| x.parse().ok()).unwrap_or(2),
                        sep_style: a.get(1).and_then(|x| x.parse().ok()).unwrap_or(0),
                        currency: a.get(4).cloned().unwrap_or_default(),
                        prepend: a.get(5).map(|x| x == "true").unwrap_or(true),
                        percent: false,
                    });
                } else if js.contains("AFPercent_Format") {
                    let a = number_args(js.split("AFPercent_Format").nth(1).unwrap_or(""));
                    rule.format = Some(NumFormat { decimals: a.first().and_then(|x| x.parse().ok()).unwrap_or(2), sep_style: a.get(1).and_then(|x| x.parse().ok()).unwrap_or(0), currency: String::new(), prepend: false, percent: true });
                }
            }
            if rule.op.is_some() || rule.format.is_some() {
                rules.push(rule);
            }
        }
    }
    rules
}

/// Parses a typed number (decimal comma or point, spaces and thousands separators ignored).
pub fn parse_number(s: &str) -> Option<f64> {
    let t: String = s.trim().chars().filter(|c| !c.is_whitespace() && *c != '€' && *c != '$' && *c != '%').collect();
    if t.is_empty() {
        return None;
    }
    // the last comma or point is the decimal separator
    let last = t.rfind(|c| c == ',' || c == '.');
    let cleaned = match last {
        Some(p) => {
            let (int, frac) = t.split_at(p);
            let int: String = int.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '+').collect();
            format!("{int}.{}", &frac[1..])
        }
        None => t,
    };
    cleaned.parse().ok()
}

pub fn format_number(v: f64, fm: &NumFormat) -> String {
    let v = if fm.percent { v * 100.0 } else { v };
    let neg = v < 0.0;
    let s = format!("{:.*}", fm.decimals, v.abs());
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s, None),
    };
    let (thou, dec) = match fm.sep_style {
        0 => (",", "."),
        1 => ("", "."),
        2 => (".", ","),
        _ => ("", ","),
    };
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 && !thou.is_empty() {
            grouped.push_str(thou);
        }
        grouped.push(c);
    }
    let mut out = grouped;
    if let Some(f) = frac {
        out.push_str(dec);
        out.push_str(&f);
    }
    if fm.percent {
        out.push('%');
    }
    if !fm.currency.is_empty() {
        out = if fm.prepend { format!("{}{out}", fm.currency) } else { format!("{out}{}", fm.currency) };
    }
    if neg {
        out = format!("-{out}");
    }
    out
}

/// Value computed by a rule from the (text) values of the source fields.
pub fn eval_rule(rule: &CalcRule, value_of: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let (op, sources) = rule.op.as_ref()?;
    let nums: Vec<f64> = sources.iter().filter_map(|s| value_of(s)).filter_map(|v| parse_number(&v)).collect();
    let v = match op {
        CalcOp::Sum => nums.iter().sum(),
        CalcOp::Avg => {
            if nums.is_empty() {
                0.0
            } else {
                nums.iter().sum::<f64>() / nums.len() as f64
            }
        }
        CalcOp::Prd => nums.iter().product(),
        CalcOp::Min => nums.iter().cloned().fold(f64::INFINITY, f64::min),
        CalcOp::Max => nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    };
    let v = if v.is_finite() { v } else { 0.0 };
    Some(match &rule.format {
        Some(fm) => format_number(v, fm),
        None => {
            let t = format!("{v:.6}");
            t.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_parse_and_format() {
        assert_eq!(parse_number("1 234,56"), Some(1234.56));
        assert_eq!(parse_number("1,234.5"), Some(1234.5));
        assert_eq!(parse_number("12"), Some(12.0));
        assert_eq!(parse_number("abc"), None);
        let fm = NumFormat { decimals: 2, sep_style: 2, currency: " €".into(), prepend: false, percent: false };
        assert_eq!(format_number(1234.5, &fm), "1.234,50 €");
        let us = NumFormat { decimals: 1, sep_style: 0, currency: "$".into(), prepend: true, percent: false };
        assert_eq!(format_number(-1234567.89, &us), "-$1,234,567.9");
        let pc = NumFormat { decimals: 0, sep_style: 1, currency: String::new(), prepend: false, percent: true };
        assert_eq!(format_number(0.256, &pc), "26%");
    }

    #[test]
    fn rule_evaluation() {
        let rule = CalcRule { target: "total".into(), op: Some((CalcOp::Sum, vec!["a".into(), "b".into(), "c".into()])), format: None };
        let vals = |n: &str| match n {
            "a" => Some("10".to_string()),
            "b" => Some("2,5".to_string()),
            _ => None,
        };
        assert_eq!(eval_rule(&rule, &vals).as_deref(), Some("12.5"));
        let avg = CalcRule { target: "m".into(), op: Some((CalcOp::Avg, vec!["a".into(), "b".into()])), format: Some(NumFormat { decimals: 2, sep_style: 1, currency: String::new(), prepend: false, percent: false }) };
        assert_eq!(eval_rule(&avg, &vals).as_deref(), Some("6.25"));
    }

    #[test]
    fn quoted_args_and_js() {
        let a = quoted_args("AFSimple_Calculate(\"SUM\", new Array (\"x\", 'y z'))");
        assert_eq!(a, vec!["SUM", "x", "y z"]);
        let n = number_args("(2, 0, 1, 0, \" €\", false)");
        assert_eq!(n, vec!["2", "0", "1", "0", " €", "false"]);
    }
}
