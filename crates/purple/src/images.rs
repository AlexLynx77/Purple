//! Loads champion portraits and skin tiles from the League client, with a disk cache.
//!
//! A worker thread fetches and decodes (so the window never blocks) and the UI thread turns the
//! decoded pixels into textures. Textures are capped so browsing every champion does not grow memory.

use crate::lcu::Lcu;
use crate::store;
use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Past this many textures, the ones not drawn in the last frame are dropped.
const MAX_TEXTURES: usize = 240;
const RETRY_AFTER: Duration = Duration::from_secs(5);

struct Request {
    path: String,
    max_width: u32,
}

struct Loaded {
    path: String,
    image: Option<ColorImage>,
}

struct Entry {
    texture: TextureHandle,
    last_used: u64,
}

pub struct Images {
    requests: Sender<Request>,
    results: Receiver<Loaded>,
    textures: HashMap<String, Entry>,
    /// `None` while a request is in flight, `Some(when)` after a failed one.
    pending: HashMap<String, Option<Instant>>,
    frame: u64,
}

impl Images {
    pub fn new(ctx: &Context) -> Self {
        let (requests, requests_rx) = mpsc::channel();
        let (results_tx, results) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || worker(&requests_rx, &results_tx, &ctx));
        Self { requests, results, textures: HashMap::new(), pending: HashMap::new(), frame: 0 }
    }

    /// Call once per frame, before drawing: turns freshly decoded images into textures.
    pub fn begin_frame(&mut self, ctx: &Context) {
        self.frame += 1;
        while let Ok(loaded) = self.results.try_recv() {
            match loaded.image {
                Some(image) => {
                    let texture = ctx.load_texture(&loaded.path, image, TextureOptions::LINEAR);
                    self.pending.remove(&loaded.path);
                    self.textures.insert(loaded.path, Entry { texture, last_used: self.frame });
                }
                None => {
                    self.pending.insert(loaded.path, Some(Instant::now()));
                    ctx.request_repaint_after(RETRY_AFTER);
                }
            }
        }
    }

    /// Call once per frame, after drawing: frees textures that were not used recently when over the cap.
    pub fn end_frame(&mut self) {
        if self.textures.len() > MAX_TEXTURES {
            let frame = self.frame;
            self.textures.retain(|_, e| e.last_used + 1 >= frame);
        }
    }

    /// The texture for client asset `path` (downscaled to `max_width`), requesting it if needed.
    pub fn get(&mut self, path: &str, max_width: u32) -> Option<&TextureHandle> {
        if path.is_empty() {
            return None;
        }
        if self.textures.contains_key(path) {
            let entry = self.textures.get_mut(path)?;
            entry.last_used = self.frame;
            return Some(&entry.texture);
        }
        let due = match self.pending.get(path) {
            None => true,
            Some(Some(failed_at)) => failed_at.elapsed() >= RETRY_AFTER,
            Some(None) => false,
        };
        if due {
            self.pending.insert(path.to_owned(), None);
            let _ = self.requests.send(Request { path: path.to_owned(), max_width });
        }
        None
    }
}

fn worker(requests: &Receiver<Request>, results: &Sender<Loaded>, ctx: &Context) {
    let mut lcu: Option<Lcu> = None;
    for request in requests {
        let image = load(&request, &mut lcu);
        if results.send(Loaded { path: request.path, image }).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

fn cache_file(path: &str) -> Option<PathBuf> {
    let name: String = path.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    Some(store::data_dir().ok()?.join("img").join(name))
}

fn load(request: &Request, lcu: &mut Option<Lcu>) -> Option<ColorImage> {
    let cache = cache_file(&request.path)?;
    let bytes = match std::fs::read(&cache) {
        Ok(bytes) => bytes,
        Err(_) => {
            if lcu.is_none() {
                *lcu = Lcu::connect().ok();
            }
            let fetched = lcu.as_ref()?.get_bytes(&request.path);
            let Ok(bytes) = fetched else {
                // The client may have restarted on another port: reconnect on the next request.
                *lcu = None;
                return None;
            };
            if let Some(dir) = cache.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&cache, &bytes);
            bytes
        }
    };
    decode(&bytes, request.max_width)
}

fn decode(bytes: &[u8], max_width: u32) -> Option<ColorImage> {
    let mut image = image::load_from_memory(bytes).ok()?.to_rgba8();
    if image.width() > max_width {
        let height = (u64::from(image.height()) * u64::from(max_width) / u64::from(image.width())).max(1);
        image = image::imageops::resize(&image, max_width, u32::try_from(height).ok()?, image::imageops::FilterType::Triangle);
    }
    let size = [image.width() as usize, image.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
}
