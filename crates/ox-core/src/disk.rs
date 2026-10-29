//! Which volume a path lives on, and whether that drive is a spinning disk.
//!
//! The transfer engine uses this to queue jobs per disk (two copies on the
//! same HDD slow each other down a lot) and to copy one file at a time on
//! HDDs, where parallel reads mostly add seeking.

use std::path::{Path, PathBuf};

/// Stable identifier of the volume holding `path` (e.g. `c:` or a device id).
/// Works for paths that don't exist yet by looking at the nearest existing parent.
pub fn volume_id(path: &Path) -> Option<String> {
    // `\\wsl$\X` and `\\wsl.localhost\X` are the same distribution.
    if let Some(distro) = crate::wsl::distro_of(path) {
        return Some(format!("wsl:{}", distro.to_lowercase()));
    }
    imp::volume_id(path)
}

/// `Some(true)` for HDDs, `Some(false)` for SSDs/NVMe, `None` if unknown
/// (network shares, virtual file systems, missing permissions, …).
pub fn is_rotational(path: &Path) -> Option<bool> {
    imp::is_rotational(path)
}

/// Nearest ancestor of `path` (including itself) that exists.
#[cfg_attr(windows, allow(dead_code))]
fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.exists()).map(Path::to_path_buf)
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Path, Prefix};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Ioctl::{
        PropertyStandardQuery, StorageDeviceSeekPenaltyProperty, DEVICE_SEEK_PENALTY_DESCRIPTOR,
        IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_QUERY,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    pub fn volume_id(path: &Path) -> Option<String> {
        match path.components().next()? {
            Component::Prefix(p) => match p.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
                    Some(format!("{}:", (d as char).to_ascii_lowercase()))
                }
                _ => Some(p.as_os_str().to_string_lossy().to_lowercase()),
            },
            _ => None,
        }
    }

    pub fn is_rotational(path: &Path) -> Option<bool> {
        let drive = match path.components().next()? {
            Component::Prefix(p) => match p.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => d as char,
                _ => return None, // network share
            },
            _ => return None,
        };
        let device: Vec<u16> = std::ffi::OsStr::new(&format!(r"\\.\{drive}:"))
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: plain Win32 calls with correctly sized buffers; the handle is closed below.
        unsafe {
            // Zero access rights are enough for this query and need no admin rights.
            let handle = CreateFileW(
                device.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return None;
            }
            let query = STORAGE_PROPERTY_QUERY {
                PropertyId: StorageDeviceSeekPenaltyProperty,
                QueryType: PropertyStandardQuery,
                AdditionalParameters: [0],
            };
            let mut out = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
            let mut returned = 0u32;
            let ok = DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                &query as *const _ as *const _,
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                &mut out as *mut _ as *mut _,
                std::mem::size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
                &mut returned,
                std::ptr::null_mut(),
            );
            CloseHandle(handle);
            (ok != 0).then_some(out.IncursSeekPenalty)
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::existing_ancestor;
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    fn dev(path: &Path) -> Option<u64> {
        fs::metadata(existing_ancestor(path)?).ok().map(|m| m.dev())
    }

    pub fn volume_id(path: &Path) -> Option<String> {
        dev(path).map(|d| format!("dev:{d}"))
    }

    pub fn is_rotational(path: &Path) -> Option<bool> {
        let dev = dev(path)?;
        // Linux dev_t encoding.
        let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
        let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
        if major == 0 {
            return None; // tmpfs, overlayfs, …
        }
        let node = fs::canonicalize(format!("/sys/dev/block/{major}:{minor}")).ok()?;
        // Partitions have no queue/ directory; their parent device does.
        let read = |dir: &Path| fs::read_to_string(dir.join("queue/rotational")).ok();
        let flag = read(&node).or_else(|| read(node.parent()?))?;
        match flag.trim() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_volume_for_new_paths() {
        let dir = tempfile::tempdir().unwrap();
        let a = volume_id(dir.path()).expect("temp dir has a volume");
        // A path that doesn't exist yet resolves to its parent's volume.
        assert_eq!(volume_id(&dir.path().join("not/created/yet")), Some(a));
    }

    #[test]
    fn wsl_distributions_are_volumes() {
        let a = volume_id(Path::new(r"\\wsl$\Ubuntu\home\a"));
        assert_eq!(a.as_deref(), Some("wsl:ubuntu"));
        assert_eq!(volume_id(Path::new(r"\\wsl.localhost\ubuntu\tmp")), a);
    }

    #[test]
    fn rotational_never_panics() {
        let dir = tempfile::tempdir().unwrap();
        let _ = is_rotational(dir.path());
        let _ = is_rotational(Path::new("/definitely/not/here"));
    }
}
