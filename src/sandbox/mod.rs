//! Process isolation for "protected view".
//!
//! Untrusted PDFs are parsed by a separate worker process (`rustpdf --pdf-worker`) that is locked down by the
//! operating system: it only receives the document bytes and sends back bitmaps, so a bug in PDFium exploited by a
//! hostile file cannot touch the rest of the system.
//!
//! * Windows: the worker is started in an AppContainer (no file access outside the folder prepared for it, no
//!   network) inside a job object (no child processes, memory cap, no clipboard or desktop access), with dynamic code
//!   generation and other mitigations switched on.
//! * Linux: the worker locks itself down once PDFium is loaded: Landlock (no file writes) and seccomp (no sockets,
//!   no new programs, no tracing of other processes).
//! * macOS: the worker enters the system "pure computation" sandbox once PDFium is loaded.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

/// A running worker: pipes to talk to it and a way to stop it.
pub struct Transport {
    pub input: Box<dyn Write + Send>,
    pub output: Box<dyn Read + Send>,
    pub kill: Arc<dyn Fn() + Send + Sync>,
    /// Exit code of the worker once it has stopped (diagnostics).
    pub exit_code: Arc<dyn Fn() -> Option<i64> + Send + Sync>,
}

/// Starts a worker process with the strongest isolation available on this system.
pub fn spawn_worker() -> Result<Transport, String> {
    #[cfg(windows)]
    {
        windows::spawn()
    }
    #[cfg(not(windows))]
    {
        spawn_plain(&[])
    }
}

/// Starts the worker as an ordinary child process (it locks itself down afterwards on Linux and macOS).
#[cfg_attr(windows, allow(dead_code))]
pub fn spawn_plain(extra_args: &[&str]) -> Result<Transport, String> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg("--pdf-worker").args(extra_args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let input = child.stdin.take().ok_or("no stdin")?;
    let output = child.stdout.take().ok_or("no stdout")?;
    let child = Arc::new(Mutex::new(child));
    let k2 = child.clone();
    let k = child.clone();
    Ok(Transport {
        input: Box::new(input),
        output: Box::new(output),
        exit_code: Arc::new(move || k2.lock().ok().and_then(|mut c| c.try_wait().ok().flatten()).and_then(|s| s.code()).map(i64::from)),
        kill: Arc::new(move || {
            if let Ok(mut c) = k.lock() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }),
    })
}

/// Called by the worker once PDFium is loaded: gives up everything the worker no longer needs.
/// Returns a one-line description of the protections that are active.
pub fn lockdown_self() -> String {
    #[cfg(target_os = "linux")]
    {
        linux::lockdown()
    }
    #[cfg(target_os = "macos")]
    {
        macos::lockdown()
    }
    #[cfg(windows)]
    {
        windows::describe_self()
    }
}

/// `rustpdf --sandbox-check [file.pdf]`: starts a worker, opens a document, renders a page and prints what happens.
/// Useful to see which protections are active on this machine and to diagnose a worker that does not start.
pub fn check(args: &[String]) -> i32 {
    let selftest = args.iter().any(|a| a == "--selftest");
    let path = args.iter().find(|a| !a.starts_with("--")).map(|s| s.as_str());
    use crate::worker_proto::{MAX_RESPONSE_FRAME, Req, Resp, read_frame, write_frame};
    let started = std::time::Instant::now();
    let t = match spawn_worker() {
        Ok(t) => t,
        Err(e) => {
            println!("worker could not be started: {e}");
            return 1;
        }
    };
    println!("worker started ({} ms)", started.elapsed().as_millis());
    let Transport { mut input, mut output, kill, exit_code } = t;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || loop {
        match read_frame(&mut output, MAX_RESPONSE_FRAME) {
            Ok(Some(f)) => {
                if tx.send(Resp::decode(&f)).is_err() {
                    return;
                }
            }
            other => {
                let _ = tx.send(Err(format!("stream ended: {:?}", other.map(|o| o.is_some()))));
                return;
            }
        }
    });
    let wait = |what: &str| -> Option<Resp> {
        match rx.recv_timeout(std::time::Duration::from_secs(20)) {
            Ok(Ok(r)) => Some(r),
            Ok(Err(e)) => {
                println!("{what}: invalid answer or end of stream: {e}");
                None
            }
            Err(_) => {
                println!("{what}: no answer after 20 s");
                None
            }
        }
    };
    let mut code = 0;
    match wait("startup") {
        Some(Resp::Ready { isolation }) => println!("ready, isolation: {isolation}"),
        _ => code = 2,
    }
    if code == 0 {
        if let Some(p) = path {
            let data = std::fs::read(p).unwrap_or_default();
            let _ = write_frame(&mut input, &Req::Open { password: String::new(), data }.encode());
            match wait("open") {
                Some(Resp::Opened { sizes }) => println!("opened: {} page(s), first {:?}", sizes.len(), sizes.first()),
                Some(other) => {
                    println!("open: {other:?}");
                    code = 3;
                }
                None => code = 3,
            }
            if code == 0 {
                let _ = write_frame(&mut input, &Req::Render { id: 1, page: 0, width: 600 }.encode());
                match wait("render") {
                    Some(Resp::Rendered { w, h, rgba, .. }) => {
                        let dark = rgba.chunks_exact(4).filter(|p| p[0] < 128).count();
                        println!("rendered page 1: {w}x{h}, {dark} dark pixels");
                    }
                    Some(other) => {
                        println!("render: {other:?}");
                        code = 4;
                    }
                    None => code = 4,
                }
            }
        }
    }
    if code == 0 && selftest {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").ok();
        let port = listener.as_ref().and_then(|l| l.local_addr().ok()).map(|a| a.port()).unwrap_or(9);
        let probe_file = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let probe_dir = std::env::temp_dir().to_string_lossy().into_owned();
        let _ = write_frame(&mut input, &Req::SelfTest { probe_file, probe_dir, port }.encode());
        match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Ok(Resp::SelfTest { lines })) => {
                for l in &lines {
                    println!("  {l}");
                }
                // on Linux reading files is allowed on purpose (fonts, libraries): nothing can leave the process anyway
                let expected_open: &[&str] = if cfg!(target_os = "linux") { &["read a user file"] } else { &[] };
                if lines.iter().any(|l| l.contains("ALLOWED") && !expected_open.iter().any(|e| l.starts_with(e))) {
                    println!("the worker is NOT fully isolated");
                    code = 5;
                } else {
                    println!("every forbidden operation was blocked");
                }
            }
            other => {
                println!("self test: no usable answer ({:?})", other.is_ok());
                code = 6;
            }
        }
        drop(listener);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    if let Some(c) = exit_code() {
        println!("worker exit code: {c} (0x{:x})", c as u32);
    }
    kill();
    code
}
