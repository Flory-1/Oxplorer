//! The Gallery: every picture and video in some folders and their
//! sub-folders (the Pictures library's, as in Windows 11), newest first.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::entry::{FileEntry, FileKind};

/// At most this many pictures and videos are collected.
pub const MAX_ITEMS: usize = 20_000;

/// What a walk found.
#[derive(Debug, Default)]
pub struct Walk {
    /// Newest first.
    pub entries: Vec<FileEntry>,
    /// Stopped at the limit; there are more.
    pub truncated: bool,
    pub cancelled: bool,
}

fn is_media(e: &FileEntry) -> bool {
    matches!(e.kind(), FileKind::Image | FileKind::Video)
}

/// Collect the pictures and videos under `roots` (recursively, not through
/// links, hidden folders only with `show_hidden`), at most `max`. Stops soon
/// after `cancel` is set. Blocks, so call it off the UI thread.
pub fn walk(roots: &[PathBuf], show_hidden: bool, max: usize, cancel: &AtomicBool) -> Walk {
    let mut out = Walk::default();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    // Roots inside other roots would be walked twice.
    let mut stack: Vec<PathBuf> = roots
        .iter()
        .filter(|r| !roots.iter().any(|o| o != *r && r.starts_with(o)))
        .rev()
        .cloned()
        .collect();
    'walk: while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            out.cancelled = true;
            break;
        }
        if !seen.insert(dir.clone()) {
            continue;
        }
        let Ok(items) = fs::read_dir(&dir) else {
            continue;
        };
        let mut files = Vec::new();
        let mut subdirs = Vec::new();
        for item in items.flatten() {
            let Ok(meta) = item.metadata() else { continue };
            let e = FileEntry::from_metadata(item.path(), &meta);
            if e.hidden && !show_hidden {
                continue;
            }
            if e.is_dir {
                if !e.is_symlink {
                    subdirs.push(e.path);
                }
            } else if is_media(&e) {
                files.push(e);
            }
        }
        if !files.is_empty() {
            crate::cloud::refine(&dir, &mut files);
        }
        for e in files {
            if out.entries.len() >= max {
                out.truncated = true;
                break 'walk;
            }
            out.entries.push(e);
        }
        subdirs.sort();
        stack.extend(subdirs.into_iter().rev());
    }
    sort_newest_first(&mut out.entries);
    out
}

/// Newest first (by date modified), then by name.
pub fn sort_newest_first(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| crate::natural::natural_cmp(&a.name, &b.name))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    fn touch(path: &Path, age_secs: u64) {
        fs::write(path, b"x").unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_secs);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    fn names(w: &Walk) -> Vec<&str> {
        w.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn finds_media_recursively_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Pictures");
        fs::create_dir_all(root.join("2024/Holiday")).unwrap();
        touch(&root.join("old.jpg"), 3000);
        touch(&root.join("notes.txt"), 10);
        touch(&root.join("2024/clip.mp4"), 100);
        touch(&root.join("2024/Holiday/beach.PNG"), 10);
        let no = AtomicBool::new(false);
        let w = walk(std::slice::from_ref(&root), false, MAX_ITEMS, &no);
        assert_eq!(names(&w), ["beach.PNG", "clip.mp4", "old.jpg"]);
        assert!(!w.truncated && !w.cancelled);

        // A root inside another isn't walked twice.
        let both = [root.clone(), root.join("2024")];
        assert_eq!(walk(&both, false, MAX_ITEMS, &no).entries.len(), 3);

        // The limit.
        let w = walk(std::slice::from_ref(&root), false, 2, &no);
        assert_eq!(w.entries.len(), 2);
        assert!(w.truncated);

        // Cancelled before it starts.
        let yes = AtomicBool::new(true);
        let w = walk(&[root], false, MAX_ITEMS, &yes);
        assert!(w.cancelled && w.entries.is_empty());
    }

    #[test]
    fn missing_roots_are_empty() {
        let no = AtomicBool::new(false);
        let w = walk(&[PathBuf::from("Z:/no/such/folder")], false, 10, &no);
        assert!(w.entries.is_empty());
    }
}
