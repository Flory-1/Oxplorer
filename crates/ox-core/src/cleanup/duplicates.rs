//! Duplicate files: files with the same size, then the same start and end,
//! then the same xxh3 hash of all of their contents.
//!
//! Comparing in stages means a big file is read in full only when another
//! file has the same size and the same first and last few KB. With
//! [`ScanOptions::byte_compare`], files with the same hash are compared
//! byte by byte too.
//!
//! The groups of files of one size are compared on a few threads (see
//! [`threads_for`]): one on a hard disk, where reading files side by side
//! mostly adds seeking, else up to four. A scan has one root, so all of its
//! files are on one drive (links and mounted folders aren't followed).

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use xxhash_rust::xxh3::Xxh3;

use super::{is_link, Progress, ScanOptions};
use crate::cloud::CloudStatus;
use crate::entry::FileEntry;

/// How much of the start and of the end is compared first.
const PART: u64 = 16 * 1024;
/// Read size for full hashes.
const CHUNK: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct DupFile {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
}

/// Files with the same contents.
#[derive(Debug, Clone, PartialEq)]
pub struct DupGroup {
    /// Size of each file.
    pub size: u64,
    /// At least two, sorted by path.
    pub files: Vec<DupFile>,
}

impl DupGroup {
    /// Space taken by all copies but one.
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64).saturating_sub(1)
    }
}

#[derive(Debug, Default)]
pub struct Report {
    /// Most wasted space first.
    pub groups: Vec<DupGroup>,
    /// Folders and files that couldn't be read.
    pub unreadable: u64,
    /// Cloud files that are online only (left out: reading them would
    /// download them).
    pub online_only: u64,
    pub cancelled: bool,
}

/// Which copy "Select all but …" keeps in each group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Newest,
    Oldest,
}

impl Report {
    /// Space freed by deleting all copies but one of each group.
    pub fn wasted(&self) -> u64 {
        self.groups.iter().map(DupGroup::wasted).sum()
    }

    /// Every file but the newest (or oldest) one of each group. Files
    /// without a date count as the oldest; ties keep the first by path.
    pub fn all_but(&self, keep: Keep) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for g in &self.groups {
            let kept = match keep {
                // `max_by_key` returns the last maximum, `min_by_key` the first.
                Keep::Newest => g
                    .files
                    .iter()
                    .enumerate()
                    .rev()
                    .max_by_key(|(_, f)| f.modified)
                    .map(|(i, _)| i),
                Keep::Oldest => g
                    .files
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, f)| f.modified)
                    .map(|(i, _)| i),
            };
            out.extend(
                g.files
                    .iter()
                    .enumerate()
                    .filter(|&(i, _)| Some(i) != kept)
                    .map(|(_, f)| f.path.clone()),
            );
        }
        out
    }

    /// Number of selected files and their total size.
    pub fn selected(&self, selection: &HashSet<PathBuf>) -> (usize, u64) {
        let mut n = 0;
        let mut bytes = 0;
        for g in &self.groups {
            let k = g
                .files
                .iter()
                .filter(|f| selection.contains(&f.path))
                .count();
            n += k;
            bytes += g.size * k as u64;
        }
        (n, bytes)
    }

    /// Groups of which every copy is selected (deleting them leaves none).
    pub fn fully_selected(&self, selection: &HashSet<PathBuf>) -> usize {
        self.groups
            .iter()
            .filter(|g| g.files.iter().all(|f| selection.contains(&f.path)))
            .count()
    }

    /// Take out files that are gone; groups left with one file go too.
    pub fn remove(&mut self, gone: &HashSet<PathBuf>) {
        for g in &mut self.groups {
            g.files.retain(|f| !gone.contains(&f.path));
        }
        self.groups.retain(|g| g.files.len() > 1);
    }
}

/// A file that may have a duplicate.
struct Candidate {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

/// Find the duplicate files under `root`. Stops soon after `cancel` is set.
pub fn find(root: &Path, opts: ScanOptions, cancel: &AtomicBool, progress: &Progress) -> Report {
    let mut report = Report::default();
    let files = list(root, opts, cancel, progress, &mut report);
    if cancel.load(Ordering::Relaxed) {
        report.cancelled = true;
        return report;
    }
    let mut by_size: HashMap<u64, Vec<Candidate>> = HashMap::new();
    for f in files {
        by_size.entry(f.size).or_default().push(f);
    }
    let mut sizes: Vec<Vec<Candidate>> = by_size
        .into_values()
        .filter(|v| v.len() > 1)
        .map(one_per_file)
        .filter(|v| v.len() > 1)
        .collect();
    Progress::add(
        &progress.candidates,
        sizes.iter().map(|v| v.len() as u64).sum(),
    );
    // Same folder order as listed, which reads files near each other together.
    sizes.sort_by(|a, b| a[0].path.cmp(&b[0].path));
    // Each thread takes the next group of one size.
    let queue: Vec<Mutex<Option<Vec<Candidate>>>> =
        sizes.into_iter().map(|s| Mutex::new(Some(s))).collect();
    let next = AtomicUsize::new(0);
    let threads = threads_for(root, opts.threads).min(queue.len()).max(1);
    let work = || {
        let mut found = Found::default();
        let mut buf = vec![0u8; CHUNK];
        while !cancel.load(Ordering::Relaxed) {
            let Some(slot) = queue.get(next.fetch_add(1, Ordering::Relaxed)) else {
                break;
            };
            if let Some(same_size) = slot.lock().unwrap().take() {
                compare_same_size(same_size, opts, cancel, progress, &mut buf, &mut found);
            }
        }
        found
    };
    let parts: Vec<Found> = if threads == 1 {
        vec![work()]
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads).map(|_| s.spawn(work)).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
    };
    for part in parts {
        report.groups.extend(part.groups);
        report.unreadable += part.unreadable;
    }
    if cancel.load(Ordering::Relaxed) {
        report.groups.clear();
        report.cancelled = true;
        return report;
    }
    report.groups.sort_by(|a, b| {
        b.wasted()
            .cmp(&a.wasted())
            .then_with(|| a.files[0].path.cmp(&b.files[0].path))
    });
    report
}

/// Hard links of one file are the same data, not copies (deleting one frees
/// nothing), so only the first of them is kept.
fn one_per_file(files: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    files
        .into_iter()
        .filter(|c| match crate::links::file_id(&c.path) {
            Ok(id) => seen.insert(id),
            Err(_) => true,
        })
        .collect()
}

/// What one thread found.
#[derive(Default)]
struct Found {
    groups: Vec<DupGroup>,
    unreadable: u64,
}

/// How many files are compared at the same time under `root`: `wanted`,
/// or for 0 one on a hard disk and up to four elsewhere.
pub fn threads_for(root: &Path, wanted: usize) -> usize {
    if wanted > 0 {
        return wanted;
    }
    if crate::disk::is_rotational(root) == Some(true) {
        return 1;
    }
    std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .min(4)
}

/// Find the duplicates among files of one size.
fn compare_same_size(
    same_size: Vec<Candidate>,
    opts: ScanOptions,
    cancel: &AtomicBool,
    progress: &Progress,
    buf: &mut [u8],
    found: &mut Found,
) {
    let size = same_size[0].size;
    // Small files are read in full by the first stage already.
    let whole = size <= 2 * PART;
    let unreadable = &mut found.unreadable;
    for same_part in split_by(same_size, cancel, unreadable, |p| {
        partial_hash(p, size, buf, progress)
    }) {
        let mut groups = if whole {
            vec![same_part]
        } else {
            split_by(same_part, cancel, unreadable, |p| {
                full_hash(p, buf, cancel, progress)
            })
        };
        if opts.byte_compare {
            groups = groups
                .into_iter()
                .flat_map(|g| split_by_bytes(g, cancel, progress, unreadable))
                .collect();
        }
        for mut files in groups {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            found.groups.push(DupGroup {
                size,
                files: files
                    .into_iter()
                    .map(|c| DupFile {
                        path: c.path,
                        modified: c.modified,
                    })
                    .collect(),
            });
        }
    }
}

/// Group files by `key`; groups of one are dropped, and so are files that
/// can't be read (counted in `unreadable`).
fn split_by(
    files: Vec<Candidate>,
    cancel: &AtomicBool,
    unreadable: &mut u64,
    mut key: impl FnMut(&Path) -> io::Result<u128>,
) -> Vec<Vec<Candidate>> {
    let mut groups: HashMap<u128, Vec<Candidate>> = HashMap::new();
    for f in files {
        if cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
        match key(&f.path) {
            Ok(k) => groups.entry(k).or_default().push(f),
            Err(_) => *unreadable += 1,
        }
    }
    groups.into_values().filter(|g| g.len() > 1).collect()
}

/// Split files with the same hash into those with really the same bytes:
/// each file is compared with the first file of each set found so far.
fn split_by_bytes(
    files: Vec<Candidate>,
    cancel: &AtomicBool,
    progress: &Progress,
    unreadable: &mut u64,
) -> Vec<Vec<Candidate>> {
    let mut sets: Vec<Vec<Candidate>> = Vec::new();
    'files: for f in files {
        for set in &mut sets {
            if cancel.load(Ordering::Relaxed) {
                return Vec::new();
            }
            match same_bytes(&set[0].path, &f.path, cancel, progress) {
                Ok(true) => {
                    set.push(f);
                    continue 'files;
                }
                Ok(false) => {}
                Err(_) => {
                    *unreadable += 1;
                    continue 'files;
                }
            }
        }
        sets.push(vec![f]);
    }
    sets.retain(|s| s.len() > 1);
    sets
}

/// Have the two files the same bytes? Reads both side by side in chunks
/// and stops at the first difference.
pub fn same_bytes(
    a: &Path,
    b: &Path,
    cancel: &AtomicBool,
    progress: &Progress,
) -> io::Result<bool> {
    let (mut fa, mut fb) = (File::open(a)?, File::open(b)?);
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let mut ba = vec![0u8; CHUNK];
    let mut bb = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let na = read_up_to(&mut fa, &mut ba)?;
        let nb = read_up_to(&mut fb, &mut bb)?;
        Progress::add(&progress.hashed, (na + nb) as u64);
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
    }
}

/// Hash of the first and the last `PART` bytes (all of a small file).
fn partial_hash(path: &Path, size: u64, buf: &mut [u8], progress: &Progress) -> io::Result<u128> {
    let mut f = File::open(path)?;
    let buf = &mut buf[..PART as usize];
    let mut hasher = Xxh3::new();
    let head = read_up_to(&mut f, buf)?;
    hasher.update(&buf[..head]);
    let mut read = head;
    if size > PART {
        // The rest of a file up to twice `PART` long, else its last `PART`.
        f.seek(SeekFrom::Start(size.saturating_sub(PART).max(PART)))?;
        let tail = read_up_to(&mut f, buf)?;
        hasher.update(&buf[..tail]);
        read += tail;
    }
    Progress::add(&progress.hashed, read as u64);
    Ok(hasher.digest128())
}

/// Hash of all of the file's contents.
fn full_hash(
    path: &Path,
    buf: &mut [u8],
    cancel: &AtomicBool,
    progress: &Progress,
) -> io::Result<u128> {
    let mut f = File::open(path)?;
    let mut hasher = Xxh3::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let n = match f.read(buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buf[..n]);
        Progress::add(&progress.hashed, n as u64);
    }
    Ok(hasher.digest128())
}

/// Read until `buf` is full or the file ends.
fn read_up_to(f: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match f.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(got)
}

/// Every file under `root` that could have a duplicate (breadth-first).
fn list(
    root: &Path,
    opts: ScanOptions,
    cancel: &AtomicBool,
    progress: &Progress,
    report: &mut Report,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        Progress::add(&progress.folders, 1);
        let Ok(items) = fs::read_dir(&dir) else {
            report.unreadable += 1;
            continue;
        };
        for item in items.flatten() {
            // Doesn't follow links: a link's own metadata.
            let Ok(meta) = item.metadata() else { continue };
            if is_link(&meta) {
                continue;
            }
            let e = FileEntry::from_metadata(item.path(), &meta);
            if e.hidden && !opts.show_hidden {
                continue;
            }
            if meta.is_dir() {
                queue.push_back(e.path);
                continue;
            }
            Progress::add(&progress.files, 1);
            if e.size == 0 && !opts.include_empty_files {
                continue;
            }
            if e.cloud == Some(CloudStatus::OnlineOnly) {
                report.online_only += 1;
                continue;
            }
            out.push(Candidate {
                path: e.path,
                size: e.size,
                modified: e.modified,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_with_the_same_hash_but_other_bytes_are_split() {
        // xxh3 collisions can't be made: hand the byte compare a "group of
        // the same hash" whose files differ.
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str, body: &[u8]| {
            let path = dir.path().join(name);
            fs::write(&path, body).unwrap();
            Candidate {
                path,
                size: body.len() as u64,
                modified: None,
            }
        };
        let group = vec![
            file("a", b"same bytes"),
            file("x", b"other byte"),
            file("b", b"same bytes"),
            file("y", b"other byte"),
            file("z", b"third one!"),
        ];
        let mut unreadable = 0;
        let sets = split_by_bytes(
            group,
            &AtomicBool::new(false),
            &Progress::default(),
            &mut unreadable,
        );
        let names: Vec<Vec<String>> = sets
            .iter()
            .map(|s| {
                s.iter()
                    .map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned())
                    .collect()
            })
            .collect();
        assert_eq!(names, [vec!["a", "b"], vec!["x", "y"]]);
        assert_eq!(unreadable, 0);
    }
}
