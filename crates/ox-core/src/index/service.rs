//! The index manager: loads the saved index, rebuilds it in the background
//! and keeps it current with live updates.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use super::live::{Live, LiveInfo};
use super::{FileIndex, IndexConfig};

/// Where the index is cached between runs.
pub fn default_cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(crate::config_dir)
        .join(crate::APP_NAME)
        .join("index.bin")
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndexState {
    /// No index yet (or indexing is turned off).
    Empty,
    Loading,
    Building,
    Ready,
    Failed(String),
}

pub(super) struct Shared {
    pub index: RwLock<Option<Arc<FileIndex>>>,
    pub state: Mutex<IndexState>,
    pub progress: AtomicU64,
    pub cancel: AtomicBool,
    pub building: AtomicBool,
    pub last_build: Mutex<Option<(Instant, Duration)>>,
    pub cache: PathBuf,
    pub notify: Box<dyn Fn() + Send + Sync>,
    live: Mutex<Option<Live>>,
}

/// Owns the current index: loads it from the cache and rebuilds it in the background.
#[derive(Clone)]
pub struct IndexService {
    shared: Arc<Shared>,
}

impl IndexService {
    /// Start loading the cached index (if any) in the background.
    pub fn start(cache: PathBuf, notify: impl Fn() + Send + Sync + 'static) -> Self {
        let svc = Self {
            shared: Arc::new(Shared {
                index: RwLock::new(None),
                state: Mutex::new(IndexState::Loading),
                progress: AtomicU64::new(0),
                cancel: AtomicBool::new(false),
                building: AtomicBool::new(false),
                last_build: Mutex::new(None),
                cache,
                notify: Box::new(notify),
                live: Mutex::new(None),
            }),
        };
        let shared = svc.shared.clone();
        std::thread::Builder::new()
            .name("index-load".into())
            .spawn(move || {
                let loaded = FileIndex::load(&shared.cache).ok();
                let ready = loaded.is_some();
                if let Some(idx) = loaded {
                    *shared.index.write().unwrap() = Some(Arc::new(idx));
                }
                let mut state = shared.state.lock().unwrap();
                if *state == IndexState::Loading {
                    *state = if ready {
                        IndexState::Ready
                    } else {
                        IndexState::Empty
                    };
                }
                drop(state);
                (shared.notify)();
            })
            .expect("failed to spawn index thread");
        svc
    }

    /// The current index, if one is loaded or built.
    pub fn index(&self) -> Option<Arc<FileIndex>> {
        self.shared.index.read().unwrap().clone()
    }

    pub fn state(&self) -> IndexState {
        self.shared.state.lock().unwrap().clone()
    }

    pub fn is_building(&self) -> bool {
        self.shared.building.load(Ordering::Relaxed)
    }

    /// Items indexed so far by the running build.
    pub fn progress(&self) -> u64 {
        self.shared.progress.load(Ordering::Relaxed)
    }

    /// When the last build finished and how long it took.
    pub fn last_build(&self) -> Option<(Instant, Duration)> {
        *self.shared.last_build.lock().unwrap()
    }

    /// Build a fresh index in the background (no-op if one is already being built).
    /// The old index keeps answering queries until the new one is ready.
    pub fn rebuild(&self, cfg: IndexConfig) {
        rebuild(&self.shared, cfg);
    }

    /// Stop a running build.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// Forget the index and delete the cache (indexing turned off).
    pub fn clear(&self) {
        self.set_live(None);
        self.cancel();
        *self.shared.index.write().unwrap() = None;
        *self.shared.state.lock().unwrap() = IndexState::Empty;
        let _ = fs::remove_file(&self.shared.cache);
        (self.shared.notify)();
    }

    /// Keep the index current by applying file changes as they happen, for
    /// the roots in `cfg`; `None` stops. Cheap to call when nothing changed.
    pub fn set_live(&self, cfg: Option<IndexConfig>) {
        let mut live = self.shared.live.lock().unwrap();
        match cfg {
            None => *live = None,
            Some(cfg) if live.as_ref().is_some_and(|l| *l.config() == cfg) => {}
            Some(cfg) => *live = Some(Live::start(&self.shared, cfg)),
        }
    }

    /// What live updates are doing, if they are on.
    pub fn live(&self) -> Option<LiveInfo> {
        self.shared.live.lock().unwrap().as_ref().map(Live::info)
    }
}

pub(super) fn rebuild(shared: &Arc<Shared>, cfg: IndexConfig) {
    if shared.building.swap(true, Ordering::SeqCst) {
        return;
    }
    let shared = shared.clone();
    shared.cancel.store(false, Ordering::Relaxed);
    shared.progress.store(0, Ordering::Relaxed);
    *shared.state.lock().unwrap() = IndexState::Building;
    (shared.notify)();
    std::thread::Builder::new()
        .name("index-build".into())
        .spawn(move || {
            let start = Instant::now();
            let idx = FileIndex::build_fast(&cfg, &shared.cancel, &shared.progress);
            if shared.cancel.load(Ordering::Relaxed) {
                let has_index = shared.index.read().unwrap().is_some();
                *shared.state.lock().unwrap() = if has_index {
                    IndexState::Ready
                } else {
                    IndexState::Empty
                };
            } else {
                let save = idx.save(&shared.cache);
                *shared.index.write().unwrap() = Some(Arc::new(idx));
                *shared.last_build.lock().unwrap() = Some((Instant::now(), start.elapsed()));
                *shared.state.lock().unwrap() = match save {
                    Ok(()) => IndexState::Ready,
                    Err(e) => IndexState::Failed(format!("Couldn't save the index: {e}")),
                };
            }
            shared.building.store(false, Ordering::SeqCst);
            (shared.notify)();
        })
        .expect("failed to spawn index thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Query;

    fn wait(svc: &IndexService) {
        let start = Instant::now();
        while svc.state() == IndexState::Loading || svc.is_building() {
            assert!(start.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn service_builds_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        fs::create_dir_all(data.join("Documents/Taxes 2026")).unwrap();
        fs::write(data.join("Documents/notes.txt"), "x").unwrap();
        fs::write(data.join("Documents/Taxes 2026/receipt.pdf"), "x").unwrap();
        fs::write(data.join("photo.jpg"), "x").unwrap();
        let cache = dir.path().join("index.bin");
        let svc = IndexService::start(cache.clone(), || {});
        wait(&svc);
        assert_eq!(svc.state(), IndexState::Empty);
        svc.rebuild(IndexConfig {
            roots: vec![data.clone()],
            exclude: vec![],
            threads: 2,
        });
        wait(&svc);
        assert_eq!(svc.state(), IndexState::Ready);
        assert!(svc.index().unwrap().len() > 4);

        // A new service picks up the cached index without rebuilding.
        let again = IndexService::start(cache, || {});
        wait(&again);
        assert_eq!(again.state(), IndexState::Ready);
        assert_eq!(again.index().unwrap().len(), svc.index().unwrap().len());
    }

    #[test]
    fn live_updates_follow_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        fs::create_dir_all(data.join("old")).unwrap();
        fs::write(data.join("old/report.txt"), "x").unwrap();
        let cfg = IndexConfig {
            roots: vec![data.clone()],
            exclude: vec![],
            threads: 2,
        };
        let svc = IndexService::start(dir.path().join("index.bin"), || {});
        wait(&svc);
        svc.rebuild(cfg.clone());
        wait(&svc);
        svc.set_live(Some(cfg.clone()));
        svc.set_live(Some(cfg)); // unchanged: keeps running
        let count = |q: &str| {
            let idx = svc.index().unwrap();
            let opts = super::super::IndexQuery {
                max_results: 100,
                ..Default::default()
            };
            idx.search(&Query::parse(q, false), &opts).0.len()
        };
        // Wait until the watcher (or the change journal) is set up.
        let start = Instant::now();
        while svc
            .live()
            .is_some_and(|l| l.watched_roots + l.journal_drives == 0)
        {
            assert!(start.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(10));
        }
        fs::write(data.join("fresh-note.txt"), "x").unwrap();
        fs::remove_dir_all(data.join("old")).unwrap();
        let start = Instant::now();
        while count("fresh-note") != 1 || count("report") != 0 {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "live update didn't arrive"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let info = svc.live().unwrap();
        assert!(info.changes >= 3, "{info:?}");
        assert_eq!(info.watched_roots + info.journal_drives, 1);
        svc.set_live(None);
        assert!(svc.live().is_none());
    }
}
