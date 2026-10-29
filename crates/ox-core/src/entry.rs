//! Directory entries and listing.

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::natural::natural_cmp;

/// A single file or folder as shown in the file view.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub hidden: bool,
    pub readonly: bool,
    /// OneDrive/Dropbox status ([`crate::cloud`]). From the attributes when
    /// listed; [`crate::cloud::refine`] adds the rest inside sync roots.
    pub cloud: Option<crate::cloud::CloudStatus>,
}

impl FileEntry {
    /// Build an entry from a path, reading its metadata.
    pub fn from_path(path: &Path) -> io::Result<Self> {
        let meta = fs::symlink_metadata(path)?;
        Ok(Self::from_metadata(path.to_path_buf(), &meta))
    }

    pub fn from_metadata(path: PathBuf, meta: &fs::Metadata) -> Self {
        let is_symlink = meta.file_type().is_symlink();
        // For symlinks, look through to the target to decide if it is a folder.
        let (is_dir, size) = if is_symlink {
            match fs::metadata(&path) {
                Ok(m) => (m.is_dir(), if m.is_dir() { 0 } else { m.len() }),
                Err(_) => (false, 0),
            }
        } else {
            (meta.is_dir(), if meta.is_dir() { 0 } else { meta.len() })
        };
        let name = display_name(&path);
        let hidden = is_hidden(&name, meta) || crate::wsl::dotfile_hidden(&path, &name);
        let cloud = cloud_status(meta);
        Self {
            path,
            name,
            is_dir,
            is_symlink,
            size,
            modified: meta.modified().ok(),
            created: meta.created().ok(),
            hidden,
            readonly: meta.permissions().readonly(),
            cloud,
        }
    }

    /// Lower-case extension without the dot (empty for folders).
    pub fn extension(&self) -> String {
        if self.is_dir {
            return String::new();
        }
        // From the name, not the path: Recycle Bin entries have internal paths.
        Path::new(&self.name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }

    /// Human readable type description, e.g. "PDF File" or "File folder".
    pub fn type_label(&self) -> String {
        use crate::i18n::t;
        if self.is_dir {
            return t("File folder").to_owned();
        }
        let ext = self.extension();
        if ext.is_empty() {
            return t("File").to_owned();
        }
        match FileKind::from_extension(&ext) {
            FileKind::Other if crate::i18n::is_german() => format!("{}-Datei", ext.to_uppercase()),
            FileKind::Other => format!("{} File", ext.to_uppercase()),
            kind => format!("{} ({})", t(kind.label()), ext.to_uppercase()),
        }
    }

    pub fn kind(&self) -> FileKind {
        if self.is_dir {
            FileKind::Folder
        } else {
            FileKind::from_extension(&self.extension())
        }
    }

    /// Name shown in the UI, optionally without the extension.
    pub fn display_name(&self, show_extension: bool) -> &str {
        if show_extension || self.is_dir {
            return &self.name;
        }
        match self.name.rfind('.') {
            Some(0) | None => &self.name,
            Some(i) => &self.name[..i],
        }
    }
}

/// Coarse file categories used for icons, colours and search filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    Folder,
    Image,
    Video,
    Audio,
    Document,
    Spreadsheet,
    Presentation,
    Pdf,
    Archive,
    Code,
    Text,
    Executable,
    Font,
    Other,
}

impl FileKind {
    pub fn from_extension(ext: &str) -> Self {
        match ext {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tif" | "tiff"
            | "heic" | "avif" | "raw" | "psd" => Self::Image,
            // Read by Windows' codecs, and SVG compressed.
            e if crate::picture::is_wic(e) || e == "svgz" => Self::Image,
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" => Self::Video,
            "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" => Self::Audio,
            "doc" | "docx" | "odt" | "rtf" => Self::Document,
            "xls" | "xlsx" | "ods" | "csv" => Self::Spreadsheet,
            "ppt" | "pptx" | "odp" => Self::Presentation,
            "pdf" => Self::Pdf,
            "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "zst" | "iso" | "cab" => {
                Self::Archive
            }
            "rs" | "c" | "h" | "cpp" | "hpp" | "cs" | "java" | "py" | "js" | "ts" | "tsx"
            | "jsx" | "go" | "rb" | "php" | "html" | "css" | "scss" | "json" | "toml" | "yaml"
            | "yml" | "xml" | "sh" | "ps1" | "bat" | "cmd" | "sql" | "kt" | "swift" | "lua" => {
                Self::Code
            }
            "txt" | "md" | "log" | "ini" | "cfg" | "conf" => Self::Text,
            "exe" | "msi" | "dll" | "appx" | "msix" | "com" | "scr" => Self::Executable,
            "ttf" | "otf" | "ttc" | "otc" | "woff" | "woff2" | "fon" | "fnt" => Self::Font,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Folder => "Folder",
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Document => "Document",
            Self::Spreadsheet => "Spreadsheet",
            Self::Presentation => "Presentation",
            Self::Pdf => "PDF document",
            Self::Archive => "Archive",
            Self::Code => "Source code",
            Self::Text => "Text document",
            Self::Executable => "Application",
            Self::Font => "Font",
            Self::Other => "File",
        }
    }
}

/// Final path component, or the full path for roots like `C:\` (for a WSL
/// distribution's root: its name).
pub fn display_name(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => crate::wsl::root_name(path)
            .map_or_else(|| path.to_string_lossy().into_owned(), str::to_owned),
    }
}

#[cfg(windows)]
pub(crate) fn is_hidden(_name: &str, meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    meta.file_attributes() & HIDDEN != 0
}

#[cfg(windows)]
fn cloud_status(meta: &fs::Metadata) -> Option<crate::cloud::CloudStatus> {
    use std::os::windows::fs::MetadataExt;
    crate::cloud::from_attributes(meta.file_attributes(), meta.is_dir())
}

#[cfg(not(windows))]
fn cloud_status(_meta: &fs::Metadata) -> Option<crate::cloud::CloudStatus> {
    None
}

#[cfg(not(windows))]
pub(crate) fn is_hidden(name: &str, _meta: &fs::Metadata) -> bool {
    name.starts_with('.')
}

/// Read all entries of a directory. Entries whose metadata can't be read are skipped.
pub fn list_dir(path: &Path, show_hidden: bool) -> io::Result<Vec<FileEntry>> {
    if let Some(server) = crate::network::server_name(path) {
        return crate::network::shares(server);
    }
    let mut out = Vec::new();
    for item in fs::read_dir(path)? {
        let Ok(item) = item else { continue };
        let Ok(meta) = item.metadata() else { continue };
        let entry = FileEntry::from_metadata(item.path(), &meta);
        if !show_hidden && entry.hidden {
            continue;
        }
        out.push(entry);
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortKey {
    #[default]
    Name,
    Modified,
    Type,
    Size,
    Created,
    /// OneDrive / Dropbox state. Not in the menus: the Status column's
    /// header sorts by it.
    Status,
    /// The values read from the files ([`crate::props`]), by their columns'
    /// headers only. Items whose value isn't read yet come last.
    Dimensions,
    Length,
    DateTaken,
    Authors,
    Version,
}

impl SortKey {
    /// Every key, in the order the menus list them.
    pub const ALL: [SortKey; 5] = [
        Self::Name,
        Self::Modified,
        Self::Created,
        Self::Type,
        Self::Size,
    ];

    /// English name (translated where it is shown).
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Modified => "Date modified",
            Self::Created => "Date created",
            Self::Type => "Type",
            Self::Size => "Size",
            Self::Status => "Status",
            Self::Dimensions => "Dimensions",
            Self::Length => "Length",
            Self::DateTaken => "Date taken",
            Self::Authors => "Authors",
            Self::Version => "File version",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

impl SortOrder {
    pub fn toggled(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// Sort entries in place. Folders always come first when `folders_first` is set.
pub fn sort_entries(
    entries: &mut [FileEntry],
    key: SortKey,
    order: SortOrder,
    folders_first: bool,
) {
    entries.sort_by(|a, b| compare(a, b, None, key, order, folders_first));
}

/// Like [`sort_entries`], also by the values read from the files: `props`
/// gives an item's values if they are read ([`SortKey::reads_props`]).
pub fn sort_entries_with<'a>(
    entries: &mut Vec<FileEntry>,
    key: SortKey,
    order: SortOrder,
    folders_first: bool,
    props: impl Fn(&FileEntry) -> Option<&'a crate::props::Props>,
) {
    if !key.reads_props() {
        return sort_entries(entries, key, order, folders_first);
    }
    // Looked up once per item, not once per comparison.
    let mut items: Vec<_> = std::mem::take(entries)
        .into_iter()
        .map(|e| (props(&e), e))
        .collect();
    items.sort_by(|(pa, a), (pb, b)| compare(a, b, Some((*pa, *pb)), key, order, folders_first));
    entries.extend(items.into_iter().map(|(_, e)| e));
}

type PropsPair<'a> = (
    Option<&'a crate::props::Props>,
    Option<&'a crate::props::Props>,
);

fn compare(
    a: &FileEntry,
    b: &FileEntry,
    props: Option<PropsPair<'_>>,
    key: SortKey,
    order: SortOrder,
    folders_first: bool,
) -> Ordering {
    if folders_first && a.is_dir != b.is_dir {
        return if a.is_dir {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    if key.reads_props() {
        // Unknown values stay last; names in their usual order between them.
        let (pa, pb) = props.unwrap_or_default();
        return crate::prop_sort::compare(key, pa, pb, order)
            .then_with(|| natural_cmp(&a.name, &b.name));
    }
    let ord = match key {
        SortKey::Name => natural_cmp(&a.name, &b.name),
        SortKey::Modified => a.modified.cmp(&b.modified),
        SortKey::Created => a.created.cmp(&b.created),
        SortKey::Type => a
            .type_label()
            .cmp(&b.type_label())
            .then_with(|| natural_cmp(&a.name, &b.name)),
        SortKey::Size => a.size.cmp(&b.size),
        SortKey::Status => crate::cloud::sort_rank(a.cloud)
            .cmp(&crate::cloud::sort_rank(b.cloud))
            .then_with(|| natural_cmp(&a.name, &b.name)),
        _ => Ordering::Equal,
    };
    let ord = ord.then_with(|| natural_cmp(&a.name, &b.name));
    match order {
        SortOrder::Ascending => ord,
        SortOrder::Descending => ord.reverse(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_and_sort() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("b10.txt"), "1234").unwrap();
        fs::write(dir.path().join("b2.txt"), "1").unwrap();
        fs::create_dir(dir.path().join("zfolder")).unwrap();
        fs::write(dir.path().join(".hidden"), "").unwrap();

        let mut entries = list_dir(dir.path(), false).unwrap();
        sort_entries(&mut entries, SortKey::Name, SortOrder::Ascending, true);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        if cfg!(windows) {
            // Dotfiles are not hidden on Windows.
            assert_eq!(names, vec!["zfolder", ".hidden", "b2.txt", "b10.txt"]);
        } else {
            assert_eq!(names, vec!["zfolder", "b2.txt", "b10.txt"]);
        }

        sort_entries(&mut entries, SortKey::Size, SortOrder::Descending, false);
        assert_eq!(entries[0].name, "b10.txt");
    }

    #[test]
    fn sorting_by_read_values_puts_unknown_ones_last() {
        let file = |name: &str| FileEntry {
            path: name.into(),
            name: name.into(),
            is_dir: false,
            is_symlink: false,
            size: 0,
            modified: None,
            created: None,
            hidden: false,
            readonly: false,
            cloud: None,
        };
        let mut entries = vec![file("c"), file("b"), file("a"), file("d")];
        let dims = |w, h| crate::props::Props {
            dimensions: Some((w, h)),
            ..Default::default()
        };
        let (big, small) = (dims(100, 100), dims(10, 10));
        let none = crate::props::Props::default();
        let props = |e: &FileEntry| match e.name.as_str() {
            "a" => Some(&big),
            "c" => Some(&small),
            "d" => Some(&none),
            _ => None,
        };
        let names = |v: &[FileEntry]| v.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        let k = SortKey::Dimensions;
        sort_entries_with(&mut entries, k, SortOrder::Ascending, true, props);
        assert_eq!(names(&entries), ["c", "a", "d", "b"]);
        sort_entries_with(&mut entries, k, SortOrder::Descending, true, props);
        assert_eq!(names(&entries), ["a", "c", "d", "b"]);
    }

    #[test]
    fn display_name_without_extension() {
        let e = FileEntry {
            path: "x/report.final.pdf".into(),
            name: "report.final.pdf".into(),
            is_dir: false,
            is_symlink: false,
            size: 0,
            modified: None,
            created: None,
            hidden: false,
            readonly: false,
            cloud: None,
        };
        assert_eq!(e.display_name(false), "report.final");
        assert_eq!(e.display_name(true), "report.final.pdf");
        assert_eq!(e.kind(), FileKind::Pdf);
    }
}
