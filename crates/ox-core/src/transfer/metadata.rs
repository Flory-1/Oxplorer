//! What a copy keeps besides the contents: permissions (ACLs), alternate
//! data streams and folder dates.
//!
//! On Windows `CopyFileExW` already copies a file's alternate data streams
//! and its modified time, but not its permissions. So:
//!
//! - **Streams** off: extra streams are removed from the copy, except the
//!   "downloaded from the internet" mark (`Zone.Identifier`), which stays so
//!   Windows keeps warning about downloaded programs. On: files keep all
//!   streams and folders get theirs copied too.
//! - **Permissions** on: the DACL is copied (explicit entries; inherited ones
//!   come from the new parent, unless the original doesn't inherit). The owner
//!   is copied when Windows allows it, otherwise the job gets one warning.
//!   Auditing rules (SACL) need a privilege and aren't copied.
//! - **Folder dates**: set after the folder's contents are copied, children
//!   before parents (copying into a folder changes its modified time).
//!
//! Elsewhere streams and ACLs don't exist in this form (the portable copy
//! keeps Unix permissions); folder dates work everywhere.

use std::fs::{self, File, FileTimes};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::copy::CopyError;
use super::{EngineConfig, Job};

/// What to keep, shared by the workers of one job.
pub(super) struct Keep {
    permissions: bool,
    streams: bool,
    folder_dates: bool,
    /// The "couldn't keep the owner" warning was given.
    owner_warned: AtomicBool,
}

impl Keep {
    pub fn new(cfg: &EngineConfig) -> Self {
        Self {
            permissions: cfg.keep_permissions,
            streams: cfg.keep_streams,
            folder_dates: cfg.keep_folder_dates,
            owner_warned: AtomicBool::new(false),
        }
    }

    /// Copy the permissions; `Err` only if the DACL couldn't be copied.
    fn security(&self, job: &Job, src: &Path, dst: &Path) -> io::Result<()> {
        if !imp::copy_security(src, dst)? && !self.owner_warned.swap(true, Ordering::Relaxed) {
            job.push_error(
                dst,
                crate::i18n::t(
                    "Couldn't keep the owner of some files and folders: that needs administrator rights.",
                ),
            );
        }
        Ok(())
    }
}

/// After a file's contents were copied (not after a same-drive move, which
/// keeps everything anyway).
pub(super) fn apply_file(job: &Job, src: &Path, dst: &Path, keep: &Keep) -> Result<(), CopyError> {
    if !keep.streams {
        imp::strip_streams(dst);
    }
    if keep.permissions {
        keep.security(job, src, dst).map_err(|e| {
            CopyError::Io(io::Error::other(crate::trf!(
                "Copied, but couldn't keep the permissions: {e}",
                e = e
            )))
        })?;
    }
    Ok(())
}

/// After all files are copied: streams, permissions and dates of the folders,
/// children before parents.
pub(super) fn finish_dirs(job: &Job, dirs: &[(PathBuf, PathBuf)], keep: &Keep) {
    if !keep.streams && !keep.permissions && !keep.folder_dates {
        return;
    }
    for (src, dst) in dirs.iter().rev() {
        if !job.checkpoint() {
            return;
        }
        if keep.streams {
            if let Err(e) = imp::copy_streams(src, dst) {
                job.push_error(
                    dst,
                    crate::trf!("Couldn't copy the alternate data streams: {e}", e = e),
                );
            }
        }
        if keep.permissions {
            if let Err(e) = keep.security(job, src, dst) {
                job.push_error(
                    dst,
                    crate::trf!("Couldn't keep the permissions: {e}", e = e),
                );
            }
        }
        if keep.folder_dates {
            if let Err(e) = copy_dir_times(src, dst) {
                job.push_error(
                    dst,
                    crate::trf!("Couldn't keep the folder dates: {e}", e = e),
                );
            }
        }
    }
}

/// Give `dst` the modified and accessed time of the folder `src`.
pub(super) fn copy_dir_times(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::metadata(src)?;
    let mut times = FileTimes::new().set_modified(meta.modified()?);
    if let Ok(accessed) = meta.accessed() {
        times = times.set_accessed(accessed);
    }
    open_for_times(dst)?.set_times(times)
}

/// A handle that may change the times of a file or folder.
fn open_for_times(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
        // Needed to open a folder.
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        // futimens works on a read-only handle, also of a folder.
        File::open(path)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{LocalFree, ERROR_HANDLE_EOF, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        EqualSid, GetSecurityDescriptorControl, ACL, DACL_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        PSID, SE_DACL_PROTECTED, UNPROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard,
        WIN32_FIND_STREAM_DATA,
    };

    /// The stream Windows uses to mark files downloaded from the internet.
    const ZONE_IDENTIFIER: &str = ":Zone.Identifier:$DATA";
    const ERROR_INVALID_FUNCTION: u32 = 1;
    const ERROR_NOT_SUPPORTED: u32 = 50;

    /// Same as in `copy.rs`: a `\\?\` path, so long paths work.
    fn wide(path: &Path) -> Vec<u16> {
        if path.as_os_str().to_string_lossy().starts_with(r"\\?\") {
            return path.as_os_str().encode_wide().chain(Some(0)).collect();
        }
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let s = path.as_os_str().to_string_lossy();
        let long: PathBuf = if !path.is_absolute() {
            path.clone()
        } else if let Some(unc) = s.strip_prefix(r"\\") {
            PathBuf::from(format!(r"\\?\UNC\{unc}"))
        } else {
            PathBuf::from(format!(r"\\?\{s}"))
        };
        long.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// `path` + `:name:$DATA`.
    fn stream_path(path: &Path, stream: &str) -> PathBuf {
        let mut s = path.as_os_str().to_owned();
        s.push(stream);
        PathBuf::from(s)
    }

    /// The named streams of a file or folder, like `:name:$DATA`.
    pub(super) fn named_streams(path: &Path) -> io::Result<Vec<String>> {
        let w = wide(path);
        // SAFETY: zeroed is a valid value for this plain C struct.
        let mut data: WIN32_FIND_STREAM_DATA = unsafe { std::mem::zeroed() };
        // SAFETY: `w` is NUL terminated and `data` has the size of the standard info level.
        let find = unsafe {
            FindFirstStreamW(
                w.as_ptr(),
                FindStreamInfoStandard,
                &mut data as *mut _ as *mut std::ffi::c_void,
                0,
            )
        };
        if find == INVALID_HANDLE_VALUE {
            let e = io::Error::last_os_error();
            // No streams at all (a folder without any).
            if e.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
                return Ok(Vec::new());
            }
            return Err(e);
        }
        let mut names = Vec::new();
        loop {
            let len = data
                .cStreamName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(data.cStreamName.len());
            let name = OsString::from_wide(&data.cStreamName[..len])
                .to_string_lossy()
                .into_owned();
            // `::$DATA` is the file's contents.
            if name != "::$DATA" && name.ends_with(":$DATA") {
                names.push(name);
            }
            // SAFETY: `find` is a valid stream search handle.
            if unsafe { FindNextStreamW(find, &mut data as *mut _ as *mut std::ffi::c_void) } == 0 {
                break;
            }
        }
        // SAFETY: `find` came from FindFirstStreamW.
        unsafe { FindClose(find) };
        Ok(names)
    }

    /// Remove all named streams except the internet mark. Best effort.
    pub(super) fn strip_streams(dst: &Path) {
        let Ok(names) = named_streams(dst) else {
            return;
        };
        let extra: Vec<String> = names
            .into_iter()
            .filter(|n| !n.eq_ignore_ascii_case(ZONE_IDENTIFIER))
            .collect();
        if extra.is_empty() {
            return;
        }
        let modified = fs::metadata(dst).and_then(|m| m.modified()).ok();
        for name in &extra {
            if let Err(e) = fs::remove_file(stream_path(dst, name)) {
                log::debug!("could not remove stream {name} of {}: {e}", dst.display());
            }
        }
        // Removing a stream may count as a change; keep the copied date.
        if let Some(m) = modified {
            if let Ok(f) = open_for_times(dst) {
                let _ = f.set_modified(m);
            }
        }
    }

    /// Copy the named streams of `src` to `dst` (for folders: files get
    /// theirs from CopyFileExW).
    pub(super) fn copy_streams(src: &Path, dst: &Path) -> io::Result<()> {
        for name in named_streams(src)? {
            let mut from = File::open(stream_path(src, &name))?;
            let mut to = File::create(stream_path(dst, &name))?;
            io::copy(&mut from, &mut to)?;
        }
        Ok(())
    }

    /// Frees a security descriptor returned by GetNamedSecurityInfoW.
    struct Descriptor(PSECURITY_DESCRIPTOR);

    impl Drop for Descriptor {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the descriptor was allocated by the system for us.
                unsafe { LocalFree(self.0) };
            }
        }
    }

    /// Copy the DACL of `src` to `dst`, then the owner if Windows allows it.
    /// `Ok(false)`: the permissions were copied, the owner couldn't be.
    pub(super) fn copy_security(src: &Path, dst: &Path) -> io::Result<bool> {
        let (ws, wd) = (wide(src), wide(dst));
        let mut owner: PSID = null_mut();
        let mut dacl: *mut ACL = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: valid NUL-terminated path and out pointers; `sd` is freed by `Descriptor`.
        let err = unsafe {
            GetNamedSecurityInfoW(
                ws.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut sd,
            )
        };
        if err != 0 {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
        let _sd = Descriptor(sd);
        if dacl.is_null() {
            // The original has no permissions (e.g. on a FAT drive): the copy
            // simply inherits from its new folder.
            return Ok(true);
        }
        let (mut control, mut revision) = (0u16, 0u32);
        // SAFETY: `sd` is a valid security descriptor.
        unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) };
        let inheritance = if control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        // SAFETY: `wd` is NUL terminated, `dacl` points into `sd`, which is alive.
        let err = unsafe {
            SetNamedSecurityInfoW(
                wd.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | inheritance,
                null_mut(),
                null_mut(),
                dacl,
                null(),
            )
        };
        match err {
            0 => {}
            // The destination can't store permissions (e.g. a FAT USB stick).
            ERROR_NOT_SUPPORTED | ERROR_INVALID_FUNCTION => return Ok(true),
            e => return Err(io::Error::from_raw_os_error(e as i32)),
        }
        if owner.is_null() {
            return Ok(true);
        }
        let mut dst_owner: PSID = null_mut();
        let mut dst_sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: as above.
        let err = unsafe {
            GetNamedSecurityInfoW(
                wd.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut dst_owner,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut dst_sd,
            )
        };
        let _dst_sd = Descriptor(dst_sd);
        // SAFETY: both SIDs point into live descriptors.
        if err == 0 && !dst_owner.is_null() && unsafe { EqualSid(owner, dst_owner) } != 0 {
            return Ok(true);
        }
        // SAFETY: `owner` points into `sd`, which is alive.
        let err = unsafe {
            SetNamedSecurityInfoW(
                wd.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                owner,
                null_mut(),
                null(),
                null(),
            )
        };
        Ok(err == 0)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub(super) fn strip_streams(_dst: &Path) {}

    pub(super) fn copy_streams(_src: &Path, _dst: &Path) -> io::Result<()> {
        Ok(())
    }

    pub(super) fn copy_security(_src: &Path, _dst: &Path) -> io::Result<bool> {
        Ok(true)
    }
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
