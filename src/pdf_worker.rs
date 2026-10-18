//! The isolated PDF worker: `rustpdf --pdf-worker`.
//!
//! It is the only process that parses an untrusted document in "protected view". It reads requests on its standard
//! input, answers on its standard output, never touches the file system by itself and, once started, runs inside the
//! strongest sandbox the operating system offers (see `sandbox`).

use crate::pdfx;
use crate::worker_proto::{LinkTarget, MAX_BITMAP_SIDE, MAX_LINKS_PER_PAGE, MAX_REQUEST_FRAME, Req, Resp, WLink, read_frame, write_frame};
use pdfium_render::prelude::PdfDocument;
use std::io::Write;

fn send(out: &mut impl Write, r: &Resp) -> bool {
    write_frame(out, &r.encode()).is_ok()
}

/// Entry point of the worker process; returns the exit code.
pub fn run() -> i32 {
    crate::sys::harden_process();
    let pdfium = match pdfx::load_pdfium() {
        Ok(p) => p,
        Err(_) => return 2,
    };
    // PDFium is loaded: from here on the process gives up what it no longer needs.
    let isolation = crate::sandbox::lockdown_self();
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    if !send(&mut output, &Resp::Ready { isolation }) {
        return 3;
    }
    let mut doc: Option<PdfDocument<'static>> = None;
    loop {
        let Ok(Some(frame)) = read_frame(&mut input, MAX_REQUEST_FRAME) else { return 0 };
        let Ok(req) = Req::decode(&frame) else { return 4 };
        drop(frame);
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(pdfium, &mut doc, req)));
        match answer {
            Ok(Some(resp)) => {
                if !send(&mut output, &resp) {
                    return 5;
                }
            }
            Ok(None) => return 0, // Quit
            Err(_) => {
                let _ = send(&mut output, &Resp::Failed { msg: "the PDF engine crashed on this document".into() });
                return 6;
            }
        }
    }
}

fn handle(pdfium: &'static pdfium_render::prelude::Pdfium, doc: &mut Option<PdfDocument<'static>>, req: Req) -> Option<Resp> {
    Some(match req {
        Req::Quit => return None,
        Req::SelfTest { probe_file, probe_dir, port } => Resp::SelfTest { lines: self_test(&probe_file, &probe_dir, port) },
        Req::Open { password, data } => {
            if doc.is_some() {
                return Some(Resp::Failed { msg: "a document is already open".into() });
            }
            let pw = if password.is_empty() { None } else { Some(password.as_str()) };
            match pdfx::open_bytes(pdfium, data, pw) {
                Ok(d) => {
                    let sizes: Vec<(f32, f32)> = match d.pages().page_sizes() {
                        Ok(v) => v.iter().map(|r| (r.width().value.clamp(1.0, 14_400.0), r.height().value.clamp(1.0, 14_400.0))).collect(),
                        Err(_) => (0..d.pages().len()).map(|i| d.pages().get(i).map(|p| (p.width().value.clamp(1.0, 14_400.0), p.height().value.clamp(1.0, 14_400.0))).unwrap_or((595.0, 842.0))).collect(),
                    };
                    *doc = Some(d);
                    Resp::Opened { sizes }
                }
                Err(pdfx::OpenErr::Password) => Resp::NeedPassword { wrong: !password.is_empty() },
                Err(pdfx::OpenErr::Damaged) => Resp::Failed { msg: "the file is damaged or is not a PDF".into() },
                Err(pdfx::OpenErr::Missing(m)) | Err(pdfx::OpenErr::Other(m)) => Resp::Failed { msg: m },
            }
        }
        Req::Render { id, page, width } => {
            let Some(d) = doc.as_ref() else { return Some(Resp::Error { id, msg: "no document".into() }) };
            let Ok(p) = d.pages().get(page.min(i32::MAX as u32) as i32) else { return Some(Resp::Error { id, msg: "no such page".into() }) };
            match pdfx::render_full(&p, width.clamp(64, 4096) as i32, true) {
                Some(img) if img.w as u32 <= MAX_BITMAP_SIDE && img.h as u32 <= MAX_BITMAP_SIDE && !img.data.is_empty() => Resp::Rendered { id, w: img.w as u32, h: img.h as u32, rgba: img.data },
                _ => Resp::Error { id, msg: "the page could not be rendered".into() },
            }
        }
        Req::Links { id, page } => {
            let Some(d) = doc.as_ref() else { return Some(Resp::Error { id, msg: "no document".into() }) };
            let Ok(p) = d.pages().get(page.min(i32::MAX as u32) as i32) else { return Some(Resp::Error { id, msg: "no such page".into() }) };
            let geom = pdfx::page_geom(&p);
            let mut items = vec![];
            for l in pdfx::page_links(&p).into_iter().take(MAX_LINKS_PER_PAGE) {
                let target = match l.target {
                    crate::model::LinkTarget::Uri(u) => LinkTarget::Uri(u),
                    crate::model::LinkTarget::Page(n) => LinkTarget::Page(n as u32),
                    crate::model::LinkTarget::None => continue,
                };
                items.push(WLink { rect: geom.rect_to_view(&l.rect), target });
            }
            Resp::Links { id, items }
        }
    })
}

/// Tries operations a sandboxed worker must not be able to do and reports what happened.
fn self_test(probe_file: &str, probe_dir: &str, port: u16) -> Vec<String> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let verdict = |ok: bool| if ok { "ALLOWED" } else { "blocked" };
    let mut v = vec![];
    v.push(format!("read a user file: {}", verdict(std::fs::read(probe_file).is_ok())));
    let probe = std::path::Path::new(probe_dir).join("rustpdf-selftest.tmp");
    let wrote = std::fs::write(&probe, b"x").is_ok();
    let _ = std::fs::remove_file(&probe);
    v.push(format!("write a file: {}", verdict(wrote)));
    let spawned = if cfg!(windows) { std::process::Command::new("cmd").args(["/C", "exit", "0"]).status().is_ok() } else { std::process::Command::new("/bin/true").status().is_ok() };
    v.push(format!("start a program: {}", verdict(spawned)));
    let internet = TcpStream::connect_timeout(&SocketAddr::from(([1, 1, 1, 1], 443)), Duration::from_secs(3)).is_ok();
    v.push(format!("connect to the internet: {}", verdict(internet)));
    let local = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(2)).is_ok();
    v.push(format!("connect to a local service: {}", verdict(local)));
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};
        // SAFETY: allocation of one page, never used.
        let exec = unsafe { !VirtualAlloc(std::ptr::null(), 4096, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE).is_null() };
        v.push(format!("allocate executable memory: {}", verdict(exec)));
    }
    v
}
