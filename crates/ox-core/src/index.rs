//! A persistent index of file names for instant search across whole drives.
//!
//! The index stores one compact 24-byte record per file or folder plus all
//! names in a single string buffer, so a million files take roughly 50 MB.
//! Queries run on all CPU cores and only reconstruct full paths for hits.
//! The index is saved to the cache folder, so search works instantly right
//! after startup while a fresh index is built in the background.
//!
//! Building uses a portable parallel directory walk. On Windows, whole NTFS
//! drives are read from the master file table instead (`mft`), which is many
//! times faster: through the index service (`helper`) when it is installed
//! and turned on, or directly when Oxplorer runs as administrator (`volume`).
//! Between builds, file changes are applied as they happen (`live`,
//! `patch`): from the NTFS change journal where it can be read (`journal`),
//! otherwise from a file watcher. The journal positions are saved with the
//! index (`checkpoint`), so the next start catches up on what changed while
//! Oxplorer was closed instead of rebuilding (`catchup`).

#[cfg_attr(not(windows), allow(dead_code))]
mod catchup;
mod checkpoint;
pub mod helper;
#[cfg(windows)]
mod journal;
mod live;
#[cfg(windows)]
mod mft;
// Only used by the Windows reader, but tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
mod ntfs;
mod patch;
mod service;
#[cfg_attr(not(windows), allow(dead_code))]
mod tree;
#[cfg_attr(not(windows), allow(dead_code))]
mod usn;
#[cfg_attr(not(windows), allow(dead_code))]
mod volume;

pub use live::LiveInfo;
pub use service::{default_cache_path, IndexService, IndexState};

use std::fs;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::entry::{self, FileEntry};
use crate::search::Query;

const NO_PARENT: u32 = u32::MAX;
const F_DIR: u8 = 1;
/// The item or one of its parent folders is hidden.
const F_HIDDEN: u8 = 2;
/// Removed by a live update; skipped by queries until the index is compacted.
const F_DELETED: u8 = 4;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Entry {
    size: u64,
    name_off: u32,
    parent: u32,
    /// Seconds since 1970 (0 = unknown).
    mtime: u32,
    name_len: u16,
    flags: u8,
}

/// What to index.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexConfig {
    pub roots: Vec<PathBuf>,
    /// Folder or file names to skip entirely (case-insensitive), e.g. `node_modules`.
    pub exclude: Vec<String>,
    /// Worker threads, 0 = one per CPU core.
    pub threads: usize,
}

/// Options for one index query.
#[derive(Debug, Clone, Default)]
pub struct IndexQuery {
    pub include_hidden: bool,
    pub max_results: usize,
    /// Only return items inside this folder.
    pub within: Option<PathBuf>,
    /// Stop early when set (e.g. the user typed the next character).
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Clone)]
pub struct FileIndex {
    roots: Vec<PathBuf>,
    names: String,
    entries: Vec<Entry>,
    built: SystemTime,
    /// Entries flagged `F_DELETED`.
    deleted: usize,
    /// Entries removed or renamed since the last compaction (dead records
    /// and names that still take memory).
    garbage: usize,
    /// Roots read from the NTFS master file table (not saved).
    mft_roots: usize,
}

fn threads(n: usize) -> usize {
    if n > 0 {
        n
    } else {
        std::thread::available_parallelism().map_or(4, |n| n.get())
    }
}

fn to_secs(t: Option<SystemTime>) -> u32 {
    t.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs().min(u32::MAX as u64) as u32)
}

/// Collects entries from all walker threads.
#[derive(Default)]
struct Builder {
    names: String,
    entries: Vec<Entry>,
}

impl Builder {
    fn push(&mut self, name: &str, parent: u32, flags: u8, size: u64, mtime: u32) -> u32 {
        push_entry(
            &mut self.names,
            &mut self.entries,
            name,
            parent,
            flags,
            size,
            mtime,
        )
    }
}

fn push_entry(
    names: &mut String,
    entries: &mut Vec<Entry>,
    name: &str,
    parent: u32,
    flags: u8,
    size: u64,
    mtime: u32,
) -> u32 {
    // Names longer than u16::MAX bytes can't exist on any common file system.
    let mut len = name.len().min(u16::MAX as usize);
    while !name.is_char_boundary(len) {
        len -= 1;
    }
    let name = &name[..len];
    let idx = entries.len() as u32;
    entries.push(Entry {
        size,
        name_off: names.len() as u32,
        parent,
        mtime,
        name_len: name.len() as u16,
        flags,
    });
    names.push_str(name);
    idx
}

/// Full path of entry `i`: its name and the names of all its parents.
fn path_in(names: &str, entries: &[Entry], i: u32) -> PathBuf {
    let mut chain = Vec::new();
    let mut cur = i;
    while cur != NO_PARENT {
        let e = &entries[cur as usize];
        let start = e.name_off as usize;
        chain.push(&names[start..start + e.name_len as usize]);
        cur = e.parent;
    }
    let mut path = PathBuf::new();
    for part in chain.iter().rev() {
        path.push(part);
    }
    path
}

#[derive(Debug, Clone, PartialEq)]
struct Child {
    name: String,
    flags: u8,
    size: u64,
    mtime: u32,
}

impl Child {
    /// The index record for `name`, or `None` if it is excluded.
    fn new(
        name: String,
        meta: &fs::Metadata,
        parent_hidden: bool,
        exclude: &[String],
    ) -> Option<Self> {
        if is_excluded(&name, exclude) {
            return None;
        }
        let ft = meta.file_type();
        // Linked folders are indexed as entries but not followed (no loops, no duplicates).
        let is_dir = ft.is_dir() && !ft.is_symlink();
        let hidden = parent_hidden || entry::is_hidden(&name, meta);
        let mut flags = 0;
        if is_dir {
            flags |= F_DIR;
        }
        if hidden {
            flags |= F_HIDDEN;
        }
        Some(Child {
            size: if is_dir { 0 } else { meta.len() },
            mtime: to_secs(meta.modified().ok()),
            name,
            flags,
        })
    }
}

/// `exclude` holds lowercase names.
fn is_excluded(name: &str, exclude: &[String]) -> bool {
    !exclude.is_empty() && exclude.contains(&name.to_lowercase())
}

/// The indexable items in `dir`.
fn read_children(dir: &Path, parent_hidden: bool, exclude: &[String]) -> Vec<Child> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            // `DirEntry::metadata` doesn't follow links (and needs no extra call on Windows).
            let meta = item.metadata().ok()?;
            Child::new(name, &meta, parent_hidden, exclude)
        })
        .collect()
}

impl FileIndex {
    /// Walk all roots in parallel. `progress` counts indexed items; setting
    /// `cancel` stops early (the partial index is returned).
    pub fn build(cfg: &IndexConfig, cancel: &AtomicBool, progress: &AtomicU64) -> FileIndex {
        Self::build_with(cfg, cancel, progress, false)
    }

    /// Like [`FileIndex::build`], but roots that are whole NTFS drives are read
    /// from the master file table when the volume can be opened (Windows, as
    /// administrator). Everything else is walked as usual.
    pub fn build_fast(cfg: &IndexConfig, cancel: &AtomicBool, progress: &AtomicU64) -> FileIndex {
        Self::build_with(cfg, cancel, progress, true)
    }

    fn build_with(
        cfg: &IndexConfig,
        cancel: &AtomicBool,
        progress: &AtomicU64,
        fast: bool,
    ) -> FileIndex {
        let exclude: Vec<String> = cfg.exclude.iter().map(|e| e.to_lowercase()).collect();
        let mut builder = Builder::default();
        let (tx, rx) = crossbeam_channel::unbounded::<(PathBuf, u32, bool)>();
        // Folders queued or being read; the walk is done when it reaches zero.
        let pending = AtomicUsize::new(0);
        let mut roots = Vec::new();
        let mut mft_roots = 0;
        for root in &cfg.roots {
            if !root.is_dir() {
                continue;
            }
            let name = root.to_string_lossy();
            let idx = builder.push(&name, NO_PARENT, F_DIR, 0, 0);
            roots.push(root.clone());
            if fast && index_volume(root, idx, &exclude, &mut builder, cancel, progress) {
                mft_roots += 1;
                continue;
            }
            pending.fetch_add(1, Ordering::SeqCst);
            let _ = tx.send((root.clone(), idx, false));
        }

        let builder = Mutex::new(builder);
        std::thread::scope(|scope| {
            for _ in 0..threads(cfg.threads) {
                let (tx, rx) = (tx.clone(), rx.clone());
                let (builder, pending, exclude) = (&builder, &pending, &exclude);
                scope.spawn(move || loop {
                    match rx.recv_timeout(Duration::from_millis(10)) {
                        Ok((dir, idx, hidden)) => {
                            if !cancel.load(Ordering::Relaxed) {
                                index_dir(
                                    &dir, idx, hidden, exclude, builder, pending, &tx, progress,
                                );
                            }
                            pending.fetch_sub(1, Ordering::SeqCst);
                        }
                        Err(_) if pending.load(Ordering::SeqCst) == 0 => break,
                        Err(_) => {}
                    }
                });
            }
        });

        let b = builder.into_inner().unwrap();
        FileIndex {
            roots,
            names: b.names,
            entries: b.entries,
            built: SystemTime::now(),
            deleted: 0,
            garbage: 0,
            mft_roots,
        }
    }

    /// Number of indexed items.
    pub fn len(&self) -> usize {
        self.entries.len() - self.deleted
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many roots were read from the NTFS master file table by the build
    /// that made this index (0 for an index loaded from the cache).
    pub fn mft_roots(&self) -> usize {
        self.mft_roots
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn built_at(&self) -> SystemTime {
        self.built
    }

    /// Approximate memory use in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.names.capacity() + self.entries.capacity() * std::mem::size_of::<Entry>()
    }

    /// Is `path` inside one of the indexed roots?
    pub fn covers(&self, path: &Path) -> bool {
        self.roots.iter().any(|r| path.starts_with(r))
    }

    fn name(&self, e: &Entry) -> &str {
        let start = e.name_off as usize;
        &self.names[start..start + e.name_len as usize]
    }

    /// Full path of entry `i`.
    pub fn path_of(&self, i: u32) -> PathBuf {
        path_in(&self.names, &self.entries, i)
    }

    /// The entry as shown in the file view.
    pub fn file_entry(&self, i: u32) -> FileEntry {
        let e = &self.entries[i as usize];
        FileEntry {
            path: self.path_of(i),
            name: self.name(e).to_owned(),
            is_dir: e.flags & F_DIR != 0,
            is_symlink: false,
            size: e.size,
            modified: (e.mtime > 0).then(|| UNIX_EPOCH + Duration::from_secs(e.mtime as u64)),
            created: None,
            hidden: e.flags & F_HIDDEN != 0,
            readonly: false,
            cloud: None,
        }
    }

    /// Index of the folder at `path`, if it is indexed.
    fn find_dir(&self, path: &Path) -> Option<u32> {
        let want = path.file_name().map(|n| n.to_string_lossy());
        self.entries.iter().enumerate().find_map(|(i, e)| {
            let i = i as u32;
            // Roots are stored under their full path, everything else by name.
            let candidate = e.flags & (F_DIR | F_DELETED) == F_DIR
                && (e.parent == NO_PARENT
                    || want.as_ref().is_some_and(|n| self.name(e) == n.as_ref()));
            (candidate && self.path_of(i) == path).then_some(i)
        })
    }

    /// How many items a query over `within` (all of the index if `None`)
    /// looks at: what a search there reports as scanned.
    pub fn count_within(&self, within: Option<&Path>, include_hidden: bool) -> usize {
        let Some(path) = within else {
            return self.len();
        };
        let Some(dir) = self.find_dir(path) else {
            return 0;
        };
        let skip = if include_hidden {
            F_DELETED
        } else {
            F_DELETED | F_HIDDEN
        };
        let chunk = self.entries.len().div_ceil(threads(0)).max(1);
        std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .entries
                .chunks(chunk)
                .enumerate()
                .map(|(c, entries)| {
                    scope.spawn(move || {
                        let base = c * chunk;
                        entries
                            .iter()
                            .enumerate()
                            .filter(|(k, e)| {
                                e.parent != NO_PARENT
                                    && e.flags & skip == 0
                                    && self.is_inside((base + k) as u32, dir)
                            })
                            .count()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).sum()
        })
    }

    fn is_inside(&self, mut i: u32, ancestor: u32) -> bool {
        while i != NO_PARENT {
            i = self.entries[i as usize].parent;
            if i == ancestor {
                return true;
            }
        }
        false
    }

    /// Find matching entries. Returns their ids (see [`FileIndex::file_entry`])
    /// and whether the result was cut off at `max_results`.
    pub fn search(&self, query: &Query, opts: &IndexQuery) -> (Vec<u32>, bool) {
        let within = match &opts.within {
            Some(p) => match self.find_dir(p) {
                Some(i) => Some(i),
                None => return (Vec::new(), false),
            },
            None => None,
        };
        let max = opts.max_results.max(1);
        let skip = if opts.include_hidden {
            F_DELETED
        } else {
            F_DELETED | F_HIDDEN
        };
        let found = AtomicUsize::new(0);
        let n = threads(0);
        let chunk = self.entries.len().div_ceil(n).max(1);
        let mut parts: Vec<Vec<u32>> = std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .entries
                .chunks(chunk)
                .enumerate()
                .map(|(c, entries)| {
                    let found = &found;
                    scope.spawn(move || {
                        let mut hits = Vec::new();
                        let base = c * chunk;
                        for (k, e) in entries.iter().enumerate() {
                            if k % 4096 == 0
                                && opts
                                    .cancel
                                    .as_ref()
                                    .is_some_and(|c| c.load(Ordering::Relaxed))
                            {
                                break;
                            }
                            if e.parent == NO_PARENT || e.flags & skip != 0 {
                                continue;
                            }
                            let mtime = (e.mtime > 0)
                                .then(|| UNIX_EPOCH + Duration::from_secs(e.mtime as u64));
                            let is_dir = e.flags & F_DIR != 0;
                            if !query.matches_meta(self.name(e), is_dir, e.size, mtime) {
                                continue;
                            }
                            let i = (base + k) as u32;
                            if within.is_some_and(|w| !self.is_inside(i, w)) {
                                continue;
                            }
                            if query.needs_content()
                                && (is_dir || !query.matches_content(&self.path_of(i)))
                            {
                                continue;
                            }
                            if query.has_tags() && !query.matches_tags(&self.path_of(i)) {
                                continue;
                            }
                            hits.push(i);
                            if found.fetch_add(1, Ordering::Relaxed) > max {
                                break;
                            }
                        }
                        hits
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let mut all: Vec<u32> = parts.iter_mut().flat_map(std::mem::take).collect();
        let truncated = all.len() > max;
        all.truncate(max);
        (all, truncated)
    }

    // ---- persistence -------------------------------------------------------

    const MAGIC: &'static [u8; 8] = b"OXIDX\0\0\x01";

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        {
            let mut w = BufWriter::new(fs::File::create(&tmp)?);
            w.write_all(Self::MAGIC)?;
            w.write_all(
                &(self
                    .built
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()))
                .to_le_bytes(),
            )?;
            w.write_all(&(self.roots.len() as u32).to_le_bytes())?;
            for r in &self.roots {
                let s = r.to_string_lossy();
                w.write_all(&(s.len() as u32).to_le_bytes())?;
                w.write_all(s.as_bytes())?;
            }
            w.write_all(&(self.names.len() as u64).to_le_bytes())?;
            w.write_all(self.names.as_bytes())?;
            w.write_all(&(self.entries.len() as u64).to_le_bytes())?;
            for e in &self.entries {
                w.write_all(&e.size.to_le_bytes())?;
                w.write_all(&e.name_off.to_le_bytes())?;
                w.write_all(&e.parent.to_le_bytes())?;
                w.write_all(&e.mtime.to_le_bytes())?;
                w.write_all(&e.name_len.to_le_bytes())?;
                w.write_all(&[e.flags])?;
            }
            w.flush()?;
        }
        fs::rename(&tmp, path)
    }

    pub fn load(path: &Path) -> io::Result<FileIndex> {
        fn bad(msg: &str) -> io::Error {
            io::Error::new(io::ErrorKind::InvalidData, format!("index file: {msg}"))
        }
        fn read_u32(r: &mut impl Read) -> io::Result<u32> {
            let mut b = [0; 4];
            r.read_exact(&mut b)?;
            Ok(u32::from_le_bytes(b))
        }
        fn read_u64(r: &mut impl Read) -> io::Result<u64> {
            let mut b = [0; 8];
            r.read_exact(&mut b)?;
            Ok(u64::from_le_bytes(b))
        }
        fn read_bytes(r: &mut impl Read, len: u64, limit: u64) -> io::Result<Vec<u8>> {
            if len > limit {
                return Err(bad("length out of range"));
            }
            // Exactly as big as needed: growing while reading would leave up
            // to twice the names' size reserved for as long as the index lives.
            let mut v = Vec::with_capacity(len as usize);
            r.take(len).read_to_end(&mut v)?;
            if v.len() as u64 != len {
                return Err(bad("truncated"));
            }
            Ok(v)
        }

        let file = fs::File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut r = BufReader::new(file);
        let mut magic = [0; 8];
        r.read_exact(&mut magic)?;
        if &magic != Self::MAGIC {
            return Err(bad("unknown format"));
        }
        let built = UNIX_EPOCH + Duration::from_secs(read_u64(&mut r)?);
        let root_count = read_u32(&mut r)?;
        let mut roots = Vec::new();
        for _ in 0..root_count.min(1024) {
            let len = read_u32(&mut r)? as u64;
            let bytes = read_bytes(&mut r, len, 64 * 1024)?;
            roots.push(PathBuf::from(
                String::from_utf8(bytes).map_err(|_| bad("root"))?,
            ));
        }
        let names_len = read_u64(&mut r)?;
        let names = String::from_utf8(read_bytes(&mut r, names_len, file_len)?)
            .map_err(|_| bad("names"))?;
        let count = read_u64(&mut r)?;
        let raw = read_bytes(&mut r, count.saturating_mul(23), file_len)?;
        let mut entries = Vec::with_capacity(count as usize);
        for b in raw.as_chunks::<23>().0 {
            let e = Entry {
                size: u64::from_le_bytes(b[0..8].try_into().unwrap()),
                name_off: u32::from_le_bytes(b[8..12].try_into().unwrap()),
                parent: u32::from_le_bytes(b[12..16].try_into().unwrap()),
                mtime: u32::from_le_bytes(b[16..20].try_into().unwrap()),
                name_len: u16::from_le_bytes(b[20..22].try_into().unwrap()),
                flags: b[22],
            };
            let end = e.name_off as usize + e.name_len as usize;
            if end > names.len()
                || !names.is_char_boundary(e.name_off as usize)
                || !names.is_char_boundary(end)
                || (e.parent != NO_PARENT && e.parent as u64 >= count)
                || (e.parent != NO_PARENT && e.parent as usize >= entries.len())
            {
                // Parents always come before their children, which also rules out cycles.
                return Err(bad("corrupt entry"));
            }
            entries.push(e);
        }
        let deleted = entries.iter().filter(|e| e.flags & F_DELETED != 0).count();
        Ok(FileIndex {
            roots,
            names,
            entries,
            built,
            deleted,
            garbage: deleted,
            mft_roots: 0,
        })
    }
}

#[cfg(not(windows))]
fn index_volume(
    _root: &Path,
    _idx: u32,
    _exclude: &[String],
    _builder: &mut Builder,
    _cancel: &AtomicBool,
    _progress: &AtomicU64,
) -> bool {
    false
}

#[cfg(windows)]
use mft::index_volume;

#[allow(clippy::too_many_arguments)]
fn index_dir(
    dir: &Path,
    idx: u32,
    parent_hidden: bool,
    exclude: &[String],
    builder: &Mutex<Builder>,
    pending: &AtomicUsize,
    tx: &crossbeam_channel::Sender<(PathBuf, u32, bool)>,
    progress: &AtomicU64,
) {
    let children = read_children(dir, parent_hidden, exclude);
    progress.fetch_add(children.len() as u64, Ordering::Relaxed);
    let mut subdirs = Vec::new();
    {
        let mut b = builder.lock().unwrap();
        for c in &children {
            let i = b.push(&c.name, idx, c.flags, c.size, c.mtime);
            if c.flags & F_DIR != 0 {
                subdirs.push((dir.join(&c.name), i, c.flags & F_HIDDEN != 0));
            }
        }
    }
    pending.fetch_add(subdirs.len(), Ordering::SeqCst);
    for s in subdirs {
        let _ = tx.send(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree(root: &Path) {
        fs::create_dir_all(root.join("Documents/Taxes 2026")).unwrap();
        fs::create_dir_all(root.join("Pictures/Vacation")).unwrap();
        fs::create_dir_all(root.join(".cache/thumbs")).unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::write(
            root.join("Documents/Taxes 2026/receipt.pdf"),
            vec![0u8; 5000],
        )
        .unwrap();
        fs::write(root.join("Documents/notes.txt"), "remember the TODO").unwrap();
        fs::write(root.join("Pictures/Vacation/IMG_2041.jpg"), [1u8; 10]).unwrap();
        fs::write(root.join("Pictures/Vacation/IMG_2042.jpg"), [1u8; 10]).unwrap();
        fs::write(root.join(".cache/thumbs/IMG_2041.jpg"), [1u8; 10]).unwrap();
        fs::write(root.join("node_modules/pkg/index.js"), "x").unwrap();
        // On Windows a leading dot doesn't hide a folder; the attribute does.
        #[cfg(windows)]
        std::process::Command::new("attrib")
            .arg("+h")
            .arg(root.join(".cache"))
            .status()
            .unwrap();
    }

    fn build(root: &Path) -> FileIndex {
        let cfg = IndexConfig {
            roots: vec![root.to_path_buf()],
            exclude: vec!["NODE_MODULES".into()],
            threads: 3,
        };
        FileIndex::build(&cfg, &AtomicBool::new(false), &AtomicU64::new(0))
    }

    fn names(idx: &FileIndex, q: &str, opts: &IndexQuery) -> Vec<String> {
        let (ids, _) = idx.search(&Query::parse(q, false), opts);
        let mut v: Vec<String> = ids.into_iter().map(|i| idx.file_entry(i).name).collect();
        v.sort();
        v
    }

    fn opts() -> IndexQuery {
        IndexQuery {
            include_hidden: false,
            max_results: 1000,
            within: None,
            cancel: None,
        }
    }

    #[test]
    fn cancelled_search_stops() {
        let dir = tempfile::tempdir().unwrap();
        sample_tree(dir.path());
        let idx = build(dir.path());
        let cancel = Arc::new(AtomicBool::new(true));
        let (ids, _) = idx.search(
            &Query::parse("img", false),
            &IndexQuery {
                cancel: Some(cancel),
                ..opts()
            },
        );
        assert!(ids.is_empty());
    }

    #[test]
    fn build_and_query() {
        let dir = tempfile::tempdir().unwrap();
        sample_tree(dir.path());
        let idx = build(dir.path());
        // 1 root + Documents, Taxes 2026, receipt, notes, Pictures, Vacation, 2 jpgs,
        // .cache, thumbs, hidden jpg (node_modules is excluded).
        assert_eq!(idx.len(), 12);
        assert!(idx.covers(&dir.path().join("Pictures")));

        assert_eq!(
            names(&idx, "img_", &opts()),
            vec!["IMG_2041.jpg", "IMG_2042.jpg"]
        );
        assert_eq!(
            names(&idx, "kind:document", &opts()),
            vec!["notes.txt", "receipt.pdf"]
        );
        assert_eq!(names(&idx, "size:>1kb", &opts()), vec!["receipt.pdf"]);
        assert_eq!(
            names(&idx, "type:folder taxes", &opts()),
            vec!["Taxes 2026"]
        );
        assert_eq!(names(&idx, "content:todo", &opts()), vec!["notes.txt"]);
        assert!(
            names(&idx, "index.js", &opts()).is_empty(),
            "excluded folder"
        );

        let hidden = IndexQuery {
            include_hidden: true,
            ..opts()
        };
        assert_eq!(names(&idx, "IMG_2041", &hidden).len(), 2, "inside .cache");

        let within = IndexQuery {
            within: Some(dir.path().join("Documents")),
            ..opts()
        };
        assert_eq!(
            names(&idx, "*", &within),
            vec!["Taxes 2026", "notes.txt", "receipt.pdf"]
        );
        // A search in a folder reports that folder's items as scanned.
        let documents = dir.path().join("Documents");
        assert_eq!(idx.count_within(Some(&documents), false), 3);
        assert_eq!(idx.count_within(None, false), idx.len());
        assert_eq!(idx.count_within(Some(&dir.path().join("nope")), false), 0);

        // Searching in an indexed root itself (e.g. the home folder).
        let in_root = IndexQuery {
            within: Some(dir.path().to_path_buf()),
            ..opts()
        };
        assert_eq!(names(&idx, "*.pdf", &in_root), vec!["receipt.pdf"]);

        let (ids, _) = idx.search(&Query::parse("receipt", false), &opts());
        let e = idx.file_entry(ids[0]);
        assert_eq!(e.path, dir.path().join("Documents/Taxes 2026/receipt.pdf"));
        assert_eq!(e.size, 5000);
        assert!(e.modified.is_some());
    }

    #[test]
    fn truncates_results() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..50 {
            fs::write(dir.path().join(format!("f{i}.txt")), "").unwrap();
        }
        let idx = build(dir.path());
        let (ids, truncated) = idx.search(
            &Query::parse("f", false),
            &IndexQuery {
                max_results: 10,
                ..opts()
            },
        );
        assert_eq!(ids.len(), 10);
        assert!(truncated);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        sample_tree(dir.path());
        let idx = build(dir.path());
        let file = dir.path().join("cache/index.bin");
        idx.save(&file).unwrap();
        let loaded = FileIndex::load(&file).unwrap();
        assert_eq!(loaded.entries, idx.entries);
        assert_eq!(loaded.names, idx.names);
        assert_eq!(loaded.roots, idx.roots);
        assert_eq!(names(&loaded, "img_2041", &opts()), vec!["IMG_2041.jpg"]);

        // Corrupt files are rejected instead of crashing.
        let mut bytes = fs::read(&file).unwrap();
        let n = bytes.len();
        bytes[n - 10] = 0xff;
        bytes[n - 9] = 0xff;
        fs::write(&file, &bytes).unwrap();
        assert!(FileIndex::load(&file).is_err());
        fs::write(&file, b"garbage").unwrap();
        assert!(FileIndex::load(&file).is_err());
    }
}
