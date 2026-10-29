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
    /// Pictures in remote folders are downloaded (up to 8 MB) for thumbnails.
    pub remote_thumbnails: bool,
    /// Icon size of the grid view in points (Ctrl+mouse wheel, see `grid_zoom`).
    pub grid_size: u32,
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
            remote_thumbnails: true,
            grid_size: crate::grid_zoom::DEFAULT,
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
    /// How folders without a view of their own group their items.
    pub default_group: crate::grouping::GroupBy,
    /// Open items with a single click instead of a double click.
    pub single_click_open: bool,
    pub confirm_delete: bool,
    /// Delete moves items to the Recycle Bin (Shift+Delete always deletes permanently).
    pub delete_to_trash: bool,
    /// Keep what can be undone across restarts (see `undo::Store`).
    pub keep_undo_history: bool,
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
    /// Each folder keeps the view it was last shown with (see `folder_views`).
    pub remember_folder_views: bool,
    /// Columns of the details view.
    pub columns: crate::columns::ColumnLayout,
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
            default_group: Default::default(),
            single_click_open: false,
            confirm_delete: true,
            delete_to_trash: true,
            keep_undo_history: true,
            date_format: "%d.%m.%Y %H:%M".into(),
            show_status_bar: true,
            show_preview: false,
            auto_refresh: true,
            folder_sizes: true,
            open_archives: true,
            git_status: true,
            remember_folder_views: true,
            columns: Default::default(),
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
    /// Ask about a failed file while the job runs (retry / skip / skip all).
    pub ask_on_error: bool,
    /// What happens when the last transfer is done.
    pub when_done: crate::transfer::when_done::WhenDone,
    /// A Windows notification when transfers are done while Oxplorer is in the background.
    pub notify_done: bool,
    /// Copy permissions (ACLs) along with the files.
    pub keep_permissions: bool,
    /// Copy alternate data streams (the internet mark is always kept).
    pub keep_streams: bool,
    /// Copied folders get the dates of the originals.
    pub keep_folder_dates: bool,
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
            ask_on_error: true,
            when_done: Default::default(),
            notify_done: true,
            keep_permissions: cfg.keep_permissions,
            keep_streams: cfg.keep_streams,
            keep_folder_dates: cfg.keep_folder_dates,
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
            ask_on_error: self.ask_on_error,
            journal_dir: Some(crate::transfer::journal::default_dir()),
            keep_permissions: self.keep_permissions,
            keep_streams: self.keep_streams,
            keep_folder_dates: self.keep_folder_dates,
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
    /// Apply file changes to the index as they happen.
    pub index_live: bool,
    /// Read NTFS drives through the Oxplorer index service (Windows; it must
    /// be installed by the installer). Off by default: the service shows the
    /// names of all files on the drives.
    pub index_service: bool,
    /// Show results while typing (needs the index).
    pub search_as_you_type: bool,
    /// Remember the searches that were run and suggest them again.
    pub remember_searches: bool,
    /// The text index of documents for `content:` searches.
    pub fulltext: crate::fulltext::FullTextSettings,
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

    /// What live updates follow: the indexed folders `cfg`, except those on
    /// removable and network drives (a watch would keep a USB stick from
    /// being ejected) and in WSL distributions (it would keep WSL running).
    /// `None` if live updates are off or nothing is left.
    pub fn live_config(
        &self,
        cfg: &crate::index::IndexConfig,
        drives: &[crate::places::Drive],
    ) -> Option<crate::index::IndexConfig> {
        if !self.index_live {
            return None;
        }
        let mut cfg = cfg.clone();
        cfg.roots.retain(|r| {
            !crate::wsl::is_wsl_path(r)
                && !drives
                    .iter()
                    .any(|d| (d.removable || d.remote.is_some()) && r.starts_with(&d.mount))
        });
        (!cfg.roots.is_empty()).then_some(cfg)
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
            index_live: true,
            index_service: false,
            search_as_you_type: true,
            remember_searches: true,
            fulltext: Default::default(),
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
    /// The Gallery and the Windows libraries (Documents, Music, …).
    pub show_libraries: bool,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            visible: true,
            show_favorites: true,
            show_drives: true,
            show_recent: true,
            recent_count: 8,
            show_libraries: true,
        }
    }
}

/// The preview pane (shown or hidden with `Browsing::show_preview`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preview {
    /// Show it as a separate window instead of a pane on the right.
    pub as_window: bool,
    /// Selecting another item stops the player, instead of it playing on
    /// in the mini player while you browse.
    pub stop_player_on_select: bool,
    /// Office documents, mails and other files Windows has a preview
    /// handler for, shown by that handler (the Document tab).
    pub documents: bool,
    /// Rotating and flipping JPEGs re-encodes them upright, instead of only
    /// changing their EXIF orientation.
    pub reencode_rotated_jpegs: bool,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            as_window: false,
            stop_player_on_select: false,
            documents: true,
            reencode_rotated_jpegs: false,
        }
    }
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
    /// The user's own commands (Settings → Custom commands).
    pub commands: crate::commands::CustomCommands,
    /// Saved SFTP/FTP/WebDAV places (never with passwords).
    pub remote: crate::remote::RemotePlaces,
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
        let mut s: Self = load_toml(&Self::path()).unwrap_or_default();
        s.browsing.columns.normalize();
        s
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
    fn live_updates_skip_removable_and_network_drives() {
        let drive = |mount: &str, removable: bool, remote: Option<&str>| crate::places::Drive {
            mount: PathBuf::from(mount),
            label: String::new(),
            file_system: String::new(),
            total: 0,
            available: 0,
            removable,
            remote: remote.map(String::from),
        };
        let drives = [
            drive("/fixed", false, None),
            drive("/usb", true, None),
            drive("/net", false, Some(r"\\server\share")),
        ];
        let mut s = Search {
            index_roots: ["/fixed", "/usb/photos", "/net", r"\\wsl$\Ubuntu\home"]
                .map(PathBuf::from)
                .to_vec(),
            ..Search::default()
        };
        let cfg = s.index_config(&[]);
        let live = s.live_config(&cfg, &drives).unwrap();
        assert_eq!(live.roots, vec![PathBuf::from("/fixed")]);
        assert_eq!(live.exclude, cfg.exclude);
        s.index_roots = vec![PathBuf::from("/usb")];
        assert!(s.live_config(&s.index_config(&[]), &drives).is_none());
        s.index_live = false;
        assert!(s.live_config(&cfg, &drives).is_none());
    }

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

    #[test]
    fn columns_round_trip() {
        use crate::columns::ColumnId;
        let mut s = Settings::default();
        s.browsing.columns.toggle(ColumnId::Dimensions);
        s.browsing.columns.set_width(ColumnId::Size, 120.0);
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.browsing.columns, s.browsing.columns);
    }
}
