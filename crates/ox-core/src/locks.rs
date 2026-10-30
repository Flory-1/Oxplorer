//! Why an item can't be deleted: which programs have it (or files inside it)
//! open, ending those programs, and deleting as administrator.

use std::path::PathBuf;

/// A program that has a file open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locker {
    pub pid: u32,
    /// The program's name as Windows shows it (e.g. "Microsoft Word").
    pub name: String,
    /// Full path of its executable, when it can be read.
    pub exe: Option<PathBuf>,
}

/// Files of a folder asked about at most: enough to find the program that
/// holds a folder, without walking a huge tree.
const MAX_FILES: usize = 5000;

/// The files to ask about for `paths`: files themselves, and the files inside
/// folders (up to [`MAX_FILES`]).
pub fn files_to_check(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = Vec::new();
    for p in paths {
        match std::fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => stack.push(p.clone()),
            Ok(_) => files.push(p.clone()),
            Err(_) => {}
        }
    }
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in rd.flatten() {
            if files.len() >= MAX_FILES {
                return files;
            }
            match item.file_type() {
                Ok(t) if t.is_dir() => stack.push(item.path()),
                Ok(_) => files.push(item.path()),
                Err(_) => {}
            }
        }
    }
    files
}

/// The programs that have `paths` (or anything inside them) open.
pub fn lockers(paths: &[PathBuf]) -> Vec<Locker> {
    let files = files_to_check(paths);
    if files.is_empty() {
        return Vec::new();
    }
    #[cfg(windows)]
    {
        // SAFETY: only reads the Restart Manager's answer about the files.
        unsafe { imp::lockers(&files) }
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// End the program with this process id.
pub fn kill(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        // SAFETY: opens the process with terminate rights only and closes it.
        unsafe { imp::kill(pid) }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Err("not supported".into())
    }
}

/// Does the error say access was denied (rather than the item being in use)?
pub fn is_access_denied(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    e.contains("os error 5)") || e.contains("access is denied") || e.contains("0x80070005")
}

/// The command that deletes `paths` permanently (run elevated by
/// [`delete_as_admin`]).
pub fn admin_delete_command(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| {
            if p.is_dir() {
                format!("rd /s /q \"{}\"", p.display())
            } else {
                format!("del /f /q /a \"{}\"", p.display())
            }
        })
        .collect::<Vec<_>>()
        .join(" & ")
}

/// Delete `paths` permanently with administrator rights (Windows asks for
/// them). Waits until done; fails if anything is left.
pub fn delete_as_admin(paths: &[PathBuf]) -> Result<(), String> {
    #[cfg(windows)]
    {
        let args = format!("/c {}", admin_delete_command(paths));
        // SAFETY: starts cmd.exe elevated and waits for it.
        unsafe { imp::run_elevated("cmd.exe", &args)? };
        let left: Vec<String> = paths
            .iter()
            .filter(|p| std::fs::symlink_metadata(p).is_ok())
            .map(|p| p.display().to_string())
            .collect();
        if left.is_empty() {
            Ok(())
        } else {
            Err(crate::trf!(
                "Still there after deleting as administrator: {v1}",
                v1 = left.join(", ")
            ))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        Err("not supported".into())
    }
}

#[cfg(windows)]
mod imp {
    use super::Locker;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::RestartManager::{
        RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
        RM_PROCESS_INFO,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE,
    };

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }

    fn from_wide(s: &[u16]) -> String {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        String::from_utf16_lossy(&s[..end])
    }

    pub(super) unsafe fn lockers(files: &[PathBuf]) -> Vec<Locker> {
        unsafe {
            let mut session = 0u32;
            let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
            if RmStartSession(&mut session, 0, key.as_mut_ptr()) != ERROR_SUCCESS {
                return Vec::new();
            }
            let names: Vec<Vec<u16>> = files.iter().map(wide).collect();
            let ptrs: Vec<*const u16> = names.iter().map(|n| n.as_ptr()).collect();
            let mut found = Vec::new();
            let registered = RmRegisterResources(
                session,
                ptrs.len() as u32,
                ptrs.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            );
            if registered == ERROR_SUCCESS {
                let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
                // The list can grow between the two calls; ask until it fits.
                for _ in 0..3 {
                    let (mut needed, mut count, mut reasons) = (0u32, infos.len() as u32, 0u32);
                    let r = RmGetList(
                        session,
                        &mut needed,
                        &mut count,
                        infos.as_mut_ptr(),
                        &mut reasons,
                    );
                    if r == ERROR_MORE_DATA {
                        infos = vec![RM_PROCESS_INFO::default(); needed as usize];
                        continue;
                    }
                    if r == ERROR_SUCCESS {
                        infos.truncate(count as usize);
                        found = infos
                            .iter()
                            .map(|i| {
                                let pid = i.Process.dwProcessId;
                                Locker {
                                    pid,
                                    name: from_wide(&i.strAppName),
                                    exe: exe_path(pid),
                                }
                            })
                            .collect();
                    }
                    break;
                }
            }
            RmEndSession(session);
            found.sort_by_key(|l| l.pid);
            found.dedup_by_key(|l| l.pid);
            found
        }
    }

    unsafe fn exe_path(pid: u32) -> Option<PathBuf> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
        }
    }

    pub(super) unsafe fn kill(pid: u32) -> Result<(), String> {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let ok = TerminateProcess(h, 1);
            let err = std::io::Error::last_os_error();
            if ok != 0 {
                // Its files are only let go once it has really ended.
                WaitForSingleObject(h, 3000);
            }
            CloseHandle(h);
            if ok != 0 {
                Ok(())
            } else {
                Err(err.to_string())
            }
        }
    }

    pub(super) unsafe fn run_elevated(exe: &str, args: &str) -> Result<(), String> {
        use windows::core::{w, PCWSTR};
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
        use windows::Win32::UI::Shell::{
            ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
        };
        use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
        unsafe {
            let file = wide(exe);
            let params = wide(args);
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
                lpVerb: w!("runas"),
                lpFile: PCWSTR(file.as_ptr()),
                lpParameters: PCWSTR(params.as_ptr()),
                nShow: SW_HIDE.0,
                ..Default::default()
            };
            ShellExecuteExW(&mut info).map_err(|e| e.message())?;
            if !info.hProcess.is_invalid() {
                WaitForSingleObject(info.hProcess, INFINITE);
                let _ = CloseHandle(info.hProcess);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_inside_folders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("sub/b.txt"), "b").unwrap();
        let mut files = files_to_check(&[dir.path().to_path_buf()]);
        files.sort();
        assert_eq!(
            files,
            vec![dir.path().join("a.txt"), dir.path().join("sub/b.txt")]
        );
        assert!(files_to_check(&[dir.path().join("missing")]).is_empty());
    }

    #[test]
    fn tells_access_denied_apart() {
        assert!(is_access_denied("Access is denied. (os error 5)"));
        assert!(!is_access_denied(
            "The process cannot access the file because it is being used by another process. (os error 32)"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn finds_who_holds_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("held.txt");
        let _open = std::fs::File::create(&path).unwrap();
        let found = lockers(std::slice::from_ref(&path));
        assert!(found.iter().any(|l| l.pid == std::process::id()));
    }
}
