//! "Open folders in Oxplorer": make Oxplorer what opens folders and drives
//! (double-click on the desktop, "Open" in other programs, Win+E), instead
//! of Explorer. Only the current user's registry (HKCU) is touched, so no
//! admin rights are needed and turning it off restores Explorer.
//!
//! What gets written (under `HKCU\Software\Classes`):
//! - `Directory\shell` and `Drive\shell`: an crate::i18n::t("Open in Oxplorer") verb that is
//!   the default one.
//! - `CLSID\{52205fd8-…}\shell\opennewwindow\command`: Win+E (the "This PC"
//!   / Home window) starts Oxplorer.

use std::io;
#[cfg(windows)]
use std::path::Path;

/// Our verb's key name under `Directory\shell` and `Drive\shell`.
pub const VERB: &str = "oxplorer";

/// The shell folder Win+E opens.
pub const WIN_E_CLSID: &str = "{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";

/// Is the Windows registry available?
pub fn supported() -> bool {
    cfg!(windows)
}

/// Whether folders currently open in Oxplorer.
pub fn is_enabled() -> bool {
    #[cfg(windows)]
    {
        imp::is_enabled()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Turn "open folders in Oxplorer" on (for the running program) or off.
pub fn set_enabled(on: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        if on {
            imp::enable(&std::env::current_exe()?)
        } else {
            imp::disable()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "only available on Windows",
        ))
    }
}

/// When folders open in Oxplorer but the program has moved since (the
/// portable zip can live anywhere), point the registration at this copy.
pub fn refresh() {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        if imp::is_enabled() && !imp::points_to(&exe) {
            let _ = imp::enable(&exe);
        }
    }
}

/// The command line that opens a folder (`%1`) in `exe`.
#[cfg_attr(not(windows), allow(dead_code))]
fn open_command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

/// Clean up a folder passed on the command line. `"C:\"` arrives as `C:"`
/// (the backslash escapes the quote), so give drives their backslash back.
pub fn folder_argument(arg: &str) -> String {
    let arg = arg.trim().trim_matches('"');
    if arg.len() == 2 && arg.ends_with(':') {
        format!("{arg}\\")
    } else {
        arg.to_owned()
    }
}

/// Set a string value under `HKCU\Software\Classes\<key>` (creating it).
#[cfg(windows)]
pub(crate) fn set_class_value(key: &str, name: &str, value: &str) -> io::Result<()> {
    imp::set(key, Some(name), value)
}

/// Delete `HKCU\Software\Classes\<key>` with everything in it.
#[cfg(windows)]
pub(crate) fn delete_class_key(key: &str) {
    let _ = imp::delete_tree(key);
}

#[cfg(windows)]
mod imp {
    use super::{open_command, Path, VERB, WIN_E_CLSID};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteKeyW, RegDeleteTreeW,
        RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE,
        REG_SZ, RRF_RT_REG_SZ,
    };
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

    const CLASSES: &str = r"Software\Classes";

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }

    fn check(err: WIN32_ERROR) -> io::Result<()> {
        if err.is_ok() {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(err.0 as i32))
        }
    }

    /// Set a string value (`None` name = the key's default value), creating
    /// the key if needed.
    pub(super) fn set(key: &str, name: Option<&str>, value: &str) -> io::Result<()> {
        let path = wide(format!(r"{CLASSES}\{key}"));
        let mut hkey = HKEY::default();
        // SAFETY: buffers outlive the calls; the key is closed below.
        unsafe {
            check(RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut hkey,
                None,
            ))?;
            let name = name.map(wide);
            let data = wide(value);
            let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
            let result = check(RegSetValueExW(
                hkey,
                name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
                None,
                REG_SZ,
                Some(bytes),
            ));
            let _ = RegCloseKey(hkey);
            result
        }
    }

    /// A string value, `None` if missing.
    fn get(key: &str, name: Option<&str>) -> Option<String> {
        let path = wide(format!(r"{CLASSES}\{key}"));
        let name = name.map(wide);
        let mut buf = [0u16; 1024];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: `buf` is `size` bytes.
        let err = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if !err.is_ok() {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
    }

    /// Delete `key` with everything in it.
    pub(super) fn delete_tree(key: &str) -> io::Result<()> {
        let path = wide(format!(r"{CLASSES}\{key}"));
        // SAFETY: plain registry call.
        let err = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) };
        if err == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(err)
    }

    /// Delete `key` if nothing is left in it (fails quietly otherwise).
    fn delete_if_empty(key: &str) {
        let path = wide(format!(r"{CLASSES}\{key}"));
        // SAFETY: plain registry call; refuses keys that still have subkeys.
        let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) };
    }

    fn delete_value(key: &str, name: Option<&str>) {
        let path = wide(format!(r"{CLASSES}\{key}"));
        let name = name.map(wide);
        // SAFETY: plain registry call; a missing value is fine.
        let _ = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            )
        };
    }

    fn notify() {
        // SAFETY: tells Explorer that file associations changed.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }

    fn win_e_key() -> String {
        format!(r"CLSID\{WIN_E_CLSID}\shell\opennewwindow\command")
    }

    pub(super) fn is_enabled() -> bool {
        get(r"Directory\shell", None).as_deref() == Some(VERB)
    }

    pub(super) fn points_to(exe: &Path) -> bool {
        let command = open_command(&exe.display().to_string());
        get(&format!(r"Directory\shell\{VERB}\command"), None).as_deref() == Some(&command)
    }

    pub(super) fn enable(exe: &Path) -> io::Result<()> {
        let exe = exe.display().to_string();
        let command = open_command(&exe);
        for class in ["Directory", "Drive"] {
            let verb = format!(r"{class}\shell\{VERB}");
            set(&verb, None, crate::i18n::t("Open in Oxplorer"))?;
            set(&verb, Some("Icon"), &format!("\"{exe}\",0"))?;
            set(&format!(r"{verb}\command"), None, &command)?;
            set(&format!(r"{class}\shell"), None, VERB)?;
        }
        set(&win_e_key(), None, &format!("\"{exe}\""))?;
        // An empty DelegateExecute makes Windows run the command above.
        set(&win_e_key(), Some("DelegateExecute"), "")?;
        notify();
        Ok(())
    }

    pub(super) fn disable() -> io::Result<()> {
        for class in ["Directory", "Drive"] {
            let shell = format!(r"{class}\shell");
            if get(&shell, None).as_deref() == Some(VERB) {
                delete_value(&shell, None);
            }
            delete_tree(&format!(r"{shell}\{VERB}"))?;
        }
        delete_tree(&format!(r"CLSID\{WIN_E_CLSID}\shell\opennewwindow"))?;
        delete_if_empty(&format!(r"CLSID\{WIN_E_CLSID}\shell"));
        delete_if_empty(&format!(r"CLSID\{WIN_E_CLSID}"));
        notify();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_and_arguments() {
        assert_eq!(
            open_command(r"C:\Apps\oxplorer.exe"),
            r#""C:\Apps\oxplorer.exe" "%1""#
        );
        assert_eq!(folder_argument(r#"C:""#), r"C:\");
        assert_eq!(folder_argument(r"D:"), r"D:\");
        assert_eq!(folder_argument(r#""C:\Users\me""#), r"C:\Users\me");
        assert_eq!(folder_argument(r"\\nas\media"), r"\\nas\media");
    }
}
