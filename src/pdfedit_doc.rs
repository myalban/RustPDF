//! Document-level operations (`lopdf`): properties, cleaning, encryption, bookmarks, links, layers, watermark.

use crate::geom::R4;
use crate::pdfedit_annot::{annot_dict_of, annots_array, page_id, pdf_text, set_annots_array, text_of};
use crate::pdfedit_pages::{flatten_pages, page_ids};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use std::collections::BTreeMap;
use std::sync::Arc;

fn load(b: &[u8]) -> Result<Document, String> {
    Document::load_mem(b).map_err(|e| format!("read: {e}"))
}

fn save(doc: &mut Document) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| format!("write: {e}"))?;
    Ok(out)
}

fn f(v: f32) -> Object {
    Object::Real(v)
}

fn name(s: &str) -> Object {
    Object::Name(s.as_bytes().to_vec())
}

fn catalog_id(doc: &Document) -> Result<ObjectId, String> {
    doc.trailer.get(b"Root").and_then(|r| r.as_reference()).map_err(|e| e.to_string())
}

// ===================================================================== properties

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocInfo {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
    pub producer: String,
    pub created: String,
    pub modified: String,
}

fn info_id(doc: &Document) -> Option<ObjectId> {
    doc.trailer.get(b"Info").ok().and_then(|i| i.as_reference().ok())
}

pub fn read_info(bytes: &[u8]) -> DocInfo {
    let Ok(doc) = load(bytes) else { return DocInfo::default() };
    let Some(d) = info_id(&doc).and_then(|i| doc.get_dictionary(i).ok().cloned()) else { return DocInfo::default() };
    let g = |k: &[u8]| d.get(k).ok().map(text_of).unwrap_or_default();
    DocInfo { title: g(b"Title"), author: g(b"Author"), subject: g(b"Subject"), keywords: g(b"Keywords"), creator: g(b"Creator"), producer: g(b"Producer"), created: g(b"CreationDate"), modified: g(b"ModDate") }
}

pub fn write_info(bytes: &[u8], info: &DocInfo) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    let id = match info_id(&doc) {
        Some(i) => i,
        None => {
            let id = doc.add_object(Object::Dictionary(Dictionary::new()));
            doc.trailer.set("Info", Object::Reference(id));
            id
        }
    };
    let d = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
    for (k, v) in [("Title", &info.title), ("Author", &info.author), ("Subject", &info.subject), ("Keywords", &info.keywords), ("Creator", &info.creator), ("Producer", &info.producer)] {
        if v.is_empty() {
            d.remove(k.as_bytes());
        } else {
            d.set(k, pdf_text(v));
        }
    }
    save(&mut doc)
}

// ======================================================================= cleaning

#[derive(Clone, Debug)]
pub struct CleanOpts {
    pub info: bool,
    pub xmp: bool,
    pub comments: bool,
    pub attachments: bool,
    pub bookmarks: bool,
    pub javascript: bool,
    pub thumbnails: bool,
}

impl Default for CleanOpts {
    fn default() -> Self {
        CleanOpts { info: true, xmp: true, comments: false, attachments: false, bookmarks: false, javascript: true, thumbnails: true }
    }
}

/// Removes the requested hidden information; returns the bytes and the list of what was removed.
pub fn clean(bytes: &[u8], o: &CleanOpts) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut doc = load(bytes)?;
    let mut report = vec![];
    let cat = catalog_id(&doc)?;
    if o.info {
        if let Some(id) = info_id(&doc) {
            if let Ok(d) = doc.get_object_mut(id).and_then(|x| x.as_dict_mut()) {
                if !d.is_empty() {
                    *d = Dictionary::new();
                    report.push("Document information (author, title, software…)".to_string());
                }
            }
        }
    }
    {
        let cd = doc.get_object_mut(cat).and_then(|x| x.as_dict_mut()).map_err(|e| e.to_string())?;
        if o.xmp && cd.remove(b"Metadata").is_some() {
            report.push("XMP metadata".into());
        }
        if o.bookmarks && cd.remove(b"Outlines").is_some() {
            report.push("Bookmarks".into());
        }
        if o.javascript {
            if cd.remove(b"OpenAction").is_some() | cd.remove(b"AA").is_some() {
                report.push("Automatic actions on open".into());
            }
        }
        cd.remove(b"PieceInfo");
    }
    // /Names tree: attachments and JavaScript
    let names_id = doc.get_dictionary(cat).ok().and_then(|c| c.get(b"Names").ok().cloned());
    let apply_names = |doc: &mut Document, key: &[u8], what: &str, report: &mut Vec<String>| {
        let nid = match &names_id {
            Some(Object::Reference(r)) => Some(*r),
            _ => None,
        };
        if let Some(nid) = nid {
            if let Ok(nd) = doc.get_object_mut(nid).and_then(|x| x.as_dict_mut()) {
                if nd.remove(key).is_some() {
                    report.push(what.to_string());
                }
            }
        } else if let Ok(cd) = doc.get_object_mut(cat).and_then(|x| x.as_dict_mut()) {
            if let Ok(Object::Dictionary(nd)) = cd.get_mut(b"Names") {
                if nd.remove(key).is_some() {
                    report.push(what.to_string());
                }
            }
        }
    };
    if o.attachments {
        apply_names(&mut doc, b"EmbeddedFiles", "Attachments", &mut report);
    }
    if o.javascript {
        apply_names(&mut doc, b"JavaScript", "Scripts JavaScript", &mut report);
    }
    // pages: comments, thumbnails, actions
    let mut removed_comments = 0;
    for pid in page_ids(&doc) {
        if o.thumbnails {
            if let Ok(pd) = doc.get_object_mut(pid).and_then(|x| x.as_dict_mut()) {
                if pd.remove(b"Thumb").is_some() {
                    report.push("Thumbnails".into());
                }
            }
        }
        if o.javascript {
            if let Ok(pd) = doc.get_object_mut(pid).and_then(|x| x.as_dict_mut()) {
                pd.remove(b"AA");
            }
        }
        if o.comments {
            let arr = annots_array(&doc, pid);
            let keep: Vec<Object> = arr
                .into_iter()
                .filter(|a| {
                    let st = annot_dict_of(&doc, a).and_then(|(_, d)| d.get(b"Subtype").ok().map(text_of)).unwrap_or_default();
                    let keep = matches!(st.as_str(), "Link" | "Widget");
                    if !keep {
                        removed_comments += 1;
                    }
                    keep
                })
                .collect();
            set_annots_array(&mut doc, pid, keep)?;
        }
    }
    if removed_comments > 0 {
        report.push(format!("{removed_comments} annotation(s) and comment(s)"));
    }
    doc.prune_objects();
    report.dedup();
    Ok((save(&mut doc)?, report))
}

// ====================================================================== encryption

#[derive(Clone, Debug, PartialEq)]
pub struct Security {
    pub user_password: String,
    pub owner_password: String,
    pub allow_print: bool,
    pub allow_print_hq: bool,
    pub allow_copy: bool,
    pub allow_modify: bool,
    pub allow_annotate: bool,
    pub allow_fill: bool,
    pub allow_assemble: bool,
}

impl Default for Security {
    fn default() -> Self {
        Security { user_password: String::new(), owner_password: String::new(), allow_print: true, allow_print_hq: true, allow_copy: true, allow_modify: true, allow_annotate: true, allow_fill: true, allow_assemble: true }
    }
}

impl Security {
    pub fn is_active(&self) -> bool {
        !self.user_password.is_empty() || !self.owner_password.is_empty() || !(self.allow_print && self.allow_print_hq && self.allow_copy && self.allow_modify && self.allow_annotate && self.allow_fill && self.allow_assemble)
    }
    fn permissions(&self) -> lopdf::Permissions {
        use lopdf::Permissions as P;
        let mut p = P::empty();
        if self.allow_print {
            p |= P::PRINTABLE;
        }
        if self.allow_print_hq {
            p |= P::PRINTABLE_IN_HIGH_QUALITY;
        }
        if self.allow_copy {
            p |= P::COPYABLE | P::COPYABLE_FOR_ACCESSIBILITY;
        } else {
            p |= P::COPYABLE_FOR_ACCESSIBILITY;
        }
        if self.allow_modify {
            p |= P::MODIFIABLE;
        }
        if self.allow_annotate {
            p |= P::ANNOTABLE;
        }
        if self.allow_fill {
            p |= P::FILLABLE;
        }
        if self.allow_assemble {
            p |= P::ASSEMBLABLE;
        }
        p
    }
}

/// Encrypts a document with AES-256 (PDF 2.0). The owner password is required as soon as there are restrictions.
pub fn encrypt(bytes: &[u8], s: &Security) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    if doc.is_encrypted() {
        return Err("the document is already encrypted".into());
    }
    if !doc.trailer.has(b"ID") {
        let mut a = [0u8; 16];
        getrandom::fill(&mut a).map_err(|e| e.to_string())?;
        let id = Object::String(a.to_vec(), StringFormat::Hexadecimal);
        doc.trailer.set("ID", Object::Array(vec![id.clone(), id]));
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| e.to_string())?;
    let filter: Arc<dyn lopdf::encryption::crypt_filters::CryptFilter> = Arc::new(lopdf::encryption::crypt_filters::Aes256CryptFilter);
    let owner = if s.owner_password.is_empty() { s.user_password.clone() } else { s.owner_password.clone() };
    let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: &owner,
        user_password: &s.user_password,
        permissions: s.permissions(),
    })
    .map_err(|e| format!("encryption: {e}"))?;
    doc.encrypt(&state).map_err(|e| format!("encryption: {e}"))?;
    save(&mut doc)
}

/// Protection read from the file (without decrypting it).
#[derive(Clone, Debug, PartialEq)]
pub struct SecurityInfo {
    pub revision: i64,
    pub aes: bool,
    pub allow: Security,
}

/// Reads the rights of an encrypted document from `/Encrypt /P`; `None` if the file is not encrypted.
pub fn read_security(bytes: &[u8]) -> Option<SecurityInfo> {
    let doc = Document::load_mem(bytes).ok()?;
    // A document without user password is decrypted on load and its /Encrypt entry is removed from the trailer:
    // the permissions then come from the decryption state.
    if let Some(state) = &doc.encryption_state {
        use lopdf::Permissions as P;
        let p = state.permissions();
        return Some(SecurityInfo {
            revision: state.revision(),
            aes: state.version() >= 4,
            allow: Security {
                user_password: String::new(),
                owner_password: String::new(),
                allow_print: p.contains(P::PRINTABLE),
                allow_print_hq: p.contains(P::PRINTABLE_IN_HIGH_QUALITY),
                allow_copy: p.contains(P::COPYABLE),
                allow_modify: p.contains(P::MODIFIABLE),
                allow_annotate: p.contains(P::ANNOTABLE),
                allow_fill: p.contains(P::FILLABLE),
                allow_assemble: p.contains(P::ASSEMBLABLE),
            },
        });
    }
    let enc = match doc.trailer.get(b"Encrypt").ok()? {
        Object::Reference(r) => doc.get_dictionary(*r).ok()?.clone(),
        Object::Dictionary(d) => d.clone(),
        _ => return None,
    };
    let p = enc.get(b"P").ok()?.as_i64().ok()?;
    let bit = |n: u32| (p >> (n - 1)) & 1 == 1;
    let cf = enc.get(b"CF").is_ok();
    Some(SecurityInfo {
        revision: enc.get(b"R").ok().and_then(|r| r.as_i64().ok()).unwrap_or(0),
        aes: cf,
        allow: Security { user_password: String::new(), owner_password: String::new(), allow_print: bit(3), allow_print_hq: bit(12), allow_copy: bit(5), allow_modify: bit(4), allow_annotate: bit(6), allow_fill: bit(9), allow_assemble: bit(11) },
    })
}

// ========================================================================== bookmarks

#[derive(Clone, Debug, PartialEq)]
pub struct Bm {
    pub title: String,
    pub page: usize,
    pub children: Vec<Bm>,
}

fn dest_page(doc: &Document, d: &Object, pages: &[ObjectId], depth: usize) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    match d {
        Object::Array(a) => match a.first()? {
            Object::Reference(r) => pages.iter().position(|p| p == r),
            Object::Integer(i) => Some(*i as usize),
            _ => None,
        },
        Object::Reference(r) => dest_page(doc, doc.get_object(*r).ok()?, pages, depth + 1),
        Object::Dictionary(dd) => dest_page(doc, dd.get(b"D").ok()?, pages, depth + 1),
        Object::String(..) | Object::Name(_) => {
            // named destination: /Dests of the catalog or /Names /Dests tree
            let key = text_of(d);
            let cat = doc.get_dictionary(catalog_id(doc).ok()?).ok()?;
            if let Ok(Object::Dictionary(dests)) = cat.get(b"Dests").map(|o| match o {
                Object::Reference(r) => doc.get_object(*r).cloned().unwrap_or(Object::Null),
                x => x.clone(),
            }) {
                if let Ok(v) = dests.get(key.as_bytes()) {
                    return dest_page(doc, v, pages, depth + 1);
                }
            }
            let names = cat.get(b"Names").ok().and_then(|n| match n {
                Object::Reference(r) => doc.get_dictionary(*r).ok().cloned(),
                Object::Dictionary(d) => Some(d.clone()),
                _ => None,
            })?;
            let tree = match names.get(b"Dests").ok()? {
                Object::Reference(r) => doc.get_dictionary(*r).ok()?.clone(),
                Object::Dictionary(d) => d.clone(),
                _ => return None,
            };
            find_in_name_tree(doc, &tree, &key).and_then(|v| dest_page(doc, &v, pages, depth + 1))
        }
        _ => None,
    }
}

fn find_in_name_tree(doc: &Document, node: &Dictionary, key: &str) -> Option<Object> {
    if let Ok(Object::Array(names)) = node.get(b"Names") {
        for pair in names.chunks(2) {
            if pair.len() == 2 && text_of(&pair[0]) == key {
                return Some(pair[1].clone());
            }
        }
    }
    if let Ok(Object::Array(kids)) = node.get(b"Kids") {
        for k in kids {
            if let Ok(r) = k.as_reference() {
                if let Ok(kd) = doc.get_dictionary(r) {
                    if let Some(v) = find_in_name_tree(doc, kd, key) {
                        return Some(v);
                    }
                }
            }
        }
    }
    None
}

fn read_outline_level(doc: &Document, first: Option<ObjectId>, pages: &[ObjectId], depth: usize) -> Vec<Bm> {
    let mut out = vec![];
    let mut cur = first;
    let mut guard = 0;
    while let Some(id) = cur {
        guard += 1;
        if guard > 5000 || depth > 32 {
            break;
        }
        let Ok(d) = doc.get_dictionary(id) else { break };
        let title = d.get(b"Title").ok().map(text_of).unwrap_or_default();
        let dest = d.get(b"Dest").ok().cloned().or_else(|| {
            d.get(b"A").ok().and_then(|a| match a {
                Object::Reference(r) => doc.get_dictionary(*r).ok().and_then(|ad| ad.get(b"D").ok().cloned()),
                Object::Dictionary(ad) => ad.get(b"D").ok().cloned(),
                _ => None,
            })
        });
        let page = dest.and_then(|x| dest_page(doc, &x, pages, 0)).unwrap_or(0);
        let kids = read_outline_level(doc, d.get(b"First").ok().and_then(|f| f.as_reference().ok()), pages, depth + 1);
        out.push(Bm { title, page, children: kids });
        cur = d.get(b"Next").ok().and_then(|n| n.as_reference().ok());
    }
    out
}

pub fn read_bookmarks(bytes: &[u8]) -> Vec<Bm> {
    let Ok(doc) = load(bytes) else { return vec![] };
    let pages = page_ids(&doc);
    let Some(cat) = catalog_id(&doc).ok().and_then(|c| doc.get_dictionary(c).ok().cloned()) else { return vec![] };
    let Some(outlines) = cat.get(b"Outlines").ok().and_then(|o| o.as_reference().ok()).and_then(|r| doc.get_dictionary(r).ok().cloned()) else { return vec![] };
    read_outline_level(&doc, outlines.get(b"First").ok().and_then(|f| f.as_reference().ok()), &pages, 0)
}

fn write_level(doc: &mut Document, nodes: &[Bm], parent: ObjectId, pages: &[ObjectId]) -> Option<(ObjectId, ObjectId, i64)> {
    let mut ids: Vec<ObjectId> = vec![];
    let mut total = 0i64;
    for n in nodes {
        let mut d = Dictionary::new();
        d.set("Title", pdf_text(&n.title));
        d.set("Parent", Object::Reference(parent));
        let p = pages.get(n.page.min(pages.len().saturating_sub(1))).copied();
        if let Some(p) = p {
            d.set("Dest", Object::Array(vec![Object::Reference(p), name("Fit")]));
        }
        let id = doc.add_object(Object::Dictionary(d));
        total += 1;
        if let Some((first, last, count)) = write_level(doc, &n.children, id, pages) {
            let dd = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()).ok()?;
            dd.set("First", Object::Reference(first));
            dd.set("Last", Object::Reference(last));
            dd.set("Count", Object::Integer(count));
            total += count;
        }
        ids.push(id);
    }
    for (i, id) in ids.iter().enumerate() {
        let d = doc.get_object_mut(*id).and_then(|o| o.as_dict_mut()).ok()?;
        if i > 0 {
            d.set("Prev", Object::Reference(ids[i - 1]));
        }
        if i + 1 < ids.len() {
            d.set("Next", Object::Reference(ids[i + 1]));
        }
    }
    Some((*ids.first()?, *ids.last()?, total))
}

pub fn write_bookmarks(bytes: &[u8], nodes: &[Bm]) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    let pages = page_ids(&doc);
    let cat = catalog_id(&doc)?;
    if nodes.is_empty() {
        doc.get_object_mut(cat).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.remove(b"Outlines");
        doc.prune_objects();
        return save(&mut doc);
    }
    let root = doc.add_object(Object::Dictionary(Dictionary::new()));
    let (first, last, count) = write_level(&mut doc, nodes, root, &pages).ok_or("invalid bookmarks")?;
    let rd = doc.get_object_mut(root).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
    rd.set("Type", name("Outlines"));
    rd.set("First", Object::Reference(first));
    rd.set("Last", Object::Reference(last));
    rd.set("Count", Object::Integer(count));
    doc.get_object_mut(cat).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("Outlines", Object::Reference(root));
    doc.prune_objects();
    save(&mut doc)
}

// ============================================================================ links

#[derive(Clone, Debug, PartialEq)]
pub enum LinkDest {
    Uri(String),
    Page(usize),
}

pub fn add_link(bytes: &[u8], page: usize, rect: R4, dest: &LinkDest) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    let pid = page_id(&doc, page)?;
    let pages = page_ids(&doc);
    let mut d = Dictionary::new();
    d.set("Type", name("Annot"));
    d.set("Subtype", name("Link"));
    d.set("Rect", Object::Array(vec![f(rect[0]), f(rect[1]), f(rect[2]), f(rect[3])]));
    d.set("Border", Object::Array(vec![Object::Integer(0), Object::Integer(0), Object::Integer(0)]));
    d.set("F", Object::Integer(4));
    d.set("P", Object::Reference(pid));
    match dest {
        LinkDest::Uri(u) => {
            let mut a = Dictionary::new();
            a.set("S", name("URI"));
            a.set("URI", Object::String(u.as_bytes().to_vec(), StringFormat::Literal));
            d.set("A", Object::Dictionary(a));
        }
        LinkDest::Page(p) => {
            let target = *pages.get(*p).ok_or("destination page does not exist")?;
            d.set("Dest", Object::Array(vec![Object::Reference(target), name("Fit")]));
        }
    }
    let id = doc.add_object(Object::Dictionary(d));
    let mut arr = annots_array(&doc, pid);
    arr.push(Object::Reference(id));
    set_annots_array(&mut doc, pid, arr)?;
    save(&mut doc)
}

// ========================================================================== layers

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Object number of the optional content group.
    pub id: u32,
    pub name: String,
    pub visible: bool,
}

fn oc_config(doc: &Document) -> Option<(Vec<ObjectId>, Dictionary)> {
    let cat = doc.get_dictionary(catalog_id(doc).ok()?).ok()?;
    let ocp = match cat.get(b"OCProperties").ok()? {
        Object::Reference(r) => doc.get_dictionary(*r).ok()?.clone(),
        Object::Dictionary(d) => d.clone(),
        _ => return None,
    };
    let ocgs: Vec<ObjectId> = ocp.get(b"OCGs").ok()?.as_array().ok()?.iter().filter_map(|o| o.as_reference().ok()).collect();
    let d = match ocp.get(b"D").ok()? {
        Object::Reference(r) => doc.get_dictionary(*r).ok()?.clone(),
        Object::Dictionary(d) => d.clone(),
        _ => Dictionary::new(),
    };
    Some((ocgs, d))
}

pub fn list_layers(bytes: &[u8]) -> Vec<Layer> {
    let Ok(doc) = load(bytes) else { return vec![] };
    let Some((ocgs, d)) = oc_config(&doc) else { return vec![] };
    let off: Vec<ObjectId> = d.get(b"OFF").ok().and_then(|a| a.as_array().ok()).map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect()).unwrap_or_default();
    let base_off = d.get(b"BaseState").ok().map(text_of).as_deref() == Some("OFF");
    let on: Vec<ObjectId> = d.get(b"ON").ok().and_then(|a| a.as_array().ok()).map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect()).unwrap_or_default();
    ocgs.iter()
        .map(|id| Layer {
            id: id.0,
            name: doc.get_dictionary(*id).ok().and_then(|g| g.get(b"Name").ok().map(text_of)).unwrap_or_else(|| format!("Layer {}", id.0)),
            visible: if base_off { on.contains(id) } else { !off.contains(id) },
        })
        .collect()
}

pub fn set_layer_visible(bytes: &[u8], layer_id: u32, visible: bool) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    let cat = catalog_id(&doc)?;
    let ocp_ref = doc.get_dictionary(cat).ok().and_then(|c| c.get(b"OCProperties").ok().cloned());
    let target: ObjectId = match &ocp_ref {
        Some(Object::Reference(r)) => *r,
        _ => {
            // OCProperties directly in the catalog: move it into an indirect object
            let Some(Object::Dictionary(d)) = ocp_ref else { return Err("no layers".into()) };
            let id = doc.add_object(Object::Dictionary(d));
            doc.get_object_mut(cat).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("OCProperties", Object::Reference(id));
            id
        }
    };
    let gid = doc
        .get_dictionary(target)
        .ok()
        .and_then(|d| d.get(b"OCGs").ok().and_then(|a| a.as_array().ok().cloned()))
        .and_then(|a| a.iter().filter_map(|o| o.as_reference().ok()).find(|r| r.0 == layer_id))
        .ok_or("layer not found")?;
    let ocp = doc.get_object_mut(target).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
    let mut d = match ocp.get(b"D") {
        Ok(Object::Dictionary(d)) => d.clone(),
        _ => Dictionary::new(),
    };
    let base_off = d.get(b"BaseState").ok().map(text_of).as_deref() == Some("OFF");
    let (list_key, add) = if base_off { ("ON", visible) } else { ("OFF", !visible) };
    let mut list: Vec<Object> = d.get(list_key.as_bytes()).ok().and_then(|a| a.as_array().ok().cloned()).unwrap_or_default();
    list.retain(|o| o.as_reference().ok() != Some(gid));
    if add {
        list.push(Object::Reference(gid));
    }
    d.set(list_key, Object::Array(list));
    ocp.set("D", Object::Dictionary(d));
    save(&mut doc)
}

// ======================================================================== watermark

pub enum WatermarkKind {
    Text(String),
    Image(image::DynamicImage),
}

pub struct WatermarkSpec {
    pub kind: WatermarkKind,
    pub pages: Vec<usize>,
    pub opacity: f32,
    pub rotation_deg: f32,
    /// Text: font size; image: width in points.
    pub size: f32,
    pub color: [u8; 3],
    pub behind: bool,
}

const HELV_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778,
    722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Width of a text in Helvetica (points).
pub fn helvetica_width(s: &str, size: f32) -> f32 {
    s.chars()
        .map(|c| {
            let u = c as u32;
            if (32..127).contains(&u) { HELV_WIDTHS[(u - 32) as usize] as f32 } else { 556.0 }
        })
        .sum::<f32>()
        * size
        / 1000.0
}

/// Text as WinAnsi bytes (escaped for a PDF literal string).
fn winansi_literal(s: &str) -> Vec<u8> {
    let mut out = vec![b'('];
    for c in s.chars() {
        let b = match c {
            '€' => 0x80,
            c if (c as u32) < 256 => c as u32 as u8,
            _ => b'?',
        };
        match b {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            _ => out.push(b),
        }
    }
    out.push(b')');
    out
}

fn page_box(doc: &Document, pid: ObjectId) -> (R4, i64) {
    let d = doc.get_dictionary(pid).ok();
    let arr = d
        .and_then(|d| d.get(b"CropBox").ok().or_else(|| d.get(b"MediaBox").ok()))
        .and_then(|o| o.as_array().ok())
        .map(|a| a.iter().filter_map(|x| x.as_float().ok()).collect::<Vec<f32>>())
        .filter(|v| v.len() == 4)
        .unwrap_or_else(|| vec![0.0, 0.0, 595.0, 842.0]);
    let rot = d.and_then(|d| d.get(b"Rotate").ok().and_then(|r| r.as_i64().ok())).unwrap_or(0);
    ([arr[0].min(arr[2]), arr[1].min(arr[3]), arr[0].max(arr[2]), arr[1].max(arr[3])], rot)
}

/// Edits the /Resources dictionary of a page (direct or indirect).
fn edit_resources<F: FnOnce(&mut Dictionary)>(doc: &mut Document, pid: ObjectId, fun: F) -> Result<(), String> {
    let res = doc.get_dictionary(pid).ok().and_then(|d| d.get(b"Resources").ok().cloned());
    match res {
        Some(Object::Reference(r)) => {
            let d = doc.get_object_mut(r).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
            fun(d);
        }
        Some(Object::Dictionary(mut d)) => {
            fun(&mut d);
            doc.get_object_mut(pid).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("Resources", Object::Dictionary(d));
        }
        _ => {
            let mut d = Dictionary::new();
            fun(&mut d);
            doc.get_object_mut(pid).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("Resources", Object::Dictionary(d));
        }
    }
    Ok(())
}

fn sub_dict(d: &mut Dictionary, key: &str) -> Dictionary {
    match d.get(key.as_bytes()) {
        Ok(Object::Dictionary(x)) => x.clone(),
        _ => Dictionary::new(),
    }
}

pub fn add_watermark(bytes: &[u8], spec: &WatermarkSpec) -> Result<Vec<u8>, String> {
    let mut doc = load(bytes)?;
    flatten_pages(&mut doc)?;
    let ids = page_ids(&doc);
    // shared Helvetica font
    let mut fd = Dictionary::new();
    fd.set("Type", name("Font"));
    fd.set("Subtype", name("Type1"));
    fd.set("BaseFont", name("Helvetica"));
    fd.set("Encoding", name("WinAnsiEncoding"));
    let font_id = doc.add_object(Object::Dictionary(fd));
    // shared image
    let image_id = match &spec.kind {
        WatermarkKind::Image(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            let mut rgb = Vec::with_capacity((w * h * 3) as usize);
            let mut alpha = Vec::with_capacity((w * h) as usize);
            for p in rgba.pixels() {
                rgb.extend_from_slice(&p.0[..3]);
                alpha.push(p.0[3]);
            }
            let mut sm = Dictionary::new();
            sm.set("Type", name("XObject"));
            sm.set("Subtype", name("Image"));
            sm.set("Width", Object::Integer(w as i64));
            sm.set("Height", Object::Integer(h as i64));
            sm.set("ColorSpace", name("DeviceGray"));
            sm.set("BitsPerComponent", Object::Integer(8));
            let mut sms = Stream::new(sm, alpha);
            let _ = sms.compress();
            let sm_id = doc.add_object(sms);
            let mut d = Dictionary::new();
            d.set("Type", name("XObject"));
            d.set("Subtype", name("Image"));
            d.set("Width", Object::Integer(w as i64));
            d.set("Height", Object::Integer(h as i64));
            d.set("ColorSpace", name("DeviceRGB"));
            d.set("BitsPerComponent", Object::Integer(8));
            d.set("SMask", Object::Reference(sm_id));
            let mut s = Stream::new(d, rgb);
            let _ = s.compress();
            Some((doc.add_object(s), w as f32, h as f32))
        }
        _ => None,
    };
    for &pi in &spec.pages {
        let pid = *ids.get(pi).ok_or("page does not exist")?;
        let (bx, rot) = page_box(&doc, pid);
        let (cx, cy) = ((bx[0] + bx[2]) / 2.0, (bx[1] + bx[3]) / 2.0);
        let ang = (spec.rotation_deg - rot as f32).to_radians();
        let (s, c) = ang.sin_cos();
        let mut content = format!("q /WMGS gs 1 0 0 1 {cx:.2} {cy:.2} cm {c:.5} {s:.5} {:.5} {c:.5} 0 0 cm\n", -s);
        match &spec.kind {
            WatermarkKind::Text(t) => {
                let w = helvetica_width(t, spec.size);
                content.push_str(&format!("{:.3} {:.3} {:.3} rg BT /WMF {:.2} Tf {:.2} {:.2} Td ", spec.color[0] as f32 / 255.0, spec.color[1] as f32 / 255.0, spec.color[2] as f32 / 255.0, spec.size, -w / 2.0, -spec.size * 0.35));
                let mut b = content.into_bytes();
                b.extend(winansi_literal(t));
                b.extend_from_slice(b" Tj ET Q\n");
                content = String::new();
                add_wm_stream(&mut doc, pid, b, font_id, None, spec)?;
            }
            WatermarkKind::Image(_) => {
                let (iid, iw, ih) = image_id.ok_or("image missing")?;
                let (w, h) = (spec.size, spec.size * ih / iw);
                content.push_str(&format!("{w:.2} 0 0 {h:.2} {:.2} {:.2} cm /WMI Do Q\n", -w / 2.0, -h / 2.0));
                add_wm_stream(&mut doc, pid, content.into_bytes(), font_id, Some(iid), spec)?;
            }
        }
    }
    save(&mut doc)
}

fn add_wm_stream(doc: &mut Document, pid: ObjectId, content: Vec<u8>, font_id: ObjectId, image: Option<ObjectId>, spec: &WatermarkSpec) -> Result<(), String> {
    let sid = doc.add_object(Stream::new(Dictionary::new(), content));
    let opacity = spec.opacity.clamp(0.0, 1.0);
    edit_resources(doc, pid, |res| {
        let mut gs = sub_dict(res, "ExtGState");
        let mut g = Dictionary::new();
        g.set("Type", name("ExtGState"));
        g.set("ca", f(opacity));
        g.set("CA", f(opacity));
        gs.set("WMGS", Object::Dictionary(g));
        res.set("ExtGState", Object::Dictionary(gs));
        let mut fonts = sub_dict(res, "Font");
        fonts.set("WMF", Object::Reference(font_id));
        res.set("Font", Object::Dictionary(fonts));
        if let Some(i) = image {
            let mut xo = sub_dict(res, "XObject");
            xo.set("WMI", Object::Reference(i));
            res.set("XObject", Object::Dictionary(xo));
        }
    })?;
    let existing = doc.get_dictionary(pid).ok().and_then(|d| d.get(b"Contents").ok().cloned());
    let mut cur: Vec<Object> = match existing {
        Some(Object::Reference(r)) => match doc.get_object(r) {
            Ok(Object::Array(a)) => a.clone(),
            _ => vec![Object::Reference(r)],
        },
        Some(Object::Array(a)) => a,
        _ => vec![],
    };
    if spec.behind {
        cur.insert(0, Object::Reference(sid));
    } else {
        cur.push(Object::Reference(sid));
    }
    doc.get_object_mut(pid).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("Contents", Object::Array(cur));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfium_render::prelude::*;

    fn doc_bytes(n: usize) -> Vec<u8> {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        for i in 0..n {
            let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
            let font = doc.fonts_mut().helvetica();
            page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), format!("Page {i}"), font, PdfPoints::new(24.0)).unwrap();
            page.regenerate_content().unwrap();
        }
        doc.save_to_bytes().unwrap()
    }

    #[test]
    fn info_roundtrip_and_clean() {
        let b = doc_bytes(1);
        let info = DocInfo { title: "My title é".into(), author: "Me".into(), subject: "Subject".into(), keywords: "a, b".into(), creator: "Test".into(), producer: "RustPDF".into(), ..Default::default() };
        let b = write_info(&b, &info).unwrap();
        let back = read_info(&b);
        assert_eq!(back.title, "My title é");
        assert_eq!(back.author, "Me");
        let (cleaned, report) = clean(&b, &CleanOpts::default()).unwrap();
        let after = read_info(&cleaned);
        assert!(after.title.is_empty() && after.author.is_empty() && after.creator.is_empty(), "{after:?}");
        assert!(report.iter().any(|r| r.contains("Document information")), "{report:?}");
        // the content stays intact
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(cleaned, None).unwrap();
        assert_eq!(doc.pages().get(0).unwrap().text().unwrap().all().trim(), "Page 0");
    }

    #[test]
    fn encryption_requires_password_and_keeps_content() {
        let b = doc_bytes(1);
        let sec = Security { user_password: "open".into(), owner_password: "master".into(), allow_copy: false, allow_print: false, allow_print_hq: false, allow_modify: false, ..Security::default() };
        assert!(sec.is_active());
        let enc = encrypt(&b, &sec).unwrap();
        assert!(!enc.windows(10).any(|w| w == b"Page 0 Tj "), "the content is no longer readable");
        let p = crate::pdfx::test_pdfium();
        assert!(matches!(crate::pdfx::open_bytes(p, enc.clone(), None), Err(crate::pdfx::OpenErr::Password)));
        assert!(matches!(crate::pdfx::open_bytes(p, enc.clone(), Some("wrong")), Err(crate::pdfx::OpenErr::Password)));
        let doc = crate::pdfx::open_bytes(p, enc.clone(), Some("open")).unwrap();
        assert_eq!(doc.pages().get(0).unwrap().text().unwrap().all().trim(), "Page 0");
        let info = read_security(&enc).expect("encrypted file");
        assert_eq!(info.revision, 6);
        assert!(!info.allow.allow_modify && !info.allow.allow_copy && !info.allow.allow_print && !info.allow.allow_print_hq, "{info:?}");
        assert!(info.allow.allow_annotate && info.allow.allow_fill && info.allow.allow_assemble, "{info:?}");
        assert!(read_security(&b).is_none(), "unencrypted document");
        // owner password: all the rights
        let doc = crate::pdfx::open_bytes(p, enc, Some("master")).unwrap();
        assert_eq!(doc.pages().len(), 1);
    }

    #[test]
    fn bookmarks_roundtrip() {
        let b = doc_bytes(3);
        let nodes = vec![
            Bm { title: "Introduction".into(), page: 0, children: vec![Bm { title: "Context é".into(), page: 1, children: vec![] }] },
            Bm { title: "End".into(), page: 2, children: vec![] },
        ];
        let out = write_bookmarks(&b, &nodes).unwrap();
        assert_eq!(read_bookmarks(&out), nodes);
        // readable by PDFium
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(out.clone(), None).unwrap();
        let first = doc.bookmarks().root().unwrap();
        assert_eq!(first.title().as_deref(), Some("Introduction"));
        // clear
        let out = write_bookmarks(&out, &[]).unwrap();
        assert!(read_bookmarks(&out).is_empty());
    }

    #[test]
    fn links_are_read_by_pdfium() {
        let b = doc_bytes(2);
        let b = add_link(&b, 0, [72.0, 690.0, 200.0, 720.0], &LinkDest::Uri("https://example.org".into())).unwrap();
        let b = add_link(&b, 0, [72.0, 600.0, 200.0, 630.0], &LinkDest::Page(1)).unwrap();
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(b, None).unwrap();
        let mut t = crate::tab::Tab::new(p, doc, None, None, 1.0);
        let info = t.info(0).unwrap();
        assert!(info.links.iter().any(|l| l.target == crate::model::LinkTarget::Uri("https://example.org".into())), "{:?}", info.links);
        assert!(info.links.iter().any(|l| l.target == crate::model::LinkTarget::Page(1)), "{:?}", info.links);
    }

    #[test]
    fn layers_toggle() {
        let b = doc_bytes(1);
        // create a layer by hand
        let mut doc = Document::load_mem(&b).unwrap();
        let g = doc.add_object(Object::Dictionary({
            let mut d = Dictionary::new();
            d.set("Type", name("OCG"));
            d.set("Name", pdf_text("Annotations"));
            d
        }));
        let mut d = Dictionary::new();
        d.set("Order", Object::Array(vec![Object::Reference(g)]));
        let mut ocp = Dictionary::new();
        ocp.set("OCGs", Object::Array(vec![Object::Reference(g)]));
        ocp.set("D", Object::Dictionary(d));
        let cat = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        doc.get_object_mut(cat).unwrap().as_dict_mut().unwrap().set("OCProperties", Object::Dictionary(ocp));
        let mut out = vec![];
        doc.save_to(&mut out).unwrap();
        let layers = list_layers(&out);
        assert_eq!(layers.len(), 1);
        assert!(layers[0].visible);
        let off = set_layer_visible(&out, layers[0].id, false).unwrap();
        assert!(!list_layers(&off)[0].visible);
        let on = set_layer_visible(&off, layers[0].id, true).unwrap();
        assert!(list_layers(&on)[0].visible);
    }

    #[test]
    fn watermark_text_and_image_are_drawn() {
        let b = doc_bytes(2);
        let spec = WatermarkSpec { kind: WatermarkKind::Text("CONFIDENTIAL".into()), pages: vec![0], opacity: 0.5, rotation_deg: 45.0, size: 80.0, color: [255, 0, 0], behind: false };
        let out = add_watermark(&b, &spec).unwrap();
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(out.clone(), None).unwrap();
        let red = |page: usize| {
            let pg = doc.pages().get(page as i32).unwrap();
            let img = crate::pdfx::render_full(&pg, 595, false).unwrap();
            img.data.chunks_exact(4).filter(|p| p[0] as i32 > p[1] as i32 + 60 && p[0] as i32 > p[2] as i32 + 60).count()
        };
        assert!(red(0) > 500, "red watermark visible on page 0: {}", red(0));
        assert_eq!(red(1), 0, "no watermark on page 1");
        // image
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(20, 20, image::Rgba([0, 0, 255, 255])));
        let spec = WatermarkSpec { kind: WatermarkKind::Image(img), pages: vec![1], opacity: 1.0, rotation_deg: 0.0, size: 200.0, color: [0, 0, 0], behind: true };
        let out2 = add_watermark(&out, &spec).unwrap();
        let doc2 = p.load_pdf_from_byte_vec(out2, None).unwrap();
        let pg = doc2.pages().get(1).unwrap();
        let im = crate::pdfx::render_full(&pg, 595, false).unwrap();
        let blue = im.data.chunks_exact(4).filter(|p| p[2] > 200 && p[0] < 60).count();
        assert!(blue > 5000, "blue image: {blue}");
        assert!((helvetica_width("HHH", 10.0) - 21.66).abs() < 0.01);
    }
}

