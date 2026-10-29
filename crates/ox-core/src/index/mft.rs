//! Windows only: building the index from the NTFS master file table, and
//! following the NTFS change journal for live updates. Both need a handle to
//! the volume (`\\.\C:`), which only administrators get; without one, the
//! callers fall back to the directory walk and a file watcher.
//!
//! Names and folders come from `FSCTL_ENUM_USN_DATA` (file reference →
//! parent reference, see `tree`). That listing has no sizes or dates, so the
//! `$MFT` itself is read from the volume in big chunks and each file record's
//! size and last write time are taken from it (see `ntfs`). A sample of the
//! result is compared with the files on disk; if it doesn't match, the drive
//! is walked instead.

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_HANDLE_EOF, ERROR_JOURNAL_DELETE_IN_PROGRESS,
    ERROR_JOURNAL_ENTRY_DELETED, ERROR_JOURNAL_NOT_ACTIVE, GENERIC_READ, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileIdType, GetFileInformationByHandle, GetFinalPathNameByHandleW,
    GetVolumeInformationW, GetVolumePathNameW, OpenFileById, ReadFile, SetFilePointerEx,
    BY_HANDLE_FILE_INFORMATION, FILE_BEGIN, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_DESCRIPTOR,
    FILE_ID_DESCRIPTOR_0, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_ENUM_USN_DATA, FSCTL_GET_NTFS_VOLUME_DATA, FSCTL_QUERY_USN_JOURNAL,
    FSCTL_READ_USN_JOURNAL, MFT_ENUM_DATA_V0, NTFS_VOLUME_DATA_BUFFER, READ_USN_JOURNAL_DATA_V0,
    USN_JOURNAL_DATA_V0,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::ntfs::{apply_fixup, data_runs, filetime_to_unix, parse_file_record};
use super::patch::Signal;
use super::tree::{record_number, RawTree, ATTR_DIRECTORY};
use super::usn::{next_value, parse_records, REASON_RENAME_NEW_NAME, REASON_RENAME_OLD_NAME};
use super::{path_in, to_secs, Builder, F_DIR};

const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const REASON_FILE_DELETE: u32 = 0x0000_0200;

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

// The handle is only used by one thread at a time.
unsafe impl Send for Handle {}

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// A zeroed buffer starting at a 4096-byte boundary (volume reads want
/// sector-aligned memory, the ioctls 8-byte aligned records).
struct Buf {
    raw: Vec<u8>,
    start: usize,
    len: usize,
}

impl Buf {
    fn new(len: usize) -> Self {
        let raw = vec![0u8; len + 4096];
        let start = raw.as_ptr().align_offset(4096);
        Buf { raw, start, len }
    }

    fn get(&mut self) -> &mut [u8] {
        &mut self.raw[self.start..self.start + self.len]
    }
}

/// `DeviceIoControl` with an optional input struct: the bytes written, or the error code.
fn ioctl<I>(h: &Handle, code: u32, input: Option<&I>, out: &mut [u8]) -> Result<usize, u32> {
    let (ip, il) = match input {
        Some(i) => (i as *const I as *const c_void, size_of::<I>() as u32),
        None => (std::ptr::null(), 0),
    };
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h.0,
            code,
            ip,
            il,
            out.as_mut_ptr().cast(),
            out.len() as u32,
            &mut ret,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(ret as usize)
    }
}

/// The drive letter of a local path like `C:\…`.
fn drive_letter(path: &Path) -> Option<char> {
    match path.components().next()? {
        Component::Prefix(p) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Some((d as char).to_ascii_uppercase()),
            _ => None,
        },
        _ => None,
    }
}

/// `C:\` itself (not a folder on it).
fn is_drive_root(path: &Path) -> bool {
    drive_letter(path).is_some() && path.components().count() == 2 && path.has_root()
}

/// Is `path` on its drive letter's volume (and not on a volume mounted in a folder)?
fn on_drive_volume(path: &Path, letter: char) -> bool {
    let mut buf = [0u16; 512];
    let ok = unsafe { GetVolumePathNameW(wide(path).as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    ok != 0 && String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case(&format!("{letter}:\\"))
}

fn is_ntfs(letter: char) -> bool {
    let mut fs = [0u16; 32];
    let ok = unsafe {
        GetVolumeInformationW(
            wide(format!("{letter}:\\")).as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    let len = fs.iter().position(|&c| c == 0).unwrap_or(0);
    ok != 0 && String::from_utf16_lossy(&fs[..len]) == "NTFS"
}

/// The NTFS volume behind a drive letter, if Oxplorer may read it directly.
fn open_volume(letter: char) -> Option<Handle> {
    if !is_ntfs(letter) {
        return None;
    }
    let h = unsafe {
        CreateFileW(
            wide(format!("\\\\.\\{letter}:")).as_ptr(),
            GENERIC_READ,
            SHARE_ALL,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE).then_some(Handle(h))
}

/// The file reference number of a file or folder.
fn file_id(path: &Path) -> Option<u64> {
    let h = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            0,
            SHARE_ALL,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let h = Handle(h);
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(h.0, &mut info) };
    (ok != 0).then_some((info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64)
}

/// Every file and folder on the volume (names and parents only).
fn enumerate(vol: &Handle, cancel: &AtomicBool, progress: &AtomicU64) -> Option<RawTree> {
    let mut input = MFT_ENUM_DATA_V0 {
        StartFileReferenceNumber: 0,
        LowUsn: 0,
        HighUsn: i64::MAX,
    };
    let mut buf = Buf::new(1 << 20);
    let mut tree = RawTree::default();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let out = buf.get();
        match ioctl(vol, FSCTL_ENUM_USN_DATA, Some(&input), out) {
            Ok(n) if n > 8 => {
                let before = tree.len();
                for r in parse_records(&out[8..n]) {
                    tree.add(r.frn, r.parent, &r.name, r.attrs);
                }
                progress.fetch_add((tree.len() - before) as u64, Ordering::Relaxed);
                input.StartFileReferenceNumber = next_value(out)?;
            }
            Ok(_) | Err(ERROR_HANDLE_EOF) => break,
            Err(e) => {
                log::info!("index: can't list the master file table (error {e})");
                return None;
            }
        }
    }
    Some(tree)
}

fn read_at(vol: &Handle, offset: u64, buf: &mut [u8]) -> Option<()> {
    let ok = unsafe { SetFilePointerEx(vol.0, offset as i64, std::ptr::null_mut(), FILE_BEGIN) };
    if ok == 0 {
        return None;
    }
    let mut done = 0;
    while done < buf.len() {
        let mut got = 0u32;
        let ok = unsafe {
            ReadFile(
                vol.0,
                buf[done..].as_mut_ptr(),
                (buf.len() - done) as u32,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || got == 0 {
            return None;
        }
        done += got as usize;
    }
    Some(())
}

/// Size and last write time (seconds since 1970) of every file record, by
/// record number. `None` if the `$MFT` can't be read completely.
fn read_metadata(vol: &Handle, cancel: &AtomicBool) -> Option<Vec<(u64, u32)>> {
    let mut vd = Buf::new(512);
    let n = ioctl::<()>(vol, FSCTL_GET_NTFS_VOLUME_DATA, None, vd.get()).ok()?;
    if n < size_of::<NTFS_VOLUME_DATA_BUFFER>() {
        return None;
    }
    let v: NTFS_VOLUME_DATA_BUFFER = unsafe { std::ptr::read_unaligned(vd.get().as_ptr().cast()) };
    let rec = v.BytesPerFileRecordSegment as usize;
    let cluster = v.BytesPerCluster as usize;
    if !rec.is_power_of_two()
        || !(512..=65536).contains(&rec)
        || !cluster.is_power_of_two()
        || cluster < rec
        || v.MftStartLcn < 0
        || v.MftValidDataLength <= 0
    {
        return None;
    }
    let total = (v.MftValidDataLength as u64 / rec as u64) as usize;

    // Record 0 is the $MFT itself and says where its pieces are.
    let mut first = Buf::new(cluster);
    read_at(vol, v.MftStartLcn as u64 * cluster as u64, first.get())?;
    let r0 = &mut first.get()[..rec];
    if !apply_fixup(r0) {
        return None;
    }
    let runs = data_runs(r0)?;

    let mut meta = vec![(0u64, 0u32); total];
    let chunk = (4 << 20usize).max(cluster);
    let mut buf = Buf::new(chunk);
    let mut recno = 0;
    'runs: for (lcn, count) in runs {
        let mut offset = lcn * cluster as u64;
        let mut left = count * cluster as u64;
        while left > 0 {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if recno >= total {
                break 'runs;
            }
            let n = left.min(chunk as u64) as usize;
            let data = &mut buf.get()[..n];
            read_at(vol, offset, data)?;
            for r in data.chunks_exact_mut(rec) {
                if recno >= total {
                    break;
                }
                if apply_fixup(r) {
                    if let Some(f) = parse_file_record(r) {
                        let target = f.base.map_or(recno, |b| b as usize);
                        if let Some(m) = meta.get_mut(target) {
                            if let Some(t) = f.modified {
                                m.1 = filetime_to_unix(t);
                            }
                            if let Some(s) = f.size {
                                m.0 = s;
                            }
                        }
                    }
                }
                recno += 1;
            }
            offset += n as u64;
            left -= n as u64;
        }
    }
    (recno >= total).then_some(meta)
}

/// Compare a sample of the files pushed from entry `from` on with the disk.
fn verify(b: &Builder, from: usize) -> bool {
    let files: Vec<u32> = (from..b.entries.len())
        .filter(|&i| b.entries[i].flags & F_DIR == 0)
        .map(|i| i as u32)
        .collect();
    let step = (files.len() / 64).max(1);
    let (mut checked, mut matched) = (0, 0);
    for &i in files.iter().step_by(step).take(64) {
        // Files that vanished or can't be opened say nothing.
        let Ok(meta) = std::fs::symlink_metadata(path_in(&b.names, &b.entries, i)) else {
            continue;
        };
        checked += 1;
        let e = &b.entries[i as usize];
        if e.size == meta.len() && e.mtime.abs_diff(to_secs(meta.modified().ok())) <= 2 {
            matched += 1;
        }
    }
    checked == 0 || matched * 10 >= checked * 8
}

/// Index the drive `root` (e.g. `C:\`) from its master file table into `b`
/// below entry `idx`. Returns false (with nothing pushed) if that isn't
/// possible, so the caller walks the drive instead.
pub(super) fn index_volume(
    root: &Path,
    idx: u32,
    exclude: &[String],
    b: &mut Builder,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> bool {
    if !is_drive_root(root) {
        return false;
    }
    let Some(vol) = drive_letter(root).and_then(open_volume) else {
        return false;
    };
    let Some(root_frn) = file_id(root) else {
        return false;
    };
    let started = Instant::now();
    let Some(tree) = enumerate(&vol, cancel, progress) else {
        return false;
    };
    progress.fetch_sub(tree.len() as u64, Ordering::Relaxed);
    let Some(meta) = read_metadata(&vol, cancel) else {
        log::info!(
            "index: can't read the master file table of {}",
            root.display()
        );
        return false;
    };
    let mark = (b.entries.len(), b.names.len());
    let lookup = |frn: u64| {
        meta.get(record_number(frn) as usize)
            .copied()
            .unwrap_or_default()
    };
    let n = tree.push_into(root_frn, idx, exclude, b, lookup, cancel, progress);
    if !cancel.load(Ordering::Relaxed) && !verify(b, mark.0) {
        log::warn!(
            "index: the master file table of {} doesn't match the files; walking it instead",
            root.display()
        );
        progress.fetch_sub(n as u64, Ordering::Relaxed);
        b.entries.truncate(mark.0);
        b.names.truncate(mark.1);
        return false;
    }
    log::info!(
        "index: read {n} items of {} from the master file table in {:.1?}",
        root.display(),
        started.elapsed()
    );
    true
}

// ---- change journal --------------------------------------------------------

/// The journal's id and the next USN to read.
fn query_journal(vol: &Handle) -> Option<(u64, i64)> {
    let mut buf = Buf::new(256);
    let n = ioctl::<()>(vol, FSCTL_QUERY_USN_JOURNAL, None, buf.get()).ok()?;
    if n < size_of::<USN_JOURNAL_DATA_V0>() {
        return None;
    }
    let j: USN_JOURNAL_DATA_V0 = unsafe { std::ptr::read_unaligned(buf.get().as_ptr().cast()) };
    Some((j.UsnJournalID, j.NextUsn))
}

/// The current path of a file reference.
fn path_of_id(vol: &Handle, frn: u64) -> Option<PathBuf> {
    let desc = FILE_ID_DESCRIPTOR {
        dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32,
        Type: FileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: frn as i64 },
    };
    let h = unsafe {
        OpenFileById(
            vol.0,
            &desc,
            0,
            SHARE_ALL,
            std::ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let h = Handle(h);
    let mut buf = vec![0u16; 1024];
    loop {
        let n = unsafe { GetFinalPathNameByHandleW(h.0, buf.as_mut_ptr(), buf.len() as u32, 0) }
            as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            let s = String::from_utf16_lossy(&buf[..n]);
            let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
            return (!s.starts_with(r"UNC\")).then(|| PathBuf::from(s));
        }
        buf.resize(n + 1, 0);
    }
}

/// Follow the change journals of the drives holding `roots`, sending what
/// changes to `tx` until `stop` is set. Returns how many drives are followed
/// and the roots that need a file watcher instead.
pub(super) fn follow_journals(
    roots: &[PathBuf],
    tx: &Sender<Signal>,
    stop: &Arc<AtomicBool>,
) -> (usize, Vec<PathBuf>) {
    let mut by_drive: Vec<(char, Vec<PathBuf>)> = Vec::new();
    let mut rest = Vec::new();
    for root in roots {
        match drive_letter(root).filter(|&l| on_drive_volume(root, l)) {
            Some(l) => match by_drive.iter_mut().find(|(d, _)| *d == l) {
                Some((_, v)) => v.push(root.clone()),
                None => by_drive.push((l, vec![root.clone()])),
            },
            None => rest.push(root.clone()),
        }
    }
    let mut followed = 0;
    for (letter, roots) in by_drive {
        let journal = open_volume(letter).and_then(|v| query_journal(&v).map(|j| (v, j)));
        let Some((vol, (id, next))) = journal else {
            rest.extend(roots);
            continue;
        };
        let (tx, stop) = (tx.clone(), stop.clone());
        let spawned = std::thread::Builder::new()
            .name("index-journal".into())
            .spawn(move || read_journal(vol, id, next, &roots, &tx, &stop));
        if spawned.is_ok() {
            followed += 1;
        }
    }
    (followed, rest)
}

fn read_journal(
    vol: Handle,
    mut id: u64,
    mut start: i64,
    roots: &[PathBuf],
    tx: &Sender<Signal>,
    stop: &AtomicBool,
) {
    // The journal reports long paths; roots may be written differently (short
    // 8.3 names, other case), so changes are mapped back onto the root as given.
    let prefixes: Vec<(String, &PathBuf)> = roots
        .iter()
        .map(|r| {
            let long = std::fs::canonicalize(r).unwrap_or_else(|_| r.clone());
            let long = long.to_string_lossy();
            let mut s = long.strip_prefix(r"\\?\").unwrap_or(&long).to_owned();
            if !s.ends_with('\\') {
                s.push('\\');
            }
            (s, r)
        })
        .collect();
    let mut buf = Buf::new(64 * 1024);
    // Folder paths by file reference, so busy folders are looked up once.
    let mut dirs: HashMap<u64, Option<PathBuf>> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        let input = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: start,
            ReasonMask: u32::MAX,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: id,
        };
        let out = buf.get();
        let n = match ioctl(&vol, FSCTL_READ_USN_JOURNAL, Some(&input), out) {
            Ok(n) if n >= 8 => n,
            Ok(_) => 0,
            Err(
                ERROR_JOURNAL_ENTRY_DELETED
                | ERROR_JOURNAL_DELETE_IN_PROGRESS
                | ERROR_JOURNAL_NOT_ACTIVE,
            ) => {
                // Changes were lost (the journal wrapped or was recreated).
                if tx.send(Signal::Rescan).is_err() {
                    return;
                }
                match query_journal(&vol) {
                    Some((i, next)) => (id, start) = (i, next),
                    None => return,
                }
                dirs.clear();
                0
            }
            Err(_) => 0,
        };
        let records = if n >= 8 {
            parse_records(&out[8..n])
        } else {
            Vec::new()
        };
        if n >= 8 {
            start = next_value(out).map_or(start, |v| v as i64);
        }
        if dirs.len() > 10_000 {
            dirs.clear();
        }
        for r in &records {
            let dir = match dirs.get(&r.parent) {
                Some(d) => d.clone(),
                None => {
                    let d = path_of_id(&vol, r.parent);
                    dirs.insert(r.parent, d.clone());
                    d
                }
            };
            if r.attrs & ATTR_DIRECTORY != 0
                && r.reason & (REASON_RENAME_OLD_NAME | REASON_RENAME_NEW_NAME | REASON_FILE_DELETE)
                    != 0
            {
                dirs.clear(); // paths below it changed
            }
            // Gone before we looked: its folder's own record covers it.
            let Some(dir) = dir else { continue };
            let Some(path) = map_to_root(&dir.join(&r.name), &prefixes) else {
                continue;
            };
            let signal = if r.reason & REASON_RENAME_OLD_NAME != 0 {
                Signal::RenamedFrom(path)
            } else if r.reason & REASON_RENAME_NEW_NAME != 0 {
                Signal::RenamedTo(path)
            } else {
                Signal::Changed(path)
            };
            if tx.send(signal).is_err() {
                return;
            }
        }
        // A full buffer means more is waiting.
        if n < out.len() / 2 {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

/// `path` below one of the roots, written the way that root is.
fn map_to_root(path: &Path, prefixes: &[(String, &PathBuf)]) -> Option<PathBuf> {
    let s = path.to_string_lossy();
    prefixes.iter().find_map(|(long, root)| {
        let n = long.len();
        (s.len() > n && s.is_char_boundary(n) && s[..n].eq_ignore_ascii_case(long))
            .then(|| root.join(&s[n..]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{FileIndex, IndexConfig};

    #[test]
    fn maps_journal_paths_onto_roots() {
        let short = PathBuf::from(r"C:\Users\RUNNER~1\Temp");
        let drive = PathBuf::from(r"D:\");
        let prefixes = [
            (r"C:\Users\runneradmin\Temp\".to_owned(), &short),
            (r"D:\".to_owned(), &drive),
        ];
        assert_eq!(
            map_to_root(Path::new(r"c:\users\RunnerAdmin\Temp\a\b.txt"), &prefixes),
            Some(PathBuf::from(r"C:\Users\RUNNER~1\Temp\a\b.txt"))
        );
        assert_eq!(
            map_to_root(Path::new(r"D:\x.txt"), &prefixes),
            Some(PathBuf::from(r"D:\x.txt"))
        );
        assert_eq!(map_to_root(Path::new(r"C:\Windows\x"), &prefixes), None);
        assert_eq!(
            map_to_root(Path::new(r"C:\Users\runneradmin\Temp"), &prefixes),
            None
        );
    }

    #[test]
    fn recognises_drive_roots() {
        assert!(is_drive_root(Path::new(r"C:\")));
        assert!(is_drive_root(Path::new(r"d:\")));
        assert!(!is_drive_root(Path::new(r"C:\Users")));
        assert!(!is_drive_root(Path::new(r"C:")));
        assert!(!is_drive_root(Path::new(r"\\server\share\")));
        assert_eq!(drive_letter(Path::new(r"e:\x")), Some('E'));
    }

    /// Run as administrator: `cargo test -p ox-core -- --ignored mft`.
    /// Reads C:\ from the master file table and checks a sample against the disk.
    #[test]
    #[ignore]
    fn mft_matches_the_disk() {
        let cfg = IndexConfig {
            roots: vec![PathBuf::from(r"C:\")],
            exclude: vec![],
            threads: 0,
        };
        let started = Instant::now();
        let idx = FileIndex::build_fast(&cfg, &AtomicBool::new(false), &AtomicU64::new(0));
        assert_eq!(idx.mft_roots(), 1, "needs administrator rights");
        println!("{} items in {:.1?}", idx.len(), started.elapsed());
        let (mut checked, mut matched) = (0, 0);
        for i in (1..idx.entries.len() as u32).step_by(997) {
            let e = idx.file_entry(i);
            let Ok(meta) = std::fs::symlink_metadata(&e.path) else {
                continue;
            };
            checked += 1;
            if e.is_dir || e.size == meta.len() {
                matched += 1;
            } else {
                println!("{}: {} vs {}", e.path.display(), e.size, meta.len());
            }
        }
        println!("{matched} of {checked} match");
        assert!(matched * 100 >= checked * 95);
    }
}
