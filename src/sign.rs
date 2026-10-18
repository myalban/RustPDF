//! Digital signatures of PDFs: PKCS#12 certificate, CMS signature (RSA / SHA-256) by incremental update,
//! verification (digest, signature, document modified after signing).

use crate::geom::R4;
use crate::pdfedit_annot::{page_id, pdf_text};
use cms::builder::{SignedDataBuilder, SignerInfoBuilder, create_signing_time_attribute};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::ContentInfo;
use cms::signed_data::{EncapsulatedContentInfo, SignedData, SignerIdentifier};
use der::{Decode, Encode};
use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, StringFormat};
use rsa::pkcs1v15::{Signature, SigningKey, VerifyingKey};
use rsa::pkcs8::DecodePrivateKey;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};
use signature::{Keypair, Verifier};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

/// Space reserved (bytes) for the CMS signature in /Contents.
const SIG_RESERVED: usize = 12_000;

/// A user certificate with its private key.
pub struct Identity {
    key: RsaPrivateKey,
    cert: Certificate,
    chain: Vec<Certificate>,
}

fn rdn_string(n: &x509_cert::name::Name) -> String {
    n.to_string()
}

/// Value of a field (CN, O…) in an X.500 name displayed as "CN=…,O=…".
pub fn name_field(name: &str, key: &str) -> Option<String> {
    name.split(',').map(str::trim).find_map(|p| p.strip_prefix(&format!("{key}=")).map(|v| v.to_string()))
}

impl Identity {
    pub fn subject(&self) -> String {
        rdn_string(&self.cert.tbs_certificate.subject)
    }
    pub fn issuer(&self) -> String {
        rdn_string(&self.cert.tbs_certificate.issuer)
    }
    pub fn common_name(&self) -> String {
        name_field(&self.subject(), "CN").unwrap_or_else(|| self.subject())
    }
    pub fn not_after(&self) -> String {
        self.cert.tbs_certificate.validity.not_after.to_string()
    }
    pub fn is_self_signed(&self) -> bool {
        self.subject() == self.issuer()
    }

    /// Loads a PKCS#12 file (.pfx / .p12).
    pub fn from_pkcs12(data: &[u8], password: &str) -> Result<Identity, String> {
        let ks = p12_keystore::KeyStore::from_pkcs12(data, password, p12_keystore::Pkcs12ImportPolicy::Relaxed).map_err(|e| format!("unreadable certificate or wrong password ({e})"))?;
        let (_, chain) = ks.private_key_chain().ok_or("the file contains no private key")?;
        let key = RsaPrivateKey::from_pkcs8_der(chain.key().as_der()).map_err(|_| "only RSA keys are supported".to_string())?;
        let certs: Vec<Certificate> = chain.certs().iter().filter_map(|c| Certificate::from_der(c.as_der()).ok()).collect();
        let first = certs.first().cloned().ok_or("no certificate in the file")?;
        // the "leaf" certificate is the one whose public key matches the private key
        let pubkey = RsaPublicKey::from(&key);
        let leaf = certs.iter().find(|c| cert_public_key(c).map(|k| k == pubkey).unwrap_or(false)).cloned().unwrap_or(first);
        Ok(Identity { key, cert: leaf.clone(), chain: certs.into_iter().filter(|c| c != &leaf).collect() })
    }

    /// Exports the identity as PKCS#12.
    pub fn to_pkcs12(&self, password: &str) -> Result<Vec<u8>, String> {
        use rsa::pkcs8::EncodePrivateKey;
        let der = self.key.to_pkcs8_der().map_err(|e| e.to_string())?;
        let key = p12_keystore::PrivateKey::from_der(der.as_bytes()).map_err(|e| e.to_string())?;
        let mut certs = vec![p12_keystore::Certificate::from_der(&self.cert.to_der().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?];
        for c in &self.chain {
            certs.push(p12_keystore::Certificate::from_der(&c.to_der().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?);
        }
        let mut ks = p12_keystore::KeyStore::new();
        ks.add_entry("signature", p12_keystore::KeyStoreEntry::PrivateKeyChain(p12_keystore::PrivateKeyChain::new(b"rustpdf".to_vec(), key, certs)));
        ks.writer(password).write().map_err(|e| e.to_string())
    }

    /// Creates a self-signed certificate (RSA).
    pub fn create_self_signed(common_name: &str, organization: &str, email: &str, years: u32, bits: usize) -> Result<Identity, String> {
        use std::str::FromStr;
        use x509_cert::builder::{Builder, CertificateBuilder, Profile};
        use x509_cert::name::Name;
        use x509_cert::serial_number::SerialNumber;
        use x509_cert::time::Validity;
        let mut rng = rand_core::OsRng;
        let key = RsaPrivateKey::new(&mut rng, bits).map_err(|e| e.to_string())?;
        let signer = SigningKey::<Sha256>::new(key.clone());
        let mut dn = format!("CN={}", common_name.replace(',', " "));
        if !organization.trim().is_empty() {
            dn.push_str(&format!(",O={}", organization.replace(',', " ")));
        }
        if !email.trim().is_empty() {
            dn.push_str(&format!(",emailAddress={}", email.replace(',', " ")));
        }
        let subject = Name::from_str(&dn).map_err(|e| e.to_string())?;
        let mut serial = [0u8; 8];
        getrandom::fill(&mut serial).map_err(|e| e.to_string())?;
        serial[0] &= 0x7f;
        let serial = SerialNumber::new(&serial).map_err(|e| e.to_string())?;
        let validity = Validity::from_now(std::time::Duration::from_secs(365 * 24 * 3600 * years.max(1) as u64)).map_err(|e| e.to_string())?;
        let spki = spki::SubjectPublicKeyInfoOwned::from_key(signer.verifying_key()).map_err(|e| e.to_string())?;
        let builder = CertificateBuilder::new(Profile::Root, serial, validity, subject, spki, &signer).map_err(|e| e.to_string())?;
        let cert = builder.build::<Signature>().map_err(|e| e.to_string())?;
        Ok(Identity { key, cert, chain: vec![] })
    }
}

fn cert_public_key(c: &Certificate) -> Option<RsaPublicKey> {
    use spki::DecodePublicKey;
    let der = c.tbs_certificate.subject_public_key_info.to_der().ok()?;
    RsaPublicKey::from_public_key_der(&der).ok()
}

#[derive(Clone, Debug)]
pub struct SignOptions {
    pub reason: String,
    pub location: String,
    pub contact: String,
    pub field_name: String,
    /// Signature visible : (page, rectangle en points).
    pub visible: Option<(usize, R4)>,
}

impl Default for SignOptions {
    fn default() -> Self {
        SignOptions { reason: String::new(), location: String::new(), contact: String::new(), field_name: "Signature1".into(), visible: None }
    }
}

fn name(s: &str) -> Object {
    Object::Name(s.as_bytes().to_vec())
}

fn r(v: f32) -> Object {
    Object::Real(v)
}

fn pdf_date_now() -> String {
    let t = crate::logx::timestamp();
    let d: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    format!("D:{d}Z")
}

/// Looks for `needle` in `hay` starting at `from`.
fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// Appearance of a visible signature.
fn signature_appearance(doc: &mut Document, w: f32, h: f32, lines: &[String]) -> ObjectId {
    let mut font = Dictionary::new();
    font.set("Type", name("Font"));
    font.set("Subtype", name("Type1"));
    font.set("BaseFont", name("Helvetica"));
    font.set("Encoding", name("WinAnsiEncoding"));
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Dictionary(font));
    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));
    let mut content = format!("q 0.96 0.98 1 rg 0.5 0.5 {:.2} {:.2} re f 0.1 0.3 0.6 RG 1 w 0.5 0.5 {:.2} {:.2} re S Q\nBT /F1 8 Tf 0.1 0.2 0.4 rg\n", w - 1.0, h - 1.0, w - 1.0, h - 1.0).into_bytes();
    let mut y = h - 12.0;
    for l in lines {
        content.extend_from_slice(format!("1 0 0 1 5 {y:.1} Tm ").as_bytes());
        let mut lit = vec![b'('];
        for c in l.chars() {
            let b = if (c as u32) < 256 { c as u32 as u8 } else { b'?' };
            if matches!(b, b'(' | b')' | b'\\') {
                lit.push(b'\\');
            }
            lit.push(b);
        }
        lit.push(b')');
        content.extend(lit);
        content.extend_from_slice(b" Tj\n");
        y -= 10.0;
    }
    content.extend_from_slice(b"ET\n");
    let mut d = Dictionary::new();
    d.set("Type", name("XObject"));
    d.set("Subtype", name("Form"));
    d.set("BBox", Object::Array(vec![r(0.0), r(0.0), r(w), r(h)]));
    d.set("Resources", Object::Dictionary(res));
    doc.add_object(Stream::new(d, content))
}

/// Signs a PDF (incremental update: the original content is not modified).
pub fn sign_pdf(bytes: &[u8], id: &Identity, opts: &SignOptions) -> Result<Vec<u8>, String> {
    let prev = Document::load_mem(bytes).map_err(|e| format!("read: {e}"))?;
    if prev.is_encrypted() {
        return Err("signing an encrypted document is not supported: remove the protection first".into());
    }
    let page_index = opts.visible.map(|v| v.0).unwrap_or(0);
    let pid = page_id(&prev, page_index)?;
    let root = prev.trailer.get(b"Root").and_then(|x| x.as_reference()).map_err(|e| e.to_string())?;
    let mut inc = IncrementalDocument::create_from(bytes.to_vec(), prev);

    // signature dictionary (value)
    let mut sig = Dictionary::new();
    sig.set("Type", name("Sig"));
    sig.set("Filter", name("Adobe.PPKLite"));
    sig.set("SubFilter", name("adbe.pkcs7.detached"));
    sig.set("ByteRange", Object::Array(vec![Object::Integer(0), Object::Integer(9_999_999_999), Object::Integer(9_999_999_999), Object::Integer(9_999_999_999)]));
    sig.set("Contents", Object::String(vec![0u8; SIG_RESERVED], StringFormat::Hexadecimal));
    sig.set("M", Object::String(pdf_date_now().into_bytes(), StringFormat::Literal));
    sig.set("Name", pdf_text(&id.common_name()));
    if !opts.reason.is_empty() {
        sig.set("Reason", pdf_text(&opts.reason));
    }
    if !opts.location.is_empty() {
        sig.set("Location", pdf_text(&opts.location));
    }
    if !opts.contact.is_empty() {
        sig.set("ContactInfo", pdf_text(&opts.contact));
    }
    let sig_id = inc.new_document.add_object(Object::Dictionary(sig));

    // field + widget
    let mut w = Dictionary::new();
    w.set("Type", name("Annot"));
    w.set("Subtype", name("Widget"));
    w.set("FT", name("Sig"));
    w.set("T", pdf_text(&opts.field_name));
    w.set("V", Object::Reference(sig_id));
    w.set("F", Object::Integer(132)); // printable + locked
    w.set("P", Object::Reference(pid));
    match opts.visible {
        Some((_, rect)) => {
            w.set("Rect", Object::Array(rect.iter().map(|v| r(*v)).collect()));
            let lines = vec![format!("Digitally signed by {}", id.common_name()), format!("Date: {}", crate::logx::timestamp()), if opts.reason.is_empty() { String::new() } else { format!("Reason: {}", opts.reason) }, if opts.location.is_empty() { String::new() } else { format!("Location: {}", opts.location) }];
            let lines: Vec<String> = lines.into_iter().filter(|l| !l.is_empty()).collect();
            let ap = signature_appearance(&mut inc.new_document, rect[2] - rect[0], rect[3] - rect[1], &lines);
            let mut apd = Dictionary::new();
            apd.set("N", Object::Reference(ap));
            w.set("AP", Object::Dictionary(apd));
        }
        None => {
            w.set("Rect", Object::Array(vec![r(0.0), r(0.0), r(0.0), r(0.0)]));
        }
    }
    let wid = inc.new_document.add_object(Object::Dictionary(w));

    // page: add the widget to /Annots
    inc.opt_clone_object_to_new_document(pid).map_err(|e| e.to_string())?;
    let annots = inc.new_document.get_dictionary(pid).ok().and_then(|d| d.get(b"Annots").ok().cloned());
    match annots {
        Some(Object::Reference(ar)) => {
            inc.opt_clone_object_to_new_document(ar).map_err(|e| e.to_string())?;
            if let Ok(Object::Array(a)) = inc.new_document.get_object_mut(ar) {
                a.push(Object::Reference(wid));
            }
        }
        Some(Object::Array(mut a)) => {
            a.push(Object::Reference(wid));
            inc.new_document.get_object_mut(pid).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("Annots", Object::Array(a));
        }
        _ => {
            inc.new_document.get_object_mut(pid).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("Annots", Object::Array(vec![Object::Reference(wid)]));
        }
    }
    // form: /AcroForm /Fields and /SigFlags
    inc.opt_clone_object_to_new_document(root).map_err(|e| e.to_string())?;
    let af = inc.new_document.get_dictionary(root).ok().and_then(|c| c.get(b"AcroForm").ok().cloned());
    match af {
        Some(Object::Reference(afr)) => {
            inc.opt_clone_object_to_new_document(afr).map_err(|e| e.to_string())?;
            let d = inc.new_document.get_object_mut(afr).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?;
            let mut fields = d.get(b"Fields").ok().and_then(|f| f.as_array().ok().cloned()).unwrap_or_default();
            fields.push(Object::Reference(wid));
            d.set("Fields", Object::Array(fields));
            d.set("SigFlags", Object::Integer(3));
        }
        other => {
            let mut d = match other {
                Some(Object::Dictionary(d)) => d,
                _ => Dictionary::new(),
            };
            let mut fields = d.get(b"Fields").ok().and_then(|f| f.as_array().ok().cloned()).unwrap_or_default();
            fields.push(Object::Reference(wid));
            d.set("Fields", Object::Array(fields));
            d.set("SigFlags", Object::Integer(3));
            inc.new_document.get_object_mut(root).and_then(|o| o.as_dict_mut()).map_err(|e| e.to_string())?.set("AcroForm", Object::Dictionary(d));
        }
    }
    let mut out: Vec<u8> = Vec::new();
    inc.save_to(&mut out).map_err(|e| format!("write: {e}"))?;

    // --- ByteRange and signature
    let start = bytes.len();
    let br_tok = find(&out, b"/ByteRange", start).ok_or("ByteRange not found")?;
    let lb = find(&out, b"[", br_tok).ok_or("malformed ByteRange")?;
    let rb = find(&out, b"]", lb).ok_or("malformed ByteRange")?;
    // /Contents of the signature: hexadecimal string (the page also has a /Contents, but it is a reference)
    let mut c_open = 0;
    let mut from = start;
    while let Some(tok) = find(&out, b"/Contents", from) {
        let mut p = tok + 9;
        while p < out.len() && out[p].is_ascii_whitespace() {
            p += 1;
        }
        if out.get(p) == Some(&b'<') && out.get(p + 1) != Some(&b'<') {
            c_open = p;
            break;
        }
        from = tok + 9;
    }
    if c_open == 0 {
        return Err("Contents not found".into());
    }
    let c_close = find(&out, b">", c_open).ok_or("malformed Contents")?;
    let (hex_start, hex_end) = (c_open + 1, c_close);
    if hex_end - hex_start != SIG_RESERVED * 2 {
        return Err("unexpected signature space".into());
    }
    let (a, b2, c2, d2) = (0usize, c_open, c_close + 1, out.len() - (c_close + 1));
    let br_text = format!("[0 {b2:010} {c2:010} {d2:010}]");
    let old_len = rb + 1 - lb;
    if br_text.len() > old_len {
        return Err("ByteRange too long".into());
    }
    let mut padded = br_text.into_bytes();
    padded.resize(old_len, b' ');
    out[lb..=rb].copy_from_slice(&padded);
    let mut hasher = Sha256::new();
    hasher.update(&out[a..a + b2]);
    hasher.update(&out[c2..c2 + d2]);
    let digest = hasher.finalize();
    let der = cms_detached(id, &digest)?;
    if der.len() * 2 > SIG_RESERVED * 2 {
        return Err("signature too large for the reserved space".into());
    }
    let mut hex = String::with_capacity(SIG_RESERVED * 2);
    for byte in &der {
        hex.push_str(&format!("{byte:02X}"));
    }
    while hex.len() < SIG_RESERVED * 2 {
        hex.push('0');
    }
    out[hex_start..hex_end].copy_from_slice(hex.as_bytes());
    Ok(out)
}

fn cms_detached(id: &Identity, digest: &[u8]) -> Result<Vec<u8>, String> {
    let econtent = EncapsulatedContentInfo { econtent_type: const_oid::db::rfc5911::ID_DATA, econtent: None };
    let signer = SigningKey::<Sha256>::new(id.key.clone());
    let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber { issuer: id.cert.tbs_certificate.issuer.clone(), serial_number: id.cert.tbs_certificate.serial_number.clone() });
    let digest_alg = AlgorithmIdentifierOwned { oid: const_oid::db::rfc5912::ID_SHA_256, parameters: None };
    let mut sib = SignerInfoBuilder::new(&signer, sid, digest_alg.clone(), &econtent, Some(digest)).map_err(|e| e.to_string())?;
    sib.add_signed_attribute(create_signing_time_attribute().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut sdb = SignedDataBuilder::new(&econtent);
    sdb.add_digest_algorithm(digest_alg).map_err(|e| e.to_string())?;
    sdb.add_certificate(CertificateChoices::Certificate(id.cert.clone())).map_err(|e| e.to_string())?;
    for c in &id.chain {
        sdb.add_certificate(CertificateChoices::Certificate(c.clone())).map_err(|e| e.to_string())?;
    }
    sdb.add_signer_info::<SigningKey<Sha256>, Signature>(sib).map_err(|e| e.to_string())?;
    let ci = sdb.build().map_err(|e| e.to_string())?;
    ci.to_der().map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ verification

#[derive(Clone, Debug, Default)]
pub struct SigReport {
    pub field: String,
    pub signer: String,
    pub subject: String,
    pub issuer: String,
    pub signing_time: String,
    pub reason: String,
    pub location: String,
    /// The document digest matches the signed one.
    pub digest_ok: bool,
    /// The cryptographic signature is valid.
    pub signature_ok: bool,
    /// The signature covers the whole file (otherwise: modified after signing).
    pub covers_whole_file: bool,
    pub cert_valid_now: bool,
    pub self_signed: bool,
    pub problem: Option<String>,
}

impl SigReport {
    pub fn is_valid(&self) -> bool {
        self.digest_ok && self.signature_ok && self.covers_whole_file
    }
}

/// Total length of a DER element (header + content); allows ignoring zero padding.
fn der_len(b: &[u8]) -> Option<usize> {
    if b.len() < 2 {
        return None;
    }
    let l = b[1] as usize;
    if l < 0x80 {
        Some(2 + l)
    } else {
        let n = l & 0x7f;
        if n == 0 || n > 4 || b.len() < 2 + n {
            return None;
        }
        let mut len = 0usize;
        for k in 0..n {
            len = (len << 8) | b[2 + k] as usize;
        }
        Some(2 + n + len)
    }
}

fn digest_by_oid(oid: &const_oid::ObjectIdentifier, parts: &[&[u8]]) -> Option<Vec<u8>> {
    use sha2::{Sha384, Sha512};
    macro_rules! run {
        ($h:ty) => {{
            let mut h = <$h>::new();
            for p in parts {
                h.update(p);
            }
            Some(h.finalize().to_vec())
        }};
    }
    if *oid == const_oid::db::rfc5912::ID_SHA_256 {
        run!(Sha256)
    } else if *oid == const_oid::db::rfc5912::ID_SHA_1 {
        run!(sha1::Sha1)
    } else if *oid == const_oid::db::rfc5912::ID_SHA_384 {
        run!(Sha384)
    } else if *oid == const_oid::db::rfc5912::ID_SHA_512 {
        run!(Sha512)
    } else {
        None
    }
}

fn verify_rsa(oid: &const_oid::ObjectIdentifier, pk: &RsaPublicKey, msg: &[u8], sig: &[u8]) -> Result<(), String> {
    use sha2::{Sha384, Sha512};
    let s = Signature::try_from(sig).map_err(|e| e.to_string())?;
    let r = if *oid == const_oid::db::rfc5912::ID_SHA_256 {
        VerifyingKey::<Sha256>::new(pk.clone()).verify(msg, &s)
    } else if *oid == const_oid::db::rfc5912::ID_SHA_1 {
        VerifyingKey::<sha1::Sha1>::new(pk.clone()).verify(msg, &s)
    } else if *oid == const_oid::db::rfc5912::ID_SHA_384 {
        VerifyingKey::<Sha384>::new(pk.clone()).verify(msg, &s)
    } else if *oid == const_oid::db::rfc5912::ID_SHA_512 {
        VerifyingKey::<Sha512>::new(pk.clone()).verify(msg, &s)
    } else {
        return Err("unsupported hash algorithm".into());
    };
    r.map_err(|e| e.to_string())
}

fn utc_to_string(t: &x509_cert::time::Time) -> String {
    t.to_string()
}

/// Verifies all the signatures of a PDF file.
pub fn verify_pdf(bytes: &[u8]) -> Vec<SigReport> {
    let Ok(doc) = Document::load_mem(bytes) else { return vec![] };
    let mut out = vec![];
    // signature fields: dictionaries containing /ByteRange and /Contents
    let mut sigs: Vec<(String, Dictionary)> = vec![];
    for (_, obj) in doc.objects.iter() {
        if let Ok(d) = obj.as_dict() {
            if d.has(b"ByteRange") && d.has(b"Contents") {
                sigs.push((String::new(), d.clone()));
            }
        }
    }
    // field names (via /V of the widgets)
    for (_, obj) in doc.objects.iter() {
        if let Ok(d) = obj.as_dict() {
            if let (Ok(Object::Reference(v)), Ok(t)) = (d.get(b"V"), d.get(b"T")) {
                if let Ok(vd) = doc.get_dictionary(*v) {
                    if vd.has(b"ByteRange") {
                        for s in sigs.iter_mut() {
                            if s.1 == *vd {
                                s.0 = crate::pdfedit_annot::text_of(t);
                            }
                        }
                    }
                }
            }
        }
    }
    for (field, d) in sigs {
        let mut rep = SigReport { field, ..Default::default() };
        rep.reason = d.get(b"Reason").ok().map(crate::pdfedit_annot::text_of).unwrap_or_default();
        rep.location = d.get(b"Location").ok().map(crate::pdfedit_annot::text_of).unwrap_or_default();
        rep.signer = d.get(b"Name").ok().map(crate::pdfedit_annot::text_of).unwrap_or_default();
        let br: Vec<usize> = d.get(b"ByteRange").ok().and_then(|b| b.as_array().ok()).map(|a| a.iter().filter_map(|x| x.as_i64().ok()).map(|x| x as usize).collect()).unwrap_or_default();
        let contents = match d.get(b"Contents") {
            Ok(Object::String(b, _)) => b.clone(),
            _ => vec![],
        };
        if br.len() != 4 || br[0] != 0 || br[1] + br[3] > bytes.len() + br[2] || br[2] + br[3] > bytes.len() {
            rep.problem = Some("invalid ByteRange".into());
            out.push(rep);
            continue;
        }
        rep.covers_whole_file = br[2] + br[3] == bytes.len() || bytes[br[2] + br[3]..].iter().all(|c| c.is_ascii_whitespace());
        let Some(len) = der_len(&contents) else {
            rep.problem = Some("unreadable signature".into());
            out.push(rep);
            continue;
        };
        let der = &contents[..len.min(contents.len())];
        let res: Result<(), String> = (|| {
            let ci = ContentInfo::from_der(der).map_err(|e| format!("unreadable CMS: {e}"))?;
            let sd: SignedData = ci.content.decode_as().map_err(|e| format!("unreadable SignedData: {e}"))?;
            let si = sd.signer_infos.0.iter().next().ok_or("no signer")?;
            let certs: Vec<Certificate> = sd.certificates.as_ref().map(|cs| cs.0.iter().filter_map(|c| if let CertificateChoices::Certificate(c) = c { Some(c.clone()) } else { None }).collect()).unwrap_or_default();
            let cert = match &si.sid {
                SignerIdentifier::IssuerAndSerialNumber(isn) => certs.iter().find(|c| c.tbs_certificate.issuer == isn.issuer && c.tbs_certificate.serial_number == isn.serial_number),
                _ => certs.first(),
            }
            .ok_or("signer certificate missing from the signature")?;
            rep.subject = rdn_string(&cert.tbs_certificate.subject);
            rep.issuer = rdn_string(&cert.tbs_certificate.issuer);
            if rep.signer.is_empty() {
                rep.signer = name_field(&rep.subject, "CN").unwrap_or_else(|| rep.subject.clone());
            }
            rep.self_signed = rep.subject == rep.issuer;
            let now = x509_cert::time::Time::try_from(std::time::SystemTime::now()).ok();
            rep.cert_valid_now = now.map(|n| n.to_unix_duration() >= cert.tbs_certificate.validity.not_before.to_unix_duration() && n.to_unix_duration() <= cert.tbs_certificate.validity.not_after.to_unix_duration()).unwrap_or(false);
            let _ = utc_to_string;
            // document digest
            let covered = [&bytes[0..br[1]], &bytes[br[2]..br[2] + br[3]]];
            let computed = digest_by_oid(&si.digest_alg.oid, &covered).ok_or("unsupported hash algorithm")?;
            let attrs = si.signed_attrs.as_ref().ok_or("signature without signed attributes (not supported)")?;
            let md_oid = const_oid::db::rfc5911::ID_MESSAGE_DIGEST;
            let md = attrs.iter().find(|a| a.oid == md_oid).and_then(|a| a.values.iter().next()).and_then(|v| v.decode_as::<der::asn1::OctetString>().ok()).ok_or("digest missing")?;
            rep.digest_ok = md.as_bytes() == computed.as_slice();
            if let Some(t) = attrs.iter().find(|a| a.oid == const_oid::db::rfc5911::ID_SIGNING_TIME).and_then(|a| a.values.iter().next()) {
                if let Some(t) = t.to_der().ok().and_then(|d| x509_cert::time::Time::from_der(&d).ok()) {
                    rep.signing_time = t.to_string();
                }
            }
            if rep.signing_time.is_empty() {
                rep.signing_time = d.get(b"M").ok().map(crate::pdfedit_annot::text_of).unwrap_or_default();
            }
            // cryptographic signature
            let signed = attrs.to_der().map_err(|e| e.to_string())?;
            let pk = cert_public_key(cert).ok_or("only RSA signatures are verified")?;
            verify_rsa(&si.digest_alg.oid, &pk, &signed, si.signature.as_bytes())?;
            rep.signature_ok = true;
            Ok(())
        })();
        if let Err(e) = res {
            rep.problem = Some(e);
        }
        out.push(rep);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfium_render::prelude::*;

    fn doc_bytes() -> Vec<u8> {
        let p = crate::pdfx::test_pdfium();
        let mut doc = p.create_new_pdf().unwrap();
        let mut page = doc.pages_mut().create_page_at_end(PdfPagePaperSize::a4()).unwrap();
        let font = doc.fonts_mut().helvetica();
        page.objects_mut().create_text_object(PdfPoints::new(72.0), PdfPoints::new(700.0), "Contract to sign", font, PdfPoints::new(20.0)).unwrap();
        page.regenerate_content().unwrap();
        drop(page);
        doc.save_to_bytes().unwrap()
    }

    fn identity() -> Identity {
        // 1024 bits: enough for the tests (the real generation uses 2048)
        Identity::create_self_signed("Jean Dupont", "My Company", "jean@example.org", 2, 1024).unwrap()
    }

    #[test]
    fn sign_then_verify_valid_invisible() {
        let id = identity();
        assert!(id.is_self_signed());
        assert_eq!(id.common_name(), "Jean Dupont");
        let b = doc_bytes();
        let signed = sign_pdf(&b, &id, &SignOptions { reason: "Approval".into(), location: "Paris".into(), ..Default::default() }).unwrap();
        assert_eq!(&signed[..b.len()], &b[..], "the original content is not modified (incremental update)");
        let reps = verify_pdf(&signed);
        assert_eq!(reps.len(), 1, "{reps:?}");
        let r = &reps[0];
        assert!(r.is_valid(), "{r:?}");
        assert_eq!(r.signer, "Jean Dupont");
        assert_eq!(r.reason, "Approval");
        assert_eq!(r.location, "Paris");
        assert!(r.self_signed && r.cert_valid_now);
        assert!(!r.signing_time.is_empty());
        // PDFium recognizes the signature and the text stays readable
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(signed.clone(), None).unwrap();
        assert_eq!(doc.signatures().len(), 1);
        assert!(doc.pages().get(0).unwrap().text().unwrap().all().contains("Contract"));
    }

    #[test]
    fn tampering_is_detected() {
        let id = identity();
        let b = doc_bytes();
        let signed = sign_pdf(&b, &id, &SignOptions::default()).unwrap();
        // 1. modification of one byte in the signed part
        let mut t = signed.clone();
        let pos = find(&t, b"Contract", 0).unwrap_or(100);
        let pos = if pos < 50 { 60 } else { pos };
        t[pos] ^= 0x01;
        let r = &verify_pdf(&t)[0];
        assert!(!r.digest_ok && !r.is_valid(), "{r:?}");
        // 2. content added after the signature
        let mut appended = signed.clone();
        appended.extend_from_slice(b"\n% added after signature\n");
        let r = &verify_pdf(&appended)[0];
        assert!(r.digest_ok && r.signature_ok);
        assert!(!r.covers_whole_file && !r.is_valid(), "document modified after signing: {r:?}");
        // 3. corrupted signature
        let mut s3 = signed.clone();
        let br = find(&s3, b"/ByteRange", 0).unwrap();
        let c_tok = (br..).find(|&i| s3[i..].starts_with(b"/Contents") && s3[i + 9..].iter().find(|c| !c.is_ascii_whitespace()) == Some(&b'<')).unwrap();
        let c = find(&s3, b"<", c_tok).unwrap() + 1 + 200;
        s3[c] = if s3[c] == b'A' { b'B' } else { b'A' };
        let r = &verify_pdf(&s3)[0];
        assert!(!r.is_valid(), "{r:?}");
    }

    #[test]
    fn visible_signature_is_drawn_and_pkcs12_roundtrip() {
        let id = identity();
        let b = doc_bytes();
        let opts = SignOptions { visible: Some((0, [72.0, 100.0, 300.0, 160.0])), ..Default::default() };
        let signed = sign_pdf(&b, &id, &opts).unwrap();
        let p = crate::pdfx::test_pdfium();
        let doc = p.load_pdf_from_byte_vec(signed.clone(), None).unwrap();
        let pg = doc.pages().get(0).unwrap();
        let img = crate::pdfx::render_full(&pg, 595, true).unwrap();
        // bluish background of the appearance in the area (≈ y 842-130)
        let px = &img.data[((842 - 130) * img.w + 150) * 4..][..3];
        assert!(px[2] > px[0] && px[2] > 230, "visible appearance: {px:?}");
        assert!(verify_pdf(&signed)[0].is_valid());
        // certificate save / reload
        let p12 = id.to_pkcs12("secret").unwrap();
        assert!(Identity::from_pkcs12(&p12, "wrong").is_err());
        let back = Identity::from_pkcs12(&p12, "secret").unwrap();
        assert_eq!(back.common_name(), "Jean Dupont");
        let signed2 = sign_pdf(&b, &back, &SignOptions::default()).unwrap();
        assert!(verify_pdf(&signed2)[0].is_valid(), "signature with the reloaded certificate");
        // two successive signatures
        let signed3 = sign_pdf(&signed2, &id, &SignOptions { field_name: "Signature2".into(), ..Default::default() }).unwrap();
        let reps = verify_pdf(&signed3);
        assert_eq!(reps.len(), 2, "{reps:?}");
        assert!(reps.iter().all(|r| r.digest_ok && r.signature_ok));
        assert!(reps.iter().filter(|r| r.is_valid()).count() >= 1, "the last signature covers the whole file");
    }
}
