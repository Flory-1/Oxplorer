//! Watches the folder shown in the active tab so the view refreshes when
//! files change on disk.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

pub struct DirWatcher {
    watcher: Option<RecommendedWatcher>,
    current: Option<PathBuf>,
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
        if self.current.as_deref() == dir {
            return;
        }
        self.watcher = None;
        self.current = dir.map(Path::to_path_buf);
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
            if w.watch(dir, RecursiveMode::NonRecursive).is_ok() {
                self.watcher = Some(w);
            }
        }
    }

    /// Returns true (once) if the watched folder changed since the last call.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }
}
