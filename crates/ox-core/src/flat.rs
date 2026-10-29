//! Flat view: every file in a folder and all its sub-folders, listed in the
//! background and streamed to the UI in batches.
//!
//! Folders are walked breadth-first, so when the limit is reached the files
//! closest to the top are the ones listed. Folders themselves aren't listed.
//! Links to folders (symbolic links, junctions) are neither listed nor
//! followed, so there are no cycles; folders that can't be read are skipped
//! and counted.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};

use crate::entry::{self, FileEntry};

/// Default limit of files in one flat view.
pub const MAX_FILES: usize = 200_000;

#[derive(Debug, Clone, Copy)]
pub struct FlatOptions {
    pub show_hidden: bool,
    pub max_files: usize,
}

impl Default for FlatOptions {
    fn default() -> Self {
        Self {
            show_hidden: false,
            max_files: MAX_FILES,
        }
    }
}

pub enum FlatEvent {
    /// More files.
    Files(Vec<FileEntry>),
    /// The walk is over (or was cancelled).
    Done {
        /// Stopped at `max_files`; there are more.
        truncated: bool,
        /// Folders that couldn't be read (access denied, …).
        unreadable: u64,
        elapsed: Duration,
    },
}

/// A running walk. Dropping it stops the walk.
pub struct FlatHandle {
    pub events: Receiver<FlatEvent>,
    cancel: Arc<AtomicBool>,
    folders: Arc<AtomicU64>,
}

impl FlatHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Folders looked into so far.
    pub fn folders(&self) -> u64 {
        self.folders.load(Ordering::Relaxed)
    }
}

impl Drop for FlatHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// List the files under `root` on another thread. `notify` is called
/// whenever new files are available (e.g. to repaint the UI).
pub fn start(
    root: PathBuf,
    opts: FlatOptions,
    notify: impl Fn() + Send + Sync + 'static,
) -> FlatHandle {
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let folders = Arc::new(AtomicU64::new(0));
    let handle = FlatHandle {
        events: rx,
        cancel: cancel.clone(),
        folders: folders.clone(),
    };
    std::thread::Builder::new()
        .name("flat view".into())
        .spawn(move || {
            let started = Instant::now();
            let mut out = Out {
                tx: &tx,
                notify: &notify,
                batch: Vec::new(),
                last_flush: Instant::now(),
            };
            let (truncated, unreadable) = walk(&root, opts, &cancel, &folders, &mut out);
            out.flush();
            let _ = tx.send(FlatEvent::Done {
                truncated,
                unreadable,
                elapsed: started.elapsed(),
            });
            notify();
        })
        .expect("failed to spawn flat view thread");
    handle
}

/// Collects files and sends them in batches.
struct Out<'a> {
    tx: &'a Sender<FlatEvent>,
    notify: &'a (dyn Fn() + Send + Sync),
    batch: Vec<FileEntry>,
    last_flush: Instant,
}

impl Out<'_> {
    fn push(&mut self, e: FileEntry) {
        self.batch.push(e);
        if self.batch.len() >= 1024 || self.last_flush.elapsed() > Duration::from_millis(100) {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if !self.batch.is_empty() {
            let _ = self
                .tx
                .send(FlatEvent::Files(std::mem::take(&mut self.batch)));
            (self.notify)();
        }
        self.last_flush = Instant::now();
    }
}

/// Returns (truncated, unreadable folders).
fn walk(
    root: &Path,
    opts: FlatOptions,
    cancel: &AtomicBool,
    folders: &AtomicU64,
    out: &mut Out<'_>,
) -> (bool, u64) {
    let mut queue = VecDeque::from([root.to_path_buf()]);
    let mut found = 0usize;
    let mut unreadable = 0u64;
    while let Some(dir) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        folders.fetch_add(1, Ordering::Relaxed);
        let Ok(items) = fs::read_dir(&dir) else {
            unreadable += 1;
            continue;
        };
        for item in items {
            if cancel.load(Ordering::Relaxed) {
                return (false, unreadable);
            }
            let Ok(item) = item else { continue };
            // Doesn't follow links: a link's own metadata.
            let Ok(meta) = item.metadata() else { continue };
            let ft = meta.file_type();
            if ft.is_dir() {
                if opts.show_hidden || !entry::is_hidden(&item.file_name().to_string_lossy(), &meta)
                {
                    queue.push_back(item.path());
                }
                continue;
            }
            if ft.is_symlink() && is_folder_link(&meta) {
                continue;
            }
            let e = FileEntry::from_metadata(item.path(), &meta);
            if e.is_dir || (e.hidden && !opts.show_hidden) {
                continue;
            }
            if found >= opts.max_files {
                return (true, unreadable);
            }
            found += 1;
            out.push(e);
        }
    }
    (false, unreadable)
}

/// A link that is a folder itself (a junction or a folder symlink), even
/// when its target is gone.
#[cfg(windows)]
fn is_folder_link(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const DIRECTORY: u32 = 0x10;
    meta.file_attributes() & DIRECTORY != 0
}

#[cfg(not(windows))]
fn is_folder_link(_meta: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(root: &Path, opts: FlatOptions) -> (Vec<String>, bool, u64) {
        let handle = start(root.to_path_buf(), opts, || {});
        let mut names = Vec::new();
        loop {
            match handle.events.recv().unwrap() {
                FlatEvent::Files(files) => names.extend(files.into_iter().map(|e| {
                    let rel = e.path.strip_prefix(root).unwrap();
                    rel.to_string_lossy().replace('\\', "/")
                })),
                FlatEvent::Done {
                    truncated,
                    unreadable,
                    ..
                } => {
                    names.sort();
                    return (names, truncated, unreadable);
                }
            }
        }
    }

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::write(p.join("top.txt"), "1").unwrap();
        fs::create_dir_all(p.join("a/b/c")).unwrap();
        fs::write(p.join("a/same.txt"), "22").unwrap();
        fs::write(p.join("a/b/same.txt"), "333").unwrap();
        fs::write(p.join("a/b/c/deep.bin"), "4444").unwrap();
        fs::create_dir(p.join("empty")).unwrap();
        dir
    }

    #[test]
    fn lists_files_of_all_sub_folders() {
        let dir = tree();
        let (names, truncated, unreadable) = collect(dir.path(), FlatOptions::default());
        assert_eq!(
            names,
            ["a/b/c/deep.bin", "a/b/same.txt", "a/same.txt", "top.txt"]
        );
        assert!(!truncated);
        assert_eq!(unreadable, 0);
    }

    #[test]
    fn stops_at_the_limit_keeping_the_top_files() {
        let dir = tree();
        let opts = FlatOptions {
            max_files: 2,
            ..Default::default()
        };
        let (names, truncated, _) = collect(dir.path(), opts);
        assert!(truncated);
        assert_eq!(names.len(), 2);
        // Breadth-first: the file at the top comes before deeper ones.
        assert!(names.contains(&"top.txt".to_owned()));
        assert!(!names.iter().any(|n| n.ends_with("deep.bin")));
    }

    #[test]
    fn missing_folder_counts_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let (names, truncated, unreadable) =
            collect(&dir.path().join("gone"), FlatOptions::default());
        assert!(names.is_empty());
        assert!(!truncated);
        assert_eq!(unreadable, 1);
    }

    #[cfg(windows)]
    #[test]
    fn hidden_files_and_folders() {
        let dir = tree();
        let p = dir.path();
        fs::write(p.join("a/secret.txt"), "").unwrap();
        crate::details::set_attributes(&p.join("a/secret.txt"), None, Some(true)).unwrap();
        crate::details::set_attributes(&p.join("a/b"), None, Some(true)).unwrap();
        let (names, ..) = collect(p, FlatOptions::default());
        assert_eq!(names, ["a/same.txt", "top.txt"]);
        let opts = FlatOptions {
            show_hidden: true,
            ..Default::default()
        };
        let (names, ..) = collect(p, opts);
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn dropping_the_handle_stops_the_walk() {
        let dir = tree();
        let handle = start(dir.path().to_path_buf(), FlatOptions::default(), || {});
        handle.cancel();
        // It ends (with or without files) and says so.
        let done = handle
            .events
            .iter()
            .any(|e| matches!(e, FlatEvent::Done { .. }));
        assert!(done);
    }
}
