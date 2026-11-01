//! Windows only: reading an NTFS volume directly, for building the index
//! from the master file table and following the change journal (see
//! `journal`). That needs a handle to the volume (`\\.\C:`), which only
//! administrators get, or the index service (`helper`), which opens it for
//! the app; without either, the callers fall back to the directory walk and
//! a file watcher.
//!
//! Names and folders come from `FSCTL_ENUM_USN_DATA` (file reference →
//! parent reference, see `tree`). That listing has no sizes or dates, so the
//! `$MFT` itself is read from the volume in big chunks and each file record's
//! size and last write time are taken from it (see `ntfs`). A sample of the
//! result is compared with the files on disk; if it doesn't match, the drive
//! is walked instead.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, Prefix};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_HANDLE_EOF, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetFileInformationByHandle, GetLogicalDrives,
    GetVolumeInformationW, GetVolumePathNameW, ReadFile, SetFilePointerEx,
    BY_HANDLE_FILE_INFORMATION, FILE_BEGIN, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_ENUM_USN_DATA, FSCTL_GET_NTFS_VOLUME_DATA, MFT_ENUM_DATA_V0, NTFS_VOLUME_DATA_BUFFER,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::ntfs::{apply_fixup, data_runs, filetime_to_unix, parse_file_record};
use super::tree::{record_number, RawTree};
use super::usn::{next_value, parse_records, UsnRecord};
use super::volume::VolumeTree;
use super::{path_in, to_secs, Builder, F_DIR};

pub(super) const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const DRIVE_FIXED: u32 = 3;

pub(super) struct Handle(pub(super) HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by us and is closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

// SAFETY: the handle is only used by one thread at a time.
unsafe impl Send for Handle {}

pub(super) fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// A zeroed buffer starting at a 4096-byte boundary (volume reads want
/// sector-aligned memory, the ioctls 8-byte aligned records).
pub(super) struct Buf {
    raw: Vec<u8>,
    start: usize,
    len: usize,
}

impl Buf {
    pub(super) fn new(len: usize) -> Self {
        let raw = vec![0u8; len + 4096];
        let start = raw.as_ptr().align_offset(4096);
        Buf { raw, start, len }
    }

    pub(super) fn get(&mut self) -> &mut [u8] {
        &mut self.raw[self.start..self.start + self.len]
    }
}

/// `DeviceIoControl` with an optional input struct: the bytes written, or the error code.
pub(super) fn ioctl<I>(
    h: &Handle,
    code: u32,
    input: Option<&I>,
    out: &mut [u8],
) -> Result<usize, u32> {
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
pub(super) fn drive_letter(path: &Path) -> Option<char> {
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
pub(super) fn on_drive_volume(path: &Path, letter: char) -> bool {
    let mut buf = [0u16; 512];
    let ok = unsafe { GetVolumePathNameW(wide(path).as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    ok != 0 && String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case(&format!("{letter}:\\"))
}

pub(super) fn is_ntfs(letter: char) -> bool {
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
pub(super) fn open_volume(letter: char) -> Option<Handle> {
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
pub(super) fn file_id(path: &Path) -> Option<u64> {
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

/// The fixed NTFS drives (the only ones the index service reads).
pub(super) fn fixed_ntfs_drives() -> Vec<u8> {
    // SAFETY: no arguments; returns a bit mask of drive letters.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| b'A' + i)
        .filter(|&d| {
            let root = wide(format!("{}:\\", d as char));
            // SAFETY: `root` is a NUL-terminated wide string.
            let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
            kind == DRIVE_FIXED && is_ntfs(d as char)
        })
        .collect()
}

/// Every file and folder on the volume (names and parents only), handed to
/// `each` one buffer at a time. False if the listing failed, was cancelled
/// or `each` said stop.
pub(super) fn enumerate_with(
    vol: &Handle,
    cancel: &AtomicBool,
    mut each: impl FnMut(Vec<UsnRecord>) -> bool,
) -> bool {
    let mut input = MFT_ENUM_DATA_V0 {
        StartFileReferenceNumber: 0,
        LowUsn: 0,
        HighUsn: i64::MAX,
    };
    let mut buf = Buf::new(1 << 20);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let out = buf.get();
        match ioctl(vol, FSCTL_ENUM_USN_DATA, Some(&input), out) {
            Ok(n) if n > 8 => {
                if !each(parse_records(&out[8..n])) {
                    return false;
                }
                let Some(next) = next_value(out) else {
                    return false;
                };
                input.StartFileReferenceNumber = next;
            }
            Ok(_) | Err(ERROR_HANDLE_EOF) => return true,
            Err(e) => {
                log::info!("index: can't list the master file table (error {e})");
                return false;
            }
        }
    }
}

/// The volume's tree and each record's size and date (by record number),
/// read directly from the volume.
pub(super) fn read_tree_direct(
    vol: &Handle,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> Option<VolumeTree> {
    let mut tree = RawTree::default();
    let listed = enumerate_with(vol, cancel, |records| {
        for r in &records {
            tree.add(r.frn, r.parent, &r.name, r.attrs);
        }
        progress.fetch_add(records.len() as u64, Ordering::Relaxed);
        true
    });
    if !listed {
        return None;
    }
    let meta = read_metadata(vol, cancel);
    if meta.is_none() {
        log::info!("index: can't read the master file table");
    }
    Some((tree, meta?))
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
pub(super) fn read_metadata(vol: &Handle, cancel: &AtomicBool) -> Option<Vec<(u64, u32)>> {
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
        if e.size() == meta.len() && e.mtime.abs_diff(to_secs(meta.modified().ok())) <= 2 {
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
    let Some(letter) = drive_letter(root) else {
        return false;
    };
    let Some(root_frn) = file_id(root) else {
        return false;
    };
    let started = Instant::now();
    let Some(((tree, meta), route)) = super::volume::read_tree(letter, cancel, progress) else {
        return false;
    };
    progress.fetch_sub(tree.len() as u64, Ordering::Relaxed);
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
        "index: read {n} items of {} from the master file table ({route:?}) in {:.1?}",
        root.display(),
        started.elapsed()
    );
    true
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::index::{FileIndex, IndexConfig};

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
