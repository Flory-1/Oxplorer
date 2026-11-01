//! The text index is in memory only while it is used: the indexer puts it
//! away when a pass is saved and nothing used it for a while, and the next
//! search or pass reads it from the disk again (a fraction of a second).
//! Most of the time no `content:` search runs, so this keeps tens of MB free.
//!
//! What the settings page shows (and whether a folder is covered) is kept
//! in a small [`Summary`], so looking doesn't load the index.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use super::store::Store;

/// What is known about the index while it is only on disk.
#[derive(Debug, Clone, Default)]
pub(super) struct Summary {
    pub roots: Vec<PathBuf>,
    /// Files known (with or without text).
    pub files: usize,
    /// Files with text, and their total size.
    pub documents: usize,
    pub bytes: u64,
    pub words: usize,
}

impl Summary {
    fn of(s: &Store) -> Self {
        let (documents, bytes) = s.documents();
        Self {
            roots: s.roots.clone(),
            files: s.len(),
            documents,
            bytes,
            words: s.word_count(),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct LazyStore {
    cache: PathBuf,
    /// `None`: only in the file at `cache`.
    store: RwLock<Option<Store>>,
    /// The index as it was put away (`None`: not read yet since starting).
    summary: Mutex<Option<Summary>>,
    last_use: Mutex<Option<Instant>>,
    /// Everything in memory is in the file too (putting it away loses nothing).
    saved: AtomicBool,
}

impl LazyStore {
    pub fn new(cache: PathBuf) -> Self {
        Self {
            cache,
            ..Default::default()
        }
    }

    fn touch(&self) {
        *self.last_use.lock().unwrap() = Some(Instant::now());
    }

    fn load(&self) -> Store {
        // No file (or a damaged one): start empty, as when starting fresh.
        let store = Store::load(&self.cache).unwrap_or_default();
        self.saved.store(true, Ordering::SeqCst);
        store
    }

    /// Run `f` on the index, reading it from the disk first if it was put away.
    pub fn read<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        self.touch();
        {
            let guard = self.store.read().unwrap();
            if let Some(s) = guard.as_ref() {
                return f(s);
            }
        }
        let mut guard = self.store.write().unwrap();
        f(guard.get_or_insert_with(|| self.load()))
    }

    /// Change the index (reading it first if it was put away).
    pub fn write<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        self.touch();
        let mut guard = self.store.write().unwrap();
        let store = guard.get_or_insert_with(|| self.load());
        self.saved.store(false, Ordering::SeqCst);
        f(store)
    }

    /// Run `f` only if the index is in memory; doesn't count as a use.
    pub fn peek<R>(&self, f: impl FnOnce(&Store) -> R) -> Option<R> {
        self.store.read().unwrap().as_ref().map(f)
    }

    /// The summary kept when the index was put away.
    pub fn summary(&self) -> Option<Summary> {
        self.summary.lock().unwrap().clone()
    }

    /// Start over with `store` (without reading the old one).
    pub fn replace(&self, store: Store) {
        self.touch();
        *self.store.write().unwrap() = Some(store);
        self.saved.store(false, Ordering::SeqCst);
    }

    /// Write the index to its file (nothing to do while it is only there).
    pub fn save(&self) -> io::Result<()> {
        let mut guard = self.store.write().unwrap();
        if let Some(s) = guard.as_mut() {
            s.save(&self.cache)?;
            self.saved.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Put the index away if it is saved and wasn't used for `idle`.
    /// Returns whether it was.
    pub fn unload_if_idle(&self, idle: Duration) -> bool {
        let used = *self.last_use.lock().unwrap();
        if used.is_some_and(|t| t.elapsed() < idle) {
            return false;
        }
        let mut guard = self.store.write().unwrap();
        // Checked under the lock: `write` clears it under the same lock.
        if !self.saved.load(Ordering::SeqCst) {
            return false;
        }
        let Some(s) = guard.as_ref() else {
            return false;
        };
        *self.summary.lock().unwrap() = Some(Summary::of(s));
        *guard = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::store::Text;
    use super::*;
    use std::path::Path;

    #[test]
    fn put_away_when_saved_and_idle_then_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let lazy = LazyStore::new(dir.path().join("fulltext.bin"));
        lazy.write(|s| {
            s.put(
                Path::new("C:/a.txt"),
                1,
                2,
                Text::Words,
                ["hello".to_owned()],
            );
            s.roots = vec![PathBuf::from("C:/")];
        });
        assert!(
            !lazy.unload_if_idle(Duration::ZERO),
            "not saved yet: putting it away would lose it"
        );
        lazy.save().unwrap();
        assert!(!lazy.unload_if_idle(Duration::from_secs(60)), "just used");
        assert!(lazy.unload_if_idle(Duration::ZERO));
        assert!(lazy.peek(|_| ()).is_none());
        let summary = lazy.summary().unwrap();
        assert_eq!((summary.files, summary.documents, summary.words), (1, 1, 1));
        assert_eq!(summary.roots, vec![PathBuf::from("C:/")]);

        // The next use reads it again, with everything in it.
        assert!(lazy.read(|s| s.is_current(Path::new("C:/a.txt"), 1, 2)));
        assert!(lazy.peek(|_| ()).is_some());
    }

    #[test]
    fn a_missing_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let lazy = LazyStore::new(dir.path().join("none.bin"));
        assert!(lazy.read(|s| s.is_empty()));
        assert!(lazy.unload_if_idle(Duration::ZERO), "nothing to lose");
    }
}
