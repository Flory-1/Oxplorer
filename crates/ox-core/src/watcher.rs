//! Watches the folder shown in the active tab so the view refreshes when
//! files change on disk.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

pub struct DirWatcher {
    watcher: Option<RecommendedWatcher>,
    /// The watched folder and whether its sub-folders are watched too.
    current: Option<(PathBuf, bool)>,
    dirty: Arc<AtomicBool>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl DirWatcher {
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            watcher: None,
            current: None,
            dirty: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(notify),
        }
    }

    /// Watch `dir` (non-recursively), replacing the previous folder. `None` stops watching.
    pub fn watch(&mut self, dir: Option<&Path>) {
        self.watch_tree(dir, false);
    }

    /// Watch `dir`, with all its sub-folders if `recursive` (for the flat
    /// view), replacing the previous folder. `None` stops watching.
    pub fn watch_tree(&mut self, dir: Option<&Path>, recursive: bool) {
        let wanted = dir.map(|d| (d.to_path_buf(), recursive));
        if self.current == wanted {
            return;
        }
        self.watcher = None;
        self.current = wanted;
        self.dirty.store(false, Ordering::Relaxed);
        let Some(dir) = dir else { return };

        let dirty = self.dirty.clone();
        let notify = self.notify.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if !ev.kind.is_access() {
                    dirty.store(true, Ordering::Relaxed);
                    notify();
                }
            }
        });
        if let Ok(mut w) = watcher {
            let mode = if recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            if w.watch(dir, mode).is_ok() {
                self.watcher = Some(w);
            }
        }
    }

    /// Returns true (once) if the watched folder changed since the last call.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }
}
