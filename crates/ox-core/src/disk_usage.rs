//! Disk usage: one scan of a folder tree into a size tree, and the
//! squarified treemap drawn from it ([`treemap`]).
//!
//! The scan walks the tree once, on one thread (easy on HDDs), adding every
//! folder and its larger files to a [`UsageTree`] that the view reads while
//! it grows: sizes add up as the files are counted. Every finished folder's
//! size also goes into the [`FolderSizes`] cache, so the Size column of the
//! file list knows it without walking the folder again.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::dirsize::{DirSize, FolderSizes};

mod treemap;
pub use treemap::{split_small, squarify, LayoutOptions, Rect, Tile, TileItem};

/// Index of a node in a [`UsageTree`].
pub type NodeId = u32;
/// The scanned folder itself.
pub const ROOT: NodeId = 0;

/// At most this many files of a folder are kept as items of their own (the
/// largest); the others are counted as one "smaller files" item.
const KEEP_FILES: usize = 24;
/// Finished folder sizes are handed to the folder-size cache in batches.
const STORE_BATCH: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Folder,
    File,
    /// The smaller files of a folder, as one item (its name is empty).
    SmallFiles,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: Box<str>,
    pub parent: NodeId,
    pub kind: NodeKind,
    pub bytes: u64,
    /// Files inside (1 for a file).
    pub files: u64,
    pub children: Vec<NodeId>,
    /// A folder whose contents are all counted.
    pub done: bool,
}

/// Sizes of a folder tree: folders and their larger files.
#[derive(Debug, Clone)]
pub struct UsageTree {
    root: PathBuf,
    nodes: Vec<Node>,
}

impl UsageTree {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            nodes: vec![Node {
                name: "".into(),
                parent: ROOT,
                kind: NodeKind::Folder,
                bytes: 0,
                files: 0,
                children: Vec::new(),
                done: false,
            }],
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    /// Full path of a node ("smaller files" have their folder's path).
    pub fn path(&self, mut id: NodeId) -> PathBuf {
        let mut names = Vec::new();
        while id != ROOT {
            let n = self.node(id);
            if !n.name.is_empty() {
                names.push(&*n.name);
            }
            id = n.parent;
        }
        let mut path = self.root.clone();
        path.extend(names.iter().rev());
        path
    }

    /// The node at `path`, if it's in the tree.
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        let rest = path.strip_prefix(&self.root).ok()?;
        let mut id = ROOT;
        for part in rest.components() {
            let name = part.as_os_str();
            id = *self.node(id).children.iter().find(|&&c| {
                let n = self.node(c);
                n.kind != NodeKind::SmallFiles && same_name(&n.name, name)
            })?;
        }
        Some(id)
    }

    /// The items in a folder, largest first.
    pub fn largest_children(&self, id: NodeId) -> Vec<NodeId> {
        let mut kids = self.node(id).children.clone();
        kids.sort_by(|&a, &b| {
            let (a, b) = (self.node(a), self.node(b));
            b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name))
        });
        kids
    }

    /// Add an item to `parent` (its size isn't added to the folders above).
    fn add(
        &mut self,
        parent: NodeId,
        name: &str,
        kind: NodeKind,
        bytes: u64,
        files: u64,
    ) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            name: name.into(),
            parent,
            kind,
            bytes,
            files,
            children: Vec::new(),
            done: kind != NodeKind::Folder,
        });
        self.nodes[parent as usize].children.push(id);
        id
    }

    /// Add to the size of `id` and every folder above it.
    fn grow(&mut self, mut id: NodeId, bytes: u64, files: u64) {
        loop {
            let n = &mut self.nodes[id as usize];
            n.bytes += bytes;
            n.files += files;
            if id == ROOT {
                break;
            }
            id = n.parent;
        }
    }

    /// Take an item out of the tree (it was deleted or moved away).
    pub fn remove(&mut self, id: NodeId) {
        if id == ROOT {
            return;
        }
        let (parent, bytes, files) = {
            let n = self.node(id);
            (n.parent, n.bytes, n.files)
        };
        self.nodes[parent as usize].children.retain(|&c| c != id);
        let mut up = parent;
        loop {
            let n = &mut self.nodes[up as usize];
            n.bytes = n.bytes.saturating_sub(bytes);
            n.files = n.files.saturating_sub(files);
            if up == ROOT {
                break;
            }
            up = n.parent;
        }
    }

    /// Something in `dir` changed: drop the items of it that are gone.
    /// Returns whether any were.
    pub fn forget_missing(&mut self, dir: &Path) -> bool {
        let Some(id) = self.find(dir) else {
            return false;
        };
        let gone: Vec<NodeId> = self
            .node(id)
            .children
            .iter()
            .copied()
            .filter(|&c| self.node(c).kind != NodeKind::SmallFiles)
            .filter(|&c| fs::symlink_metadata(self.path(c)).is_err())
            .collect();
        for &c in &gone {
            self.remove(c);
        }
        !gone.is_empty()
    }
}

/// File names are compared without case on Windows.
fn same_name(a: &str, b: &std::ffi::OsStr) -> bool {
    if cfg!(windows) {
        b.to_str()
            .is_some_and(|b| a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase())
    } else {
        std::ffi::OsStr::new(a) == b
    }
}

struct Shared {
    root: PathBuf,
    tree: Mutex<UsageTree>,
    cancel: AtomicBool,
    done: AtomicBool,
    /// Folders that couldn't be read.
    unreadable: AtomicU64,
    /// Bumped whenever the tree changed.
    version: AtomicU64,
    started: Instant,
    /// How long the scan took, once it ended (done or stopped).
    took: Mutex<Option<Duration>>,
}

/// A scan in progress (or finished). Dropping it stops the walk.
pub struct Scan {
    shared: Arc<Shared>,
}

/// Start scanning `root` in the background. Finished folder sizes go into
/// `sizes`; `notify` is called (now and then) when the tree grew.
pub fn scan(
    root: PathBuf,
    sizes: Option<FolderSizes>,
    notify: impl Fn() + Send + Sync + 'static,
) -> Scan {
    let shared = Arc::new(Shared {
        tree: Mutex::new(UsageTree::new(root.clone())),
        root: root.clone(),
        cancel: AtomicBool::new(false),
        done: AtomicBool::new(false),
        unreadable: AtomicU64::new(0),
        version: AtomicU64::new(0),
        started: Instant::now(),
        took: Mutex::new(None),
    });
    let mut walker = Walker {
        shared: shared.clone(),
        sizes,
        batch: Vec::new(),
        notify: Box::new(notify),
        notified: Instant::now(),
    };
    // Deep trees recurse deeply; give the walker room.
    std::thread::Builder::new()
        .name("disk-usage".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let finished = walker.walk(&root, ROOT).is_some();
            walker.flush();
            let s = &walker.shared;
            *s.took.lock().unwrap() = Some(s.started.elapsed());
            s.done.store(finished, Ordering::Relaxed);
            s.version.fetch_add(1, Ordering::Relaxed);
            (walker.notify)();
        })
        .expect("spawn disk-usage walker");
    Scan { shared }
}

impl Scan {
    pub fn root(&self) -> &Path {
        &self.shared.root
    }

    /// The tree as far as it's scanned (hold the lock only briefly).
    pub fn tree(&self) -> MutexGuard<'_, UsageTree> {
        self.shared.tree.lock().unwrap()
    }

    /// Every folder was counted.
    pub fn is_done(&self) -> bool {
        self.shared.done.load(Ordering::Relaxed)
    }

    /// Still walking.
    pub fn is_running(&self) -> bool {
        self.shared.took.lock().unwrap().is_none()
    }

    /// Stopped before it was done: the sizes are incomplete.
    pub fn is_stopped(&self) -> bool {
        !self.is_running() && !self.is_done()
    }

    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// Folders that couldn't be read (no permission, …).
    pub fn unreadable(&self) -> u64 {
        self.shared.unreadable.load(Ordering::Relaxed)
    }

    /// Changes whenever the tree changed.
    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::Relaxed)
    }

    /// How long the scan has been running, or took.
    pub fn elapsed(&self) -> Duration {
        self.shared
            .took
            .lock()
            .unwrap()
            .unwrap_or_else(|| self.shared.started.elapsed())
    }

    /// Something in `dir` changed: drop what is gone from the tree.
    pub fn forget_missing(&self, dir: &Path) {
        // While walking, the walker still adds to the folders it found.
        if self.is_running() || !dir.starts_with(self.root()) {
            return;
        }
        if self.tree().forget_missing(dir) {
            self.shared.version.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct Walker {
    shared: Arc<Shared>,
    sizes: Option<FolderSizes>,
    batch: Vec<(PathBuf, DirSize)>,
    notify: Box<dyn Fn() + Send + Sync>,
    notified: Instant,
}

impl Walker {
    /// Count `dir` (node `id`) and everything below it; `None` if stopped.
    fn walk(&mut self, dir: &Path, id: NodeId) -> Option<DirSize> {
        if self.shared.cancel.load(Ordering::Relaxed) {
            return None;
        }
        let mut subdirs: Vec<OsString> = Vec::new();
        let mut files: Vec<(OsString, u64)> = Vec::new();
        match fs::read_dir(dir) {
            Ok(rd) => {
                for item in rd.flatten() {
                    let Ok(ft) = item.file_type() else { continue };
                    // `file_type` doesn't follow links, so symlinks and
                    // junctions are neither counted nor entered.
                    if ft.is_dir() {
                        subdirs.push(item.file_name());
                    } else if ft.is_file() {
                        if let Ok(m) = item.metadata() {
                            files.push((item.file_name(), m.len()));
                        }
                    }
                }
            }
            Err(_) => {
                self.shared.unreadable.fetch_add(1, Ordering::Relaxed);
            }
        }
        let own = DirSize {
            bytes: files.iter().map(|(_, b)| b).sum(),
            files: files.len() as u64,
            folders: subdirs.len() as u64,
        };
        files.sort_unstable_by_key(|f| std::cmp::Reverse(f.1));
        // The largest files, if they are a noticeable part of the folder's files.
        let keep = files
            .iter()
            .take(KEEP_FILES)
            .take_while(|(_, b)| *b > 0 && *b >= own.bytes / 100)
            .count();
        let ids: Vec<NodeId> = {
            let mut tree = self.shared.tree.lock().unwrap();
            for (name, bytes) in &files[..keep] {
                tree.add(id, &name.to_string_lossy(), NodeKind::File, *bytes, 1);
            }
            let rest = &files[keep..];
            if !rest.is_empty() {
                let bytes = rest.iter().map(|(_, b)| b).sum();
                tree.add(id, "", NodeKind::SmallFiles, bytes, rest.len() as u64);
            }
            tree.grow(id, own.bytes, own.files);
            subdirs
                .iter()
                .map(|name| tree.add(id, &name.to_string_lossy(), NodeKind::Folder, 0, 0))
                .collect()
        };
        self.changed();

        let mut total = own;
        for (name, child) in subdirs.iter().zip(ids) {
            total += self.walk(&dir.join(name), child)?;
        }
        self.shared.tree.lock().unwrap().nodes[id as usize].done = true;
        if self.sizes.is_some() {
            self.batch.push((dir.to_path_buf(), total));
            if self.batch.len() >= STORE_BATCH {
                self.flush();
            }
        }
        Some(total)
    }

    fn changed(&mut self) {
        self.shared.version.fetch_add(1, Ordering::Relaxed);
        if self.notified.elapsed() >= Duration::from_millis(100) {
            self.notified = Instant::now();
            (self.notify)();
        }
    }

    /// Hand the finished folder sizes to the folder-size cache.
    fn flush(&mut self) {
        if let Some(sizes) = &self.sizes {
            if !self.batch.is_empty() {
                sizes.store(std::mem::take(&mut self.batch));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(scan: &Scan) {
        for _ in 0..500 {
            if !scan.is_running() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("scan didn't finish");
    }

    #[test]
    fn scans_a_tree_once_into_sizes() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        fs::create_dir_all(a.join("b/c")).unwrap();
        fs::write(a.join("big.bin"), [0u8; 1000]).unwrap();
        // Many small files next to a big one are counted as one item.
        for i in 0..30 {
            fs::write(a.join(format!("small{i}.txt")), "x").unwrap();
        }
        fs::write(a.join("b/two.txt"), "123").unwrap();
        fs::write(a.join("b/c/three.txt"), "1").unwrap();

        let sizes = FolderSizes::new(|| {});
        let scan = scan(a.clone(), Some(sizes.clone()), || {});
        wait(&scan);
        assert!(scan.is_done() && !scan.is_stopped());
        let tree = scan.tree();
        let root = tree.node(ROOT);
        assert_eq!(root.bytes, 1000 + 30 + 3 + 1);
        assert_eq!(root.files, 33);

        let kids = tree.largest_children(ROOT);
        let names: Vec<_> = kids.iter().map(|&k| tree.node(k)).collect();
        assert_eq!(&*names[0].name, "big.bin");
        let small = names
            .iter()
            .find(|n| n.kind == NodeKind::SmallFiles)
            .unwrap();
        assert_eq!((small.bytes, small.files), (30, 30));
        assert_eq!(kids.len(), 3, "big.bin, the small files and b");

        let c = tree.find(&a.join("b").join("c")).unwrap();
        assert_eq!(tree.node(c).bytes, 1);
        assert_eq!(tree.path(c), a.join("b").join("c"));
        assert!(tree.node(c).done);
        assert_eq!(tree.find(&a), Some(ROOT));
        assert_eq!(tree.find(&a.join("nope")), None);
        drop(tree);

        // The folder-size cache learned every folder from the one walk.
        assert_eq!(sizes.get(&a).unwrap().bytes, 1034);
        assert_eq!(sizes.get(&a.join("b")).unwrap().bytes, 4);
        assert_eq!(sizes.get(&a.join("b/c")).unwrap().folders, 0);
    }

    #[test]
    fn forgets_deleted_items() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().to_path_buf();
        fs::create_dir_all(a.join("b")).unwrap();
        fs::write(a.join("b/two.bin"), [0u8; 200]).unwrap();
        fs::write(a.join("one.bin"), [0u8; 100]).unwrap();
        let scan = scan(a.clone(), None, || {});
        wait(&scan);
        assert_eq!(scan.tree().node(ROOT).bytes, 300);

        let v = scan.version();
        fs::remove_dir_all(a.join("b")).unwrap();
        scan.forget_missing(&a);
        assert!(scan.version() > v);
        let tree = scan.tree();
        assert_eq!(tree.node(ROOT).bytes, 100);
        assert_eq!(tree.node(ROOT).files, 1);
        assert_eq!(tree.find(&a.join("b")), None);
    }

    #[test]
    fn a_stopped_scan_keeps_what_it_has() {
        let tmp = tempfile::tempdir().unwrap();
        let scan = scan(tmp.path().to_path_buf(), None, || {});
        scan.cancel();
        wait(&scan);
        // Either it was quick enough to finish or it stopped; it never hangs.
        assert!(scan.is_done() != scan.is_stopped());
    }
}
