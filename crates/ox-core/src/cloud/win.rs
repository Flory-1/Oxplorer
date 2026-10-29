//! The Windows side of cloud files: the Cloud Files API (`cldapi`) and the
//! shell's sync properties.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, PROPERTYKEY};
use windows::Win32::Storage::CloudFilters::{
    CfGetPlaceholderStateFromAttributeTag, CfGetSyncRootInfoByPath, CfSetPinState,
    CF_PIN_STATE_PINNED, CF_PIN_STATE_UNPINNED, CF_PIN_STATE_UNSPECIFIED, CF_SET_PIN_FLAG_NONE,
    CF_SET_PIN_FLAG_RECURSE, CF_SYNC_ROOT_INFO_PROVIDER, CF_SYNC_ROOT_PROVIDER_INFO,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FIND_FIRST_EX_LARGE_FETCH, OPEN_EXISTING, WIN32_FIND_DATAW,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{IShellItem2, SHCreateItemFromParsingName};

const REPARSE_POINT: u32 = 0x400;

/// System.SyncTransferStatus (propkey.h).
const PKEY_SYNC_TRANSFER_STATUS: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xfceff153_e839_4cf3_a9e7_ea22832094b8),
    pid: 103,
};

#[link(name = "ntdll")]
unsafe extern "system" {
    /// Windows 10 1803+. Returns the previous mode, negative on errors.
    fn RtlSetThreadPlaceholderCompatibilityMode(mode: i8) -> i8;
}

const PHCM_EXPOSE_PLACEHOLDERS: i8 = 2;

/// Lets this thread see placeholders as they are (reparse point and tag),
/// whatever Windows decided for the process; restores the mode on drop.
struct ExposePlaceholders(i8);

impl ExposePlaceholders {
    fn new() -> Self {
        // SAFETY: only changes how this thread sees placeholders.
        Self(unsafe { RtlSetThreadPlaceholderCompatibilityMode(PHCM_EXPOSE_PLACEHOLDERS) })
    }
}

impl Drop for ExposePlaceholders {
    fn drop(&mut self) {
        if self.0 >= 0 {
            // SAFETY: as above, back to the previous mode.
            unsafe { RtlSetThreadPlaceholderCompatibilityMode(self.0) };
        }
    }
}

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

pub(super) fn provider(path: &Path) -> Option<String> {
    let name = wide(path);
    let mut info = CF_SYNC_ROOT_PROVIDER_INFO::default();
    // SAFETY: `name` is NUL-terminated; `info` is as large as we say.
    unsafe {
        CfGetSyncRootInfoByPath(
            PCWSTR(name.as_ptr()),
            CF_SYNC_ROOT_INFO_PROVIDER,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<CF_SYNC_ROOT_PROVIDER_INFO>() as u32,
            None,
        )
        .ok()?;
    }
    let len = info
        .ProviderName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.ProviderName.len());
    let name = String::from_utf16_lossy(&info.ProviderName[..len]);
    Some(if name.trim().is_empty() {
        "Cloud".to_owned()
    } else {
        name
    })
}

fn state_of(data: &WIN32_FIND_DATAW) -> (u32, u32) {
    let attrs = data.dwFileAttributes;
    let tag = if attrs & REPARSE_POINT != 0 {
        data.dwReserved0
    } else {
        0
    };
    // SAFETY: plain function of two numbers.
    let state = unsafe { CfGetPlaceholderStateFromAttributeTag(attrs, tag) };
    (attrs, state.0)
}

fn file_name(data: &WIN32_FIND_DATAW) -> String {
    let len = data
        .cFileName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(data.cFileName.len());
    String::from_utf16_lossy(&data.cFileName[..len])
}

/// Attributes and placeholder state of every item in `dir`, by lower-case name.
pub(super) fn placeholder_states(dir: &Path) -> Option<HashMap<String, (u32, u32)>> {
    let _expose = ExposePlaceholders::new();
    let pattern = wide(dir.join("*"));
    let mut data = WIN32_FIND_DATAW::default();
    // SAFETY: `pattern` is NUL-terminated, `data` is a WIN32_FIND_DATAW and
    // the handle is closed below.
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
        .ok()?
    };
    let mut out = HashMap::new();
    loop {
        let name = file_name(&data);
        if name != "." && name != ".." {
            out.insert(name.to_lowercase(), state_of(&data));
        }
        // SAFETY: `handle` is an open find handle.
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    // SAFETY: opened above.
    let _ = unsafe { FindClose(handle) };
    Some(out)
}

/// Attributes and placeholder state of one item.
pub(super) fn placeholder_state(path: &Path) -> Option<(u32, u32)> {
    let _expose = ExposePlaceholders::new();
    let name = wide(path);
    let mut data = WIN32_FIND_DATAW::default();
    // SAFETY: as in `placeholder_states`.
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(name.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            Default::default(),
        )
        .ok()?
    };
    // SAFETY: opened above.
    let _ = unsafe { FindClose(handle) };
    Some(state_of(&data))
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

/// The shell's System.SyncTransferStatus of an item, if the provider sets it.
pub(super) fn transfer_status(path: &Path) -> Option<u32> {
    let _com = Com::new();
    let name = wide(path);
    // SAFETY: `name` is NUL-terminated; the item is released on drop.
    unsafe {
        let item: IShellItem2 = SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None).ok()?;
        item.GetUInt32(&PKEY_SYNC_TRANSFER_STATUS).ok()
    }
}

/// Pin or unpin an item; folders with everything in them.
pub(super) fn set_pin(path: &Path, pin: super::Pin) -> Result<(), String> {
    let name = wide(path);
    let is_dir = path.is_dir();
    // SAFETY: `name` is NUL-terminated; an attributes-only handle is enough
    // for CfSetPinState and doesn't download anything. Closed below.
    unsafe {
        let handle = CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .map_err(|e| e.message())?;
        let state = match pin {
            super::Pin::Always => CF_PIN_STATE_PINNED,
            super::Pin::Default => CF_PIN_STATE_UNSPECIFIED,
            super::Pin::Online => CF_PIN_STATE_UNPINNED,
        };
        let flags = if is_dir {
            CF_SET_PIN_FLAG_RECURSE
        } else {
            CF_SET_PIN_FLAG_NONE
        };
        let result = CfSetPinState(handle, state, flags, None).map_err(|e| e.message());
        let _ = CloseHandle(handle);
        result
    }
}
