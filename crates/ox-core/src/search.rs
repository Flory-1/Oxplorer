//! Fast recursive search across folders and drives.
//!
//! The walk runs on many threads (the `ignore` crate's parallel walker, the
//! same one ripgrep uses) and streams results back in batches, so the first
//! hits appear right away even while big drives are still being scanned.
//!
//! # Query syntax
//!
//! | Example            | Meaning                                               |
//! |--------------------|-------------------------------------------------------|
//! | `report`           | name contains "report" (case-insensitive)             |
//! | `"annual report"`  | name contains the exact phrase                        |
//! | `*.rs` / `img_??`  | wildcard match on the whole name                      |
//! | `-tmp`             | name must *not* contain "tmp"                         |
//! | `ext:pdf,docx`     | extension is one of the list                          |
//! | `type:folder`      | only folders (`type:file` for files)                  |
//! | `size:>10mb`       | size filter (`<`, `>`, units b/kb/mb/gb)              |
//! | `modified:7d`      | modified in the last 7 days (`today`, `24h`, `2w`)    |
//! | `content:"TODO"`   | file contents contain the text (small files only)     |

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, Sender};

use crate::entry::FileEntry;

#[derive(Debug, Clone, PartialEq)]
enum Term {
    Contains(String),
    Wildcard(Vec<char>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFilter {
    Files,
    Folders,
}

/// A parsed search query.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    include: Vec<Term>,
    exclude: Vec<Term>,
    extensions: Vec<String>,
    kind: Option<KindFilter>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    modified_within: Option<Duration>,
    content: Option<String>,
    case_sensitive: bool,
}

impl Query {
    pub fn parse(input: &str, case_sensitive: bool) -> Self {
        let mut q = Query {
            case_sensitive,
            ..Default::default()
        };
        for token in tokenize(input) {
            let (negate, token) = match token.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_owned()),
                _ => (false, token),
            };
            if let Some((key, value)) = token.split_once(':') {
                let value = value.trim_matches('"');
                match key.to_ascii_lowercase().as_str() {
                    "ext" => {
                        q.extensions.extend(
                            value
                                .split([',', ';', '|'])
                                .map(|e| e.trim().trim_start_matches('.').to_lowercase())
                                .filter(|e| !e.is_empty()),
                        );
                        continue;
                    }
                    "type" | "kind" => {
                        q.kind = match value.to_ascii_lowercase().as_str() {
                            "folder" | "folders" | "dir" | "directory" => Some(KindFilter::Folders),
                            "file" | "files" => Some(KindFilter::Files),
                            _ => None,
                        };
                        continue;
                    }
                    "size" => {
                        if let Some(rest) = value.strip_prefix('>') {
                            q.min_size = parse_size(rest);
                        } else if let Some(rest) = value.strip_prefix('<') {
                            q.max_size = parse_size(rest);
                        } else if let Some(n) = parse_size(value) {
                            q.min_size = Some(n);
                        }
                        continue;
                    }
                    "modified" | "date" => {
                        q.modified_within = parse_age(value);
                        continue;
                    }
                    "content" | "contains" => {
                        q.content = Some(value.to_owned());
                        continue;
                    }
                    _ => {} // Not a filter, e.g. "C:" — treat as a normal term.
                }
            }
            let term = make_term(&token, case_sensitive);
            if negate {
                q.exclude.push(term);
            } else {
                q.include.push(term);
            }
        }
        q
    }

    /// True if there is nothing to search for.
    pub fn is_empty(&self) -> bool {
        self.include.is_empty()
            && self.exclude.is_empty()
            && self.extensions.is_empty()
            && self.kind.is_none()
            && self.min_size.is_none()
            && self.max_size.is_none()
            && self.modified_within.is_none()
            && self.content.is_none()
    }

    /// Does a file name match the name-based parts of the query?
    pub fn matches_name(&self, name: &str) -> bool {
        let folded;
        let name = if self.case_sensitive {
            name
        } else {
            folded = name.to_lowercase();
            &folded
        };
        self.include.iter().all(|t| term_matches(t, name))
            && !self.exclude.iter().any(|t| term_matches(t, name))
    }

    fn matches(
        &self,
        name: &str,
        is_dir: bool,
        meta: impl Fn() -> Option<fs::Metadata>,
        path: &Path,
    ) -> bool {
        match self.kind {
            Some(KindFilter::Files) if is_dir => return false,
            Some(KindFilter::Folders) if !is_dir => return false,
            _ => {}
        }
        if !self.extensions.is_empty() {
            if is_dir {
                return false;
            }
            let ext = Path::new(name)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !self.extensions.contains(&ext) {
                return false;
            }
        }
        if !self.matches_name(name) {
            return false;
        }
        let needs_meta =
            self.min_size.is_some() || self.max_size.is_some() || self.modified_within.is_some();
        if needs_meta {
            let Some(m) = meta() else { return false };
            if self.min_size.is_some_and(|min| is_dir || m.len() < min) {
                return false;
            }
            if self.max_size.is_some_and(|max| is_dir || m.len() > max) {
                return false;
            }
            if let Some(within) = self.modified_within {
                let fresh = m
                    .modified()
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .is_some_and(|age| age <= within);
                if !fresh {
                    return false;
                }
            }
        }
        if let Some(needle) = &self.content {
            if is_dir || !file_contains(path, needle, self.case_sensitive) {
                return false;
            }
        }
        true
    }
}

fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn make_term(token: &str, case_sensitive: bool) -> Term {
    let unquoted = token.trim_matches('"');
    let text = if case_sensitive {
        unquoted.to_owned()
    } else {
        unquoted.to_lowercase()
    };
    if !token.starts_with('"') && (text.contains('*') || text.contains('?')) {
        Term::Wildcard(text.chars().collect())
    } else {
        Term::Contains(text)
    }
}

fn term_matches(term: &Term, name: &str) -> bool {
    match term {
        Term::Contains(s) => name.contains(s.as_str()),
        Term::Wildcard(p) => {
            let n: Vec<char> = name.chars().collect();
            wildcard(p, &n)
        }
    }
}

/// Iterative `*`/`?` matcher with backtracking (linear in practice).
fn wildcard(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = n;
            p += 1;
        } else if let Some(s) = star {
            p = s + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let idx = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let num: f64 = num.parse().ok()?;
    let mult: f64 = match unit.trim() {
        "" | "b" => 1.0,
        "k" | "kb" => 1024.0,
        "m" | "mb" => 1024.0 * 1024.0,
        "g" | "gb" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" => 1024.0f64.powi(4),
        _ => return None,
    };
    Some((num * mult) as u64)
}

fn parse_age(s: &str) -> Option<Duration> {
    let s = s.trim().to_ascii_lowercase();
    match s.as_str() {
        "today" => return Some(Duration::from_secs(24 * 3600)),
        "week" => return Some(Duration::from_secs(7 * 24 * 3600)),
        "month" => return Some(Duration::from_secs(31 * 24 * 3600)),
        "year" => return Some(Duration::from_secs(365 * 24 * 3600)),
        _ => {}
    }
    let s = s.trim_start_matches('<');
    let idx = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let n: u64 = num.parse().ok()?;
    let secs = match unit {
        "m" | "min" => 60,
        "h" => 3600,
        "" | "d" => 24 * 3600,
        "w" => 7 * 24 * 3600,
        "y" => 365 * 24 * 3600,
        _ => return None,
    };
    Some(Duration::from_secs(n * secs))
}

/// Max file size scanned by `content:` searches.
const CONTENT_LIMIT: u64 = 32 * 1024 * 1024;

fn file_contains(path: &Path, needle: &str, case_sensitive: bool) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    if file
        .metadata()
        .map(|m| m.len() > CONTENT_LIMIT)
        .unwrap_or(true)
    {
        return false;
    }
    let mut data = Vec::new();
    if file.take(CONTENT_LIMIT).read_to_end(&mut data).is_err() {
        return false;
    }
    // Skip binary files (a NUL byte in the first 8 KB).
    if data.iter().take(8192).any(|&b| b == 0) {
        return false;
    }
    let text = String::from_utf8_lossy(&data);
    if case_sensitive {
        text.contains(needle)
    } else {
        text.to_lowercase().contains(&needle.to_lowercase())
    }
}

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
        .spawn(move || run(roots, query, opts, tx, cancel, scanned, Arc::new(notify)))
        .expect("failed to spawn search thread");
    handle
}

fn run(
    roots: Vec<PathBuf>,
    query: Query,
    opts: SearchOptions,
    tx: Sender<SearchEvent>,
    cancel: Arc<AtomicBool>,
    scanned: Arc<AtomicU64>,
    notify: Arc<dyn Fn() + Send + Sync>,
) {
    let started = Instant::now();
    let found = Arc::new(AtomicU64::new(0));
    let truncated = Arc::new(AtomicBool::new(false));
    let Some((first, rest)) = roots.split_first() else {
        let _ = tx.send(SearchEvent::Done {
            elapsed: started.elapsed(),
            scanned: 0,
            truncated: false,
        });
        notify();
        return;
    };

    let mut builder = ignore::WalkBuilder::new(first);
    for r in rest {
        builder.add(r);
    }
    builder
        .standard_filters(false)
        .hidden(!opts.include_hidden)
        .follow_links(false)
        .threads(opts.threads);

    let query = Arc::new(query);
    let max = opts.max_results as u64;
    builder.build_parallel().run(|| {
        let mut batch = Batch {
            items: Vec::new(),
            tx: tx.clone(),
            notify: notify.clone(),
            last_flush: Instant::now(),
            scanned: scanned.clone(),
            local_scanned: 0,
        };
        let query = query.clone();
        let cancel = cancel.clone();
        let found = found.clone();
        let truncated = truncated.clone();
        Box::new(move |result| {
            if cancel.load(Ordering::Relaxed) {
                return ignore::WalkState::Quit;
            }
            let Ok(entry) = result else {
                return ignore::WalkState::Continue;
            };
            if entry.depth() == 0 {
                return ignore::WalkState::Continue;
            }
            batch.local_scanned += 1;
            if batch.local_scanned >= 512 {
                batch.flush_scanned();
            }
            let name = entry.file_name().to_string_lossy();
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            if query.matches(&name, is_dir, || entry.metadata().ok(), entry.path()) {
                if found.fetch_add(1, Ordering::Relaxed) >= max {
                    truncated.store(true, Ordering::Relaxed);
                    cancel.store(true, Ordering::Relaxed);
                    return ignore::WalkState::Quit;
                }
                if let Ok(meta) = entry.metadata() {
                    batch
                        .items
                        .push(FileEntry::from_metadata(entry.path().to_path_buf(), &meta));
                    batch.maybe_flush();
                }
            }
            ignore::WalkState::Continue
        })
    });

    let _ = tx.send(SearchEvent::Done {
        elapsed: started.elapsed(),
        scanned: scanned.load(Ordering::Relaxed),
        truncated: truncated.load(Ordering::Relaxed),
    });
    notify();
}

/// Per-thread result buffer that is flushed periodically and on drop.
struct Batch {
    items: Vec<FileEntry>,
    tx: Sender<SearchEvent>,
    notify: Arc<dyn Fn() + Send + Sync>,
    last_flush: Instant,
    scanned: Arc<AtomicU64>,
    local_scanned: u64,
}

impl Batch {
    fn flush_scanned(&mut self) {
        self.scanned
            .fetch_add(self.local_scanned, Ordering::Relaxed);
        self.local_scanned = 0;
    }

    fn maybe_flush(&mut self) {
        if self.items.len() >= 256 || self.last_flush.elapsed() > Duration::from_millis(100) {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if !self.items.is_empty() {
            let _ = self
                .tx
                .send(SearchEvent::Results(std::mem::take(&mut self.items)));
            (self.notify)();
        }
        self.last_flush = Instant::now();
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        self.flush_scanned();
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query() {
        let q = Query::parse(
            r#"report "big file" -tmp ext:PDF,.docx size:>1mb type:file modified:7d"#,
            false,
        );
        assert_eq!(
            q.include,
            vec![
                Term::Contains("report".into()),
                Term::Contains("big file".into())
            ]
        );
        assert_eq!(q.exclude, vec![Term::Contains("tmp".into())]);
        assert_eq!(q.extensions, vec!["pdf", "docx"]);
        assert_eq!(q.min_size, Some(1024 * 1024));
        assert_eq!(q.kind, Some(KindFilter::Files));
        assert_eq!(q.modified_within, Some(Duration::from_secs(7 * 86400)));
        assert!(!q.is_empty());
        assert!(Query::parse("   ", false).is_empty());
    }

    #[test]
    fn name_matching() {
        let q = Query::parse("*.RS", false);
        assert!(q.matches_name("main.rs"));
        assert!(!q.matches_name("main.rs.bak"));
        let q = Query::parse("img_?? -raw", false);
        assert!(q.matches_name("IMG_01"));
        assert!(!q.matches_name("img_001"));
        let q = Query::parse("Read", true);
        assert!(q.matches_name("Readme"));
        assert!(!q.matches_name("readme"));
    }

    #[test]
    fn wildcards() {
        let w = |p: &str, n: &str| {
            wildcard(
                &p.chars().collect::<Vec<_>>(),
                &n.chars().collect::<Vec<_>>(),
            )
        };
        assert!(w("*", ""));
        assert!(w("a*b*c", "aXXbYYc"));
        assert!(!w("a*b*c", "aXXbYY"));
        assert!(w("*.tar.*", "x.tar.gz"));
    }

    #[test]
    fn sizes_and_ages() {
        assert_eq!(parse_size("10kb"), Some(10 * 1024));
        assert_eq!(
            parse_size("1.5 GB"),
            Some((1.5 * 1024.0 * 1024.0 * 1024.0) as u64)
        );
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_age("24h"), Some(Duration::from_secs(86400)));
        assert_eq!(parse_age("2w"), Some(Duration::from_secs(14 * 86400)));
    }

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
    }
}
