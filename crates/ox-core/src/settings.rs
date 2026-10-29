//! User settings, stored as TOML in the config directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::entry::{SortKey, SortOrder};
use crate::transfer::{ConflictPolicy, EngineConfig};

/// Language of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    /// The language Windows is set to (German if it is German, else English).
    #[default]
    System,
    English,
    German,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Windows 11 material behind the window's panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Backdrop {
    #[default]
    Off,
    Mica,
    Acrylic,
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
    pub compact_rows: bool,
    /// Show the real Windows file icons instead of the built-in ones.
    pub system_icons: bool,
    /// Pictures and videos show a preview in the grid view.
    pub thumbnails: bool,
    /// Mica or Acrylic behind the sidebar and header (Windows 11).
    pub backdrop: Backdrop,
    /// The tabs take the place of the window's title bar (Windows).
    pub tabs_in_title_bar: bool,
    /// Interface language.
    pub language: Language,
    /// Colour scheme used in dark mode (see `themes`).
    pub dark_theme: String,
    /// Colour scheme used in light mode.
    pub light_theme: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent: [0x4c, 0x8d, 0xf6],
            compact_rows: false,
            system_icons: true,
            thumbnails: true,
            backdrop: Backdrop::Off,
            tabs_in_title_bar: true,
            language: Language::System,
            dark_theme: crate::themes::DEFAULT_DARK.to_owned(),
            light_theme: crate::themes::DEFAULT_LIGHT.to_owned(),
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
    /// Show the preview pane on the right.
    pub show_preview: bool,
    /// Refresh folders automatically when they change on disk.
    pub auto_refresh: bool,
    /// Calculate folder sizes in the background for the Size column.
    pub folder_sizes: bool,
    /// Open zip, 7z and tar files like folders (instead of the default app).
    pub open_archives: bool,
    /// Show the Git column and branch inside repositories.
    pub git_status: bool,
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
            show_preview: false,
            auto_refresh: true,
            folder_sizes: true,
            open_archives: true,
            git_status: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Startup {
    pub start_page: StartPage,
    pub start_folder: Option<PathBuf>,
    /// Look for a newer release on GitHub (once a day) and say so on Home.
    pub check_updates: bool,
}

impl Default for Startup {
    fn default() -> Self {
        Self {
            start_page: StartPage::default(),
            start_folder: None,
            check_updates: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transfer {
    /// Files copied in parallel within one job. `0` = automatic (by drive type).
    pub workers: usize,
    pub buffer_kib: usize,
    /// Files at least this large use unbuffered I/O.
    pub unbuffered_threshold_mib: u64,
    pub max_concurrent_jobs: usize,
    /// Run jobs on the same drive one after another.
    pub per_disk_queue: bool,
    /// Check every copied file against the original.
    pub verify: bool,
    /// Per-transfer speed limit in MiB/s, `0` = unlimited.
    pub speed_limit_mib: u32,
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
            per_disk_queue: cfg.per_disk_queue,
            verify: cfg.verify,
            speed_limit_mib: 0,
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
            workers: self.workers,
            buffer_size: self.buffer_kib.max(64) * 1024,
            unbuffered_threshold: self.unbuffered_threshold_mib.max(1) * 1024 * 1024,
            max_concurrent_jobs: self.max_concurrent_jobs.max(1),
            per_disk_queue: self.per_disk_queue,
            preserve_timestamps: self.preserve_timestamps,
            verify: self.verify,
            speed_limit: u64::from(self.speed_limit_mib) * 1024 * 1024,
            journal_dir: Some(crate::transfer::journal::default_dir()),
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
    /// Keep an index of all file names for instant search across drives.
    pub index_enabled: bool,
    /// Folders to index; empty = automatic (all drives on Windows, the home folder elsewhere).
    pub index_roots: Vec<PathBuf>,
    /// Folder/file names that are never indexed.
    pub index_exclude: Vec<String>,
    /// Rebuild the index every N minutes (0 = only on start and on demand).
    pub index_refresh_minutes: u32,
    /// Show results while typing (needs the index).
    pub search_as_you_type: bool,
}

impl Search {
    /// What to index, given the currently mounted drives.
    pub fn index_config(&self, drives: &[PathBuf]) -> crate::index::IndexConfig {
        let roots = if !self.index_roots.is_empty() {
            self.index_roots.clone()
        } else if cfg!(windows) {
            drives.to_vec()
        } else {
            vec![crate::places::home_dir()]
        };
        crate::index::IndexConfig {
            roots,
            exclude: self.index_exclude.clone(),
            threads: self.threads,
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Self {
            include_hidden: false,
            case_sensitive: false,
            max_results: 50_000,
            threads: 0,
            instant_filter: true,
            index_enabled: true,
            index_roots: Vec::new(),
            index_exclude: [
                "node_modules",
                ".git",
                "$Recycle.Bin",
                "System Volume Information",
            ]
            .map(String::from)
            .to_vec(),
            index_refresh_minutes: 30,
            search_as_you_type: true,
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

/// The preview pane (shown or hidden with `Browsing::show_preview`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Preview {
    /// Show it as a separate window instead of a pane on the right.
    pub as_window: bool,
}

/// Settings → Developer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Developer {
    /// Command line that opens a file or folder in the editor; `{path}` is
    /// replaced with it.
    pub editor: String,
    /// "Open in editor", "Copy path as" and "Git" in the context menus.
    pub context_menus: bool,
}

impl Default for Developer {
    fn default() -> Self {
        Self {
            editor: "code \"{path}\"".into(),
            context_menus: true,
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
    pub preview: Preview,
    /// Extra entries of the "New" menus (Python script, Markdown, …).
    pub new_items: NewItems,
    /// Changed keyboard shortcuts: command name → keys ("" = none).
    pub shortcuts: std::collections::BTreeMap<String, String>,
    pub terminal: Terminal,
    pub developer: Developer,
}

/// A file type in the "New" menus: its label, the file name it starts with
/// and what is written into it. `{name}` (the file name without extension)
/// and `{date}` (today, YYYY-MM-DD) in the content are filled in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NewItem {
    pub label: String,
    pub file_name: String,
    pub content: String,
}

impl NewItem {
    fn new(label: &str, file_name: &str, content: &str) -> Self {
        Self {
            label: label.to_owned(),
            file_name: file_name.to_owned(),
            content: content.to_owned(),
        }
    }

    /// The content for a file called `file_name`, placeholders filled in.
    pub fn content_for(&self, file_name: &str) -> String {
        let stem = std::path::Path::new(file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.content
            .replace("{name}", &stem)
            .replace("{date}", &today)
    }
}

/// The user's "New" menu entries (shown after Folder and Text document).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NewItems(pub Vec<NewItem>);

impl Default for NewItems {
    fn default() -> Self {
        Self(vec![
            NewItem::new("Markdown document", "Document.md", "# {name}\n\n"),
            NewItem::new(
                "Python script",
                "script.py",
                "#!/usr/bin/env python3\n\"\"\"{name}\"\"\"\n\n\ndef main():\n    pass\n\n\nif __name__ == \"__main__\":\n    main()\n",
            ),
            NewItem::new(
                "PowerShell script",
                "script.ps1",
                "# {name}.ps1 ({date})\n\nparam()\n\n",
            ),
            NewItem::new("JSON file", "data.json", "{\n}\n"),
        ])
    }
}

/// The built-in terminal panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Terminal {
    /// Shell command line; empty = PowerShell on Windows, `$SHELL` elsewhere.
    pub shell: String,
    /// `cd` into the folder whenever the active tab changes folder.
    pub follow_folder: bool,
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
