//! Archives as folders: list, read and extract zip, 7z and tar (.tar,
//! .tar.gz, .tgz) files, and compress files to zip.
//!
//! Items inside an archive have *virtual* paths: the archive's path joined
//! with the path inside it (`C:\Downloads\photos.zip\2024\a.jpg`).
//! [`split_virtual`] turns such a path back into (archive, inner path).

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::entry::FileEntry;
use crate::fsops;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    SevenZip,
    Tar,
    TarGz,
}

/// The archive type of `path`, judged by its name.
pub fn kind(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if name.ends_with(".zip") {
        Some(Kind::Zip)
    } else if name.ends_with(".7z") {
        Some(Kind::SevenZip)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some(Kind::TarGz)
    } else if name.ends_with(".tar") {
        Some(Kind::Tar)
    } else {
        None
    }
}

/// The archive's name without its archive extension (`photos.tar.gz` → `photos`).
pub fn base_name(path: &Path) -> String {
    let name = crate::entry::display_name(path);
    let lower = name.to_lowercase();
    for ext in [".tar.gz", ".tgz", ".tar", ".zip", ".7z"] {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].to_owned();
        }
    }
    name
}

/// `path` is an archive file Oxplorer can open like a folder.
pub fn is_archive(path: &Path) -> bool {
    kind(path).is_some() && path.is_file()
}

/// Split a virtual path into (archive file, path inside the archive).
/// Returns `None` for ordinary paths.
pub fn split_virtual(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let archive = path
        .ancestors()
        .find(|a| kind(a).is_some() && a.is_file())?;
    let inner = path.strip_prefix(archive).ok()?.to_path_buf();
    Some((archive.to_path_buf(), inner))
}

/// One file or folder stored in an archive.
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    /// Path inside the archive (only normal components).
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Position in the zip's central directory.
    index: usize,
}

#[derive(Debug, Default)]
pub struct Index {
    pub entries: Vec<ArchiveEntry>,
}

impl Index {
    /// Whether `inner` is a folder in the archive (stored or implied).
    pub fn is_dir(&self, inner: &Path) -> bool {
        inner.as_os_str().is_empty()
            || self.entries.iter().any(|e| {
                (e.is_dir && e.path == inner) || (e.path != inner && e.path.starts_with(inner))
            })
    }
}

type Stamp = (Option<SystemTime>, u64);
type Cache = HashMap<PathBuf, (Stamp, Arc<Index>)>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The table of contents of `archive` (cached until the file changes).
pub fn index(archive: &Path) -> io::Result<Arc<Index>> {
    let meta = fs::metadata(archive)?;
    let stamp = (meta.modified().ok(), meta.len());
    if let Some((s, idx)) = CACHE
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .get(archive)
    {
        if *s == stamp {
            return Ok(idx.clone());
        }
    }
    let kind = kind(archive)
        .ok_or_else(|| io::Error::other(crate::i18n::t("Not a supported archive.")))?;
    let idx = Arc::new(match kind {
        Kind::Zip => zip_index(archive)?,
        Kind::SevenZip => sevenz_index(archive)?,
        Kind::Tar | Kind::TarGz => tar_index(archive, kind)?,
    });
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    if cache.len() > 32 {
        cache.clear();
    }
    cache.insert(archive.to_path_buf(), (stamp, idx.clone()));
    Ok(idx)
}

/// Only plain relative paths are accepted (no `..`, roots or drive letters),
/// so nothing can be written outside the target folder.
fn clean(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn zip_err(e: zip::result::ZipError) -> io::Error {
    match e {
        zip::result::ZipError::Io(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

fn zip_time(t: Option<zip::DateTime>) -> Option<SystemTime> {
    use chrono::TimeZone;
    let naive = chrono::NaiveDateTime::try_from(t?).ok()?;
    let local = chrono::Local.from_local_datetime(&naive).earliest()?;
    Some(local.into())
}

fn open_zip(archive: &Path) -> io::Result<zip::ZipArchive<BufReader<File>>> {
    zip::ZipArchive::new(BufReader::new(File::open(archive)?)).map_err(zip_err)
}

fn zip_index(archive: &Path) -> io::Result<Index> {
    let mut zip = open_zip(archive)?;
    let mut entries = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let f = zip.by_index_raw(i).map_err(zip_err)?;
        let Some(path) = f.enclosed_name().as_deref().and_then(clean) else {
            continue;
        };
        entries.push(ArchiveEntry {
            path,
            is_dir: f.is_dir(),
            size: f.size(),
            modified: zip_time(f.last_modified()),
            index: i,
        });
    }
    Ok(Index { entries })
}

fn sevenz_err(e: sevenz_rust2::Error) -> io::Error {
    io::Error::other(e.to_string())
}

fn open_sevenz(archive: &Path) -> io::Result<sevenz_rust2::ArchiveReader<File>> {
    sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty()).map_err(sevenz_err)
}

/// 7z entry names may use either slash.
fn sevenz_path(name: &str) -> Option<PathBuf> {
    clean(Path::new(&name.replace('\\', "/")))
}

fn sevenz_index(archive: &Path) -> io::Result<Index> {
    let reader = open_sevenz(archive)?;
    let entries = reader
        .archive()
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.is_anti_item)
        .filter_map(|(i, f)| {
            Some(ArchiveEntry {
                path: sevenz_path(&f.name)?,
                is_dir: f.is_directory,
                size: f.size,
                modified: f
                    .has_last_modified_date
                    .then(|| SystemTime::from(f.last_modified_date)),
                index: i,
            })
        })
        .collect();
    Ok(Index { entries })
}

fn tar_reader(archive: &Path, kind: Kind) -> io::Result<tar::Archive<Box<dyn Read>>> {
    let file = BufReader::new(File::open(archive)?);
    let reader: Box<dyn Read> = match kind {
        Kind::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
        _ => Box::new(file),
    };
    Ok(tar::Archive::new(reader))
}

fn tar_index(archive: &Path, kind: Kind) -> io::Result<Index> {
    let mut tar = tar_reader(archive, kind)?;
    let mut entries = Vec::new();
    for (i, entry) in tar.entries()?.enumerate() {
        let entry = entry?;
        let header = entry.header();
        let ty = header.entry_type();
        if !(ty.is_file() || ty.is_dir()) {
            continue; // links, devices, PAX headers
        }
        let Some(path) = clean(&entry.path()?) else {
            continue;
        };
        entries.push(ArchiveEntry {
            path,
            is_dir: ty.is_dir(),
            size: header.size().unwrap_or(0),
            modified: header
                .mtime()
                .ok()
                .map(|s| UNIX_EPOCH + Duration::from_secs(s)),
            index: i,
        });
    }
    Ok(Index { entries })
}

/// The items directly inside folder `inner` of `archive`, as file-view entries
/// with virtual paths.
pub fn list(archive: &Path, inner: &Path) -> io::Result<Vec<FileEntry>> {
    let idx = index(archive)?;
    if !idx.is_dir(inner) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            crate::i18n::t("This folder doesn't exist in the archive."),
        ));
    }
    let base = archive.join(inner);
    let mut children: HashMap<String, FileEntry> = HashMap::new();
    for e in &idx.entries {
        let Ok(rest) = e.path.strip_prefix(inner) else {
            continue;
        };
        let mut comps = rest.components();
        let Some(first) = comps.next() else { continue };
        let name = first.as_os_str().to_string_lossy().into_owned();
        let deeper = comps.next().is_some();
        let item = children.entry(name.clone()).or_insert_with(|| FileEntry {
            path: base.join(&name),
            name,
            is_dir: true,
            is_symlink: false,
            size: 0,
            modified: None,
            created: None,
            hidden: false,
            readonly: true,
            cloud: None,
        });
        if !deeper {
            item.is_dir = e.is_dir;
            item.size = if e.is_dir { 0 } else { e.size };
            item.modified = e.modified;
        }
    }
    Ok(children.into_values().collect())
}

/// Read one file from the archive, at most `max` bytes.
pub fn read_entry(archive: &Path, inner: &Path, max: u64) -> io::Result<Vec<u8>> {
    let idx = index(archive)?;
    let entry = idx
        .entries
        .iter()
        .find(|e| e.path == inner && !e.is_dir)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                crate::i18n::t("Not found in the archive."),
            )
        })?;
    let mut buf = Vec::new();
    match kind(archive) {
        Some(Kind::Zip) => {
            let mut zip = open_zip(archive)?;
            let f = zip.by_index(entry.index).map_err(zip_err)?;
            f.take(max).read_to_end(&mut buf)?;
        }
        Some(Kind::SevenZip) => {
            let mut reader = open_sevenz(archive)?;
            let name = reader.archive().files[entry.index].name.clone();
            buf = reader.read_file(&name).map_err(sevenz_err)?;
            buf.truncate(max as usize);
        }
        Some(k) => {
            let mut tar = tar_reader(archive, k)?;
            for (i, e) in tar.entries()?.enumerate() {
                let e = e?;
                if i == entry.index {
                    e.take(max).read_to_end(&mut buf)?;
                    break;
                }
            }
        }
        None => return Err(io::Error::other(crate::i18n::t("Not a supported archive."))),
    }
    Ok(buf)
}

/// Where each selected item goes: the whole archive (empty inner path) goes
/// straight into `dest`, other items into `dest/<name>` (renamed if taken).
fn targets(items: &[PathBuf], dest: &Path) -> Vec<(PathBuf, PathBuf)> {
    items
        .iter()
        .map(|item| {
            let out = match item.file_name() {
                Some(name) => fsops::unique_path(dest, &name.to_string_lossy()),
                None => dest.to_path_buf(),
            };
            (item.clone(), out)
        })
        .collect()
}

fn out_path(targets: &[(PathBuf, PathBuf)], entry: &Path) -> Option<PathBuf> {
    targets.iter().find_map(|(item, out)| {
        let rest = entry.strip_prefix(item).ok()?;
        Some(if rest.as_os_str().is_empty() {
            out.clone()
        } else {
            out.join(rest)
        })
    })
}

fn write_file(out: &Path, data: &mut dyn Read, modified: Option<SystemTime>) -> io::Result<()> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = BufWriter::new(File::create(out)?);
    io::copy(data, &mut file)?;
    let file = file.into_inner().map_err(|e| e.into_error())?;
    if let Some(t) = modified {
        let _ = file.set_modified(t);
    }
    Ok(())
}

/// Extract `items` (paths inside the archive; an empty path means
/// everything) into `dest`. Returns the top-level paths that were created.
pub fn extract(
    archive: &Path,
    items: &[PathBuf],
    dest: &Path,
    cancel: &AtomicBool,
) -> io::Result<Vec<PathBuf>> {
    let idx = index(archive)?;
    fs::create_dir_all(dest)?;
    let targets = targets(items, dest);
    let wanted: HashMap<usize, (PathBuf, &ArchiveEntry)> = idx
        .entries
        .iter()
        .filter_map(|e| out_path(&targets, &e.path).map(|out| (e.index, (out, e))))
        .collect();
    for (out, e) in wanted.values() {
        if e.is_dir {
            fs::create_dir_all(out)?;
        }
    }
    let cancelled = || io::Error::new(io::ErrorKind::Interrupted, "Cancelled.");
    match kind(archive) {
        Some(Kind::Zip) => {
            let mut zip = open_zip(archive)?;
            for (&i, (out, e)) in &wanted {
                if cancel.load(Ordering::Relaxed) {
                    return Err(cancelled());
                }
                if !e.is_dir {
                    let mut f = zip.by_index(i).map_err(zip_err)?;
                    write_file(out, &mut f, e.modified)?;
                }
            }
        }
        Some(Kind::SevenZip) => {
            let mut reader = open_sevenz(archive)?;
            // 7z streams entries in order; pick the wanted ones by name.
            let by_name: HashMap<String, (&PathBuf, &ArchiveEntry)> = wanted
                .iter()
                .map(|(&i, (out, e))| (reader.archive().files[i].name.clone(), (out, *e)))
                .collect();
            let mut failed = None;
            reader
                .for_each_entries(|entry, data| {
                    if cancel.load(Ordering::Relaxed) {
                        failed = Some(cancelled());
                        return Ok(false);
                    }
                    if let Some((out, e)) = by_name.get(&entry.name) {
                        if !e.is_dir {
                            if let Err(err) = write_file(out, data, e.modified) {
                                failed = Some(err);
                                return Ok(false);
                            }
                        }
                    }
                    Ok(true)
                })
                .map_err(sevenz_err)?;
            if let Some(err) = failed {
                return Err(err);
            }
        }
        Some(k) => {
            let mut tar = tar_reader(archive, k)?;
            for (i, entry) in tar.entries()?.enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return Err(cancelled());
                }
                let mut entry = entry?;
                if let Some((out, e)) = wanted.get(&i) {
                    if !e.is_dir {
                        write_file(out, &mut entry, e.modified)?;
                    }
                }
            }
        }
        None => return Err(io::Error::other(crate::i18n::t("Not a supported archive."))),
    }
    Ok(targets.into_iter().map(|(_, out)| out).collect())
}

/// A free name for a zip of `sources` in `dir`: `name.zip` for one item
/// (without its extension), the first item's name for several.
pub fn zip_target(sources: &[PathBuf], dir: &Path) -> PathBuf {
    let first = sources
        .first()
        .map(|p| crate::entry::display_name(p))
        .unwrap_or_else(|| "Archive".into());
    let stem = match sources.first() {
        Some(p) if p.is_file() => fsops::split_name(&first).0.to_owned(),
        _ => first,
    };
    fsops::unique_path(dir, &format!("{stem}.zip"))
}

/// Compress `sources` (files and folders) into a new zip file at `dest`.
/// Returns the number of files added.
pub fn compress_zip(sources: &[PathBuf], dest: &Path, cancel: &AtomicBool) -> io::Result<u64> {
    let part = dest.with_extension("zip.part");
    let result = (|| {
        let mut zip = zip::ZipWriter::new(BufWriter::new(File::create(&part)?));
        let mut files = 0;
        for src in sources {
            let name = crate::entry::display_name(src);
            add_to_zip(&mut zip, src, &name, cancel, &mut files)?;
        }
        zip.finish().map_err(zip_err)?.flush()?;
        Ok(files)
    })();
    match result {
        Ok(files) => {
            fs::rename(&part, dest)?;
            Ok(files)
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

fn add_to_zip<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &Path,
    name: &str,
    cancel: &AtomicBool,
    files: &mut u64,
) -> io::Result<()> {
    use zip::write::SimpleFileOptions;
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled."));
    }
    let meta = fs::metadata(path)?;
    let mut options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if let Some(t) = meta.modified().ok().and_then(to_zip_time) {
        options = options.last_modified_time(t);
    }
    if meta.is_dir() {
        zip.add_directory(format!("{name}/"), options)
            .map_err(zip_err)?;
        let mut children: Vec<_> = fs::read_dir(path)?.flatten().collect();
        children.sort_by_key(|c| c.file_name());
        for child in children {
            // Links are not followed (they could point outside or loop).
            if child.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }
            let child_name = format!("{name}/{}", child.file_name().to_string_lossy());
            add_to_zip(zip, &child.path(), &child_name, cancel, files)?;
        }
    } else {
        let options = options.large_file(meta.len() >= u32::MAX as u64);
        zip.start_file(name, options).map_err(zip_err)?;
        io::copy(&mut BufReader::new(File::open(path)?), zip)?;
        *files += 1;
    }
    Ok(())
}

fn to_zip_time(t: SystemTime) -> Option<zip::DateTime> {
    let local: chrono::DateTime<chrono::Local> = t.into();
    zip::DateTime::try_from(local.naive_local()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(mut v: Vec<FileEntry>) -> Vec<(String, bool)> {
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v.into_iter().map(|e| (e.name, e.is_dir)).collect()
    }

    fn sample(dir: &Path) -> PathBuf {
        let src = dir.join("Docs");
        fs::create_dir_all(src.join("sub/deep")).unwrap();
        fs::write(src.join("a.txt"), "alpha").unwrap();
        fs::write(src.join("sub/b.txt"), "bravo").unwrap();
        fs::write(src.join("sub/deep/c.txt"), "charlie").unwrap();
        src
    }

    #[test]
    fn zip_round_trip_list_read_extract() {
        let tmp = tempfile::tempdir().unwrap();
        let src = sample(tmp.path());
        let dest = zip_target(std::slice::from_ref(&src), tmp.path());
        assert_eq!(dest, tmp.path().join("Docs.zip"));
        let files = compress_zip(&[src], &dest, &AtomicBool::new(false)).unwrap();
        assert_eq!(files, 3);
        assert!(is_archive(&dest));

        assert_eq!(
            names(list(&dest, Path::new("")).unwrap()),
            [("Docs".into(), true)]
        );
        let docs = list(&dest, Path::new("Docs")).unwrap();
        assert_eq!(
            names(docs.clone()),
            [("a.txt".into(), false), ("sub".into(), true)]
        );
        let a = docs.iter().find(|e| e.name == "a.txt").unwrap();
        assert_eq!(a.size, 5);
        assert_eq!(a.path, dest.join("Docs").join("a.txt"));
        assert!(a.modified.is_some());
        assert!(list(&dest, Path::new("nope")).is_err());

        // Virtual paths map back to the archive.
        let (arc, inner) = split_virtual(&a.path).unwrap();
        assert_eq!(
            (arc.as_path(), inner.as_path()),
            (dest.as_path(), Path::new("Docs/a.txt"))
        );
        assert_eq!(read_entry(&arc, &inner, 100).unwrap(), b"alpha");
        assert_eq!(read_entry(&arc, &inner, 2).unwrap(), b"al");
        assert!(split_virtual(tmp.path()).is_none());

        // Extract one folder, then everything.
        let out = tmp.path().join("out");
        let made = extract(&dest, &["Docs/sub".into()], &out, &AtomicBool::new(false)).unwrap();
        assert_eq!(made, [out.join("sub")]);
        assert_eq!(
            fs::read_to_string(out.join("sub/deep/c.txt")).unwrap(),
            "charlie"
        );
        assert!(!out.join("a.txt").exists());
        // Taken names get a new one instead of being overwritten.
        let made = extract(&dest, &["Docs/sub".into()], &out, &AtomicBool::new(false)).unwrap();
        assert_ne!(made[0], out.join("sub"));

        let all = tmp.path().join("all");
        extract(&dest, &[PathBuf::new()], &all, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            fs::read_to_string(all.join("Docs/sub/b.txt")).unwrap(),
            "bravo"
        );
    }

    #[test]
    fn tar_gz_list_and_extract() {
        let tmp = tempfile::tempdir().unwrap();
        let src = sample(tmp.path());
        let arc = tmp.path().join("docs.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(
                File::create(&arc).unwrap(),
                flate2::Compression::fast(),
            );
            let mut tar = tar::Builder::new(gz);
            tar.append_dir_all("Docs", &src).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        assert_eq!(kind(&arc), Some(Kind::TarGz));
        assert_eq!(base_name(&arc), "docs");
        assert_eq!(
            names(list(&arc, Path::new("Docs/sub")).unwrap()),
            [("b.txt".into(), false), ("deep".into(), true)]
        );
        assert_eq!(
            read_entry(&arc, Path::new("Docs/sub/deep/c.txt"), 99).unwrap(),
            b"charlie"
        );
        let out = tmp.path().join("out");
        extract(&arc, &["Docs/a.txt".into()], &out, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "alpha");
    }

    #[test]
    fn sevenz_list_read_extract() {
        let tmp = tempfile::tempdir().unwrap();
        let arc = tmp.path().join("photos.7z");
        {
            use sevenz_rust2::{ArchiveEntry as SzEntry, ArchiveWriter};
            let mut w = ArchiveWriter::create(&arc).unwrap();
            w.push_archive_entry::<&[u8]>(SzEntry::new_directory("2024"), None)
                .unwrap();
            w.push_archive_entry(SzEntry::new_file("2024/a.txt"), Some(&b"alpha"[..]))
                .unwrap();
            w.push_archive_entry(SzEntry::new_file("readme.md"), Some(&b"# hi"[..]))
                .unwrap();
            w.finish().unwrap();
        }
        assert_eq!(kind(&arc), Some(Kind::SevenZip));
        assert_eq!(base_name(&arc), "photos");
        assert_eq!(
            names(list(&arc, Path::new("")).unwrap()),
            [("2024".into(), true), ("readme.md".into(), false)]
        );
        assert_eq!(
            read_entry(&arc, Path::new("2024/a.txt"), 99).unwrap(),
            b"alpha"
        );
        let out = tmp.path().join("out");
        extract(&arc, &[PathBuf::new()], &out, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read_to_string(out.join("2024/a.txt")).unwrap(), "alpha");
        assert_eq!(fs::read_to_string(out.join("readme.md")).unwrap(), "# hi");
    }

    #[test]
    fn entries_escaping_the_target_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let arc = tmp.path().join("evil.zip");
        {
            let mut zip = zip::ZipWriter::new(File::create(&arc).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("../evil.txt", opts).unwrap();
            zip.write_all(b"x").unwrap();
            zip.start_file("ok.txt", opts).unwrap();
            zip.write_all(b"y").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(
            names(list(&arc, Path::new("")).unwrap()),
            [("ok.txt".into(), false)]
        );
        let out = tmp.path().join("out");
        extract(&arc, &[PathBuf::new()], &out, &AtomicBool::new(false)).unwrap();
        assert!(!tmp.path().join("evil.txt").exists());
        assert!(out.join("ok.txt").exists());
    }
}
