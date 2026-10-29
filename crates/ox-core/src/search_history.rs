//! Search history: the searches that were run, newest first, offered again
//! under the search box. Stored in `search_history.toml` in the config
//! folder; nothing leaves the machine.
//!
//! Several Oxplorer windows may change the file, so every change reads it
//! again first ([`SearchHistory::update`]), like the named sessions.

mod suggest;

pub use suggest::{completions, history_matches, suggest, Suggestion, SuggestionKind};

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Searches remembered at most; older ones are forgotten.
pub const MAX_ENTRIES: usize = 30;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchHistory {
    /// Newest first, without duplicates.
    #[serde(default)]
    pub entries: Vec<String>,
}

/// Searches are compared without surrounding spaces and case.
fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

impl SearchHistory {
    pub fn path() -> PathBuf {
        crate::config_dir().join("search_history.toml")
    }

    /// The saved history; empty if the file is missing or unreadable.
    pub fn load(path: &Path) -> Self {
        let mut h: Self = crate::settings::load_toml(path).unwrap_or_default();
        h.tidy();
        h
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        crate::settings::save_toml(path, self)
    }

    /// Read the file again, change it and write it back if `change` says it
    /// changed something; returns the history as it is now.
    pub fn update(path: &Path, change: impl FnOnce(&mut Self) -> bool) -> io::Result<Self> {
        let mut h = Self::load(path);
        if change(&mut h) {
            h.save(path)?;
        }
        Ok(h)
    }

    /// A search was run: put it first (moving it up if it was there
    /// already). Returns whether the history changed.
    pub fn add(&mut self, query: &str) -> bool {
        let query = query.trim();
        if query.is_empty() || self.entries.first().is_some_and(|e| e == query) {
            return false;
        }
        self.entries.retain(|e| !same(e, query));
        self.entries.insert(0, query.to_owned());
        self.entries.truncate(MAX_ENTRIES);
        true
    }

    /// Forget one search; `false` if it wasn't there.
    pub fn remove(&mut self, query: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| !same(e, query));
        self.entries.len() != before
    }

    /// Forget all searches; `false` if there were none.
    pub fn clear(&mut self) -> bool {
        let had = !self.entries.is_empty();
        self.entries.clear();
        had
    }

    /// Clean up a file that was edited by hand: no empty entries or
    /// duplicates, not too many.
    fn tidy(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        self.entries.retain_mut(|e| {
            *e = e.trim().to_owned();
            let key = e.to_lowercase();
            if e.is_empty() || seen.contains(&key) {
                return false;
            }
            seen.push(key);
            true
        });
        self.entries.truncate(MAX_ENTRIES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_puts_newest_first_without_duplicates() {
        let mut h = SearchHistory::default();
        assert!(h.add("report"));
        assert!(h.add(" kind:image "));
        assert!(!h.add("   "));
        assert!(!h.add("kind:image"), "already first");
        // Again, other case: moved up, newest spelling kept.
        assert!(h.add("Report"));
        assert_eq!(h.entries, ["Report", "kind:image"]);
    }

    #[test]
    fn keeps_only_the_newest() {
        let mut h = SearchHistory::default();
        for i in 0..MAX_ENTRIES + 5 {
            h.add(&format!("q{i}"));
        }
        assert_eq!(h.entries.len(), MAX_ENTRIES);
        assert_eq!(h.entries[0], format!("q{}", MAX_ENTRIES + 4));
        assert!(!h.entries.contains(&"q0".to_owned()));
    }

    #[test]
    fn remove_and_clear() {
        let mut h = SearchHistory::default();
        h.add("a");
        h.add("b");
        assert!(h.remove("A "));
        assert!(!h.remove("a"));
        assert_eq!(h.entries, ["b"]);
        assert!(h.clear());
        assert!(!h.clear());
        assert!(h.entries.is_empty());
    }

    #[test]
    fn update_reads_what_others_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search_history.toml");
        assert!(SearchHistory::load(&path).entries.is_empty());
        SearchHistory::update(&path, |h| h.add("one")).unwrap();
        // Another window adds one; ours keeps it.
        SearchHistory::update(&path, |h| h.add("two")).unwrap();
        let h = SearchHistory::update(&path, |h| h.add("three")).unwrap();
        assert_eq!(h.entries, ["three", "two", "one"]);
        assert_eq!(SearchHistory::load(&path), h);
        // Nothing changed: not written.
        std::fs::remove_file(&path).unwrap();
        SearchHistory::update(&path, |h| h.remove("nope")).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn hand_edited_files_are_tidied() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search_history.toml");
        std::fs::write(&path, "entries = [\" a \", \"\", \"A\", \"b\"]\n").unwrap();
        assert_eq!(SearchHistory::load(&path).entries, ["a", "b"]);
        std::fs::write(&path, "not toml [").unwrap();
        assert!(SearchHistory::load(&path).entries.is_empty());
    }
}
