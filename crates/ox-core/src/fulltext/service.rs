//! The background indexer: loads the saved text index, walks the indexed
//! folders, reads the files that are new or changed and forgets the ones
//! that are gone; again every [`RESCAN`] and when the settings change.
//!
//! It runs on one thread with below-normal priority, waits while files are
//! being copied ([`FullTextService::set_paused`]), never reads online-only
//! cloud files, and saves the index now and then and after each pass.
//! (Windows' background mode would lower the I/O priority too, but then
//! the indexer hardly gets ahead while anything else reads the disk.)

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use super::store::{mtime_of, path_key, Store, Text};
use super::{extract, tokens, FullTextConfig, FullTextIndex};

/// Time between two passes over the folders.
const RESCAN: Duration = Duration::from_secs(20 * 60);
/// The index is saved at least this often during a long pass.
const SAVE_EVERY: Duration = Duration::from_secs(120);
/// Changed settings start a pass after this pause.
const SETTLE: Duration = Duration::from_secs(3);
/// A file with more different words isn't split into words (it is then
/// read for every search, like without the index).
const MAX_WORDS_PER_FILE: usize = 200_000;

/// Where the text index is kept between runs.
pub fn default_cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(crate::config_dir)
        .join(crate::APP_NAME)
        .join("fulltext.bin")
}

/// What the indexer is doing.
#[derive(Debug, Clone, PartialEq)]
pub enum FullTextState {
    /// Turned off.
    Off,
    /// Reading the saved index.
    Loading,
    /// Walking the folders; files looked at so far.
    Indexing(u64),
    /// Waiting while files are copied.
    Paused,
    /// Up to date (until the next pass).
    Ready,
}

enum Command {
    Configure(FullTextConfig),
    Rebuild,
}

struct Shared {
    index: Arc<FullTextIndex>,
    cache: PathBuf,
    paused: AtomicBool,
    /// The current pass should stop (settings changed, turned off).
    stop: AtomicBool,
    /// Turned off: the thread forgets the index and deletes the saved one.
    clear: AtomicBool,
    indexing: AtomicBool,
    loading: AtomicBool,
    progress: AtomicU64,
    last_pass: Mutex<Option<(SystemTime, Duration)>>,
    error: Mutex<Option<String>>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn new(cache: PathBuf, notify: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            index: Arc::new(FullTextIndex::default()),
            cache,
            paused: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            clear: AtomicBool::new(false),
            indexing: AtomicBool::new(false),
            loading: AtomicBool::new(false),
            progress: AtomicU64::new(0),
            last_pass: Mutex::new(None),
            error: Mutex::new(None),
            notify,
        })
    }
}

/// Owns the text index and its indexer thread.
pub struct FullTextService {
    shared: Arc<Shared>,
    worker: Option<Sender<Command>>,
}

impl FullTextService {
    /// A service that does nothing until it is configured.
    pub fn new(cache: PathBuf, notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            shared: Shared::new(cache, Arc::new(notify)),
            worker: None,
        }
    }

    /// Index these folders (`None`: turn off, forget the index and delete
    /// the saved one). Starts the indexer the first time.
    pub fn configure(&mut self, cfg: Option<FullTextConfig>) {
        match cfg {
            None => {
                if let Some(tx) = self.worker.take() {
                    let old = &self.shared;
                    old.clear.store(true, Ordering::Relaxed);
                    old.stop.store(true, Ordering::Relaxed);
                    drop(tx); // the thread ends and clears the index
                    let fresh = Shared::new(old.cache.clone(), old.notify.clone());
                    let paused = old.paused.load(Ordering::Relaxed);
                    fresh.paused.store(paused, Ordering::Relaxed);
                    self.shared = fresh;
                } else {
                    let _ = fs::remove_file(&self.shared.cache);
                }
            }
            Some(cfg) => {
                let tx = self
                    .worker
                    .get_or_insert_with(|| spawn(self.shared.clone()));
                self.shared.stop.store(true, Ordering::Relaxed);
                let _ = tx.send(Command::Configure(cfg));
            }
        }
    }

    /// Read every file again (keeps the folders).
    pub fn rebuild(&self) {
        if let Some(tx) = &self.worker {
            self.shared.stop.store(true, Ordering::Relaxed);
            let _ = tx.send(Command::Rebuild);
        }
    }

    /// Wait (while files are being copied, for example).
    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    /// The index for searches, once it is on.
    pub fn index(&self) -> Option<Arc<FullTextIndex>> {
        self.worker.as_ref().map(|_| self.shared.index.clone())
    }

    pub fn state(&self) -> FullTextState {
        let s = &self.shared;
        if self.worker.is_none() {
            FullTextState::Off
        } else if s.loading.load(Ordering::Relaxed) {
            FullTextState::Loading
        } else if s.indexing.load(Ordering::Relaxed) && s.paused.load(Ordering::Relaxed) {
            FullTextState::Paused
        } else if s.indexing.load(Ordering::Relaxed) {
            FullTextState::Indexing(s.progress.load(Ordering::Relaxed))
        } else {
            FullTextState::Ready
        }
    }

    /// When the last complete pass ended and how long it took.
    pub fn last_pass(&self) -> Option<(SystemTime, Duration)> {
        *self.shared.last_pass.lock().unwrap()
    }

    /// Why the index couldn't be saved, if it couldn't.
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    /// Size of the saved index file.
    pub fn file_size(&self) -> Option<u64> {
        fs::metadata(&self.shared.cache).ok().map(|m| m.len())
    }
}

impl Drop for FullTextService {
    fn drop(&mut self) {
        // Stops a running pass; the thread saves and ends.
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

fn spawn(shared: Arc<Shared>) -> Sender<Command> {
    let (tx, rx) = crossbeam_channel::unbounded();
    shared.loading.store(true, Ordering::Relaxed);
    std::thread::Builder::new()
        .name("fulltext-index".into())
        .spawn(move || run(&shared, &rx))
        .expect("failed to spawn the text index thread");
    tx
}

/// Lower this thread's CPU priority (below normal: the UI and copies come first).
fn background_priority() {
    #[cfg(windows)]
    // SAFETY: changes the priority of the calling thread only.
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

fn run(shared: &Shared, rx: &Receiver<Command>) {
    background_priority();
    if let Ok(store) = Store::load(&shared.cache) {
        *shared.index.store.write().unwrap() = store;
    }
    shared.loading.store(false, Ordering::Relaxed);
    (shared.notify)();
    let mut cfg: Option<FullTextConfig> = None;
    let mut next = Instant::now();
    loop {
        let wait = next.saturating_duration_since(Instant::now());
        let command = if cfg.is_some() {
            rx.recv_timeout(wait)
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match command {
            Ok(Command::Configure(new)) => {
                if cfg.as_ref() != Some(&new) {
                    // Files of folders that aren't indexed any more go at
                    // the end of the next complete pass, not now: while a
                    // folder is being typed in the settings, its files stay.
                    // The pass waits a moment for the typing to end.
                    let first = cfg.is_none();
                    cfg = Some(new);
                    next = Instant::now() + if first { Duration::ZERO } else { SETTLE };
                }
            }
            Ok(Command::Rebuild) => {
                let mut store = shared.index.store.write().unwrap();
                *store = Store::default();
                drop(store);
                next = Instant::now();
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                // Turned off: forget the index.
                if shared.clear.load(Ordering::Relaxed) {
                    *shared.index.store.write().unwrap() = Store::default();
                    let _ = fs::remove_file(&shared.cache);
                }
                (shared.notify)();
                return;
            }
        }
        // Take all commands that are waiting before starting a pass.
        if !rx.is_empty() {
            continue;
        }
        let Some(c) = &cfg else { continue };
        if Instant::now() < next {
            continue;
        }
        shared.stop.store(false, Ordering::Relaxed);
        let started = Instant::now();
        shared.indexing.store(true, Ordering::Relaxed);
        let done = pass(shared, c);
        if shared.clear.load(Ordering::Relaxed) {
            continue; // turned off meanwhile: not saved
        }
        save(shared);
        if done {
            *shared.last_pass.lock().unwrap() = Some((SystemTime::now(), started.elapsed()));
            next = Instant::now() + RESCAN;
        }
        shared.indexing.store(false, Ordering::Relaxed);
        (shared.notify)();
    }
}

fn save(shared: &Shared) {
    let result = shared.index.store.write().unwrap().save(&shared.cache);
    *shared.error.lock().unwrap() = result.err().map(|e| e.to_string());
}

/// Is the path with key `key` inside one of the folders with keys `roots`?
pub(super) fn under(key: &str, roots: &[String]) -> bool {
    roots.iter().any(|r| {
        key.strip_prefix(r.as_str()).is_some_and(|rest| {
            rest.is_empty()
                || r.ends_with(std::path::MAIN_SEPARATOR)
                || rest.starts_with(std::path::MAIN_SEPARATOR)
        })
    })
}

/// One walk over the folders. Returns `false` if it was stopped.
fn pass(shared: &Shared, cfg: &FullTextConfig) -> bool {
    shared.progress.store(0, Ordering::Relaxed);
    (shared.notify)();
    let mut seen: HashSet<String> = HashSet::new();
    let mut last_save = Instant::now();
    let mut last_notify = Instant::now();
    let mut stopped = false;
    'roots: for root in &cfg.folders {
        let exclude: HashSet<String> = cfg.exclude.iter().map(|e| e.to_lowercase()).collect();
        let walk = ignore::WalkBuilder::new(root)
            .standard_filters(false)
            .hidden(true)
            .follow_links(false)
            .filter_entry(move |e| {
                !exclude.contains(&e.file_name().to_string_lossy().to_lowercase())
            })
            .build();
        for entry in walk {
            while shared.paused.load(Ordering::Relaxed) && !shared.stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(500));
            }
            if shared.stop.load(Ordering::Relaxed) {
                stopped = true;
                break 'roots;
            }
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            shared.progress.fetch_add(1, Ordering::Relaxed);
            if let Some(key) = index_file(shared, cfg, entry.path()) {
                seen.insert(key);
            }
            if last_save.elapsed() > SAVE_EVERY {
                save(shared);
                last_save = Instant::now();
            }
            if last_notify.elapsed() > Duration::from_millis(500) {
                (shared.notify)();
                last_notify = Instant::now();
            }
        }
    }
    if !stopped {
        let mut store = shared.index.store.write().unwrap();
        store.retain(|f| seen.contains(&path_key(&f.path)));
        store.roots = cfg.folders.clone();
        store.complete = true;
        store.compact_if_needed();
        store.shrink();
    }
    !stopped
}

/// Bring the index up to date for one file. Returns its key if the index
/// knows it afterwards (files that are too large or only online aren't
/// kept: a search reads those as without the index).
fn index_file(shared: &Shared, cfg: &FullTextConfig, path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if meta.len() > cfg.max_file_bytes || crate::cloud::online(&meta) {
        return None;
    }
    let (size, mtime) = (meta.len(), mtime_of(meta.modified().ok()));
    if shared
        .index
        .store
        .read()
        .unwrap()
        .is_current(path, size, mtime)
    {
        return Some(path_key(path));
    }
    let (text, words) = match extract::extract(path, cfg.max_file_bytes) {
        Ok(Some(t)) => match tokens::words(&t, MAX_WORDS_PER_FILE) {
            Some(w) => (Text::Words, w),
            None => (Text::Always, HashSet::new()),
        },
        Ok(None) => (Text::None, HashSet::new()),
        // Locked or gone: tried again next time.
        Err(_) => return None,
    };
    shared
        .index
        .store
        .write()
        .unwrap()
        .put(path, size, mtime, text, words);
    Some(path_key(path))
}
