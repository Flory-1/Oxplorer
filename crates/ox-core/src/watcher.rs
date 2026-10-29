//! Watches the folder shown in the active tab (or the folders of a library)
//! so the view refreshes when files change on disk.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

pub struct DirWatcher {
    watcher: Option<RecommendedWatcher>,
    /// The watched folders and whether their sub-folders are watched too.
    current: Option<(Vec<PathBuf>, bool)>,
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
        let dirs: Vec<PathBuf> = dir.map(Path::to_path_buf).into_iter().collect();
        self.watch_all(&dirs, recursive);
    }

    /// Watch several folders at once (a library's), replacing the previous
    /// ones; a change in any of them counts. None stops watching.
    pub fn watch_all(&mut self, dirs: &[PathBuf], recursive: bool) {
        let wanted = (!dirs.is_empty()).then(|| (dirs.to_vec(), recursive));
        if self.current == wanted {
            return;
        }
        self.watcher = None;
        self.current = wanted;
        self.dirty.store(false, Ordering::Relaxed);
        if dirs.is_empty() {
            return;
        }

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
            // Folders that can't be watched (gone, offline) are left out.
            let watched = dirs.iter().filter(|d| w.watch(d, mode).is_ok()).count();
            if watched > 0 {
                self.watcher = Some(w);
            }
        }
    }

    /// Returns true (once) if the watched folder changed since the last call.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_change_in_any_of_several_folders_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let mut w = DirWatcher::new(|| {});
        w.watch_all(&[a, b.clone()], false);
        std::fs::write(b.join("new.txt"), "x").unwrap();
        let start = Instant::now();
        while !w.take_dirty() {
            assert!(start.elapsed() < Duration::from_secs(10), "no change seen");
            std::thread::sleep(Duration::from_millis(20));
        }
        // Watching nothing.
        w.watch_all(&[], false);
        assert!(w.current.is_none() && w.watcher.is_none());
    }
}
