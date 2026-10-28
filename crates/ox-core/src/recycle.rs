//! Recycle Bin: list, restore and permanently delete items.
//!
//! Uses the `trash` crate (Shell APIs on Windows, the freedesktop.org trash
//! spec on Linux). Items are exposed as [`FileEntry`]s whose `path` is the
//! trash's own id for the item, so two deleted files with the same original
//! path stay distinct.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use trash::TrashItem;

use crate::entry::FileEntry;

/// One item in the Recycle Bin.
#[derive(Debug, Clone)]
pub struct BinItem {
    pub entry: FileEntry,
    pub item: TrashItem,
}

impl BinItem {
    /// Where the item lived before it was deleted.
    pub fn original_path(&self) -> PathBuf {
        self.item.original_path()
    }
}

/// Recycle Bin contents, keyed by the entry path (the trash id).
#[derive(Debug, Clone, Default)]
pub struct BinListing {
    pub items: HashMap<PathBuf, TrashItem>,
}

impl BinListing {
    pub fn get(&self, path: &std::path::Path) -> Option<&TrashItem> {
        self.items.get(path)
    }

    /// Trash items for a set of entry paths (unknown paths are ignored).
    pub fn select(&self, paths: &[PathBuf]) -> Vec<TrashItem> {
        paths
            .iter()
            .filter_map(|p| self.items.get(p).cloned())
            .collect()
    }
}

pub fn supported() -> bool {
    imp::SUPPORTED
}

/// List everything in the Recycle Bin, newest first.
pub fn list() -> Result<Vec<BinItem>, String> {
    let mut items: Vec<BinItem> = imp::list()?
        .into_iter()
        .map(|item| {
            let (is_dir, size) = imp::size(&item);
            let deleted = deleted_at(&item);
            let name = item.name.to_string_lossy().into_owned();
            let entry = FileEntry {
                path: PathBuf::from(&item.id),
                name,
                is_dir,
                is_symlink: false,
                size,
                modified: Some(deleted),
                created: None,
                hidden: false,
                readonly: false,
            };
            BinItem { entry, item }
        })
        .collect();
    items.sort_by_key(|b| std::cmp::Reverse(b.item.time_deleted));
    Ok(items)
}

/// Put items back where they came from. Fails (and restores nothing more)
/// if something already exists at an original location.
pub fn restore(items: Vec<TrashItem>) -> Result<(), String> {
    imp::restore(items)
}

/// Delete items from the Recycle Bin for good.
pub fn purge(items: Vec<TrashItem>) -> Result<(), String> {
    imp::purge(items)
}

/// Permanently delete everything in the Recycle Bin.
pub fn empty() -> Result<(), String> {
    purge(imp::list()?)
}

/// When the item was moved to the Recycle Bin.
pub fn deleted_at(item: &TrashItem) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(item.time_deleted.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_ignores_unknown_paths() {
        let item = TrashItem {
            id: "trash-id-1".into(),
            name: "a.txt".into(),
            original_parent: "/home/me".into(),
            time_deleted: 10,
        };
        let mut listing = BinListing::default();
        listing
            .items
            .insert(PathBuf::from("trash-id-1"), item.clone());
        let picked = listing.select(&["trash-id-1".into(), "nope".into()]);
        assert_eq!(picked, vec![item]);
        assert_eq!(deleted_at(&picked[0]), UNIX_EPOCH + Duration::from_secs(10));
    }
}

#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
mod imp {
    use trash::{os_limited, Error, TrashItem, TrashItemSize};

    pub const SUPPORTED: bool = true;

    pub fn list() -> Result<Vec<TrashItem>, String> {
        os_limited::list().map_err(describe)
    }

    pub fn size(item: &TrashItem) -> (bool, u64) {
        match os_limited::metadata(item).map(|m| m.size) {
            Ok(TrashItemSize::Bytes(b)) => (false, b),
            Ok(TrashItemSize::Entries(_)) => (true, 0),
            Err(_) => (false, 0),
        }
    }

    pub fn restore(items: Vec<TrashItem>) -> Result<(), String> {
        os_limited::restore_all(items).map_err(describe)
    }

    pub fn purge(items: Vec<TrashItem>) -> Result<(), String> {
        os_limited::purge_all(items).map_err(describe)
    }

    fn describe(e: Error) -> String {
        match e {
            Error::RestoreCollision { path, .. } => format!(
                "\"{}\" already exists. Move or rename it, then restore again.",
                path.display()
            ),
            Error::RestoreTwins { path, .. } => format!(
                "Several deleted items would be restored to \"{}\". Restore them one at a time.",
                path.display()
            ),
            other => other.to_string(),
        }
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
mod imp {
    use trash::TrashItem;

    pub const SUPPORTED: bool = false;
    const MSG: &str = "Browsing the trash is not supported on this platform.";

    pub fn list() -> Result<Vec<TrashItem>, String> {
        Err(MSG.into())
    }

    pub fn size(_item: &TrashItem) -> (bool, u64) {
        (false, 0)
    }

    pub fn restore(_items: Vec<TrashItem>) -> Result<(), String> {
        Err(MSG.into())
    }

    pub fn purge(_items: Vec<TrashItem>) -> Result<(), String> {
        Err(MSG.into())
    }
}
