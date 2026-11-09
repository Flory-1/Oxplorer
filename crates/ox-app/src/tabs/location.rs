//! Where a tab points (`Location`) and the listing loaded for it.

use super::*;
use ox_core::cleanup::CleanupMode;

/// What a tab is showing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Location {
    /// Quick access / favorites start page.
    Home,
    Folder(PathBuf),
    /// Search results. `root: None` searches all drives.
    Search {
        root: Option<PathBuf>,
        query: String,
    },
    Settings,
    /// The Help page.
    Help,
    RecycleBin,
    /// Everything with this colour tag, wherever it is.
    Tagged(TagColor),
    /// A folder inside a zip or tar file (read-only). `inner` is empty at
    /// the archive's top level.
    Archive {
        archive: PathBuf,
        inner: PathBuf,
    },
    /// A Windows library (Documents, Music, …): the items of all its folders.
    Library {
        /// Its name when it was opened (for titles).
        name: String,
        /// The `.library-ms` file.
        file: PathBuf,
    },
    /// Every picture and video of the Pictures library, newest first.
    Gallery,
    /// Duplicate files or empty folders under `root`, to review and delete.
    Cleanup {
        root: PathBuf,
        mode: CleanupMode,
    },
    /// What takes up the space in a folder or drive (a treemap).
    DiskUsage(PathBuf),
    /// A folder on an SFTP, FTP or WebDAV server (see `app/remote.rs`).
    Remote(ox_core::remote::RemoteAddr),
}

impl Location {
    pub fn title(&self) -> String {
        match self {
            Location::Home => tr("Home").into(),
            Location::Folder(p) => entry::display_name(p),
            Location::Search { query, .. } if query.trim().is_empty() => tr("Search").into(),
            Location::Search { query, .. } => ox_core::trf!("Search: {query}", query = query),
            Location::Settings => tr("Settings").into(),
            Location::Help => tr("Help").into(),
            Location::RecycleBin => tr("Recycle Bin").into(),
            Location::Tagged(c) => c.name().into(),
            Location::Archive { archive, inner } => match inner.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => entry::display_name(archive),
            },
            Location::Library { name, .. } => name.clone(),
            Location::Gallery => tr("Gallery").into(),
            Location::Cleanup { root, mode } => {
                let name = entry::display_name(root);
                match mode {
                    CleanupMode::Duplicates => ox_core::trf!("Duplicates: {name}", name = name),
                    CleanupMode::EmptyFolders => {
                        ox_core::trf!("Empty folders: {name}", name = name)
                    }
                }
            }
            Location::DiskUsage(p) => {
                ox_core::trf!("Disk usage: {name}", name = entry::display_name(p))
            }
            Location::Remote(addr) => addr.title(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Location::Home => "🏠",
            Location::Folder(p) if p.parent().is_none() => "🖴",
            Location::Folder(_) => "📁",
            Location::Search { .. } => "🔍",
            Location::Settings => "⚙",
            Location::Help => "❓",
            Location::RecycleBin => "🗑",
            Location::Tagged(_) => "🏷",
            Location::Archive { .. } => "📦",
            Location::Library { .. } => "📚",
            Location::Gallery => "🎞",
            Location::Cleanup { mode, .. } => Self::cleanup_icon(*mode),
            Location::DiskUsage(_) => "🖴",
            Location::Remote(addr) if addr.parent().is_none() => "🌐",
            Location::Remote(_) => "📁",
        }
    }

    /// The icon of a clean-up page.
    pub fn cleanup_icon(mode: CleanupMode) -> &'static str {
        match mode {
            CleanupMode::Duplicates => "📋",
            CleanupMode::EmptyFolders => "📂",
        }
    }

    /// Where a search's `root` is: a folder, or a library (searched from a
    /// library, the root is its `.library-ms` file).
    pub fn search_root(root: PathBuf) -> Location {
        if ox_core::library::is_library_file(&root) {
            let name = root
                .file_stem()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            return Location::Library { name, file: root };
        }
        Location::Folder(root)
    }

    /// The folder this location represents, if any (used for paste, new folder, …).
    pub fn folder(&self) -> Option<&Path> {
        match self {
            Location::Folder(p) => Some(p),
            _ => None,
        }
    }

    pub fn shows_files(&self) -> bool {
        matches!(
            self,
            Location::Folder(_)
                | Location::Search { .. }
                | Location::RecycleBin
                | Location::Tagged(_)
                | Location::Archive { .. }
                | Location::Library { .. }
                | Location::Gallery
                | Location::Remote(_)
        )
    }

    /// Items from several folders (a library, the Gallery): shown with
    /// their folder, like search results.
    pub fn is_collection(&self) -> bool {
        matches!(self, Location::Library { .. } | Location::Gallery)
    }

    pub fn is_bin(&self) -> bool {
        matches!(self, Location::RecycleBin)
    }

    /// Settings, Help and the Recycle Bin: there is one of each, so opening
    /// one again switches to the tab that already shows it.
    pub fn is_page(&self) -> bool {
        matches!(
            self,
            Location::Settings | Location::Help | Location::RecycleBin
        )
    }

    /// A folder on a server.
    pub fn is_remote(&self) -> bool {
        matches!(self, Location::Remote(_))
    }

    /// The folder above a remote folder (for "Up"; `None` elsewhere).
    pub fn remote_up(&self) -> Option<Location> {
        match self {
            Location::Remote(addr) => addr.parent().map(Location::Remote),
            _ => None,
        }
    }

    /// Items here can't be changed (inside an archive).
    pub fn is_read_only(&self) -> bool {
        match self {
            Location::Archive { .. } => true,
            // Phones are read-only for now (see `ox_core::remote::mtp`).
            Location::Remote(addr) => addr.is_phone(),
            _ => false,
        }
    }
}

/// Result of loading a location in the background.
pub struct Listing {
    pub entries: Vec<FileEntry>,
    /// Trash items behind the entries, when showing the Recycle Bin.
    pub bin: BinListing,
}

impl Listing {
    pub fn folder(path: &Path, show_hidden: bool) -> io::Result<Self> {
        let mut entries = entry::list_dir(path, show_hidden)?;
        // Inside OneDrive & co.: "on this device" and "sync pending".
        ox_core::cloud::refine(path, &mut entries);
        Ok(Self {
            entries,
            bin: BinListing::default(),
        })
    }

    /// The items in `paths` that still exist.
    pub fn items(paths: &[PathBuf]) -> Self {
        Self {
            entries: paths
                .iter()
                .filter_map(|p| FileEntry::from_path(p).ok())
                .collect(),
            bin: BinListing::default(),
        }
    }

    /// The items of all folders of a library.
    pub fn library(file: &Path, show_hidden: bool) -> Result<Self, String> {
        let lib = ox_core::library::load(file)
            .ok_or_else(|| tr("This library no longer exists.").to_owned())?;
        Ok(Self {
            entries: ox_core::library::entries(&lib.folders, show_hidden),
            bin: BinListing::default(),
        })
    }

    /// The Gallery's pictures and videos (stops soon after `cancel` is set).
    pub fn gallery(show_hidden: bool, cancel: &std::sync::atomic::AtomicBool) -> Self {
        let roots = ox_core::library::pictures_folders();
        let walk = ox_core::gallery::walk(&roots, show_hidden, ox_core::gallery::MAX_ITEMS, cancel);
        Self {
            entries: walk.entries,
            bin: BinListing::default(),
        }
    }

    pub fn archive(archive: &Path, inner: &Path) -> io::Result<Self> {
        Ok(Self {
            entries: ox_core::archive::list(archive, inner)?,
            bin: BinListing::default(),
        })
    }

    /// Entries listed on a server.
    pub fn remote(entries: Vec<FileEntry>) -> Self {
        Self {
            entries,
            bin: BinListing::default(),
        }
    }

    pub fn recycle_bin() -> Result<Self, String> {
        let items = ox_core::recycle::list()?;
        let mut bin = BinListing::default();
        let entries = items
            .into_iter()
            .map(|b| {
                bin.items.insert(b.entry.path.clone(), b.item);
                b.entry
            })
            .collect();
        Ok(Self { entries, bin })
    }
}
