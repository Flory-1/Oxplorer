//! Windows' own *Open* dialog to choose a folder or a file
//! (`IFileOpenDialog`). It blocks until it's closed, so run it on its own
//! thread; with the app's window as owner it stays in front of it.

use std::path::{Path, PathBuf};

/// What there is to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    Folder,
    File,
}

/// The active window of the calling thread (call it on the UI thread), as
/// the owner of a dialog shown from another thread. 0 if there is none.
pub fn owner_window() -> isize {
    #[cfg(windows)]
    {
        // SAFETY: no arguments; returns a handle or null.
        let hwnd = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow() };
        hwnd.0 as isize
    }
    #[cfg(not(windows))]
    0
}

/// Let the user choose a folder or file, starting in `start`. `None` when
/// cancelled (or elsewhere than on Windows). Blocks until the dialog closes.
pub fn pick(kind: PickKind, start: Option<&Path>, title: &str, owner: isize) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        win::pick(kind, start, title, owner)
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, start, title, owner);
        None
    }
}

/// The folder to start in for what was typed: the typed folder, or the
/// nearest existing folder above it.
pub fn start_folder(typed: &Path) -> Option<PathBuf> {
    typed
        .ancestors()
        .find(|p| !p.as_os_str().is_empty() && p.is_dir())
        .map(Path::to_path_buf)
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
        FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NODEREFERENCELINKS, FOS_PATHMUSTEXIST,
        FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    use super::PickKind;

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn pick(
        kind: PickKind,
        start: Option<&Path>,
        title: &str,
        owner: isize,
    ) -> Option<PathBuf> {
        // SAFETY: paired with CoUninitialize when it succeeded.
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        // SAFETY: COM is set up for this thread.
        let picked = unsafe { show(kind, start, title, owner) };
        if com {
            // SAFETY: CoInitializeEx succeeded on this thread.
            unsafe { CoUninitialize() };
        }
        picked
    }

    /// # Safety
    /// COM must be initialised on the calling thread.
    unsafe fn show(
        kind: PickKind,
        start: Option<&Path>,
        title: &str,
        owner: isize,
    ) -> Option<PathBuf> {
        // SAFETY: every interface is released on drop and the path string
        // is freed once.
        unsafe {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let mut options = dialog.GetOptions().ok()?
                | FOS_FORCEFILESYSTEM
                | FOS_PATHMUSTEXIST
                // A link is what's chosen, not where it points.
                | FOS_NODEREFERENCELINKS;
            options |= match kind {
                PickKind::Folder => FOS_PICKFOLDERS,
                PickKind::File => FOS_FILEMUSTEXIST,
            };
            dialog.SetOptions(options).ok()?;
            let title = wide(title);
            let _ = dialog.SetTitle(PCWSTR(title.as_ptr()));
            if let Some(start) = start {
                let name = wide(start);
                if let Ok(item) =
                    SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(name.as_ptr()), None)
                {
                    let _ = dialog.SetFolder(&item);
                }
            }
            let owner = (owner != 0).then_some(HWND(owner as *mut _));
            // Cancelling is an error too.
            dialog.Show(owner).ok()?;
            let item = dialog.GetResult().ok()?;
            let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let path = p.to_string().ok().map(PathBuf::from);
            CoTaskMemFree(Some(p.0 as *const _));
            path
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_the_nearest_existing_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert_eq!(start_folder(root).as_deref(), Some(root));
        let missing = root.join("no").join("such.txt");
        assert_eq!(start_folder(&missing).as_deref(), Some(root));
        assert_eq!(start_folder(Path::new("")), None);
    }
}
