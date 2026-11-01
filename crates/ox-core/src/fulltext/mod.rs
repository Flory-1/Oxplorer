//! The optional text index: the words of documents (text, code, Markdown,
//! PDF, Word, Excel, PowerPoint, OpenDocument) in some folders, so that
//! `content:` searches don't have to read every file.
//!
//! # Design
//!
//! - [`tokens`]: words are runs of letters and digits, lower-cased. Query
//!   words are looked up as *parts* of index words, so the index finds
//!   every file the plain matcher would (and `content:"TOD"` still finds
//!   "TODO").
//! - [`store`]: a small inverted index of its own (word → delta-coded list
//!   of file numbers, no positions), persisted as one file in the cache
//!   folder. `tantivy` would bring a whole search engine for what is a
//!   candidate filter; `fst` would make the lists immutable, and the index
//!   is updated file by file.
//! - [`service`]: the background indexer (one thread, below-normal priority,
//!   paused while files are copied); changed files (size or date) are read
//!   again, deleted ones forgotten.
//! - [`FullTextIndex::resolve`]: for a search, which files may contain the
//!   text. A search then reads only those (the exact matcher confirms them;
//!   for a single word nothing needs to be read), skips the files the index
//!   says don't, and reads files the index doesn't know or that changed
//!   since, as without the index. So results are never missing because the
//!   index is behind.

pub mod extract;
mod lazy;
mod office;
mod pdf_text;
pub mod service;
pub mod store;
pub mod tokens;
mod words;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use service::{FullTextService, FullTextState};
use store::{mtime_of, path_key, Store, Text};

/// Settings of the text index (Settings → Search index).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FullTextSettings {
    /// Index the text of documents (off by default: it reads many files).
    pub enabled: bool,
    /// Folders to index; empty = [`default_folders`].
    pub folders: Vec<PathBuf>,
    /// Larger files aren't indexed (searches read them as without the index).
    pub max_file_mb: u32,
}

impl Default for FullTextSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            folders: Vec::new(),
            max_file_mb: 10,
        }
    }
}

impl FullTextSettings {
    /// What the indexer should do; `None` when it is off. `exclude` are the
    /// folder names the file index skips too (node_modules, .git, …).
    pub fn config(&self, exclude: &[String]) -> Option<FullTextConfig> {
        if !self.enabled {
            return None;
        }
        let folders = if self.folders.is_empty() {
            default_folders()
        } else {
            self.folders.clone()
        };
        Some(FullTextConfig {
            folders,
            exclude: exclude.to_vec(),
            max_file_bytes: u64::from(self.max_file_mb.max(1)) * 1024 * 1024,
        })
    }
}

/// What the indexer indexes.
#[derive(Debug, Clone, PartialEq)]
pub struct FullTextConfig {
    pub folders: Vec<PathBuf>,
    /// File and folder names that are skipped.
    pub exclude: Vec<String>,
    pub max_file_bytes: u64,
}

/// Names of the usual code folders in the home folder.
const CODE_FOLDERS: [&str; 9] = [
    "source/repos",
    "Projects",
    "repos",
    "code",
    "dev",
    "src",
    "git",
    "workspace",
    "Coding",
];

/// Documents, Desktop and the code folders in the home folder that exist
/// (`source\repos` of Visual Studio, `Projects`, `repos`, `code`, …).
pub fn default_folders() -> Vec<PathBuf> {
    let home = crate::places::home_dir();
    let mut all: Vec<PathBuf> = [dirs::document_dir(), dirs::desktop_dir()]
        .into_iter()
        .flatten()
        .chain(CODE_FOLDERS.iter().map(|f| home.join(f)))
        .filter(|p| p.is_dir())
        .collect();
    // One of each (Windows ignores case), and none inside another.
    let keys: Vec<String> = all.iter().map(|p| path_key(p)).collect();
    let mut keep = vec![true; all.len()];
    for i in 0..all.len() {
        for j in 0..all.len() {
            let inside = i != j && service::under(&keys[i], std::slice::from_ref(&keys[j]));
            if inside && (keys[i] != keys[j] || j < i) {
                keep[i] = false;
            }
        }
    }
    let mut k = keep.into_iter();
    all.retain(|_| k.next().unwrap_or(false));
    all
}

/// The text index, shared by the indexer and searches. In memory only
/// while it is used (see [`lazy`]).
#[derive(Debug, Default)]
pub struct FullTextIndex {
    store: lazy::LazyStore,
}

/// What the index says about a file for one search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Doesn't contain the text.
    No,
    /// May contain it: read and check.
    Maybe,
    /// Contains it (a single word, found in the index).
    Yes,
}

#[derive(Debug)]
struct Known {
    size: u64,
    mtime: u64,
    verdict: Verdict,
}

/// The index's answer to one `content:` text, for the files under the
/// searched folders (see [`FullTextIndex::resolve`]).
#[derive(Debug, Default)]
pub struct Resolved {
    files: HashMap<String, Known>,
}

/// Numbers of the index, for the settings page.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    /// Files with text.
    pub documents: usize,
    /// Their total size.
    pub bytes: u64,
    /// Files known (with or without text).
    pub files: usize,
    pub words: usize,
    pub memory: usize,
    /// In memory now (else read again for the next search).
    pub loaded: bool,
}

impl FullTextIndex {
    fn new(cache: PathBuf) -> Self {
        Self {
            store: lazy::LazyStore::new(cache),
        }
    }

    /// Doesn't read the index if it was put away.
    pub fn stats(&self) -> Stats {
        let loaded = self.store.peek(|s| {
            let (documents, bytes) = s.documents();
            Stats {
                documents,
                bytes,
                files: s.len(),
                words: s.word_count(),
                memory: s.memory_bytes(),
                loaded: true,
            }
        });
        loaded.unwrap_or_else(|| {
            let m = self.store.summary().unwrap_or_default();
            Stats {
                documents: m.documents,
                bytes: m.bytes,
                files: m.files,
                words: m.words,
                memory: 0,
                loaded: false,
            }
        })
    }

    /// Does the index have folders that overlap `path` (or any, if `None`)?
    /// Doesn't read the index if it was put away.
    pub fn covers(&self, path: Option<&Path>) -> bool {
        let (files, roots) = match self.store.peek(|s| (s.len(), s.roots.clone())) {
            Some(known) => known,
            None => match self.store.summary() {
                Some(m) => (m.files, m.roots),
                None => return false,
            },
        };
        if files == 0 {
            return false;
        }
        let Some(path) = path else { return true };
        let key = path_key(path);
        roots.iter().any(|r| {
            let r = path_key(r);
            service::under(&key, std::slice::from_ref(&r))
                || service::under(&r, std::slice::from_ref(&key))
        })
    }

    /// Look up `text` (a `content:` text) for the files under `roots` (all
    /// files if empty). `None` if the index can't help: it is empty, doesn't
    /// cover the folders, or the text has no letters or digits.
    pub fn resolve(&self, text: &str, case_sensitive: bool, roots: &[PathBuf]) -> Option<Resolved> {
        let words = tokens::query_words(text);
        let covered = if roots.is_empty() {
            self.covers(None)
        } else {
            roots.iter().any(|r| self.covers(Some(r)))
        };
        if words.is_empty() || !covered {
            return None;
        }
        let exact = !case_sensitive && tokens::is_exact_word(text);
        let root_keys: Vec<String> = roots.iter().map(|r| path_key(r)).collect();
        self.store
            .read(|s| Some(Resolved::of(s, &words, exact, &root_keys)))
    }
}

impl Resolved {
    fn of(s: &Store, words: &[String], exact: bool, root_keys: &[String]) -> Self {
        let hits = s.candidates(words);
        let mut files = HashMap::new();
        for (id, f) in s.files() {
            let key = path_key(&f.path);
            if !root_keys.is_empty() && !service::under(&key, root_keys) {
                continue;
            }
            let verdict = match f.text {
                Text::None => Verdict::No,
                Text::Always => Verdict::Maybe,
                Text::Words if !hits.get(id) => Verdict::No,
                Text::Words if exact => Verdict::Yes,
                Text::Words => Verdict::Maybe,
            };
            let (size, mtime) = (f.size, f.mtime);
            files.insert(
                key,
                Known {
                    size,
                    mtime,
                    verdict,
                },
            );
        }
        Resolved { files }
    }
}

impl Resolved {
    /// Does `path` contain the text (`pattern` is the exact matcher)?
    /// `meta` is its size and modification time if the caller has them
    /// (else they are read). `None` if the index doesn't know the file as
    /// it is now: the caller reads it as without the index.
    pub fn check(
        &self,
        path: &Path,
        meta: Option<(u64, Option<std::time::SystemTime>)>,
        pattern: &regex::bytes::Regex,
    ) -> Option<bool> {
        let known = self.files.get(&path_key(path))?;
        let (size, modified) = match meta {
            Some(m) => m,
            None => {
                let m = std::fs::metadata(path).ok()?;
                (m.len(), m.modified().ok())
            }
        };
        if known.size != size || known.mtime != mtime_of(modified) {
            return None;
        }
        match known.verdict {
            Verdict::No => Some(false),
            Verdict::Yes => Some(true),
            Verdict::Maybe => {
                // Never read an online-only cloud file (it would download).
                let online = std::fs::metadata(path)
                    .map(|m| crate::cloud::online(&m))
                    .unwrap_or(true);
                if online {
                    return Some(false);
                }
                let text = extract::extract(path, size.saturating_add(1)).ok()??;
                Some(pattern.is_match(text.as_bytes()))
            }
        }
    }

    /// Number of files the index answers for.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}
