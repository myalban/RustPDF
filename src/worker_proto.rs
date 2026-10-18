//! Messages exchanged between the application and the isolated PDF worker (`rustpdf --pdf-worker`).
//!
//! Frame = `u32` little-endian length + body; the body starts with a one-byte tag. Both sides check every length and
//! index: the worker is the process that parses untrusted PDFs, so the application treats whatever it sends as untrusted.

use std::io::{Read, Write};

/// Largest frame accepted from the worker (a rendered page).
pub const MAX_RESPONSE_FRAME: usize = 192 * 1024 * 1024;
/// Largest frame accepted by the worker (the document itself).
pub const MAX_REQUEST_FRAME: usize = 1100 * 1024 * 1024;
/// Largest bitmap side the worker may produce.
pub const MAX_BITMAP_SIDE: u32 = 6000;
pub const MAX_PAGES: usize = 200_000;
pub const MAX_LINKS_PER_PAGE: usize = 5000;
pub const MAX_TEXT: usize = 4096;

#[derive(Debug, PartialEq, Clone)]
pub enum Req {
    Open { password: String, data: Vec<u8> },
    Render { id: u32, page: u32, width: u32 },
    Links { id: u32, page: u32 },
    Quit,
    /// Diagnostics (`--sandbox-check --selftest`): the worker tries forbidden operations and reports the result.
    SelfTest { probe_file: String, probe_dir: String, port: u16 },
}

#[derive(Debug, PartialEq, Clone)]
pub enum LinkTarget {
    Uri(String),
    Page(u32),
}

/// A link on a page; the rectangle is in view space (points from the top-left corner of the page).
#[derive(Debug, PartialEq, Clone)]
pub struct WLink {
    pub rect: [f32; 4],
    pub target: LinkTarget,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Resp {
    /// The worker is running; `isolation` describes the protections that are active.
    Ready { isolation: String },
    Opened { sizes: Vec<(f32, f32)> },
    NeedPassword { wrong: bool },
    Failed { msg: String },
    Rendered { id: u32, w: u32, h: u32, rgba: Vec<u8> },
    Links { id: u32, items: Vec<WLink> },
    Error { id: u32, msg: String },
    SelfTest { lines: Vec<String> },
}

// ------------------------------------------------------------------------------------------ encoding

#[derive(Default)]
struct Wr(Vec<u8>);

impl Wr {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        let s: String = s.chars().take(MAX_TEXT).collect();
        self.bytes(s.as_bytes());
    }
}

struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).filter(|&e| e <= self.b.len()).ok_or("truncated message")?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        if v.is_finite() { Ok(v) } else { Err("invalid number".into()) }
    }
    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn str(&mut self) -> Result<String, String> {
        let b = self.bytes()?;
        if b.len() > MAX_TEXT * 4 {
            return Err("text too long".into());
        }
        Ok(String::from_utf8_lossy(b).into_owned())
    }
    fn done(&self) -> Result<(), String> {
        if self.i == self.b.len() { Ok(()) } else { Err("trailing bytes".into()) }
    }
}

impl Req {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Wr::default();
        match self {
            Req::Open { password, data } => {
                w.u8(1);
                w.str(password);
                w.bytes(data);
            }
            Req::Render { id, page, width } => {
                w.u8(2);
                w.u32(*id);
                w.u32(*page);
                w.u32(*width);
            }
            Req::Links { id, page } => {
                w.u8(3);
                w.u32(*id);
                w.u32(*page);
            }
            Req::Quit => w.u8(4),
            Req::SelfTest { probe_file, probe_dir, port } => {
                w.u8(5);
                w.str(probe_file);
                w.str(probe_dir);
                w.u32(*port as u32);
            }
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> Result<Req, String> {
        let mut r = Rd { b, i: 0 };
        let m = match r.u8()? {
            1 => Req::Open { password: r.str()?, data: r.bytes()?.to_vec() },
            2 => Req::Render { id: r.u32()?, page: r.u32()?, width: r.u32()? },
            3 => Req::Links { id: r.u32()?, page: r.u32()? },
            4 => Req::Quit,
            5 => Req::SelfTest { probe_file: r.str()?, probe_dir: r.str()?, port: r.u32()?.min(65535) as u16 },
            t => return Err(format!("unknown request {t}")),
        };
        r.done()?;
        Ok(m)
    }
}

impl Resp {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Wr::default();
        match self {
            Resp::Ready { isolation } => {
                w.u8(1);
                w.str(isolation);
            }
            Resp::Opened { sizes } => {
                w.u8(2);
                w.u32(sizes.len() as u32);
                for (a, b) in sizes {
                    w.f32(*a);
                    w.f32(*b);
                }
            }
            Resp::NeedPassword { wrong } => {
                w.u8(3);
                w.u8(*wrong as u8);
            }
            Resp::Failed { msg } => {
                w.u8(4);
                w.str(msg);
            }
            Resp::Rendered { id, w: bw, h, rgba } => {
                w.u8(5);
                w.u32(*id);
                w.u32(*bw);
                w.u32(*h);
                w.bytes(rgba);
            }
            Resp::Links { id, items } => {
                w.u8(6);
                w.u32(*id);
                w.u32(items.len() as u32);
                for l in items {
                    for v in l.rect {
                        w.f32(v);
                    }
                    match &l.target {
                        LinkTarget::Uri(u) => {
                            w.u8(0);
                            w.str(u);
                        }
                        LinkTarget::Page(p) => {
                            w.u8(1);
                            w.u32(*p);
                        }
                    }
                }
            }
            Resp::Error { id, msg } => {
                w.u8(7);
                w.u32(*id);
                w.str(msg);
            }
            Resp::SelfTest { lines } => {
                w.u8(8);
                w.u32(lines.len() as u32);
                for l in lines {
                    w.str(l);
                }
            }
        }
        w.0
    }

    /// Decodes and validates a message coming from the (untrusted) worker.
    pub fn decode(b: &[u8]) -> Result<Resp, String> {
        let mut r = Rd { b, i: 0 };
        let m = match r.u8()? {
            1 => Resp::Ready { isolation: r.str()? },
            2 => {
                let n = r.u32()? as usize;
                if n > MAX_PAGES || n.checked_mul(8).map(|x| x > b.len()).unwrap_or(true) {
                    return Err("invalid page count".into());
                }
                let mut sizes = Vec::with_capacity(n);
                for _ in 0..n {
                    let (a, c) = (r.f32()?, r.f32()?);
                    if !(1.0..=200_000.0).contains(&a) || !(1.0..=200_000.0).contains(&c) {
                        return Err("invalid page size".into());
                    }
                    sizes.push((a, c));
                }
                Resp::Opened { sizes }
            }
            3 => Resp::NeedPassword { wrong: r.u8()? != 0 },
            4 => Resp::Failed { msg: r.str()? },
            5 => {
                let (id, w, h) = (r.u32()?, r.u32()?, r.u32()?);
                let rgba = r.bytes()?.to_vec();
                if w == 0 || h == 0 || w > MAX_BITMAP_SIDE || h > MAX_BITMAP_SIDE || rgba.len() != (w as usize) * (h as usize) * 4 {
                    return Err("invalid bitmap".into());
                }
                Resp::Rendered { id, w, h, rgba }
            }
            6 => {
                let id = r.u32()?;
                let n = r.u32()? as usize;
                if n > MAX_LINKS_PER_PAGE {
                    return Err("too many links".into());
                }
                let mut items = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    let rect = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
                    let target = match r.u8()? {
                        0 => LinkTarget::Uri(r.str()?),
                        1 => LinkTarget::Page(r.u32()?),
                        _ => return Err("invalid link".into()),
                    };
                    items.push(WLink { rect, target });
                }
                Resp::Links { id, items }
            }
            7 => Resp::Error { id: r.u32()?, msg: r.str()? },
            8 => {
                let n = r.u32()? as usize;
                if n > 64 {
                    return Err("too many lines".into());
                }
                let mut lines = Vec::with_capacity(n);
                for _ in 0..n {
                    lines.push(r.str()?);
                }
                Resp::SelfTest { lines }
            }
            t => return Err(format!("unknown response {t}")),
        };
        r.done()?;
        Ok(m)
    }
}

// ------------------------------------------------------------------------------------------ framing

pub fn write_frame(w: &mut impl Write, body: &[u8]) -> std::io::Result<()> {
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// Reads one frame; `Ok(None)` at a clean end of stream.
pub fn read_frame(r: &mut impl Read, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > max {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_roundtrip() {
        for m in [Req::Open { password: "pw é".into(), data: vec![1, 2, 3] }, Req::Render { id: 7, page: 3, width: 800 }, Req::Links { id: 1, page: 0 }, Req::Quit] {
            assert_eq!(Req::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn responses_roundtrip() {
        let items = vec![WLink { rect: [1.0, 2.0, 3.0, 4.0], target: LinkTarget::Uri("https://example.org".into()) }, WLink { rect: [0.0, 0.0, 1.0, 1.0], target: LinkTarget::Page(4) }];
        for m in [
            Resp::Ready { isolation: "test".into() },
            Resp::Opened { sizes: vec![(595.0, 842.0), (612.0, 792.0)] },
            Resp::NeedPassword { wrong: true },
            Resp::Failed { msg: "boom".into() },
            Resp::Rendered { id: 2, w: 2, h: 1, rgba: vec![0; 8] },
            Resp::Links { id: 3, items },
            Resp::Error { id: 9, msg: "x".into() },
        ] {
            assert_eq!(Resp::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn hostile_responses_are_rejected() {
        // bitmap whose size does not match its dimensions
        let mut w = Wr::default();
        w.u8(5);
        w.u32(1);
        w.u32(100);
        w.u32(100);
        w.bytes(&[0; 10]);
        assert!(Resp::decode(&w.0).is_err());
        // enormous bitmap
        let mut w = Wr::default();
        w.u8(5);
        w.u32(1);
        w.u32(60_000);
        w.u32(60_000);
        w.bytes(&[]);
        assert!(Resp::decode(&w.0).is_err());
        // absurd page count
        let mut w = Wr::default();
        w.u8(2);
        w.u32(u32::MAX);
        assert!(Resp::decode(&w.0).is_err());
        // NaN size
        let mut w = Wr::default();
        w.u8(2);
        w.u32(1);
        w.f32(f32::NAN);
        w.f32(10.0);
        assert!(Resp::decode(&w.0).is_err());
        // truncated, trailing bytes, unknown tag, empty
        assert!(Resp::decode(&[5, 1]).is_err());
        let mut ok = Resp::NeedPassword { wrong: false }.encode();
        ok.push(0);
        assert!(Resp::decode(&ok).is_err());
        assert!(Resp::decode(&[99]).is_err());
        assert!(Resp::decode(&[]).is_err());
        // length prefix larger than the limit
        let data = (MAX_RESPONSE_FRAME as u32 + 1).to_le_bytes();
        assert!(read_frame(&mut &data[..], MAX_RESPONSE_FRAME).is_err());
    }

    #[test]
    fn frames_roundtrip_and_detect_end_of_stream() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &Req::Quit.encode()).unwrap();
        write_frame(&mut buf, &Req::Links { id: 1, page: 2 }.encode()).unwrap();
        let mut rd = &buf[..];
        assert_eq!(Req::decode(&read_frame(&mut rd, 1000).unwrap().unwrap()).unwrap(), Req::Quit);
        assert_eq!(Req::decode(&read_frame(&mut rd, 1000).unwrap().unwrap()).unwrap(), Req::Links { id: 1, page: 2 });
        assert!(read_frame(&mut rd, 1000).unwrap().is_none());
    }
}
