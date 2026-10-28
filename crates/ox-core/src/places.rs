//! Quick access: pinned favorites, recent folders/files, known folders and drives.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::entry::display_name;
use crate::settings::{load_toml, save_toml};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Favorite {
    pub path: PathBuf,
    /// Custom display name; defaults to the folder name.
    pub name: Option<String>,
}

impl Favorite {
    pub fn label(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| display_name(&self.path))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentItem {
    pub path: PathBuf,
    /// Unix timestamp of the last visit.
    pub last_used: u64,
    pub count: u32,
}

/// Persistent quick access data (`quick_access.toml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuickAccess {
    pub favorites: Vec<Favorite>,
    pub recent_folders: Vec<RecentItem>,
    pub recent_files: Vec<RecentItem>,
    pub saved_searches: Vec<SavedSearch>,
}

/// A search kept in the sidebar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
    pub query: String,
    /// Folder to search in; `None` = everywhere.
    pub root: Option<PathBuf>,
}

impl Default for QuickAccess {
    fn default() -> Self {
        let favorites = known_folders()
            .into_iter()
            .map(|(name, path)| Favorite {
                path,
                name: Some(name.to_owned()),
            })
            .collect();
        Self {
            favorites,
            recent_folders: Vec::new(),
            recent_files: Vec::new(),
            saved_searches: Vec::new(),
        }
    }
}

const MAX_RECENT: usize = 50;

impl QuickAccess {
    pub fn path() -> PathBuf {
        crate::config_dir().join("quick_access.toml")
    }

    pub fn load() -> Self {
        load_toml(&Self::path()).unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        save_toml(&Self::path(), self)
    }

    pub fn is_favorite(&self, path: &Path) -> bool {
        self.favorites.iter().any(|f| f.path == path)
    }

    pub fn pin(&mut self, path: &Path) {
        if !self.is_favorite(path) {
            self.favorites.push(Favorite {
                path: path.to_path_buf(),
                name: None,
            });
        }
    }

    pub fn unpin(&mut self, path: &Path) {
        self.favorites.retain(|f| f.path != path);
    }

    /// Move a favorite up (`-1`) or down (`+1`).
    pub fn move_favorite(&mut self, path: &Path, delta: isize) {
        if let Some(i) = self.favorites.iter().position(|f| f.path == path) {
            let j = (i as isize + delta).clamp(0, self.favorites.len() as isize - 1) as usize;
            self.favorites.swap(i, j);
        }
    }

    pub fn visit_folder(&mut self, path: &Path) {
        touch(&mut self.recent_folders, path);
    }

    pub fn open_file(&mut self, path: &Path) {
        touch(&mut self.recent_files, path);
    }

    pub fn forget(&mut self, path: &Path) {
        self.recent_folders.retain(|r| r.path != path);
        self.recent_files.retain(|r| r.path != path);
    }

    /// Save a search (replacing one with the same query and folder).
    pub fn save_search(&mut self, query: &str, root: Option<PathBuf>) {
        self.saved_searches
            .retain(|s| !(s.query == query && s.root == root));
        let place = root
            .as_deref()
            .map(display_name)
            .map(|p| format!(" in {p}"))
            .unwrap_or_default();
        self.saved_searches.push(SavedSearch {
            name: format!("{query}{place}"),
            query: query.to_owned(),
            root,
        });
    }

    pub fn clear_recent(&mut self) {
        self.recent_folders.clear();
        self.recent_files.clear();
    }

    /// Recently visited folders, newest first.
    pub fn recent_folders(&self, n: usize) -> impl Iterator<Item = &RecentItem> {
        self.recent_folders.iter().take(n)
    }

    /// Most frequently visited folders ("frecency" lite).
    pub fn frequent_folders(&self, n: usize) -> Vec<&RecentItem> {
        let mut v: Vec<_> = self.recent_folders.iter().collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then(b.last_used.cmp(&a.last_used)));
        v.truncate(n);
        v
    }
}

fn touch(list: &mut Vec<RecentItem>, path: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let count = match list.iter().position(|r| r.path == path) {
        Some(i) => list.remove(i).count + 1,
        None => 1,
    };
    list.insert(
        0,
        RecentItem {
            path: path.to_path_buf(),
            last_used: now,
            count,
        },
    );
    list.truncate(MAX_RECENT);
}

/// Well-known user folders that exist on this machine.
pub fn known_folders() -> Vec<(&'static str, PathBuf)> {
    [
        ("Desktop", dirs::desktop_dir()),
        ("Downloads", dirs::download_dir()),
        ("Documents", dirs::document_dir()),
        ("Pictures", dirs::picture_dir()),
        ("Music", dirs::audio_dir()),
        ("Videos", dirs::video_dir()),
    ]
    .into_iter()
    .filter_map(|(n, p)| p.filter(|p| p.is_dir()).map(|p| (n, p)))
    .collect()
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

#[derive(Debug, Clone)]
pub struct Drive {
    pub mount: PathBuf,
    pub label: String,
    pub file_system: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
    /// `\\server\share` for a mapped network drive.
    pub remote: Option<String>,
}

impl Drive {
    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        1.0 - self.available as f32 / self.total as f32
    }

    pub fn title(&self) -> String {
        let mount = self.mount.to_string_lossy();
        let mount = mount.trim_end_matches(['\\', '/']);
        if let Some(remote) = &self.remote {
            format!("{} ({mount})", crate::network::share_title(remote))
        } else if self.label.is_empty() {
            if self.removable {
                format!("USB Drive ({mount})")
            } else {
                format!("Local Disk ({mount})")
            }
        } else if cfg!(windows) {
            format!("{} ({mount})", self.label)
        } else if mount.is_empty() {
            "/".into()
        } else {
            mount.to_owned()
        }
    }
}

/// List mounted drives. On Linux, pseudo file systems are filtered out.
pub fn drives() -> Vec<Drive> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<Drive> = disks
        .list()
        .iter()
        .filter(|d| {
            let fs = d.file_system().to_string_lossy().to_lowercase();
            !matches!(
                fs.as_str(),
                "overlay" | "tmpfs" | "squashfs" | "devtmpfs" | "proc" | "sysfs"
            )
        })
        .map(|d| Drive {
            mount: d.mount_point().to_path_buf(),
            label: d.name().to_string_lossy().into_owned(),
            file_system: d.file_system().to_string_lossy().into_owned(),
            total: d.total_space(),
            available: d.available_space(),
            removable: d.is_removable(),
            remote: None,
        })
        .collect();
    // Space isn't asked for: a server that's offline can take long to answer.
    out.extend(crate::network::mapped_drives().into_iter().map(|m| Drive {
        mount: m.mount,
        label: String::new(),
        file_system: String::new(),
        total: 0,
        available: 0,
        removable: false,
        remote: Some(m.remote),
    }));
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out.dedup_by(|a, b| a.mount == b.mount);
    if out.is_empty() {
        out.push(Drive {
            mount: PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }),
            label: String::new(),
            file_system: String::new(),
            total: 0,
            available: 0,
            removable: false,
            remote: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_and_recent() {
        let mut qa = QuickAccess {
            favorites: vec![],
            recent_folders: vec![],
            recent_files: vec![],
            saved_searches: vec![],
        };
        qa.pin(Path::new("/a"));
        qa.pin(Path::new("/a"));
        qa.pin(Path::new("/b"));
        assert_eq!(qa.favorites.len(), 2);
        qa.move_favorite(Path::new("/b"), -1);
        assert_eq!(qa.favorites[0].path, Path::new("/b"));
        qa.unpin(Path::new("/b"));
        assert!(!qa.is_favorite(Path::new("/b")));

        qa.visit_folder(Path::new("/x"));
        qa.visit_folder(Path::new("/y"));
        qa.visit_folder(Path::new("/x"));
        let recent: Vec<_> = qa.recent_folders(10).map(|r| r.path.clone()).collect();
        assert_eq!(recent, vec![PathBuf::from("/x"), PathBuf::from("/y")]);
        assert_eq!(qa.frequent_folders(1)[0].count, 2);
    }
}
