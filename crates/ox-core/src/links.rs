//! Symbolic links, junctions and hard links: creating them, telling where
//! they point and removing them again (only the link, never its target).
//!
//! On Windows a symbolic link needs Developer Mode or admin rights; a
//! junction (folders only, on this computer's drives) and a hard link (files
//! only, on the same drive) don't.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::entry::display_name;
use crate::fsops;
use crate::i18n::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LinkKind {
    Symbolic,
    Junction,
    Hard,
}

impl LinkKind {
    pub fn label(self) -> &'static str {
        t(match self {
            Self::Symbolic => "Symbolic link",
            Self::Junction => "Junction",
            Self::Hard => "Hard link",
        })
    }
}

/// Where a symbolic link or junction points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    pub kind: LinkKind,
    /// The target as the link stores it (relative for some symbolic links).
    pub target: PathBuf,
}

impl LinkInfo {
    /// The target as a full path, readable (without `\\?\`).
    pub fn resolved(&self, link: &Path) -> PathBuf {
        resolve(link, &self.target)
    }
}

/// Is `path` a symbolic link or junction, and where does it point? Hard
/// links are ordinary files and aren't reported.
pub fn info(path: &Path) -> Option<LinkInfo> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_symlink() {
        return None;
    }
    let target = fs::read_link(path).ok()?;
    let kind = if imp::is_junction(path) {
        LinkKind::Junction
    } else {
        LinkKind::Symbolic
    };
    Some(LinkInfo { kind, target })
}

/// `target` of the link at `link` as a full path: relative targets are
/// taken from the link's folder, and `\\?\` prefixes are dropped.
pub fn resolve(link: &Path, target: &Path) -> PathBuf {
    let target = plain(target);
    if target.is_absolute() {
        return target;
    }
    let dir = link.parent().unwrap_or(Path::new(""));
    let joined = dir.join(target);
    // Tidy `..` and `.` without touching the disk.
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// A path without the `\\?\` (or `\\?\UNC\`) prefix Windows uses inside links.
pub fn plain(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\").or_else(|| s.strip_prefix(r"\??\")) {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// Create a link of `kind` at `link` that points to `target` (a full path).
/// The error says in words what went wrong.
pub fn create(kind: LinkKind, link: &Path, target: &Path) -> Result<(), String> {
    check(kind, link, target)?;
    create_raw(kind, link, target).map_err(|e| describe(kind, target, &e))
}

/// What has to hold before trying.
fn check(kind: LinkKind, link: &Path, target: &Path) -> Result<(), String> {
    if fsops::exists(link) {
        return Err(crate::trf!(
            "\"{new_name}\" already exists.",
            new_name = display_name(link)
        ));
    }
    let Ok(meta) = fs::metadata(target) else {
        return Err(crate::trf!(
            "\"{v1}\" doesn't exist.",
            v1 = target.display()
        ));
    };
    match kind {
        LinkKind::Junction if !meta.is_dir() => {
            Err(t("A junction can only point to a folder.").into())
        }
        LinkKind::Junction if is_network(target) => Err(t(
            "A junction can only point to a folder on this computer's drives, not on the network.",
        )
        .into()),
        LinkKind::Hard if meta.is_dir() => Err(t("A hard link can only point to a file.").into()),
        LinkKind::Hard if !fsops::same_volume(target, link) => {
            Err(t("A hard link can only point to a file on the same drive.").into())
        }
        _ => Ok(()),
    }
}

pub(crate) fn is_network(path: &Path) -> bool {
    let p = plain(path);
    p.to_string_lossy().starts_with(r"\\") || crate::network::server_name(&p).is_some()
}

fn create_raw(kind: LinkKind, link: &Path, target: &Path) -> io::Result<()> {
    match kind {
        LinkKind::Symbolic => imp::symlink(link, target, fs::metadata(target)?.is_dir()),
        LinkKind::Junction => imp::junction(link, target),
        LinkKind::Hard => fs::hard_link(target, link),
    }
}

/// Windows refused a symbolic link for lack of the privilege (Developer
/// Mode off, not run as administrator).
pub fn is_privilege_error(e: &io::Error) -> bool {
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
    e.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD)
}

const NO_SYMLINK_PRIVILEGE: &str =
    "Windows allows symbolic links only with Developer Mode on or for administrators.";

fn describe(kind: LinkKind, target: &Path, e: &io::Error) -> String {
    let reason = if is_privilege_error(e) {
        t(NO_SYMLINK_PRIVILEGE).to_owned()
    } else {
        e.to_string()
    };
    crate::trf!(
        "{kind} to \"{name}\" couldn't be created: {reason}",
        kind = kind.label(),
        name = display_name(target),
        reason = reason
    )
}

/// A link made by [`link_into`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Made {
    pub link: PathBuf,
    pub kind: LinkKind,
    /// A hard link, because Windows refused the symbolic link.
    pub fallback: bool,
}

/// Link to `target` from the folder `dir`, under the target's name (made
/// unique): folders get a junction (a symbolic link when they're on the
/// network), files a symbolic link — or a hard link on the same drive, if
/// Windows doesn't allow symbolic links.
pub fn link_into(target: &Path, dir: &Path) -> Result<Made, String> {
    let link = fsops::unique_path(dir, &display_name(target));
    let is_dir = fs::metadata(target).is_ok_and(|m| m.is_dir());
    let kind = match is_dir {
        true if cfg!(windows) && !is_network(target) => LinkKind::Junction,
        _ => LinkKind::Symbolic,
    };
    check(kind, &link, target)?;
    match create_raw(kind, &link, target) {
        Ok(()) => Ok(Made {
            link,
            kind,
            fallback: false,
        }),
        Err(e) if is_privilege_error(&e) && !is_dir && fsops::same_volume(target, dir) => {
            fs::hard_link(target, &link).map_err(|e| describe(LinkKind::Hard, target, &e))?;
            Ok(Made {
                link,
                kind: LinkKind::Hard,
                fallback: true,
            })
        }
        Err(e) if is_privilege_error(&e) && !is_dir => Err(crate::trf!(
            "Couldn't link to \"{name}\": {reason} A hard link isn't possible either, as it's on another drive.",
            name = display_name(target),
            reason = t(NO_SYMLINK_PRIVILEGE)
        )),
        Err(e) => Err(describe(kind, target, &e)),
    }
}

/// Is `link` still the link of `kind` that was made to `target`?
pub fn is_link(link: &Path, kind: LinkKind, target: &Path) -> bool {
    match kind {
        LinkKind::Hard => {
            // The same file under two names: same size and time, and more
            // than one name (so removing this one loses nothing).
            let (Ok(a), Ok(b)) = (fs::symlink_metadata(link), fs::metadata(target)) else {
                return false;
            };
            a.is_file()
                && a.len() == b.len()
                && a.modified().ok() == b.modified().ok()
                && imp::link_count(link).is_ok_and(|n| n > 1)
        }
        _ => info(link).is_some_and(|i| i.kind == kind && same(&i.resolved(link), target)),
    }
}

fn same(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = plain(p)
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_owned();
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s
        }
    };
    norm(a) == norm(b)
}

/// Remove the link at `link` (the link only: what it points to stays).
pub fn remove(link: &Path, kind: LinkKind) -> io::Result<()> {
    let meta = fs::symlink_metadata(link)?;
    match kind {
        LinkKind::Hard => fs::remove_file(link),
        // A link to a folder is removed like an (empty) folder; that
        // removes the link, not the folder's contents.
        _ if imp::is_folder_link(&meta) => fs::remove_dir(link),
        _ => fs::remove_file(link),
    }
}

/// Which file `path` is on its volume: the same for all hard links of a file.
pub fn file_id(path: &Path) -> io::Result<(u64, u64)> {
    imp::file_id(path)
}

#[cfg(windows)]
mod imp {
    use std::fs;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES,
        MAXIMUM_REPARSE_DATA_BUFFER_SIZE,
    };
    use windows_sys::Win32::System::Ioctl::{FSCTL_GET_REPARSE_POINT, FSCTL_SET_REPARSE_POINT};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
    const DIRECTORY: u32 = 0x10;
    const GENERIC_WRITE: u32 = 0x4000_0000;

    pub fn symlink(link: &Path, target: &Path, is_dir: bool) -> io::Result<()> {
        if is_dir {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }

    /// The link itself (a reparse point), not what it points to.
    fn open_link(path: &Path, access: u32) -> io::Result<fs::File> {
        fs::OpenOptions::new()
            .access_mode(access)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }

    pub fn is_junction(path: &Path) -> bool {
        let Ok(f) = open_link(path, FILE_READ_ATTRIBUTES) else {
            return false;
        };
        let mut buf = vec![0u8; MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize];
        let mut got = 0u32;
        // SAFETY: the handle is open for the call; `buf` is writable for its length.
        let ok = unsafe {
            DeviceIoControl(
                f.as_raw_handle(),
                FSCTL_GET_REPARSE_POINT,
                std::ptr::null(),
                0,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        ok != 0
            && got >= 4
            && u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) == IO_REPARSE_TAG_MOUNT_POINT
    }

    pub fn is_folder_link(meta: &fs::Metadata) -> bool {
        meta.file_attributes() & DIRECTORY != 0
    }

    /// The reparse data of a junction to `target` (a full path to a folder).
    pub(super) fn mount_point_buffer(target: &Path) -> io::Result<Vec<u8>> {
        let print = super::plain(&std::path::absolute(target)?);
        let mut print = print.to_string_lossy().into_owned();
        // `C:\` keeps its backslash, `C:\Users\` doesn't need one.
        if print.len() > 3 {
            while print.ends_with('\\') {
                print.pop();
            }
        }
        let subst: Vec<u16> = std::ffi::OsStr::new(&format!(r"\??\{print}"))
            .encode_wide()
            .collect();
        let print: Vec<u16> = std::ffi::OsStr::new(&print).encode_wide().collect();
        let (s_len, p_len) = (subst.len() * 2, print.len() * 2);
        // Four lengths/offsets, then both names, each with a NUL.
        let data_len = 8 + s_len + 2 + p_len + 2;
        if 8 + data_len > MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "path too long"));
        }
        let mut buf = Vec::with_capacity(8 + data_len);
        buf.extend(IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
        buf.extend((data_len as u16).to_le_bytes());
        buf.extend(0u16.to_le_bytes());
        for v in [0, s_len, s_len + 2, p_len] {
            buf.extend((v as u16).to_le_bytes());
        }
        for name in [&subst, &print] {
            for c in name.iter().chain(Some(&0)) {
                buf.extend(c.to_le_bytes());
            }
        }
        Ok(buf)
    }

    pub fn junction(link: &Path, target: &Path) -> io::Result<()> {
        let buf = mount_point_buffer(target)?;
        fs::create_dir(link)?;
        let set = || -> io::Result<()> {
            let f = open_link(link, GENERIC_WRITE | FILE_WRITE_ATTRIBUTES)?;
            let mut got = 0u32;
            // SAFETY: the handle is open for the call; `buf` is readable for its length.
            let ok = unsafe {
                DeviceIoControl(
                    f.as_raw_handle(),
                    FSCTL_SET_REPARSE_POINT,
                    buf.as_ptr().cast(),
                    buf.len() as u32,
                    std::ptr::null_mut(),
                    0,
                    &mut got,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        };
        set().inspect_err(|_| {
            let _ = fs::remove_dir(link);
        })
    }

    fn file_info(path: &Path) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
        let f = open_link(path, FILE_READ_ATTRIBUTES)?;
        // SAFETY: zeroed is a valid value for this plain C struct.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the handle is open and `info` is writable.
        if unsafe { GetFileInformationByHandle(f.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info)
    }

    pub fn link_count(path: &Path) -> io::Result<u32> {
        Ok(file_info(path)?.nNumberOfLinks)
    }

    pub fn file_id(path: &Path) -> io::Result<(u64, u64)> {
        let info = file_info(path)?;
        let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
        Ok((u64::from(info.dwVolumeSerialNumber), index))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::fs;
    use std::io;
    use std::path::Path;

    pub fn symlink(link: &Path, target: &Path, _is_dir: bool) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    pub fn is_junction(_path: &Path) -> bool {
        false
    }

    pub fn is_folder_link(_meta: &fs::Metadata) -> bool {
        false
    }

    pub fn junction(_link: &Path, _target: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "junctions exist on Windows only",
        ))
    }

    pub fn link_count(path: &Path) -> io::Result<u32> {
        use std::os::unix::fs::MetadataExt;
        Ok(fs::symlink_metadata(path)?.nlink() as u32)
    }

    pub fn file_id(path: &Path) -> io::Result<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::symlink_metadata(path)?;
        Ok((meta.dev(), meta.ino()))
    }
}

#[cfg(test)]
mod tests;
