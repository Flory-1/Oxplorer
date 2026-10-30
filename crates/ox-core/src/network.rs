//! Network locations: browsing `\\server` (which lists its shares), mapped
//! network drives and the Windows "Map network drive" dialogs.
//!
//! `\\server\share\…` paths are ordinary folders for the file system; only
//! the server itself needs the network API to list what it shares.

use std::io;
use std::path::{Path, PathBuf};

use crate::entry::FileEntry;

/// The server of a `\\server` path (no share), e.g. `"nas"`.
pub fn server_name(path: &Path) -> Option<&str> {
    let rest = path.to_str()?.strip_prefix(r"\\")?;
    // `\\?\…` and `\\.\…` are device paths, not servers.
    if rest.starts_with(['?', '.']) {
        return None;
    }
    let rest = rest.trim_end_matches(['\\', '/']);
    (!rest.is_empty() && !rest.contains(['\\', '/'])).then_some(rest)
}

/// `\\server` for a path on one of its shares (`\\server\share\…`). WSL
/// distributions (`\\wsl$\Ubuntu`) are roots of their own, like drives.
pub fn server_of(path: &Path) -> Option<PathBuf> {
    let rest = path.to_str()?.strip_prefix(r"\\")?;
    if rest.starts_with(['?', '.']) {
        return None;
    }
    let (server, share) = rest.split_once(['\\', '/'])?;
    (!server.is_empty()
        && !crate::wsl::is_wsl_host(server)
        && !share.trim_matches(['\\', '/']).is_empty())
    .then(|| PathBuf::from(format!(r"\\{server}")))
}

/// Whether `path` can be opened as a folder (a server counts as one). A
/// remembered network drive that isn't connected yet is connected first.
pub fn is_dir(path: &Path) -> bool {
    server_name(path).is_some() || path.is_dir() || (reconnect(path) && path.is_dir())
}

/// Connect `X:\` again if it is a remembered network drive that Windows
/// hasn't reconnected yet (after logon they stay away until opened).
/// Returns whether it was connected.
pub fn reconnect(path: &Path) -> bool {
    #[cfg(windows)]
    {
        imp::reconnect(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// The folder above `path`; for a share's root that is its server.
pub fn parent(path: &Path) -> Option<PathBuf> {
    if server_name(path).is_some() {
        return None;
    }
    path.parent()
        .map(Path::to_path_buf)
        .or_else(|| server_of(path))
}

/// The shares of `\\server`, as folders.
pub fn shares(server: &str) -> io::Result<Vec<FileEntry>> {
    #[cfg(windows)]
    {
        imp::shares(server)
    }
    #[cfg(not(windows))]
    {
        let _ = server;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "network shares can only be listed on Windows",
        ))
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
/// A share folder entry (its metadata isn't read: that can take long).
fn share_entry(server: &str, share: &str) -> FileEntry {
    FileEntry {
        path: PathBuf::from(format!(r"\\{server}\{share}")),
        name: share.to_owned(),
        is_dir: true,
        is_symlink: false,
        size: 0,
        modified: None,
        created: None,
        hidden: false,
        readonly: false,
        cloud: None,
    }
}

/// A drive letter connected to a network share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedDrive {
    /// `Z:\`
    pub mount: PathBuf,
    /// `\\server\share`
    pub remote: String,
}

/// Drive letters mapped to network shares (Windows; empty elsewhere).
pub fn mapped_drives() -> Vec<MappedDrive> {
    #[cfg(windows)]
    {
        imp::mapped_drives()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// "share (\\server)" for `\\server\share`, like Explorer names mapped drives.
pub fn share_title(remote: &str) -> String {
    let trimmed = remote.trim_start_matches('\\');
    match trimmed.split_once('\\') {
        Some((server, share)) => format!(r"{} (\\{server})", share.trim_end_matches('\\')),
        None => remote.to_owned(),
    }
}

/// Show Windows' "Map Network Drive" dialog (does nothing elsewhere).
pub fn map_drive_dialog() {
    #[cfg(windows)]
    imp::dialog(true);
}

/// Show Windows' "Disconnect Network Drives" dialog (does nothing elsewhere).
pub fn disconnect_drive_dialog() {
    #[cfg(windows)]
    imp::dialog(false);
}

#[cfg(windows)]
mod imp {
    use super::{share_entry, MappedDrive};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, HANDLE, HWND, NO_ERROR};
    use windows::Win32::NetworkManagement::WNet::{
        WNetAddConnection2W, WNetCloseEnum, WNetConnectionDialog, WNetDisconnectDialog,
        WNetEnumResourceW, WNetGetConnectionW, WNetOpenEnumW, NETRESOURCEW, NET_CONNECT_FLAGS,
        RESOURCETYPE_DISK, RESOURCEUSAGE_CONTAINER, RESOURCE_GLOBALNET, RESOURCE_REMEMBERED,
        WNET_OPEN_ENUM_USAGE,
    };
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    const DRIVE_REMOTE: u32 = 4;

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }

    fn from_wide(p: PWSTR) -> Option<String> {
        // SAFETY: the API hands out NUL-terminated strings (or null).
        (!p.is_null())
            .then(|| unsafe { p.to_string() }.ok())
            .flatten()
    }

    pub(super) fn shares(server: &str) -> io::Result<Vec<crate::entry::FileEntry>> {
        let mut remote = wide(format!(r"\\{server}"));
        let container = NETRESOURCEW {
            dwScope: RESOURCE_GLOBALNET,
            dwType: RESOURCETYPE_DISK,
            dwUsage: RESOURCEUSAGE_CONTAINER.0,
            lpRemoteName: PWSTR(remote.as_mut_ptr()),
            ..Default::default()
        };
        let mut handle = HANDLE::default();
        // SAFETY: `container` and `remote` outlive the call.
        let err = unsafe {
            WNetOpenEnumW(
                RESOURCE_GLOBALNET,
                RESOURCETYPE_DISK,
                WNET_OPEN_ENUM_USAGE(0),
                Some(&container),
                &mut handle,
            )
        };
        if err != NO_ERROR {
            return Err(io::Error::from_raw_os_error(err.0 as i32));
        }
        let mut out = Vec::new();
        // u64s keep the buffer aligned for NETRESOURCEW.
        let mut buf = vec![0u64; 4096];
        let result = loop {
            let mut count = u32::MAX;
            let mut size = (buf.len() * 8) as u32;
            // SAFETY: `buf` is `size` bytes long.
            let err = unsafe {
                WNetEnumResourceW(handle, &mut count, buf.as_mut_ptr().cast(), &mut size)
            };
            if err == ERROR_NO_MORE_ITEMS {
                break Ok(());
            }
            if err != NO_ERROR {
                break Err(io::Error::from_raw_os_error(err.0 as i32));
            }
            // SAFETY: the call filled `count` entries at the start of `buf`.
            let items = unsafe {
                std::slice::from_raw_parts(buf.as_ptr().cast::<NETRESOURCEW>(), count as usize)
            };
            for item in items {
                let Some(name) = from_wide(item.lpRemoteName) else {
                    continue;
                };
                if let Some(share) = name.rsplit('\\').next().filter(|s| !s.is_empty()) {
                    out.push(share_entry(server, share));
                }
            }
        };
        // SAFETY: opened above.
        let _ = unsafe { WNetCloseEnum(handle) };
        result.map(|()| out)
    }

    /// Remembered network drives (connected or not): drive letter and
    /// `\\server\share`.
    fn remembered() -> Vec<(char, String)> {
        let mut handle = HANDLE::default();
        // SAFETY: no container; `handle` is closed below.
        let err = unsafe {
            WNetOpenEnumW(
                RESOURCE_REMEMBERED,
                RESOURCETYPE_DISK,
                WNET_OPEN_ENUM_USAGE(0),
                None,
                &mut handle,
            )
        };
        if err != NO_ERROR {
            return Vec::new();
        }
        let mut out = Vec::new();
        // u64s keep the buffer aligned for NETRESOURCEW.
        let mut buf = vec![0u64; 4096];
        loop {
            let mut count = u32::MAX;
            let mut size = (buf.len() * 8) as u32;
            // SAFETY: `buf` is `size` bytes long.
            let err = unsafe {
                WNetEnumResourceW(handle, &mut count, buf.as_mut_ptr().cast(), &mut size)
            };
            if err != NO_ERROR {
                break;
            }
            // SAFETY: the call filled `count` entries at the start of `buf`.
            let items = unsafe {
                std::slice::from_raw_parts(buf.as_ptr().cast::<NETRESOURCEW>(), count as usize)
            };
            for item in items {
                let local = from_wide(item.lpLocalName).unwrap_or_default();
                let letter = local.chars().next().filter(char::is_ascii_alphabetic);
                if let (Some(letter), Some(remote)) = (letter, from_wide(item.lpRemoteName)) {
                    out.push((letter.to_ascii_uppercase(), remote));
                }
            }
        }
        // SAFETY: opened above.
        let _ = unsafe { WNetCloseEnum(handle) };
        out
    }

    fn connected(letter: char) -> bool {
        // SAFETY: no arguments.
        let mask = unsafe { GetLogicalDrives() };
        mask & (1 << (letter as u8 - b'A')) != 0
    }

    pub(super) fn reconnect(path: &std::path::Path) -> bool {
        let Some(s) = path.to_str() else {
            return false;
        };
        let letter = match s.as_bytes() {
            [l, b':'] | [l, b':', b'\\'] if l.is_ascii_alphabetic() => {
                l.to_ascii_uppercase() as char
            }
            _ => return false,
        };
        if connected(letter) {
            return false;
        }
        let Some((_, remote)) = remembered().into_iter().find(|(l, _)| *l == letter) else {
            return false;
        };
        let mut local = wide(format!("{letter}:"));
        let mut remote = wide(remote);
        let res = NETRESOURCEW {
            dwType: RESOURCETYPE_DISK,
            lpLocalName: PWSTR(local.as_mut_ptr()),
            lpRemoteName: PWSTR(remote.as_mut_ptr()),
            ..Default::default()
        };
        // SAFETY: `res` and its strings outlive the call; the saved
        // credentials are used.
        let err = unsafe {
            WNetAddConnection2W(&res, PCWSTR::null(), PCWSTR::null(), NET_CONNECT_FLAGS(0))
        };
        err == NO_ERROR
    }

    pub(super) fn mapped_drives() -> Vec<MappedDrive> {
        // SAFETY: plain Win32 queries with buffers owned here.
        let mask = unsafe { GetLogicalDrives() };
        // Remembered drives Windows hasn't connected again yet: shown like
        // Explorer does, and connected when opened.
        let waiting = remembered()
            .into_iter()
            .filter(|(l, _)| mask & (1 << (*l as u8 - b'A')) == 0)
            .map(|(l, remote)| MappedDrive {
                mount: PathBuf::from(format!(r"{l}:\")),
                remote,
            });
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .filter_map(|i| {
                let letter = (b'A' + i) as char;
                let root = wide(format!(r"{letter}:\"));
                if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != DRIVE_REMOTE {
                    return None;
                }
                let local = wide(format!("{letter}:"));
                let mut buf = [0u16; 1024];
                let mut len = buf.len() as u32;
                let err = unsafe {
                    WNetGetConnectionW(
                        PCWSTR(local.as_ptr()),
                        Some(PWSTR(buf.as_mut_ptr())),
                        &mut len,
                    )
                };
                let remote = (err == NO_ERROR)
                    .then(|| from_wide(PWSTR(buf.as_mut_ptr())))
                    .flatten()
                    .unwrap_or_default();
                Some(MappedDrive {
                    mount: PathBuf::from(format!(r"{letter}:\")),
                    remote,
                })
            })
            .chain(waiting)
            .collect()
    }

    /// The dialogs are modal, so they get their own thread; the app window
    /// owns them so they stay on top of it.
    pub(super) fn dialog(map: bool) {
        // SAFETY: just reads the foreground window handle.
        let owner = unsafe { GetForegroundWindow() }.0 as isize;
        std::thread::spawn(move || {
            let hwnd = HWND(owner as *mut _);
            // SAFETY: modal system dialogs; `hwnd` is only used as their owner.
            unsafe {
                if map {
                    let _ = WNetConnectionDialog(hwnd, RESOURCETYPE_DISK.0);
                } else {
                    let _ = WNetDisconnectDialog(Some(hwnd), RESOURCETYPE_DISK.0);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_and_shares() {
        assert_eq!(server_name(Path::new(r"\\nas")), Some("nas"));
        assert_eq!(server_name(Path::new(r"\\nas\")), Some("nas"));
        assert_eq!(server_name(Path::new(r"\\nas\media")), None);
        assert_eq!(server_name(Path::new(r"\\?\C:\x")), None);
        assert_eq!(server_name(Path::new(r"\\.\pipe")), None);
        assert_eq!(server_name(Path::new(r"C:\x")), None);

        assert_eq!(
            server_of(Path::new(r"\\nas\media\films")),
            Some(PathBuf::from(r"\\nas"))
        );
        assert_eq!(
            server_of(Path::new(r"\\nas\media\")),
            Some(PathBuf::from(r"\\nas"))
        );
        assert_eq!(server_of(Path::new(r"\\nas")), None);
        assert_eq!(server_of(Path::new(r"\\?\UNC\nas\media")), None);

        assert!(is_dir(Path::new(r"\\nas")));
        assert_eq!(parent(Path::new(r"\\nas")), None);

        // A WSL distribution is a root, not a share of `\\wsl$`.
        assert_eq!(server_of(Path::new(r"\\wsl.localhost\Ubuntu\home")), None);
        assert_eq!(parent(Path::new(r"\\wsl$\Ubuntu\")), None);
    }

    #[test]
    fn share_titles() {
        assert_eq!(share_title(r"\\nas\media"), r"media (\\nas)");
        assert_eq!(share_title(r"\\nas\media\"), r"media (\\nas)");
        assert_eq!(share_title("weird"), "weird");
        let e = share_entry("nas", "media");
        assert_eq!(e.path, PathBuf::from(r"\\nas\media"));
        assert!(e.is_dir);
    }
}
