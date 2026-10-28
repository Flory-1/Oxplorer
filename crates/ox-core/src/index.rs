//! A persistent index of file names for instant search across whole drives.
//!
//! The index stores one compact 24-byte record per file or folder plus all
//! names in a single string buffer, so a million files take roughly 50 MB.
//! Queries run on all CPU cores and only reconstruct full paths for hits.
//! The index is saved to the cache folder, so search works instantly right
//! after startup while a fresh index is built in the background.
//!
//! Building uses a portable parallel directory walk. (Reading the NTFS
//! master file table directly is planned as a faster builder on Windows.)

use std::fs;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::entry::{self, FileEntry};
use crate::search::Query;

const NO_PARENT: u32 = u32::MAX;
const F_DIR: u8 = 1;
/// The item or one of its parent folders is hidden.
const F_HIDDEN: u8 = 2;

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

pub struct FileIndex {
    roots: Vec<PathBuf>,
    names: String,
    entries: Vec<Entry>,
    built: SystemTime,
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
        // Names longer than u16::MAX bytes can't exist on any common file system.
        let name = &name[..name.len().min(u16::MAX as usize)];
        let idx = self.entries.len() as u32;
        self.entries.push(Entry {
            size,
            name_off: self.names.len() as u32,
            parent,
            mtime,
            name_len: name.len() as u16,
            flags,
        });
        self.names.push_str(name);
        idx
    }
}

struct Child {
    name: String,
    flags: u8,
    size: u64,
    mtime: u32,
}

impl FileIndex {
    /// Walk all roots in parallel. `progress` counts indexed items; setting
    /// `cancel` stops early (the partial index is returned).
    pub fn build(cfg: &IndexConfig, cancel: &AtomicBool, progress: &AtomicU64) -> FileIndex {
        let exclude: Vec<String> = cfg.exclude.iter().map(|e| e.to_lowercase()).collect();
        let builder = Mutex::new(Builder::default());
        let (tx, rx) = crossbeam_channel::unbounded::<(PathBuf, u32, bool)>();
        // Folders queued or being read; the walk is done when it reaches zero.
        let pending = AtomicUsize::new(0);
        let mut roots = Vec::new();
        for root in &cfg.roots {
            if !root.is_dir() {
                continue;
            }
            let name = root.to_string_lossy();
            let idx = builder.lock().unwrap().push(&name, NO_PARENT, F_DIR, 0, 0);
            roots.push(root.clone());
            pending.fetch_add(1, Ordering::SeqCst);
            let _ = tx.send((root.clone(), idx, false));
        }

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
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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
        let mut chain = Vec::new();
        let mut cur = i;
        while cur != NO_PARENT {
            let e = &self.entries[cur as usize];
            chain.push(self.name(e));
            cur = e.parent;
        }
        let mut path = PathBuf::new();
        for part in chain.iter().rev() {
            path.push(part);
        }
        path
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
        }
    }

    /// Index of the folder at `path`, if it is indexed.
    fn find_dir(&self, path: &Path) -> Option<u32> {
        let want = path.file_name().map(|n| n.to_string_lossy());
        self.entries.iter().enumerate().find_map(|(i, e)| {
            let i = i as u32;
            let candidate = e.flags & F_DIR != 0
                && match &want {
                    Some(n) => self.name(e) == n.as_ref(),
                    None => e.parent == NO_PARENT,
                };
            (candidate && self.path_of(i) == path).then_some(i)
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
                            if e.parent == NO_PARENT
                                || (!opts.include_hidden && e.flags & F_HIDDEN != 0)
                            {
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
            let mut v = Vec::new();
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
        Ok(FileIndex {
            roots,
            names,
            entries,
            built,
        })
    }
}

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
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut children = Vec::new();
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if !exclude.is_empty() && exclude.contains(&name.to_lowercase()) {
            continue;
        }
        let Ok(ft) = item.file_type() else { continue };
        let Ok(meta) = item.metadata() else { continue };
        // Linked folders are indexed as entries but not followed (no loops, no duplicates).
        let is_dir = ft.is_dir() && !ft.is_symlink();
        let hidden = parent_hidden || entry::is_hidden(&name, &meta);
        let mut flags = 0;
        if is_dir {
            flags |= F_DIR;
        }
        if hidden {
            flags |= F_HIDDEN;
        }
        children.push(Child {
            size: if is_dir { 0 } else { meta.len() },
            mtime: to_secs(meta.modified().ok()),
            name,
            flags,
        });
    }
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

struct Shared {
    index: RwLock<Option<Arc<FileIndex>>>,
    state: Mutex<IndexState>,
    progress: AtomicU64,
    cancel: AtomicBool,
    building: AtomicBool,
    last_build: Mutex<Option<(Instant, Duration)>>,
    cache: PathBuf,
    notify: Box<dyn Fn() + Send + Sync>,
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
        if self.shared.building.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = self.shared.clone();
        shared.cancel.store(false, Ordering::Relaxed);
        shared.progress.store(0, Ordering::Relaxed);
        *shared.state.lock().unwrap() = IndexState::Building;
        (shared.notify)();
        std::thread::Builder::new()
            .name("index-build".into())
            .spawn(move || {
                let start = Instant::now();
                let idx = FileIndex::build(&cfg, &shared.cancel, &shared.progress);
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

    /// Stop a running build.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// Forget the index and delete the cache (indexing turned off).
    pub fn clear(&self) {
        self.cancel();
        *self.shared.index.write().unwrap() = None;
        *self.shared.state.lock().unwrap() = IndexState::Empty;
        let _ = fs::remove_file(&self.shared.cache);
        (self.shared.notify)();
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

    #[test]
    fn service_builds_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        sample_tree(&dir.path().join("data"));
        let cache = dir.path().join("index.bin");
        let wait = |svc: &IndexService| {
            let start = Instant::now();
            while svc.state() == IndexState::Loading || svc.is_building() {
                assert!(start.elapsed() < Duration::from_secs(20));
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let svc = IndexService::start(cache.clone(), || {});
        wait(&svc);
        assert_eq!(svc.state(), IndexState::Empty);
        svc.rebuild(IndexConfig {
            roots: vec![dir.path().join("data")],
            exclude: vec![],
            threads: 2,
        });
        wait(&svc);
        assert_eq!(svc.state(), IndexState::Ready);
        assert!(svc.index().unwrap().len() > 5);

        // A new service picks up the cached index without rebuilding.
        let again = IndexService::start(cache, || {});
        wait(&again);
        assert_eq!(again.state(), IndexState::Ready);
        assert_eq!(again.index().unwrap().len(), svc.index().unwrap().len());
    }
}
