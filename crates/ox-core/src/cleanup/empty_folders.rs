//! Empty folders: folders with no files in them or in their sub-folders.
//!
//! Only the top-most folder of an empty tree is listed (deleting it deletes
//! the empty folders inside). `desktop.ini` and `Thumbs.db` don't count as
//! content; anything else does, hidden files and links included, so a
//! listed folder never holds anything that would be lost.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::{is_link, Progress, ScanOptions};
use crate::entry;

#[derive(Debug, Default)]
pub struct Report {
    /// Sorted by path.
    pub folders: Vec<PathBuf>,
    /// Folders that couldn't be read (they and the folders above them
    /// don't count as empty).
    pub unreadable: u64,
    pub cancelled: bool,
}

/// Files Windows puts into folders by itself: a folder with only these is empty.
pub fn is_junk(name: &str) -> bool {
    name.eq_ignore_ascii_case("desktop.ini") || name.eq_ignore_ascii_case("thumbs.db")
}

/// Find the empty folders under `root` (never `root` itself). Stops soon
/// after `cancel` is set. Hidden folders are looked into either way (their
/// files count), but listed only with `show_hidden`.
pub fn find(root: &Path, opts: ScanOptions, cancel: &AtomicBool, progress: &Progress) -> Report {
    let mut report = Report::default();
    let mut found = Vec::new();
    visit(root, true, opts, cancel, progress, &mut found, &mut report);
    if cancel.load(Ordering::Relaxed) {
        return Report {
            cancelled: true,
            ..Default::default()
        };
    }
    found.sort();
    report.folders = found;
    report
}

/// Whether `dir` is empty. The top-most empty folders below it go into `found`
/// unless `dir` is empty itself (then it stands for them).
fn visit(
    dir: &Path,
    is_root: bool,
    opts: ScanOptions,
    cancel: &AtomicBool,
    progress: &Progress,
    found: &mut Vec<PathBuf>,
    report: &mut Report,
) -> bool {
    Progress::add(&progress.folders, 1);
    let Ok(items) = fs::read_dir(dir) else {
        report.unreadable += 1;
        return false;
    };
    let mut empty = true;
    // Empty sub-folders, and whether they're hidden.
    let mut below = Vec::new();
    for item in items {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let Some((item, meta)) = item.ok().and_then(|i| {
            let meta = i.metadata().ok()?;
            Some((i, meta))
        }) else {
            // Something that can't be looked at: better not empty.
            empty = false;
            continue;
        };
        let path = item.path();
        if meta.is_dir() && !is_link(&meta) {
            if visit(&path, false, opts, cancel, progress, found, report) {
                let hidden = entry::is_hidden(&item.file_name().to_string_lossy(), &meta);
                below.push((path, hidden));
            } else {
                empty = false;
            }
            continue;
        }
        Progress::add(&progress.files, 1);
        if is_link(&meta) || !is_junk(&item.file_name().to_string_lossy()) {
            empty = false;
        }
    }
    if empty && !is_root {
        return true;
    }
    found.extend(
        below
            .into_iter()
            .filter(|(_, hidden)| opts.show_hidden || !hidden)
            .map(|(p, _)| p),
    );
    false
}
