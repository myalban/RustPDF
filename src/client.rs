//! The application side of protected view: runs the isolated worker and exposes its answers through channels.

use crate::sandbox;
use crate::worker_proto::{MAX_RESPONSE_FRAME, Req, Resp, read_frame, write_frame};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

pub struct Client {
    tx: Sender<Req>,
    pub rx: Receiver<Resp>,
    kill: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
    killed: Arc<AtomicBool>,
}

impl Client {
    /// Starts the worker in the background and sends it the document. Never blocks the caller.
    pub fn start(data: Vec<u8>, password: String, ctx: eframe::egui::Context) -> Client {
        let (tx, req_rx) = channel::<Req>();
        let (resp_tx, rx) = channel::<Resp>();
        let kill: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>> = Arc::new(Mutex::new(None));
        let killed = Arc::new(AtomicBool::new(false));
        let (kill2, killed2) = (kill.clone(), killed.clone());
        std::thread::spawn(move || {
            let fail = |msg: String| {
                let _ = resp_tx.send(Resp::Failed { msg });
                ctx.request_repaint();
            };
            let transport = match sandbox::spawn_worker() {
                Ok(t) => t,
                Err(e) => return fail(e),
            };
            *kill2.lock().unwrap() = Some(transport.kill.clone());
            let mut input = transport.input;
            let mut output = transport.output;
            // reader: decodes and validates every answer
            let (reader_tx, reader_ctx, reader_killed) = (resp_tx.clone(), ctx.clone(), killed2.clone());
            std::thread::spawn(move || {
                loop {
                    match read_frame(&mut output, MAX_RESPONSE_FRAME) {
                        Ok(Some(frame)) => match Resp::decode(&frame) {
                            Ok(r) => {
                                if reader_tx.send(r).is_err() {
                                    return;
                                }
                                reader_ctx.request_repaint();
                            }
                            Err(_) => {
                                let _ = reader_tx.send(Resp::Failed { msg: "the isolated viewer sent an invalid answer".into() });
                                reader_ctx.request_repaint();
                                return;
                            }
                        },
                        _ => {
                            if !reader_killed.load(Ordering::Relaxed) {
                                let _ = reader_tx.send(Resp::Failed { msg: "the isolated viewer stopped unexpectedly (the document may be damaged or hostile)".into() });
                                reader_ctx.request_repaint();
                            }
                            return;
                        }
                    }
                }
            });
            if write_frame(&mut input, &Req::Open { password, data }.encode()).is_err() {
                return;
            }
            while let Ok(req) = req_rx.recv() {
                let quit = req == Req::Quit;
                if write_frame(&mut input, &req.encode()).is_err() || quit {
                    break;
                }
            }
            drop(input);
        });
        Client { tx, rx, kill, killed }
    }

    pub fn send(&self, req: Req) {
        let _ = self.tx.send(req);
    }

    /// Stops the worker immediately.
    pub fn kill(&self) {
        self.killed.store(true, Ordering::Relaxed);
        if let Some(k) = self.kill.lock().unwrap().clone() {
            k();
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.tx.send(Req::Quit);
        self.kill();
    }
}
