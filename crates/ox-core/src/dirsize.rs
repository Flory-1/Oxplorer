//! Folder sizes for the Size column, computed in the background.
//!
//! One worker walks the requested folders, one after another (easy on
//! HDDs). Every sub-folder it passes is cached too, so opening a folder
//! whose parent was measured shows its sizes at once. Cached values are
//! reused while fresh and shown (while being refreshed) when older.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::ops::AddAssign;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Values younger than this are not measured again.
const FRESH: Duration = Duration::from_secs(10 * 60);
/// Drop the cache when it grows beyond this many folders.
const MAX_CACHED: usize = 500_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirSize {
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
}

impl AddAssign for DirSize {
    fn add_assign(&mut self, o: Self) {
        self.bytes += o.bytes;
        self.files += o.files;
        self.folders += o.folders;
    }
}

#[derive(Default)]
struct Queue {
    pending: VecDeque<PathBuf>,
    /// Folder the worker is measuring right now.
    current: Option<PathBuf>,
}

struct Inner {
    cache: Mutex<HashMap<PathBuf, (DirSize, Instant)>>,
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Stops the walk of `current` (it is no longer wanted).
    cancel: AtomicBool,
    /// Bumped whenever a requested folder finished.
    version: AtomicU64,
    /// Bumped whenever cached values are dropped.
    epoch: AtomicU64,
    notify: Box<dyn Fn() + Send + Sync>,
}

/// Background folder-size calculator with a shared cache.
#[derive(Clone)]
pub struct FolderSizes {
    inner: Arc<Inner>,
}

impl FolderSizes {
    /// Start the worker. `notify` is called whenever new sizes are available.
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        let inner = Arc::new(Inner {
            cache: Mutex::new(HashMap::new()),
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            cancel: AtomicBool::new(false),
            version: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            notify: Box::new(notify),
        });
        let worker = inner.clone();
        // Deep trees recurse deeply; give the walker room.
        std::thread::Builder::new()
            .name("folder-sizes".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || worker.run())
            .expect("spawn folder-size worker");
        Self { inner }
    }

    /// Measure these folders (replacing earlier requests). Folders with a
    /// fresh cached size are skipped; a walk of a folder that is no longer
    /// wanted is stopped.
    pub fn request(&self, dirs: Vec<PathBuf>) {
        let todo: VecDeque<PathBuf> = {
            let cache = self.inner.cache.lock().unwrap();
            dirs.into_iter()
                .filter(|d| !cache.get(d).is_some_and(|(_, at)| at.elapsed() < FRESH))
                .collect()
        };
        let mut q = self.inner.queue.lock().unwrap();
        if q.current.as_ref().is_some_and(|c| !todo.contains(c)) {
            self.inner.cancel.store(true, Ordering::Relaxed);
        }
        q.pending = todo;
        self.inner.wake.notify_one();
    }

    /// Stop all pending work.
    pub fn clear_requests(&self) {
        self.request(Vec::new());
    }

    /// Last known size of `dir` (possibly being refreshed).
    pub fn get(&self, dir: &Path) -> Option<DirSize> {
        self.inner.cache.lock().unwrap().get(dir).map(|(s, _)| *s)
    }

    /// Whether `dir` is queued or being measured.
    pub fn is_pending(&self, dir: &Path) -> bool {
        let q = self.inner.queue.lock().unwrap();
        q.current.as_deref() == Some(dir) || q.pending.iter().any(|p| p == dir)
    }

    /// Changes whenever a requested folder finished.
    pub fn version(&self) -> u64 {
        self.inner.version.load(Ordering::Relaxed)
    }

    /// Changes whenever cached sizes are dropped (time to request again).
    pub fn epoch(&self) -> u64 {
        self.inner.epoch.load(Ordering::Relaxed)
    }

    /// Something inside `path` changed: forget its size and its ancestors'.
    pub fn invalidate(&self, path: &Path) {
        let mut cache = self.inner.cache.lock().unwrap();
        for p in path.ancestors() {
            cache.remove(p);
        }
        drop(cache);
        self.inner.epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Forget `path`, everything below it and its ancestors (e.g. on F5).
    pub fn invalidate_tree(&self, path: &Path) {
        let mut cache = self.inner.cache.lock().unwrap();
        cache.retain(|p, _| !p.starts_with(path));
        for p in path.ancestors() {
            cache.remove(p);
        }
        drop(cache);
        self.inner.epoch.fetch_add(1, Ordering::Relaxed);
    }
}

impl Inner {
    fn run(&self) {
        loop {
            let dir = {
                let mut q = self.queue.lock().unwrap();
                q.current = None;
                loop {
                    if let Some(d) = q.pending.pop_front() {
                        q.current = Some(d.clone());
                        self.cancel.store(false, Ordering::Relaxed);
                        break d;
                    }
                    q = self.wake.wait(q).unwrap();
                }
            };
            if self.walk(&dir).is_some() {
                self.version.fetch_add(1, Ordering::Relaxed);
                (self.notify)();
            }
        }
    }

    fn fresh(&self, dir: &Path) -> Option<DirSize> {
        let cache = self.cache.lock().unwrap();
        cache
            .get(dir)
            .filter(|(_, at)| at.elapsed() < FRESH)
            .map(|(s, _)| *s)
    }

    /// Size of `dir`, or `None` if the walk was cancelled.
    fn walk(&self, dir: &Path) -> Option<DirSize> {
        if self.cancel.load(Ordering::Relaxed) {
            return None;
        }
        if let Some(s) = self.fresh(dir) {
            return Some(s);
        }
        let mut total = DirSize::default();
        if let Ok(rd) = fs::read_dir(dir) {
            for item in rd.flatten() {
                let Ok(ft) = item.file_type() else { continue };
                // `file_type` doesn't follow links, so symlinks and
                // junctions are neither counted nor entered.
                if ft.is_dir() {
                    total.folders += 1;
                    total += self.walk(&item.path())?;
                } else if ft.is_file() {
                    if let Ok(m) = item.metadata() {
                        total.files += 1;
                        total.bytes += m.len();
                    }
                }
            }
        }
        let mut cache = self.cache.lock().unwrap();
        if cache.len() >= MAX_CACHED {
            cache.clear();
        }
        cache.insert(dir.to_path_buf(), (total, Instant::now()));
        Some(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(sizes: &FolderSizes, dir: &Path) -> DirSize {
        for _ in 0..500 {
            if let Some(s) = sizes.get(dir) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no size for {}", dir.display());
    }

    #[test]
    fn measures_trees_and_caches_sub_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        fs::create_dir_all(a.join("b/c")).unwrap();
        fs::write(a.join("one.txt"), "12345").unwrap();
        fs::write(a.join("b/two.txt"), "123").unwrap();
        fs::write(a.join("b/c/three.txt"), "1").unwrap();

        let sizes = FolderSizes::new(|| {});
        sizes.request(vec![a.clone()]);
        let s = wait_for(&sizes, &a);
        assert_eq!(
            s,
            DirSize {
                bytes: 9,
                files: 3,
                folders: 2
            }
        );
        // Sub-folders were measured on the way.
        assert_eq!(sizes.get(&a.join("b")).unwrap().bytes, 4);
        assert_eq!(sizes.get(&a.join("b/c")).unwrap().bytes, 1);
        assert!(sizes.version() >= 1);
    }

    #[test]
    fn invalidate_drops_the_folder_and_its_ancestors() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        fs::create_dir_all(a.join("b/c")).unwrap();
        let sizes = FolderSizes::new(|| {});
        sizes.request(vec![a.clone()]);
        wait_for(&sizes, &a);

        let epoch = sizes.epoch();
        sizes.invalidate(&a.join("b"));
        assert!(sizes.get(&a).is_none());
        assert!(sizes.get(&a.join("b")).is_none());
        assert!(sizes.get(&a.join("b/c")).is_some());
        assert!(sizes.epoch() > epoch);

        sizes.invalidate_tree(&a);
        assert!(sizes.get(&a.join("b/c")).is_none());

        // Measured again after a new request, including new files.
        fs::write(a.join("b/c/new.bin"), [0u8; 10]).unwrap();
        sizes.request(vec![a.clone()]);
        assert_eq!(wait_for(&sizes, &a).bytes, 10);
    }
}
