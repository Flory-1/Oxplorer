//! Copying a single file as fast as the platform allows.

use std::io;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct CopyOptions {
    pub buffer_size: usize,
    pub unbuffered_threshold: u64,
    pub preserve_timestamps: bool,
}

#[derive(Debug)]
pub(crate) enum CopyError {
    Cancelled,
    Io(io::Error),
}

impl From<io::Error> for CopyError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Copy `src` to `dst`, overwriting `dst`.
///
/// `progress` is called with the number of newly copied bytes and returns
/// `false` to cancel. It may block (that is how pausing works).
pub(crate) fn copy_file(
    src: &Path,
    dst: &Path,
    size: u64,
    opts: &CopyOptions,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<(), CopyError> {
    imp::copy_file(src, dst, size, opts, progress)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        CopyFileExW, COPY_FILE_NO_BUFFERING, PROGRESS_CANCEL, PROGRESS_CONTINUE,
    };

    const ERROR_REQUEST_ABORTED: i32 = 1235;

    struct Ctx<'a> {
        progress: &'a mut dyn FnMut(u64) -> bool,
        reported: u64,
    }

    unsafe extern "system" fn routine(
        _total: i64,
        transferred: i64,
        _stream_size: i64,
        _stream_transferred: i64,
        _stream_number: u32,
        _reason: u32,
        _src: HANDLE,
        _dst: HANDLE,
        data: *const c_void,
    ) -> u32 {
        // SAFETY: `data` is the `Ctx` passed to CopyFileExW below, which outlives the call.
        let ctx = unsafe { &mut *(data as *mut Ctx) };
        let transferred = transferred.max(0) as u64;
        let delta = transferred.saturating_sub(ctx.reported);
        ctx.reported = ctx.reported.max(transferred);
        if (ctx.progress)(delta) {
            PROGRESS_CONTINUE
        } else {
            PROGRESS_CANCEL
        }
    }

    /// Paths get the `\\?\` prefix so paths longer than 260 characters work.
    /// Windows doesn't normalise prefixed paths, so `/`, `.` and `..` are
    /// resolved first (`GetFullPathNameW`).
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

    pub(super) fn copy_file(
        src: &Path,
        dst: &Path,
        size: u64,
        opts: &CopyOptions,
        progress: &mut dyn FnMut(u64) -> bool,
    ) -> Result<(), CopyError> {
        let wsrc = wide(src);
        let wdst = wide(dst);
        let mut flags = 0;
        if size >= opts.unbuffered_threshold {
            // Bypasses the cache manager: much faster for huge files, and doesn't
            // evict everything else from the file cache.
            flags |= COPY_FILE_NO_BUFFERING;
        }
        let mut ctx = Ctx {
            progress,
            reported: 0,
        };
        // SAFETY: both strings are NUL terminated and `ctx` lives across the call.
        let ok = unsafe {
            CopyFileExW(
                wsrc.as_ptr(),
                wdst.as_ptr(),
                Some(routine),
                &mut ctx as *mut Ctx as *const c_void,
                std::ptr::null_mut(),
                flags,
            )
        };
        if ok == 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_REQUEST_ABORTED) {
                return Err(CopyError::Cancelled);
            }
            return Err(CopyError::Io(err));
        }
        // CopyFileExW keeps timestamps and attributes by itself.
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};

    /// Below this size the OS copy (copy_file_range / reflink) is used directly.
    const SMALL_FILE: u64 = 4 * 1024 * 1024;

    pub(super) fn copy_file(
        src: &Path,
        dst: &Path,
        size: u64,
        opts: &CopyOptions,
        progress: &mut dyn FnMut(u64) -> bool,
    ) -> Result<(), CopyError> {
        if size <= SMALL_FILE {
            fs::copy(src, dst)?;
            preserve_times(src, dst, opts)?;
            return if progress(size) {
                Ok(())
            } else {
                Err(CopyError::Cancelled)
            };
        }

        let mut input = fs::File::open(src)?;
        let mut output = fs::File::create(dst)?;
        let mut buf = vec![0u8; opts.buffer_size];
        let result = (|| -> Result<(), CopyError> {
            loop {
                let n = match input.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e.into()),
                };
                output.write_all(&buf[..n])?;
                if !progress(n as u64) {
                    return Err(CopyError::Cancelled);
                }
            }
            output.flush()?;
            Ok(())
        })();
        drop(output);
        if result.is_err() {
            let _ = fs::remove_file(dst);
            return result;
        }
        if let Ok(meta) = fs::metadata(src) {
            let _ = fs::set_permissions(dst, meta.permissions());
        }
        preserve_times(src, dst, opts)?;
        Ok(())
    }

    fn preserve_times(src: &Path, dst: &Path, opts: &CopyOptions) -> io::Result<()> {
        if !opts.preserve_timestamps {
            return Ok(());
        }
        let modified = fs::metadata(src)?.modified()?;
        // Opening for write fails on read-only copies; timestamps are best effort.
        if let Ok(f) = fs::OpenOptions::new().write(true).open(dst) {
            let _ = f.set_modified(modified);
        }
        Ok(())
    }
}
