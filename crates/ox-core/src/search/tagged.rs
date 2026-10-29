//! `tag:` queries: only the tagged items need a look, not the whole disk
//! (or index), so both back ends answer them from the tag store.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use super::query::path_key;
use super::{Query, SearchOptions};
use crate::entry::FileEntry;

/// The tagged items inside `roots` (anywhere if empty) that match `query`.
/// Returns them, the number looked at and whether `max_results` cut it off.
pub(super) fn search(
    roots: &[PathBuf],
    query: &Query,
    opts: &SearchOptions,
    cancel: &AtomicBool,
) -> (Vec<FileEntry>, u64, bool) {
    let roots: Vec<PathBuf> = roots.iter().map(|r| PathBuf::from(path_key(r))).collect();
    let mut found = Vec::new();
    let mut scanned = 0;
    for path in query.tagged_paths() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let key = PathBuf::from(path_key(path));
        if !roots.is_empty() && !roots.iter().any(|r| key.starts_with(r) && key != *r) {
            continue;
        }
        scanned += 1;
        let Ok(meta) = fs::symlink_metadata(path) else {
            continue;
        };
        let name = crate::entry::display_name(path);
        if !query.matches(&name, meta.is_dir(), || Some(meta.clone()), path) {
            continue;
        }
        let entry = FileEntry::from_metadata(path.clone(), &meta);
        if entry.hidden && !opts.include_hidden {
            continue;
        }
        if found.len() >= opts.max_results {
            return (found, scanned, true);
        }
        found.push(entry);
    }
    (found, scanned, false)
}
