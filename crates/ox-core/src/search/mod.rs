//! Fast search across folders and drives.
//!
//! Two back ends share one query language: a multi-threaded directory walk
//! (the `ignore` crate's parallel walker, the same one ripgrep uses) that
//! streams results while it runs, and the persistent file index in
//! [`crate::index`] that answers name queries over whole drives instantly.
//!
//! # Query syntax
//!
//! | Example              | Meaning                                               |
//! |----------------------|-------------------------------------------------------|
//! | `report`             | name contains "report" (case-insensitive)             |
//! | `"annual report"`    | name contains the exact phrase                        |
//! | `*.rs` / `img_??`    | wildcard match on the whole name                      |
//! | `/^IMG_\d+\.jpe?g$/` | regular expression on the name (also `regex:…`)       |
//! | `-tmp`               | name must *not* contain "tmp"                         |
//! | `ext:pdf,docx`       | extension is one of the list                          |
//! | `type:folder`        | only folders (`type:file` for files)                  |
//! | `kind:image,video`   | by file kind (image, video, audio, document, archive, code, app) |
//! | `size:>10mb`         | size filter (`<`, `>`, units b/kb/mb/gb)              |
//! | `modified:7d`        | modified in the last 7 days (`today`, `24h`, `2w`)    |
//! | `content:"TODO"`     | file contents contain the text (≤ 32 MB, text files)  |

mod query;
mod walk;

pub(crate) use query::wildcard;
pub use query::{KindFilter, Query};

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use crate::entry::FileEntry;
use query::tokenize;

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub include_hidden: bool,
    pub case_sensitive: bool,
    pub max_results: usize,
    /// 0 = number of CPU cores.
    pub threads: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            include_hidden: false,
            case_sensitive: false,
            max_results: 50_000,
            threads: 0,
        }
    }
}

pub enum SearchEvent {
    Results(Vec<FileEntry>),
    Done {
        elapsed: Duration,
        scanned: u64,
        truncated: bool,
        /// Answered from the file index instead of walking the disk.
        from_index: bool,
    },
}

/// A running search. Dropping it cancels the search.
pub struct SearchHandle {
    pub events: Receiver<SearchEvent>,
    cancel: Arc<AtomicBool>,
    scanned: Arc<AtomicU64>,
}

impl SearchHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Number of files and folders looked at so far.
    pub fn scanned(&self) -> u64 {
        self.scanned.load(Ordering::Relaxed)
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Add or remove a filter token in a query string (used by the filter chips).
/// Tokens in the same `group` (e.g. `kind:` / `type:`) replace each other.
pub fn toggle_filter(query: &str, token: &str, group: &[&str]) -> String {
    let had = tokenize(query)
        .iter()
        .any(|t| t.eq_ignore_ascii_case(token));
    let mut parts: Vec<String> = tokenize(query)
        .into_iter()
        .filter(|t| {
            let lower = t.to_ascii_lowercase();
            !group.iter().any(|g| lower.starts_with(g))
        })
        .collect();
    if !had {
        parts.push(token.to_owned());
    }
    parts.join(" ")
}

/// Is `token` part of the query string?
pub fn has_filter(query: &str, token: &str) -> bool {
    tokenize(query)
        .iter()
        .any(|t| t.eq_ignore_ascii_case(token))
}

/// Answer a query from the file index in the background (results arrive in
/// one batch, usually within milliseconds). `within` limits it to a folder.
pub fn start_indexed(
    index: Arc<crate::index::FileIndex>,
    within: Option<PathBuf>,
    query: Query,
    opts: SearchOptions,
    notify: impl Fn() + Send + Sync + 'static,
) -> SearchHandle {
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let handle = SearchHandle {
        events: rx,
        cancel: cancel.clone(),
        scanned: Arc::new(AtomicU64::new(index.len() as u64)),
    };
    std::thread::Builder::new()
        .name("search-index".into())
        .spawn(move || {
            let started = Instant::now();
            let (ids, truncated) = index.search(
                &query,
                &crate::index::IndexQuery {
                    include_hidden: opts.include_hidden,
                    max_results: opts.max_results,
                    within,
                    cancel: Some(cancel.clone()),
                },
            );
            if cancel.load(Ordering::Relaxed) {
                return; // superseded by a newer search
            }
            let entries: Vec<FileEntry> = ids.into_iter().map(|i| index.file_entry(i)).collect();
            if !entries.is_empty() {
                let _ = tx.send(SearchEvent::Results(entries));
            }
            let _ = tx.send(SearchEvent::Done {
                elapsed: started.elapsed(),
                scanned: index.len() as u64,
                truncated,
                from_index: true,
            });
            notify();
        })
        .expect("failed to spawn search thread");
    handle
}

/// Start searching `roots` in the background. `notify` is called whenever new
/// results are available (e.g. to repaint the UI).
pub fn start(
    roots: Vec<PathBuf>,
    query: Query,
    opts: SearchOptions,
    notify: impl Fn() + Send + Sync + 'static,
) -> SearchHandle {
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let scanned = Arc::new(AtomicU64::new(0));
    let handle = SearchHandle {
        events: rx,
        cancel: cancel.clone(),
        scanned: scanned.clone(),
    };
    std::thread::Builder::new()
        .name("search".into())
        .spawn(move || walk::run(roots, query, opts, tx, cancel, scanned, Arc::new(notify)))
        .expect("failed to spawn search thread");
    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn search_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("a/b/c")).unwrap();
        fs::write(root.join("a/notes.txt"), "remember the TODO").unwrap();
        fs::write(root.join("a/b/c/notes.md"), "nothing").unwrap();
        fs::write(root.join("a/b/image.png"), [0u8, 1, 2]).unwrap();

        let collect = |q: &str| {
            let h = start(
                vec![root.to_path_buf()],
                Query::parse(q, false),
                SearchOptions::default(),
                || {},
            );
            let mut names = Vec::new();
            for ev in h.events.iter() {
                match ev {
                    SearchEvent::Results(r) => names.extend(r.into_iter().map(|e| e.name)),
                    SearchEvent::Done { scanned, .. } => {
                        assert_eq!(scanned, 6, "3 folders + 3 files");
                        break;
                    }
                }
            }
            names.sort();
            names
        };
        assert_eq!(collect("notes"), vec!["notes.md", "notes.txt"]);
        assert_eq!(collect("ext:png"), vec!["image.png"]);
        assert_eq!(collect("type:folder"), vec!["a", "b", "c"]);
        assert_eq!(collect("content:todo"), vec!["notes.txt"]);
        assert_eq!(collect("kind:image"), vec!["image.png"]);
        assert_eq!(
            collect(r"/^notes\.(md|txt)$/"),
            vec!["notes.md", "notes.txt"]
        );
    }

    #[test]
    fn indexed_search_reports_results() {
        use crate::index::{FileIndex, IndexConfig};
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/b/report.pdf"), "x").unwrap();
        fs::write(dir.path().join("a/other.pdf"), "x").unwrap();
        let cfg = IndexConfig {
            roots: vec![dir.path().to_path_buf()],
            exclude: vec![],
            threads: 2,
        };
        let idx = Arc::new(FileIndex::build(
            &cfg,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
        ));
        let h = start_indexed(
            idx,
            Some(dir.path().join("a/b")),
            Query::parse("ext:pdf", false),
            SearchOptions::default(),
            || {},
        );
        let mut names = Vec::new();
        for ev in h.events.iter() {
            match ev {
                SearchEvent::Results(r) => names.extend(r.into_iter().map(|e| e.path)),
                SearchEvent::Done { from_index, .. } => {
                    assert!(from_index);
                    break;
                }
            }
        }
        assert_eq!(names, vec![dir.path().join("a/b/report.pdf")]);
    }

    #[test]
    fn filter_chips_toggle_tokens() {
        let kind = &["kind:", "type:"];
        assert_eq!(
            toggle_filter("report", "kind:image", kind),
            "report kind:image"
        );
        assert_eq!(
            toggle_filter("report kind:image", "kind:video", kind),
            "report kind:video"
        );
        assert_eq!(
            toggle_filter("report kind:image", "kind:image", kind),
            "report"
        );
        assert_eq!(
            toggle_filter("\"big file\" type:folder", "kind:image", kind),
            "\"big file\" kind:image"
        );
        assert!(has_filter("a SIZE:>10mb", "size:>10mb"));
    }
}
