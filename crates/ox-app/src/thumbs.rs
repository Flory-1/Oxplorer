//! Thumbnails of pictures and videos for the grid view. They come from the
//! Windows Shell (which also covers videos, PDFs and anything else with a
//! thumbnail handler); elsewhere pictures are decoded with the `image` crate.
//!
//! A few background threads load them, newest request first, so the tiles
//! on screen are served before ones that were scrolled past.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

use crossbeam_channel::{Receiver, Sender};
use egui::{ColorImage, Context, TextureHandle, TextureOptions, Vec2};
use ox_core::entry::{FileEntry, FileKind};

/// Long side of a thumbnail in pixels (sharp on 2× screens).
const SIZE: u32 = 128;
const WORKERS: usize = 2;
/// Requests beyond this are dropped, oldest first (long scrolled past).
const MAX_QUEUED: usize = 256;
/// Textures kept; the oldest are freed first.
const MAX_LOADED: usize = 600;
/// Don't decode huge pictures without the Shell's cached thumbnails.
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

struct Job {
    path: PathBuf,
    modified: Option<SystemTime>,
    /// An online-only cloud file: only the Shell's cached thumbnail, so
    /// the file isn't downloaded.
    cache_only: bool,
}

enum Slot {
    Loading,
    Ready(TextureHandle, Option<SystemTime>),
    /// No thumbnail for this version, and whether only the cache was asked.
    Failed(Option<SystemTime>, bool),
}

struct Queue {
    jobs: Mutex<Vec<Job>>,
    ready: Condvar,
}

/// `None` image: no thumbnail. `dropped`: never tried (queue overflow).
struct Done {
    path: PathBuf,
    modified: Option<SystemTime>,
    cache_only: bool,
    image: Option<ColorImage>,
    dropped: bool,
}

pub struct Thumbnails {
    queue: Arc<Queue>,
    done_tx: Sender<Done>,
    done: Receiver<Done>,
    slots: RefCell<HashMap<PathBuf, Slot>>,
    loaded: RefCell<VecDeque<PathBuf>>,
}

/// Files that can have a thumbnail.
pub fn wanted(e: &FileEntry) -> bool {
    if e.is_dir {
        return false;
    }
    match e.kind() {
        FileKind::Image => ox_core::shell::supported() || decodable(&e.extension()),
        FileKind::Video | FileKind::Pdf => ox_core::shell::supported(),
        _ => false,
    }
}

fn decodable(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tif" | "tiff"
    )
}

impl Thumbnails {
    pub fn new(ctx: &Context) -> Self {
        let queue = Arc::new(Queue {
            jobs: Mutex::new(Vec::new()),
            ready: Condvar::new(),
        });
        let (done_tx, done) = crossbeam_channel::unbounded();
        for i in 0..WORKERS {
            let queue = queue.clone();
            let tx = done_tx.clone();
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new()
                .name(format!("thumbnails-{i}"))
                .spawn(move || worker(&queue, &tx, &ctx));
        }
        Self {
            queue,
            done_tx,
            done,
            slots: RefCell::default(),
            loaded: RefCell::default(),
        }
    }

    /// Upload finished thumbnails. Call once per frame.
    pub fn poll(&self, ctx: &Context) {
        let mut slots = self.slots.borrow_mut();
        let mut loaded = self.loaded.borrow_mut();
        for done in self.done.try_iter() {
            if done.dropped {
                slots.remove(&done.path);
                continue;
            }
            let slot = match done.image {
                Some(image) => {
                    let options = TextureOptions {
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                        ..TextureOptions::LINEAR
                    };
                    let name = format!("thumb:{}", done.path.display());
                    loaded.push_back(done.path.clone());
                    Slot::Ready(ctx.load_texture(name, image, options), done.modified)
                }
                None => Slot::Failed(done.modified, done.cache_only),
            };
            slots.insert(done.path, slot);
        }
        while loaded.len() > MAX_LOADED {
            if let Some(old) = loaded.pop_front() {
                slots.remove(&old);
            }
        }
    }

    /// The thumbnail of `e` and its size in pixels, if loaded. Asks for it
    /// otherwise (again when the file changed).
    pub fn get(&self, e: &FileEntry) -> Option<(egui::TextureId, Vec2)> {
        let cache_only = e.cloud == Some(ox_core::cloud::CloudStatus::OnlineOnly);
        if let Some(slot) = self.slots.borrow().get(&e.path) {
            match slot {
                Slot::Loading => return None,
                Slot::Ready(tex, modified) if *modified == e.modified => {
                    return Some((tex.id(), tex.size_vec2()));
                }
                // Tried again once the file is downloaded.
                Slot::Failed(modified, c) if *modified == e.modified && *c == cache_only => {
                    return None
                }
                _ => {}
            }
        }
        self.slots
            .borrow_mut()
            .insert(e.path.clone(), Slot::Loading);
        let mut jobs = self.queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
        jobs.push(Job {
            path: e.path.clone(),
            modified: e.modified,
            cache_only,
        });
        if jobs.len() > MAX_QUEUED {
            let old = jobs.remove(0);
            let _ = self.done_tx.send(Done {
                path: old.path,
                modified: old.modified,
                cache_only: old.cache_only,
                image: None,
                dropped: true,
            });
        }
        self.queue.ready.notify_one();
        None
    }
}

fn worker(queue: &Queue, tx: &Sender<Done>, ctx: &Context) {
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(job) = jobs.pop() {
                    break job;
                }
                jobs = queue.ready.wait(jobs).unwrap_or_else(|p| p.into_inner());
            }
        };
        let image = load(&job.path, job.cache_only);
        let done = Done {
            path: job.path,
            modified: job.modified,
            cache_only: job.cache_only,
            image,
            dropped: false,
        };
        if tx.send(done).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

fn load(path: &Path, cache_only: bool) -> Option<ColorImage> {
    let shell = if cache_only {
        ox_core::shell::thumbnail_in_cache(path, SIZE)
    } else {
        ox_core::shell::thumbnail(path, SIZE)
    };
    if let Some(t) = shell {
        let size = [t.width as usize, t.height as usize];
        return Some(ColorImage::from_rgba_premultiplied(size, &t.rgba));
    }
    if ox_core::shell::supported() || cache_only {
        return None;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_DECODE_BYTES);
    if !decodable(&ext) || !small {
        return None;
    }
    let img = image::open(path).ok()?.thumbnail(SIZE, SIZE).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// Where a `size` image goes inside `area`: as large as fits, centred,
/// never scaled up beyond 2×.
pub fn fit(area: egui::Rect, size: Vec2) -> egui::Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return area;
    }
    let scale = (area.width() / size.x).min(area.height() / size.y).min(2.0);
    egui::Rect::from_center_size(area.center(), size * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2, Rect};

    #[test]
    fn fitting_keeps_the_aspect_ratio() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(90.0, 50.0));
        let wide = fit(area, vec2(128.0, 64.0));
        assert_eq!(wide.size(), vec2(90.0, 45.0));
        assert_eq!(wide.center(), area.center());
        let tall = fit(area, vec2(64.0, 128.0));
        assert_eq!(tall.size(), vec2(25.0, 50.0));
        // Tiny pictures are not blown up.
        assert_eq!(fit(area, vec2(10.0, 10.0)).size(), vec2(20.0, 20.0));
    }

    #[test]
    fn pictures_are_loaded_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("red.png");
        image::RgbaImage::from_pixel(300, 150, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        if ox_core::shell::supported() {
            return; // needs the Shell's thumbnail cache
        }
        let img = load(&path, false).unwrap();
        assert_eq!(img.size, [128, 64]);
        assert!(load(&dir.path().join("missing.png"), false).is_none());
    }
}
