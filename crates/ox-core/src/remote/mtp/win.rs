//! Phones and cameras through the Windows Shell: the devices under This PC
//! that aren't file system folders, their items and their files' content.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{FILETIME, PROPERTYKEY};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, IStream, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::SystemServices::{
    SFGAO_FILESYSTEM, SFGAO_FLAGS, SFGAO_FOLDER, SFGAO_STREAM,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, BHID_Stream, FOLDERID_ComputerFolder, IEnumShellItems, IShellItem, IShellItem2,
    SHCreateItemFromParsingName, SHGetKnownFolderItem, KF_FLAG_DEFAULT, SIGDN,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};

use super::{ShellChild, ShellTree};

/// `System.FileName`, `System.Size` and `System.DateModified`.
const PKEY_FILE_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x41cf5ae0_f75a_4806_bd87_59c7d9248eb9),
    pid: 100,
};
const PKEY_SIZE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac),
    pid: 12,
};
const PKEY_DATE_MODIFIED: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac),
    pid: 14,
};

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn io_err(e: windows::core::Error) -> io::Error {
    io::Error::other(e.message())
}

/// COM for this thread while it lives.
struct Com(bool);

impl Com {
    fn new() -> Self {
        // SAFETY: paired with CoUninitialize on drop when it succeeded.
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: CoInitializeEx succeeded on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// Take a Shell-allocated string.
///
/// # Safety
/// `p` must come from the Shell (CoTaskMemAlloc) or be null.
unsafe fn take(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated string from the Shell, freed once here.
    unsafe {
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

fn display(item: &IShellItem, sigdn: SIGDN) -> Option<String> {
    // SAFETY: the string is taken (and freed) right away.
    unsafe { take(item.GetDisplayName(sigdn).ok()?) }
}

fn item(parsing: &str) -> io::Result<IShellItem> {
    let name = wide(parsing);
    // SAFETY: `name` is NUL-terminated and outlives the call.
    unsafe { SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None) }
        // Gone: the device was unplugged (or locked again).
        .map_err(|_| io::Error::from(io::ErrorKind::NotFound))
}

/// The items in a Shell folder.
fn items(folder: &IShellItem) -> io::Result<Vec<IShellItem>> {
    // SAFETY: plain COM calls on live objects.
    unsafe {
        let list: IEnumShellItems = folder
            .BindToHandler(None, &BHID_EnumItems)
            .map_err(io_err)?;
        let mut out = Vec::new();
        loop {
            let mut next = [None];
            let mut fetched = 0;
            if list.Next(&mut next, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            if let Some(i) = next[0].take() {
                out.push(i);
            }
        }
        Ok(out)
    }
}

fn attributes(item: &IShellItem, mask: SFGAO_FLAGS) -> u32 {
    // SAFETY: plain COM call.
    unsafe { item.GetAttributes(mask) }.map_or(0, |a| a.0)
}

/// The connected phones and cameras: (parsing name, name).
pub(super) fn devices() -> Vec<(String, String)> {
    let _com = Com::new();
    // SAFETY: plain COM call.
    let Ok(pc) = (unsafe {
        SHGetKnownFolderItem::<IShellItem>(&FOLDERID_ComputerFolder, KF_FLAG_DEFAULT, None)
    }) else {
        return Vec::new();
    };
    let Ok(children) = items(&pc) else {
        return Vec::new();
    };
    children
        .iter()
        .filter(|i| {
            let a = attributes(i, SFGAO_FOLDER | SFGAO_FILESYSTEM);
            a & SFGAO_FOLDER.0 != 0 && a & SFGAO_FILESYSTEM.0 == 0
        })
        .filter_map(|i| {
            let parsing = display(i, SIGDN_DESKTOPABSOLUTEPARSING)?;
            // Portable devices are reached by a device path (`\\?\usb#…`).
            parsing.contains(r"\\?\").then_some(())?;
            let name = display(i, SIGDN_NORMALDISPLAY).unwrap_or_else(|| parsing.clone());
            Some((parsing, name))
        })
        .collect()
}

fn to_system_time(ft: FILETIME) -> Option<SystemTime> {
    let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    // 100 ns ticks since 1601; Unix time starts 11 644 473 600 s later.
    let secs = (ticks / 10_000_000).checked_sub(11_644_473_600)?;
    Some(UNIX_EPOCH + Duration::new(secs, (ticks % 10_000_000) as u32 * 100))
}

fn child(i: &IShellItem) -> Option<ShellChild> {
    let parsing = display(i, SIGDN_DESKTOPABSOLUTEPARSING)?;
    let props: Option<IShellItem2> = windows::core::Interface::cast(i).ok();
    // SAFETY: plain COM calls; the string is taken (and freed) right away.
    let (name, size, modified) = unsafe {
        let name = props
            .as_ref()
            .and_then(|p| p.GetString(&PKEY_FILE_NAME).ok())
            .and_then(|s| take(s));
        let size = props
            .as_ref()
            .and_then(|p| p.GetUInt64(&PKEY_SIZE).ok())
            .unwrap_or(0);
        let modified = props
            .as_ref()
            .and_then(|p| p.GetFileTime(&PKEY_DATE_MODIFIED).ok())
            .and_then(to_system_time);
        (name, size, modified)
    };
    let name = name.or_else(|| display(i, SIGDN_NORMALDISPLAY))?;
    let a = attributes(i, SFGAO_FOLDER | SFGAO_STREAM);
    Some(ShellChild {
        name,
        parsing,
        // Zip files are folders with a stream too: those are files.
        is_dir: a & SFGAO_FOLDER.0 != 0 && a & SFGAO_STREAM.0 == 0,
        size,
        modified,
    })
}

/// The Windows Shell.
pub(crate) struct Shell;

impl ShellTree for Shell {
    fn children(&self, parsing: &str) -> io::Result<Vec<ShellChild>> {
        let _com = Com::new();
        let folder = item(parsing)?;
        Ok(items(&folder)?.iter().filter_map(child).collect())
    }

    fn read(&self, parsing: &str, out: &mut dyn Write) -> io::Result<u64> {
        let _com = Com::new();
        let file = item(parsing)?;
        // SAFETY: plain COM calls; `buf` outlives each read.
        unsafe {
            let stream: IStream = file.BindToHandler(None, &BHID_Stream).map_err(io_err)?;
            let mut buf = vec![0u8; 256 * 1024];
            let mut total = 0u64;
            loop {
                let mut got = 0u32;
                stream
                    .Read(buf.as_mut_ptr().cast(), buf.len() as u32, Some(&mut got))
                    .ok()
                    .map_err(io_err)?;
                if got == 0 {
                    break;
                }
                // An error here (the transfer was cancelled) stops reading.
                out.write_all(&buf[..got as usize])?;
                total += u64::from(got);
            }
            Ok(total)
        }
    }
}
