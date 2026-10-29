//! Thumbnails in remote folders: the pictures are downloaded in the
//! background (`ox_core::remote::thumbs`: small ones only, over open
//! connections, stopped when the folder is left), decoded like local ones
//! and kept with the others (in memory).

use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use egui::{ColorImage, Context};
use ox_core::remote::thumbs::{Fetched, ThumbFetcher};
use ox_core::remote::{Connections, RemoteAddr};

use super::{Done, Job};

pub(super) struct RemoteThumbs {
    conns: Arc<Mutex<Option<Arc<Connections>>>>,
    fetcher: ThumbFetcher<Job>,
}

impl RemoteThumbs {
    pub fn new(tx: Sender<Done>, ctx: Context) -> Self {
        let conns: Arc<Mutex<Option<Arc<Connections>>>> = Arc::default();
        let known = conns.clone();
        let fetcher = ThumbFetcher::new(
            move |addr: &RemoteAddr| known.lock().ok()?.as_ref()?.cached(addr),
            move |job: Job, got: Fetched| {
                let (image, dropped) = match got {
                    Fetched::Bytes(bytes) => (decode(&job.path, &bytes, job.size), false),
                    Fetched::Failed => (None, false),
                    Fetched::Dropped => (None, true),
                };
                let _ = tx.send(Done {
                    path: job.path,
                    modified: job.modified,
                    size: job.size,
                    cache_only: job.cache_only,
                    image,
                    dropped,
                });
                ctx.request_repaint();
            },
        );
        Self { conns, fetcher }
    }

    /// Ask for the thumbnail of a remote picture of `bytes` bytes; `false`
    /// if it isn't in the folder shown (asked again later).
    pub fn request(&self, job: Job, bytes: u64) -> bool {
        match RemoteAddr::from_path(&job.path) {
            Some(addr) => self.fetcher.request(addr, bytes, job),
            None => false,
        }
    }

    pub fn set_folder(&self, folder: Option<RemoteAddr>, conns: &Arc<Connections>) {
        if let Ok(mut c) = self.conns.lock() {
            if c.is_none() {
                *c = Some(conns.clone());
            }
        }
        self.fetcher.set_folder(folder);
    }
}

fn decode(path: &std::path::Path, bytes: &[u8], size: u32) -> Option<ColorImage> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let source = ox_core::picture::Source::Bytes(bytes);
    crate::ui::preview::still(source, &ext, size)
        .ok()
        .map(|(img, _)| img)
}
