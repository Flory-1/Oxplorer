//! The view each folder was last shown with (view mode, sort order,
//! columns, grouping), so that e.g. *Downloads* opens sorted by date and *Pictures*
//! as a grid. Stored in `folder_views.toml` in the config directory; only
//! the most recently used folders are kept.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize};

use crate::columns::ColumnLayout;
use crate::entry::{SortKey, SortOrder};
use crate::grouping::GroupBy;
use crate::settings::{load_toml, save_toml, Browsing, ViewMode};

/// Folders remembered at most; the least recently used are forgotten.
pub const MAX_FOLDERS: usize = 1000;

/// How a folder is shown. New fields need a default, so that older files
/// still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedView {
    pub mode: ViewMode,
    pub sort: SortKey,
    pub order: SortOrder,
    /// The folder's own columns; `None` = the default columns.
    pub columns: Option<ColumnLayout>,
    pub group: GroupBy,
}

impl Default for SavedView {
    fn default() -> Self {
        Self::defaults(&Browsing::default())
    }
}

impl SavedView {
    /// The view of folders that don't have their own.
    pub fn defaults(b: &Browsing) -> Self {
        Self {
            mode: b.default_view,
            sort: b.default_sort,
            order: b.default_order,
            columns: None,
            group: b.default_group,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Folder {
    /// The folder, as given (the key is made from it).
    path: PathBuf,
    /// When it was last opened or changed (Unix seconds).
    used: u64,
    view: SavedView,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default, deserialize_with = "readable_folders")]
    folders: Vec<Folder>,
}

/// The entries that can be read; one this version doesn't understand (from
/// a newer one) is skipped instead of losing the whole file.
fn readable_folders<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Folder>, D::Error> {
    let values = Vec::<toml::Value>::deserialize(d)?;
    Ok(values
        .into_iter()
        .filter_map(|v| Folder::deserialize(v).ok())
        .collect())
}

/// The remembered views, by folder.
#[derive(Debug, Default)]
pub struct FolderViews {
    folders: HashMap<String, Folder>,
    /// Changed since it was loaded or saved.
    pub dirty: bool,
}

/// The same key for every spelling of a folder: case-insensitive on
/// Windows, `/` and `\` alike, without a trailing separator.
pub fn key(folder: &Path) -> String {
    let mut s = folder.to_string_lossy().into_owned();
    if cfg!(windows) {
        s = s.replace('/', "\\").to_lowercase();
    }
    let sep = std::path::MAIN_SEPARATOR;
    // Keep the separator of roots (`c:\`, `/`).
    while s.len() > 1 && s.ends_with(sep) && !s[..s.len() - 1].ends_with(':') {
        s.pop();
    }
    s
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl FolderViews {
    pub fn path() -> PathBuf {
        crate::config_dir().join("folder_views.toml")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn save(&mut self) -> io::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        let file: File = load_toml(path).unwrap_or_default();
        let mut views = Self::default();
        for mut f in file.folders {
            if let Some(c) = &mut f.view.columns {
                c.normalize();
            }
            views.folders.insert(key(&f.path), f);
        }
        views.trim();
        views.dirty = false;
        views
    }

    pub fn save_to(&mut self, path: &Path) -> io::Result<()> {
        let mut folders: Vec<Folder> = self.folders.values().cloned().collect();
        // Most recently used first: easier to read.
        folders.sort_by(|a, b| b.used.cmp(&a.used).then_with(|| a.path.cmp(&b.path)));
        save_toml(path, &File { folders })?;
        self.dirty = false;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.folders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
    }

    /// Every folder with a view of its own and that view, the most recently
    /// used first.
    pub fn list(&self) -> Vec<(&Path, &SavedView)> {
        let mut folders: Vec<&Folder> = self.folders.values().collect();
        folders.sort_by(|a, b| b.used.cmp(&a.used).then_with(|| a.path.cmp(&b.path)));
        folders
            .into_iter()
            .map(|f| (f.path.as_path(), &f.view))
            .collect()
    }

    /// The view `folder` has of its own.
    pub fn get(&self, folder: &Path) -> Option<&SavedView> {
        self.folders.get(&key(folder)).map(|f| &f.view)
    }

    /// Remember `view` for `folder`. A view that is just the default one
    /// isn't kept (the folder then follows the defaults).
    pub fn set(&mut self, folder: &Path, view: SavedView, defaults: &SavedView) {
        if view == *defaults {
            self.remove(folder);
            return;
        }
        let f = Folder {
            path: folder.to_path_buf(),
            used: now(),
            view,
        };
        self.folders.insert(key(folder), f);
        self.dirty = true;
        self.trim();
    }

    /// The folder was opened: it is kept longer.
    pub fn touch(&mut self, folder: &Path) {
        if let Some(f) = self.folders.get_mut(&key(folder)) {
            f.used = now();
            self.dirty = true;
        }
    }

    /// Forget the view of `folder`; it gets the default view again.
    pub fn remove(&mut self, folder: &Path) -> bool {
        let removed = self.folders.remove(&key(folder)).is_some();
        self.dirty |= removed;
        removed
    }

    pub fn clear(&mut self) {
        self.dirty |= !self.folders.is_empty();
        self.folders.clear();
    }

    fn trim(&mut self) {
        if self.folders.len() <= MAX_FOLDERS {
            return;
        }
        let mut by_age: Vec<(u64, String)> = self
            .folders
            .iter()
            .map(|(k, f)| (f.used, k.clone()))
            .collect();
        by_age.sort();
        let excess = self.folders.len() - MAX_FOLDERS;
        for (_, k) in by_age.into_iter().take(excess) {
            self.folders.remove(&k);
        }
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columns::ColumnId;

    fn grid() -> SavedView {
        SavedView {
            mode: ViewMode::Grid,
            ..Default::default()
        }
    }

    #[test]
    fn keys_ignore_spelling() {
        assert_eq!(key(Path::new("/home/a/")), key(Path::new("/home/a")));
        if cfg!(windows) {
            assert_eq!(key(Path::new("C:\\Users\\Me\\")), "c:\\users\\me");
            assert_eq!(key(Path::new("c:/users/ME")), "c:\\users\\me");
            assert_eq!(key(Path::new("C:\\")), "c:\\");
        } else {
            assert_eq!(key(Path::new("/")), "/");
        }
    }

    #[test]
    fn remembers_and_forgets() {
        let defaults = SavedView::default();
        let mut v = FolderViews::default();
        let pictures = Path::new("/home/me/Pictures");
        v.set(pictures, grid(), &defaults);
        assert_eq!(v.get(pictures), Some(&grid()));
        assert!(v.dirty);
        if cfg!(windows) {
            assert!(v.get(Path::new("/HOME/me/pictures/")).is_some());
        }
        // Back to the default view: nothing to remember.
        v.set(pictures, defaults.clone(), &defaults);
        assert!(v.get(pictures).is_none());
        v.set(pictures, grid(), &defaults);
        assert!(v.remove(pictures));
        assert!(!v.remove(pictures));
    }

    #[test]
    fn lists_the_most_recently_used_first() {
        let defaults = SavedView::default();
        let mut v = FolderViews::default();
        assert!(v.list().is_empty());
        let (a, b) = (Path::new("/a"), Path::new("/b"));
        v.set(a, grid(), &defaults);
        v.set(b, grid(), &defaults);
        v.folders.get_mut(&key(a)).unwrap().used += 10;
        let list = v.list();
        assert_eq!(list, vec![(a, &grid()), (b, &grid())]);
    }

    #[test]
    fn saves_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("folder_views.toml");
        let defaults = SavedView::default();
        let mut v = FolderViews::default();
        let mut columns = ColumnLayout::default();
        columns.toggle(ColumnId::Dimensions);
        let view = SavedView {
            sort: SortKey::Modified,
            order: SortOrder::Descending,
            columns: Some(columns),
            group: GroupBy::Modified,
            ..grid()
        };
        v.set(Path::new("/a"), view.clone(), &defaults);
        v.set(Path::new("/b"), grid(), &defaults);
        v.save_to(&file).unwrap();
        assert!(!v.dirty);
        let back = FolderViews::load_from(&file);
        assert_eq!(back.len(), 2);
        assert_eq!(back.get(Path::new("/a")), Some(&view));
        // A missing file is no views.
        assert!(FolderViews::load_from(&dir.path().join("none.toml")).is_empty());
    }

    #[test]
    fn entries_it_cannot_read_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("folder_views.toml");
        let text = "[[folders]]\npath = \"/new\"\nused = 2\n[folders.view]\nmode = \"Carousel\"\n\
                    [[folders]]\npath = \"/old\"\nused = 1\n[folders.view]\nmode = \"Grid\"\n";
        std::fs::write(&file, text).unwrap();
        let v = FolderViews::load_from(&file);
        assert_eq!(v.len(), 1);
        assert_eq!(v.get(Path::new("/old")).unwrap().mode, ViewMode::Grid);
        // Missing fields are defaults.
        assert_eq!(v.get(Path::new("/old")).unwrap().sort, SortKey::Name);
    }

    #[test]
    fn only_the_most_recently_used_are_kept() {
        let defaults = SavedView::default();
        let mut v = FolderViews::default();
        for i in 0..MAX_FOLDERS + 5 {
            v.set(Path::new(&format!("/f{i}")), grid(), &defaults);
            // Make the order clear despite the clock's resolution.
            if let Some(f) = v.folders.get_mut(&key(Path::new(&format!("/f{i}")))) {
                f.used = i as u64;
            }
        }
        v.set(Path::new("/last"), grid(), &defaults);
        assert_eq!(v.len(), MAX_FOLDERS);
        assert!(v.get(Path::new("/f0")).is_none());
        assert!(v.get(Path::new("/last")).is_some());
    }
}
