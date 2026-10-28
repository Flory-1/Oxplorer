//! User settings, stored as TOML in the config directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::entry::{SortKey, SortOrder};
use crate::transfer::{ConflictPolicy, EngineConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewMode {
    #[default]
    Details,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StartPage {
    /// The Quick Access / Favorites page.
    #[default]
    Home,
    /// Reopen the tabs and groups from last time.
    RestoreSession,
    /// Always open a specific folder.
    Folder,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: Theme,
    /// Accent colour as RGB.
    pub accent: [u8; 3],
    /// UI zoom factor (1.0 = 100 %).
    pub ui_scale: f32,
    pub compact_rows: bool,
    /// Show the real Windows file icons instead of the built-in ones.
    pub system_icons: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent: [0x4c, 0x8d, 0xf6],
            ui_scale: 1.0,
            compact_rows: false,
            system_icons: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Browsing {
    pub show_hidden: bool,
    pub show_extensions: bool,
    pub folders_first: bool,
    pub default_view: ViewMode,
    pub default_sort: SortKey,
    pub default_order: SortOrder,
    /// Open items with a single click instead of a double click.
    pub single_click_open: bool,
    pub confirm_delete: bool,
    /// Delete moves items to the Recycle Bin (Shift+Delete always deletes permanently).
    pub delete_to_trash: bool,
    /// `chrono` format string for dates.
    pub date_format: String,
    pub show_status_bar: bool,
    /// Refresh folders automatically when they change on disk.
    pub auto_refresh: bool,
}

impl Default for Browsing {
    fn default() -> Self {
        Self {
            show_hidden: false,
            show_extensions: true,
            folders_first: true,
            default_view: ViewMode::Details,
            default_sort: SortKey::Name,
            default_order: SortOrder::Ascending,
            single_click_open: false,
            confirm_delete: true,
            delete_to_trash: true,
            date_format: "%d.%m.%Y %H:%M".into(),
            show_status_bar: true,
            auto_refresh: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Startup {
    pub start_page: StartPage,
    pub start_folder: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transfer {
    /// Files copied in parallel within one job.
    pub workers: usize,
    pub buffer_kib: usize,
    /// Files at least this large use unbuffered I/O.
    pub unbuffered_threshold_mib: u64,
    pub max_concurrent_jobs: usize,
    pub default_conflict: ConflictPolicy,
    pub preserve_timestamps: bool,
    /// Share copied/cut files with Windows Explorer and other apps.
    pub use_system_clipboard: bool,
    /// Dragging onto a folder on the same drive moves (like Explorer), otherwise copies.
    pub drag_moves_on_same_drive: bool,
}

impl Default for Transfer {
    fn default() -> Self {
        let cfg = EngineConfig::default();
        Self {
            workers: cfg.workers,
            buffer_kib: cfg.buffer_size / 1024,
            unbuffered_threshold_mib: cfg.unbuffered_threshold / (1024 * 1024),
            max_concurrent_jobs: cfg.max_concurrent_jobs,
            default_conflict: ConflictPolicy::Ask,
            preserve_timestamps: cfg.preserve_timestamps,
            use_system_clipboard: true,
            drag_moves_on_same_drive: true,
        }
    }
}

impl Transfer {
    pub fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            workers: self.workers.max(1),
            buffer_size: self.buffer_kib.max(64) * 1024,
            unbuffered_threshold: self.unbuffered_threshold_mib.max(1) * 1024 * 1024,
            max_concurrent_jobs: self.max_concurrent_jobs.max(1),
            preserve_timestamps: self.preserve_timestamps,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Search {
    pub include_hidden: bool,
    pub case_sensitive: bool,
    pub max_results: usize,
    /// 0 = automatic (one per CPU core).
    pub threads: usize,
    /// Filter the current folder while typing in the search box.
    pub instant_filter: bool,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            include_hidden: false,
            case_sensitive: false,
            max_results: 50_000,
            threads: 0,
            instant_filter: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sidebar {
    pub visible: bool,
    pub show_favorites: bool,
    pub show_drives: bool,
    pub show_recent: bool,
    pub recent_count: usize,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            visible: true,
            show_favorites: true,
            show_drives: true,
            show_recent: true,
            recent_count: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub browsing: Browsing,
    pub startup: Startup,
    pub transfer: Transfer,
    pub search: Search,
    pub sidebar: Sidebar,
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::config_dir().join("settings.toml")
    }

    /// Load settings, falling back to defaults if the file is missing or broken.
    pub fn load() -> Self {
        load_toml(&Self::path()).unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        save_toml(&Self::path(), self)
    }
}

/// Read a TOML file into `T`. Returns `None` if missing or unparsable.
pub fn load_toml<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

/// Write `value` as TOML atomically (temp file + rename).
pub fn save_toml<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let text = toml::to_string_pretty(value).map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut s = Settings::default();
        s.browsing.show_hidden = true;
        s.transfer.workers = 8;
        save_toml(&path, &s).unwrap();
        let loaded: Settings = load_toml(&path).unwrap();
        assert_eq!(loaded, s);

        // Unknown or missing keys fall back to defaults.
        fs::write(&path, "[browsing]\nshow_hidden = true\n[unknown]\nx = 1\n").unwrap();
        let partial: Settings = load_toml(&path).unwrap();
        assert!(partial.browsing.show_hidden);
        assert_eq!(partial.transfer, Transfer::default());
    }
}
