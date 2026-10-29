//! Where a tab points (`Location`) and the listing loaded for it.

use super::*;

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
}

impl Location {
    pub fn title(&self) -> String {
        match self {
            Location::Home => tr("Home").into(),
            Location::Folder(p) => entry::display_name(p),
            Location::Search { query, .. } => ox_core::trf!("Search: {query}", query = query),
            Location::Settings => tr("Settings").into(),
            Location::Help => tr("Help").into(),
            Location::RecycleBin => tr("Recycle Bin").into(),
            Location::Tagged(c) => c.name().into(),
            Location::Archive { archive, inner } => match inner.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => entry::display_name(archive),
            },
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
        }
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
        )
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

    /// Items here can't be changed (inside an archive).
    pub fn is_read_only(&self) -> bool {
        matches!(self, Location::Archive { .. })
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
        Ok(Self {
            entries: entry::list_dir(path, show_hidden)?,
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

    pub fn archive(archive: &Path, inner: &Path) -> io::Result<Self> {
        Ok(Self {
            entries: ox_core::archive::list(archive, inner)?,
            bin: BinListing::default(),
        })
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
