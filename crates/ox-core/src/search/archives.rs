//! `archives:yes`: matching the names of the items inside zip, 7z and tar
//! archives that the walk meets.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::Query;
use crate::archive::{self, Kind};
use crate::entry::FileEntry;

/// Biggest tar / tar.gz searched: those are read from start to end to list
/// them. Zip and 7z keep a table of contents, so their size doesn't matter.
pub(super) const TAR_LIMIT: u64 = 256 * 1024 * 1024;

/// The items of `path` (if it's an archive) that match `query`, with virtual
/// paths (`archive.zip\inner\file`), and how many items it holds.
pub(super) fn search(path: &Path, meta: &fs::Metadata, query: &Query) -> (Vec<FileEntry>, u64) {
    let Some(kind) = archive::kind(path) else {
        return (Vec::new(), 0);
    };
    // Contents aren't searched inside archives and their items carry no tags;
    // online-only cloud files would have to be downloaded.
    if query.needs_content() || query.has_tags() || !meta.is_file() || crate::cloud::online(meta) {
        return (Vec::new(), 0);
    }
    if matches!(kind, Kind::Tar | Kind::TarGz) && meta.len() > TAR_LIMIT {
        return (Vec::new(), 0);
    }
    let Ok(index) = archive::read_index(path) else {
        return (Vec::new(), 0);
    };
    // Folders are often only implied by the paths of the files inside them.
    let mut items: BTreeMap<PathBuf, (bool, u64, Option<SystemTime>)> = BTreeMap::new();
    for e in &index.entries {
        for dir in e.path.ancestors().skip(1) {
            if dir.as_os_str().is_empty() {
                break;
            }
            items.entry(dir.to_path_buf()).or_insert((true, 0, None));
        }
        let size = if e.is_dir { 0 } else { e.size };
        items.insert(e.path.clone(), (e.is_dir, size, e.modified));
    }
    let found = items
        .iter()
        .filter_map(|(inner, &(is_dir, size, modified))| {
            let name = inner.file_name()?.to_string_lossy().into_owned();
            query
                .matches_meta(&name, is_dir, size, modified)
                .then(|| FileEntry {
                    path: path.join(inner),
                    name,
                    is_dir,
                    is_symlink: false,
                    size,
                    modified,
                    created: None,
                    hidden: false,
                    readonly: true,
                    cloud: None,
                })
        })
        .collect();
    (found, items.len() as u64)
}
