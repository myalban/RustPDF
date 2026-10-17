//! Texture cache (previews, tiles, thumbnails) with LRU eviction and a per-frame time budget.

use eframe::egui::{self, TextureHandle};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
pub enum Key {
    /// Low-resolution preview of a whole page.
    Preview { tab: u64, page: usize, ver: u64, night: bool },
    /// High-resolution tile (scale expressed as f32 bits).
    Tile { tab: u64, page: usize, ver: u64, scale: u32, tx: u16, ty: u16, night: bool },
    Thumb { tab: u64, page: usize, ver: u64 },
}

struct Entry {
    tex: TextureHandle,
    bytes: usize,
    last_used: u64,
}

pub struct RenderCache {
    map: HashMap<Key, Entry>,
    frame: u64,
    bytes: usize,
    cap: usize,
    deadline: Instant,
    rendered_this_frame: u32,
    /// Some items are still to be produced: request a new frame.
    pub pending: bool,
}

impl RenderCache {
    pub fn new(cap_mb: u32) -> Self {
        RenderCache {
            map: HashMap::new(),
            frame: 0,
            bytes: 0,
            cap: cap_mb as usize * 1024 * 1024,
            deadline: Instant::now(),
            rendered_this_frame: 0,
            pending: false,
        }
    }

    pub fn set_cap_mb(&mut self, mb: u32) {
        self.cap = mb as usize * 1024 * 1024;
    }

    /// To be called at the start of every frame: sets the render budget (≈ 14 ms).
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.deadline = Instant::now() + Duration::from_millis(14);
        self.rendered_this_frame = 0;
        self.pending = false;
    }

    /// Is there still something to produce this frame? Always yes for the first item.
    pub fn may_render(&mut self) -> bool {
        if self.rendered_this_frame == 0 || Instant::now() < self.deadline {
            true
        } else {
            self.pending = true;
            false
        }
    }

    pub fn get(&mut self, key: &Key) -> Option<TextureHandle> {
        let f = self.frame;
        self.map.get_mut(key).map(|e| {
            e.last_used = f;
            e.tex.clone()
        })
    }

    pub fn contains(&self, key: &Key) -> bool {
        self.map.contains_key(key)
    }

    pub fn insert(&mut self, key: Key, tex: TextureHandle, bytes: usize) {
        self.rendered_this_frame += 1;
        if let Some(old) = self.map.insert(key, Entry { tex, bytes, last_used: self.frame }) {
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
        self.evict();
    }

    /// Removes the least recently used entries beyond the memory cap.
    fn evict(&mut self) {
        if self.bytes <= self.cap {
            return;
        }
        let mut items: Vec<(Key, u64)> = self.map.iter().map(|(k, e)| (*k, e.last_used)).collect();
        items.sort_by_key(|(_, u)| *u);
        for (k, used) in items {
            if self.bytes <= self.cap {
                break;
            }
            // Never remove what is used by the current frame.
            if used >= self.frame {
                break;
            }
            if let Some(e) = self.map.remove(&k) {
                self.bytes -= e.bytes;
            }
        }
    }

    /// Forgets everything about a closed tab.
    pub fn purge_tab(&mut self, tab: u64) {
        let keys: Vec<Key> = self
            .map
            .keys()
            .filter(|k| match k {
                Key::Preview { tab: t, .. } | Key::Tile { tab: t, .. } | Key::Thumb { tab: t, .. } => *t == tab,
            })
            .copied()
            .collect();
        for k in keys {
            if let Some(e) = self.map.remove(&k) {
                self.bytes -= e.bytes;
            }
        }
    }

    pub fn used_bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

/// Converts a raw RGBA image into a texture; inverts the colors in night mode.
pub fn upload(ctx: &egui::Context, name: String, rgba: &crate::pdfx::Rgba, night: bool) -> (TextureHandle, usize) {
    let mut data = std::borrow::Cow::Borrowed(&rgba.data[..]);
    if night {
        let mut v = rgba.data.clone();
        for px in v.chunks_exact_mut(4) {
            px[0] = 255 - px[0];
            px[1] = 255 - px[1];
            px[2] = 255 - px[2];
        }
        data = std::borrow::Cow::Owned(v);
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([rgba.w, rgba.h], &data);
    let tex = ctx.load_texture(name, img, egui::TextureOptions::LINEAR);
    (tex, rgba.w * rgba.h * 4)
}
