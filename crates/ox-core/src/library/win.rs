//! Libraries through the Windows Shell (`IShellLibrary`).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::{
    IShellItem, IShellItemArray, IShellLibrary, SHCreateItemFromParsingName, SHGetKnownFolderPath,
    ShellLibrary, DSFT_DETECT, KF_FLAG_DEFAULT, LFF_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
    SIGDN_NORMALDISPLAY,
};

use super::Library;

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
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

fn display(item: &IShellItem, sigdn: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
    // SAFETY: the returned string is freed by `take`.
    unsafe { take(item.GetDisplayName(sigdn).ok()?) }
}

pub(super) fn load(file: &Path) -> Option<Library> {
    let _com = Com::new();
    let name = wide(file);
    // SAFETY: COM is initialised; every interface is released on drop.
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None).ok()?;
        let lib: IShellLibrary =
            CoCreateInstance(&ShellLibrary, None, CLSCTX_INPROC_SERVER).ok()?;
        lib.LoadLibraryFromItem(&item, STGM_READ.0).ok()?;
        let mut folders = Vec::new();
        if let Ok(array) = lib.GetFolders::<IShellItemArray>(LFF_FORCEFILESYSTEM) {
            for i in 0..array.GetCount().unwrap_or(0) {
                let path = array
                    .GetItemAt(i)
                    .ok()
                    .and_then(|f| display(&f, SIGDN_FILESYSPATH))
                    .map(PathBuf::from);
                if let Some(p) = path.filter(|p| p.is_dir() && !folders.contains(p)) {
                    folders.push(p);
                }
            }
        }
        let save_folder = lib
            .GetDefaultSaveFolder::<IShellItem>(DSFT_DETECT)
            .ok()
            .and_then(|f| display(&f, SIGDN_FILESYSPATH))
            .map(PathBuf::from)
            .or_else(|| folders.first().cloned());
        let shown = display(&item, SIGDN_NORMALDISPLAY)
            .map(|n| n.trim_end_matches(".library-ms").to_owned())
            .filter(|n| !n.trim().is_empty());
        Some(Library {
            name: shown.unwrap_or_else(|| super::stem(file)),
            file: file.to_path_buf(),
            folders,
            save_folder,
            pinned: true,
        })
    }
}

/// The path of a known folder from its `{GUID}`.
pub(super) fn known_folder(id: &str) -> Option<PathBuf> {
    let id = id.trim().trim_start_matches('{').trim_end_matches('}');
    let guid = GUID::try_from(id).ok()?;
    // SAFETY: the returned string is freed by `take`.
    unsafe { take(SHGetKnownFolderPath(&guid, KF_FLAG_DEFAULT, None).ok()?) }.map(PathBuf::from)
}
