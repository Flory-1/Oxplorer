//! Thumbnails of pictures and videos for the grid view. They come from the
//! Windows Shell (which also covers videos, PDFs and anything else with a
//! thumbnail handler); elsewhere pictures are decoded with the `image` crate.
//!
//! A few background threads load them, newest request first, so the tiles
//! on screen are served before ones that were scrolled past. They come in a
//! few resolutions (`ox_core::grid_zoom::THUMB_BUCKETS`): a tile asks for
//! the one matching its size, and a smaller one is shown while a larger one
//! loads.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

use crossbeam_channel::{Receiver, Sender};
use egui::{ColorImage, Context, TextureHandle, TextureOptions, Vec2};
use ox_core::entry::{FileEntry, FileKind};

const WORKERS: usize = 2;
/// Requests beyond this are dropped, oldest first (long scrolled past).
const MAX_QUEUED: usize = 256;
/// Textures kept, and their pixels at most; the oldest are freed first.
const MAX_LOADED: usize = 600;
const MAX_PIXELS: usize = 24 * 1024 * 1024;
/// Don't decode huge pictures without the Shell's cached thumbnails.
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

struct Job {
    path: PathBuf,
    modified: Option<SystemTime>,
    /// Long side in pixels (a bucket).
    size: u32,
}

/// A loaded thumbnail: its texture, the file's date and the bucket.
struct Loaded(TextureHandle, Option<SystemTime>, u32);

enum Slot {
    /// Being loaded; the smaller one meanwhile, if any.
    Loading(Option<Loaded>),
    Ready(Loaded),
    Failed(Option<SystemTime>),
}

struct Queue {
    jobs: Mutex<Vec<Job>>,
    ready: Condvar,
}

/// `None` image: no thumbnail. `dropped`: never tried (queue overflow).
struct Done {
    path: PathBuf,
    modified: Option<SystemTime>,
    size: u32,
    image: Option<ColorImage>,
    dropped: bool,
}

pub struct Thumbnails {
    queue: Arc<Queue>,
    done_tx: Sender<Done>,
    done: Receiver<Done>,
    slots: RefCell<HashMap<PathBuf, Slot>>,
    /// Loaded paths, oldest first, with the texture's pixels.
    loaded: RefCell<VecDeque<(PathBuf, usize)>>,
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
            let old = match slots.remove(&done.path) {
                Some(Slot::Loading(old)) => old,
                _ => None,
            };
            if done.dropped {
                // Keep showing the smaller one; it is asked for again when seen.
                if let Some(old) = old {
                    slots.insert(done.path, Slot::Ready(old));
                }
                continue;
            }
            let slot = match done.image {
                Some(image) => {
                    let options = TextureOptions {
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                        ..TextureOptions::LINEAR
                    };
                    let name = format!("thumb:{}:{}", done.size, done.path.display());
                    let pixels = image.pixels.len();
                    loaded.retain(|(p, _)| *p != done.path);
                    loaded.push_back((done.path.clone(), pixels));
                    let tex = ctx.load_texture(name, image, options);
                    Slot::Ready(Loaded(tex, done.modified, done.size))
                }
                // A larger one failed: keep the smaller one, and don't ask again.
                None => match old {
                    Some(Loaded(tex, modified, _)) if modified == done.modified => {
                        Slot::Ready(Loaded(tex, modified, u32::MAX))
                    }
                    _ => Slot::Failed(done.modified),
                },
            };
            slots.insert(done.path, slot);
        }
        let mut total: usize = loaded.iter().map(|(_, n)| n).sum();
        while loaded.len() > MAX_LOADED || (total > MAX_PIXELS && loaded.len() > 1) {
            if let Some((old, n)) = loaded.pop_front() {
                slots.remove(&old);
                total -= n;
            }
        }
    }

    /// The thumbnail of `e` and its size in pixels, if loaded, for a box
    /// whose long side is `pixels` on screen. Asks for it otherwise (again
    /// when the file changed, or in a larger size when the box grew).
    pub fn get(&self, e: &FileEntry, pixels: f32) -> Option<(egui::TextureId, Vec2)> {
        let size = ox_core::grid_zoom::thumb_bucket(pixels);
        let shown = |l: &Loaded| (l.0.id(), l.0.size_vec2());
        let mut slots = self.slots.borrow_mut();
        let old = match slots.get(&e.path) {
            Some(Slot::Loading(old)) => {
                return old.as_ref().filter(|l| l.1 == e.modified).map(shown);
            }
            Some(Slot::Ready(l)) if l.1 == e.modified && l.2 >= size => return Some(shown(l)),
            Some(Slot::Failed(modified)) if *modified == e.modified => return None,
            // Too small: shown while the larger one loads.
            Some(Slot::Ready(l)) if l.1 == e.modified => match slots.remove(&e.path) {
                Some(Slot::Ready(l)) => Some(l),
                _ => None,
            },
            _ => None,
        };
        let current = old.as_ref().map(shown);
        slots.insert(e.path.clone(), Slot::Loading(old));
        drop(slots);
        let mut jobs = self.queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
        jobs.push(Job {
            path: e.path.clone(),
            modified: e.modified,
            size,
        });
        if jobs.len() > MAX_QUEUED {
            let old = jobs.remove(0);
            let _ = self.done_tx.send(Done {
                path: old.path,
                modified: old.modified,
                size: old.size,
                image: None,
                dropped: true,
            });
        }
        self.queue.ready.notify_one();
        current
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
        let image = load(&job.path, job.size);
        let done = Done {
            path: job.path,
            modified: job.modified,
            size: job.size,
            image,
            dropped: false,
        };
        if tx.send(done).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

fn load(path: &Path, size: u32) -> Option<ColorImage> {
    if let Some(t) = ox_core::shell::thumbnail(path, size) {
        let size = [t.width as usize, t.height as usize];
        return Some(ColorImage::from_rgba_premultiplied(size, &t.rgba));
    }
    if ox_core::shell::supported() {
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
    let img = image::open(path).ok()?.thumbnail(size, size).to_rgba8();
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
        let img = load(&path, 128).unwrap();
        assert_eq!(img.size, [128, 64]);
        assert_eq!(load(&path, 256).unwrap().size, [256, 128]);
        assert!(load(&dir.path().join("missing.png"), 128).is_none());
    }
}
