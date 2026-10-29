//! Clean up a folder: find duplicate files and empty folders under it, in
//! the background, for the user to review before anything is deleted.
//!
//! Neither scan follows links (symbolic links, junctions), so there are no
//! cycles and nothing outside the folder is looked at. Folders that can't
//! be read are skipped and counted.

pub mod duplicates;
pub mod empty_folders;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use serde::{Deserialize, Serialize};

/// What a clean-up scan looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CleanupMode {
    /// Files with the same contents.
    Duplicates,
    /// Folders with no files in them (or in their sub-folders).
    EmptyFolders,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScanOptions {
    /// Look at hidden files and folders too.
    pub show_hidden: bool,
    /// Duplicates: count empty (zero-byte) files as duplicates of each other.
    pub include_empty_files: bool,
    /// Duplicates: after the hashes match, compare the files byte by byte.
    pub byte_compare: bool,
    /// Duplicates: files compared at the same time; 0 = automatic (one on a
    /// hard disk, else up to four).
    pub threads: usize,
}

/// How a scan is going; read by the UI while it runs.
#[derive(Debug, Default)]
pub struct Progress {
    pub folders: AtomicU64,
    pub files: AtomicU64,
    /// Duplicates: files whose contents are being compared (0 while listing).
    pub candidates: AtomicU64,
    /// Duplicates: bytes read to compare contents.
    pub hashed: AtomicU64,
}

impl Progress {
    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }

    fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }
}

/// What a finished (or cancelled) scan found.
pub enum Outcome {
    Duplicates(duplicates::Report),
    EmptyFolders(empty_folders::Report),
}

impl Outcome {
    pub fn cancelled(&self) -> bool {
        match self {
            Outcome::Duplicates(r) => r.cancelled,
            Outcome::EmptyFolders(r) => r.cancelled,
        }
    }

    /// Folders (and, for duplicates, files) that couldn't be read.
    pub fn unreadable(&self) -> u64 {
        match self {
            Outcome::Duplicates(r) => r.unreadable,
            Outcome::EmptyFolders(r) => r.unreadable,
        }
    }
}

/// A running scan. Dropping it stops the scan.
pub struct ScanHandle {
    pub result: Receiver<(Outcome, Duration)>,
    pub progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Scan `root` on another thread; `notify` is called when it's done.
pub fn start(
    root: PathBuf,
    mode: CleanupMode,
    opts: ScanOptions,
    notify: impl Fn() + Send + 'static,
) -> ScanHandle {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(Progress::default());
    let handle = ScanHandle {
        result: rx,
        progress: progress.clone(),
        cancel: cancel.clone(),
    };
    std::thread::Builder::new()
        .name("clean-up scan".into())
        // The empty-folder walk recurses once per folder level.
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let started = Instant::now();
            let outcome = match mode {
                CleanupMode::Duplicates => {
                    Outcome::Duplicates(duplicates::find(&root, opts, &cancel, &progress))
                }
                CleanupMode::EmptyFolders => {
                    Outcome::EmptyFolders(empty_folders::find(&root, opts, &cancel, &progress))
                }
            };
            let _ = tx.send((outcome, started.elapsed()));
            notify();
        })
        .expect("failed to spawn clean-up thread");
    handle
}

/// A link: neither listed nor followed. Junctions count too (the standard
/// library reports every name-surrogate reparse point as a symlink), cloud
/// placeholders don't.
fn is_link(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[cfg(test)]
mod tests;
