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
//! | `tag:red,"To do"`    | tagged with one of the colours (name or custom label) |
//! | `archives:yes`       | also match the items inside zip, 7z and tar archives  |
//!
//! `tag:` needs [`Query::resolve_tags`]; both back ends then only look at the
//! tagged items. `archives:yes` works in the walk only (the index holds file
//! system names): it lists every archive the walk meets (tar and tar.gz up
//! to 256 MB) and applies the name, type, size and date filters to the items
//! inside, whose results carry virtual paths (`a.zip\inner\file`, see
//! [`crate::archive::split_virtual`]). `content:` isn't searched in archives.

mod archives;
mod query;
mod tagged;
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
    if query.has_tags() {
        // Quicker from the tag store than from millions of index entries.
        return start(within.into_iter().collect(), query, opts, notify);
    }
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
/// results are available (e.g. to repaint the UI). A `tag:` query with no
/// `roots` finds tagged items anywhere.
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

    /// All results of a search, sorted by path.
    fn results(h: SearchHandle) -> Vec<FileEntry> {
        let mut out = Vec::new();
        for ev in h.events.iter() {
            match ev {
                SearchEvent::Results(r) => out.extend(r),
                SearchEvent::Done { .. } => break,
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    #[test]
    fn tag_queries_in_both_back_ends() {
        use crate::index::{FileIndex, IndexConfig};
        use crate::tags::{TagColor, TagStore};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("a/b")).unwrap();
        for f in ["a/report.pdf", "a/b/plan.txt", "a/b/other.txt"] {
            fs::write(root.join(f), "x").unwrap();
        }
        let mut store = TagStore::default();
        store.toggle(&[root.join("a/report.pdf")], TagColor::Red);
        store.toggle(
            &[root.join("a/b/plan.txt"), root.join("a/b")],
            TagColor::Blue,
        );
        store.set_label(TagColor::Blue, "Work");
        // Tagged, but outside the searched folder.
        store.toggle(&[PathBuf::from("/elsewhere/x.txt")], TagColor::Red);
        let query = |q: &str| {
            let mut q = Query::parse(q, false);
            q.resolve_tags(&store);
            q
        };
        let paths = |r: Vec<FileEntry>| r.into_iter().map(|e| e.path).collect::<Vec<_>>();

        let walk = |q: &str| {
            paths(results(start(
                vec![root.to_path_buf()],
                query(q),
                SearchOptions::default(),
                || {},
            )))
        };
        assert_eq!(walk("tag:red"), [root.join("a/report.pdf")]);
        assert_eq!(
            walk("tag:work"),
            [root.join("a/b"), root.join("a/b/plan.txt")]
        );
        assert_eq!(
            walk("tag:red,blue type:file plan"),
            [root.join("a/b/plan.txt")]
        );
        assert!(walk("tag:green").is_empty());

        let idx = Arc::new(FileIndex::build(
            &IndexConfig {
                roots: vec![root.to_path_buf()],
                exclude: vec![],
                threads: 2,
            },
            &AtomicBool::new(false),
            &AtomicU64::new(0),
        ));
        let indexed = |q: &str, within: Option<PathBuf>| {
            paths(results(start_indexed(
                idx.clone(),
                within,
                query(q),
                SearchOptions::default(),
                || {},
            )))
        };
        assert_eq!(
            indexed("tag:blue ext:txt", Some(root.to_path_buf())),
            [root.join("a/b/plan.txt")]
        );
        assert_eq!(
            indexed("tag:red", None),
            [PathBuf::from("/elsewhere/x.txt"), root.join("a/report.pdf")]
                .into_iter()
                .filter(|p| p.exists())
                .collect::<Vec<_>>()
        );
        // The index applies `tag:` on its own too.
        let (ids, _) = idx.search(
            &query("tag:red"),
            &crate::index::IndexQuery {
                max_results: 10,
                ..Default::default()
            },
        );
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn archive_items_are_searched_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let src = root.join("src");
        fs::create_dir_all(src.join("Docs/sub")).unwrap();
        fs::write(src.join("Docs/report.pdf"), "x".repeat(2000)).unwrap();
        fs::write(src.join("Docs/sub/notes.txt"), "TODO").unwrap();
        let zip = root.join("in").join("docs.zip");
        fs::create_dir_all(root.join("in")).unwrap();
        crate::archive::compress_zip(&[src.join("Docs")], &zip, &AtomicBool::new(false)).unwrap();
        let tar = root.join("in").join("docs.tar");
        {
            let mut b = tar::Builder::new(fs::File::create(&tar).unwrap());
            b.append_dir_all("Docs", src.join("Docs")).unwrap();
            b.finish().unwrap();
        }
        fs::remove_dir_all(&src).unwrap();

        let walk = |q: &str| {
            let r = results(start(
                vec![root.join("in")],
                Query::parse(q, false),
                SearchOptions::default(),
                || {},
            ));
            r.into_iter().map(|e| e.path).collect::<Vec<_>>()
        };
        assert!(walk("report").is_empty(), "off by default");
        // Sorted by path: the tar comes first.
        let reports = vec![
            tar.join("Docs").join("report.pdf"),
            zip.join("Docs").join("report.pdf"),
        ];
        assert_eq!(walk("report archives:yes"), reports);
        assert_eq!(walk("archives:yes size:>1kb kind:document"), reports);
        // Folders count, also when only implied by the paths of their files.
        let folders = walk("archives:yes type:folder sub");
        assert_eq!(
            folders,
            [tar.join("Docs").join("sub"), zip.join("Docs").join("sub")]
        );
        // The archives themselves still match, and `content:` stays outside.
        assert_eq!(
            walk("archives:yes docs"),
            [tar.clone(), tar.join("Docs"), zip.clone(), zip.join("Docs")]
        );
        assert!(walk("archives:yes content:todo").is_empty());

        // Results open in the archive view.
        let (arc, inner) = crate::archive::split_virtual(&folders[1]).unwrap();
        assert_eq!(
            (arc, inner),
            (zip.clone(), PathBuf::from("Docs").join("sub"))
        );
    }
}
