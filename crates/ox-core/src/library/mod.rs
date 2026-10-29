//! Windows libraries (Documents, Music, Pictures, Videos and the user's
//! own): the `.library-ms` files in `%APPDATA%\Microsoft\Windows\Libraries`,
//! each a list of folders that Explorer shows together.
//!
//! On Windows the Shell reads them (it knows moved folders and localized
//! names); the XML is parsed as a fallback and for the "show in the
//! navigation pane" flag.

use std::path::{Path, PathBuf};

use crate::entry::{self, FileEntry};

#[cfg(windows)]
mod win;

/// A library and its folders.
#[derive(Debug, Clone, PartialEq)]
pub struct Library {
    /// The name shown ("Documents", "Dokumente", or the user's own).
    pub name: String,
    /// The `.library-ms` file.
    pub file: PathBuf,
    /// Its folders that exist, in the library's order.
    pub folders: Vec<PathBuf>,
    /// Where files saved or dropped into the library go.
    pub save_folder: Option<PathBuf>,
    /// Shown in Explorer's navigation pane ("Show in navigation pane").
    pub pinned: bool,
}

impl Library {
    /// The Pictures library (whatever it is called in this language).
    pub fn is_pictures(&self) -> bool {
        stem(&self.file).eq_ignore_ascii_case("Pictures")
    }
}

/// What a `.library-ms` file says.
#[derive(Debug, Default, PartialEq)]
pub struct Description {
    /// `<name>`: a name, or a resource reference such as `@shell32.dll,-34575`.
    pub name: Option<String>,
    pub pinned: bool,
    /// The `<url>` of each location and whether it is the default save location.
    pub locations: Vec<(String, bool)>,
}

/// Parse the XML of a `.library-ms` file (only what we need, no full XML).
pub fn parse(xml: &str) -> Description {
    let mut d = Description {
        name: tag(xml, "name").map(unescape),
        pinned: tag(xml, "isLibraryPinned").is_none_or(|v| v.trim() != "false"),
        locations: Vec::new(),
    };
    for block in xml.split("<searchConnectorDescription").skip(1) {
        let block = block
            .split("</searchConnectorDescription>")
            .next()
            .unwrap_or("");
        if let Some(url) = tag(block, "url") {
            let save = tag(block, "isDefaultSaveLocation").is_some_and(|v| v.trim() == "true");
            d.locations.push((unescape(url.trim()), save));
        }
    }
    d
}

/// Text inside the first `<name>…</name>` (with or without attributes).
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let mut rest = xml;
    loop {
        let start = rest.find(&open)?;
        let after = &rest[start + open.len()..];
        // `<name>` or `<name attr…>`, not `<nameOther>`.
        if after.starts_with('>') || after.starts_with(char::is_whitespace) {
            let body = &after[after.find('>')? + 1..];
            return Some(&body[..body.find(&format!("</{name}>"))?]);
        }
        rest = after;
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn stem(file: &Path) -> String {
    file.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The folder with the user's libraries (Windows only).
pub fn libraries_dir() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    Some(
        dirs::data_dir()?
            .join("Microsoft")
            .join("Windows")
            .join("Libraries"),
    )
}

/// The standard libraries first, in Explorer's order, then the others by name.
fn order(lib: &Library) -> (usize, String) {
    const STANDARD: [&str; 4] = ["documents", "music", "pictures", "videos"];
    let stem = stem(&lib.file).to_lowercase();
    let pos = STANDARD.iter().position(|s| *s == stem);
    (pos.unwrap_or(STANDARD.len()), lib.name.to_lowercase())
}

/// All libraries of the user, standard ones first. Blocks (the Shell reads
/// them), so call it off the UI thread.
pub fn list() -> Vec<Library> {
    let Some(dir) = libraries_dir() else {
        return Vec::new();
    };
    list_in(&dir)
}

fn list_in(dir: &Path) -> Vec<Library> {
    let Ok(items) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Library> = items
        .flatten()
        .map(|i| i.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("library-ms"))
        })
        .filter_map(|p| load(&p))
        .collect();
    out.sort_by_key(order);
    out
}

/// Read one library.
pub fn load(file: &Path) -> Option<Library> {
    let xml = std::fs::read_to_string(file).ok();
    let desc = xml.as_deref().map(parse).unwrap_or_default();
    #[cfg(windows)]
    if let Some(mut lib) = win::load(file) {
        lib.pinned = desc.pinned;
        return Some(lib);
    }
    xml.as_ref()?;
    from_description(file, &desc)
}

/// A library from its parsed XML: locations resolved and checked.
fn from_description(file: &Path, desc: &Description) -> Option<Library> {
    let mut folders = Vec::new();
    let mut save_folder = None;
    for (url, save) in &desc.locations {
        let Some(path) = resolve(url).filter(|p| p.is_dir()) else {
            continue;
        };
        if *save && save_folder.is_none() {
            save_folder = Some(path.clone());
        }
        if !folders.contains(&path) {
            folders.push(path);
        }
    }
    let name = match &desc.name {
        Some(n) if !n.starts_with('@') && !n.trim().is_empty() => n.trim().to_owned(),
        _ => stem(file),
    };
    Some(Library {
        name,
        file: file.to_path_buf(),
        save_folder: save_folder.or_else(|| folders.first().cloned()),
        folders,
        pinned: desc.pinned,
    })
}

/// A location's `<url>` as a path: a plain path, a `file:` URL or a known
/// folder (`knownfolder:{GUID}`, Windows).
fn resolve(url: &str) -> Option<PathBuf> {
    if let Some(id) = url.strip_prefix("knownfolder:") {
        #[cfg(windows)]
        return win::known_folder(id);
        #[cfg(not(windows))]
        {
            let _ = id;
            return None;
        }
    }
    let path = url.strip_prefix("file:///").unwrap_or(url);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// The folders of the Pictures library, or the Pictures folder if there is
/// no such library (what the Gallery shows).
pub fn pictures_folders() -> Vec<PathBuf> {
    let from_library = list()
        .into_iter()
        .find(Library::is_pictures)
        .map(|l| l.folders)
        .filter(|f| !f.is_empty());
    from_library
        .or_else(|| dirs::picture_dir().map(|p| vec![p]))
        .unwrap_or_default()
}

/// The items of all `folders` together (a library's contents), with their
/// cloud status. Folders that can't be read are skipped.
pub fn entries(folders: &[PathBuf], show_hidden: bool) -> Vec<FileEntry> {
    let mut out = Vec::new();
    for folder in folders {
        let Ok(mut items) = entry::list_dir(folder, show_hidden) else {
            continue;
        };
        crate::cloud::refine(folder, &mut items);
        out.append(&mut items);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<libraryDescription xmlns="http://schemas.microsoft.com/windows/2009/library">
  <name>@shell32.dll,-34575</name>
  <version>2</version>
  <isLibraryPinned>true</isLibraryPinned>
  <iconReference>imageres.dll,-1002</iconReference>
  <searchConnectorDescriptionList>
    <searchConnectorDescription publisher="Microsoft" product="Windows">
      <description>@shell32.dll,-34577</description>
      <isDefaultSaveLocation>true</isDefaultSaveLocation>
      <simpleLocation>
        <url>knownfolder:{FDD39AD0-238F-46AF-ADB4-6C85480369C7}</url>
        <serialized>MBAAAEAFCAAAAAAAADAAAAAAAY0gAAQDRAAAA</serialized>
      </simpleLocation>
    </searchConnectorDescription>
  </searchConnectorDescriptionList>
</libraryDescription>"#;

    #[test]
    fn parses_the_documents_library() {
        let d = parse(DOCUMENTS);
        assert_eq!(d.name.as_deref(), Some("@shell32.dll,-34575"));
        assert!(d.pinned);
        assert_eq!(
            d.locations,
            vec![(
                "knownfolder:{FDD39AD0-238F-46AF-ADB4-6C85480369C7}".to_owned(),
                true
            )]
        );
    }

    #[test]
    fn parses_own_libraries() {
        let xml = r#"<libraryDescription>
  <name>Photos &amp; Scans</name>
  <isLibraryPinned>false</isLibraryPinned>
  <searchConnectorDescriptionList>
    <searchConnectorDescription>
      <simpleLocation><url>D:\Photos</url></simpleLocation>
    </searchConnectorDescription>
    <searchConnectorDescription>
      <isDefaultSaveLocation>true</isDefaultSaveLocation>
      <isSupported>false</isSupported>
      <simpleLocation><url>E:\Scans</url></simpleLocation>
    </searchConnectorDescription>
  </searchConnectorDescriptionList>
</libraryDescription>"#;
        let d = parse(xml);
        assert_eq!(d.name.as_deref(), Some("Photos & Scans"));
        assert!(!d.pinned);
        assert_eq!(
            d.locations,
            vec![
                (r"D:\Photos".to_owned(), false),
                (r"E:\Scans".to_owned(), true)
            ]
        );
        // No `<isLibraryPinned>`: shown.
        assert!(parse("<libraryDescription/>").pinned);
        assert_eq!(parse("<x>").locations, Vec::new());
    }

    #[test]
    fn library_from_folders_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::fs::write(a.join("one.txt"), "1").unwrap();
        std::fs::write(b.join("two.txt"), "2").unwrap();
        let xml = format!(
            "<libraryDescription><name>Work</name><searchConnectorDescriptionList>\
             <searchConnectorDescription><url>{}</url></searchConnectorDescription>\
             <searchConnectorDescription><isDefaultSaveLocation>true</isDefaultSaveLocation>\
             <url>{}</url></searchConnectorDescription>\
             <searchConnectorDescription><url>{}</url></searchConnectorDescription>\
             </searchConnectorDescriptionList></libraryDescription>",
            a.display(),
            b.display(),
            tmp.path().join("gone").display()
        );
        let file = tmp.path().join("Work.library-ms");
        let lib = from_description(&file, &parse(&xml)).unwrap();
        assert_eq!(lib.name, "Work");
        assert_eq!(lib.folders, vec![a.clone(), b.clone()]);
        assert_eq!(lib.save_folder, Some(b.clone()));
        assert!(!lib.is_pictures());

        let mut names: Vec<String> = entries(&lib.folders, true)
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, vec!["one.txt", "two.txt"]);

        // Without a save location the first folder is used; resource names
        // fall back to the file name.
        let xml = xml
            .replace("<isDefaultSaveLocation>true</isDefaultSaveLocation>", "")
            .replace("<name>Work</name>", "<name>@shell32.dll,-1</name>");
        let lib = from_description(&tmp.path().join("Pictures.library-ms"), &parse(&xml)).unwrap();
        assert_eq!(lib.save_folder, Some(a));
        assert_eq!(lib.name, "Pictures");
        assert!(lib.is_pictures());
    }

    #[test]
    fn standard_libraries_come_first() {
        let lib = |stem: &str, name: &str| Library {
            name: name.into(),
            file: PathBuf::from(format!("{stem}.library-ms")),
            folders: Vec::new(),
            save_folder: None,
            pinned: true,
        };
        let mut libs = [
            lib("Zeta", "Aaa"),
            lib("Videos", "Videos"),
            lib("Documents", "Dokumente"),
            lib("Camera Roll", "Camera Roll"),
        ];
        libs.sort_by_key(order);
        let names: Vec<&str> = libs.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Dokumente", "Videos", "Aaa", "Camera Roll"]);
    }

    #[test]
    fn urls() {
        assert_eq!(resolve(r"D:\Photos"), Some(PathBuf::from(r"D:\Photos")));
        assert_eq!(resolve("file:///D:/x"), Some(PathBuf::from("D:/x")));
        assert_eq!(resolve(""), None);
        if cfg!(windows) {
            // FOLDERID_Documents
            let docs = resolve("knownfolder:{FDD39AD0-238F-46AF-ADB4-6C85480369C7}");
            assert!(docs.is_some_and(|p| p.is_absolute()));
            assert_eq!(resolve("knownfolder:{not a guid}"), None);
        }
    }

    /// The real libraries of this user, read-only: whatever there is must load.
    #[test]
    fn real_libraries_load() {
        for lib in list() {
            assert!(!lib.name.is_empty());
            assert!(lib.folders.iter().all(|f| f.is_absolute()));
        }
        let _ = pictures_folders();
    }
}
