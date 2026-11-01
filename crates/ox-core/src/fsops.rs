//! Small, synchronous file operations (create, rename, delete, shell helpers).
//!
//! Bulk copying/moving lives in [`crate::transfer`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Return `dir/name`, or `dir/stem (2).ext`, `dir/stem (3).ext`, … if it already exists.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !exists(&candidate) {
        return candidate;
    }
    let (stem, ext) = split_name(name);
    for i in 2.. {
        let candidate = dir.join(format!("{stem} ({i}){ext}"));
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Return a name for a copy placed next to the original, like Explorer's `name - Copy.txt`.
pub fn copy_name_path(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = split_name(name);
    let first = dir.join(format!("{stem} - Copy{ext}"));
    if !exists(&first) {
        return first;
    }
    for i in 2.. {
        let candidate = dir.join(format!("{stem} - Copy ({i}){ext}"));
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Split `report.pdf` into (`report`, `.pdf`). Dotfiles keep their full name as stem.
pub fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(i) => (&name[..i], &name[i..]),
    }
}

/// Like `Path::exists`, but also true for broken symlinks.
pub fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Are both paths on the same volume? Used to decide whether a drag moves or copies.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::Component;
        let prefix = |p: &Path| match p.components().next() {
            Some(Component::Prefix(pre)) => Some(pre.as_os_str().to_string_lossy().to_lowercase()),
            _ => None,
        };
        prefix(a).is_some() && prefix(a) == prefix(b)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::symlink_metadata(a), fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev(),
            _ => false,
        }
    }
}

/// Create a new folder with a unique name and return its path.
pub fn create_folder(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let path = unique_path(dir, name);
    fs::create_dir(&path)?;
    Ok(path)
}

/// Create a new empty file with a unique name and return its path.
pub fn create_file(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let path = unique_path(dir, name);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    Ok(path)
}

/// Create a new file with `content` in `dir`, numbering the name if it is
/// taken (like [`create_file`]).
pub fn create_file_with(dir: &Path, name: &str, content: &str) -> io::Result<PathBuf> {
    use std::io::Write;
    let path = unique_path(dir, name);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(content.as_bytes())?;
    Ok(path)
}

/// Characters Windows does not allow in file names.
pub const INVALID_NAME_CHARS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Check a new file name, returning a user facing error message if it is invalid.
pub fn validate_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(crate::i18n::t("The name can't be empty.").into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| INVALID_NAME_CHARS.contains(c) || c.is_control())
    {
        return Err(crate::trf!(
            "A file name can't contain the character {c}.",
            c = format!("{c:?}")
        ));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(crate::i18n::t("A file name can't end with a dot or a space.").into());
    }
    let upper = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&upper.as_str()) {
        return Err(format!("\"{trimmed}\" is a reserved name on Windows."));
    }
    Ok(())
}

/// Rename `path` to `new_name` in the same folder. Returns the new path.
pub fn rename(path: &Path, new_name: &str) -> io::Result<PathBuf> {
    validate_name(new_name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let parent = path.parent().unwrap_or(Path::new(""));
    let target = parent.join(new_name);
    // Allow case-only renames ("readme" -> "README"), which look like a collision.
    let same_file = target
        .to_string_lossy()
        .eq_ignore_ascii_case(&path.to_string_lossy());
    if exists(&target) && !same_file {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            crate::trf!("\"{new_name}\" already exists.", new_name = new_name),
        ));
    }
    fs::rename(path, &target)?;
    Ok(target)
}

/// Delete items, either to the Recycle Bin or permanently.
pub fn delete(paths: &[PathBuf], to_trash: bool) -> Result<(), String> {
    let failed = delete_each(paths, to_trash);
    if failed.is_empty() {
        Ok(())
    } else {
        let lines: Vec<String> = failed
            .iter()
            .map(|(p, e)| format!("{}: {e}", p.display()))
            .collect();
        Err(lines.join("\n"))
    }
}

/// Delete items; returns those that couldn't be deleted, each with why.
pub fn delete_each(paths: &[PathBuf], to_trash: bool) -> Vec<(PathBuf, String)> {
    if to_trash {
        let Err(all) = trash::delete_all(paths) else {
            return Vec::new();
        };
        // Find out which of them failed, and why.
        let mut failed: Vec<(PathBuf, String)> = paths
            .iter()
            .filter(|p| fs::symlink_metadata(p).is_ok())
            .filter_map(|p| trash::delete(p).err().map(|e| (p.clone(), e.to_string())))
            .collect();
        if failed.is_empty() && paths.iter().any(|p| fs::symlink_metadata(p).is_ok()) {
            failed.push((paths[0].clone(), all.to_string()));
        }
        return failed;
    }
    let mut failed = Vec::new();
    for p in paths {
        let result = match fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(p),
            Ok(_) => remove_file_forced(p),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            failed.push((p.clone(), e.to_string()));
        }
    }
    failed
}

/// Remove a file, clearing the read-only flag first if needed.
pub fn remove_file_forced(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            clear_readonly(path)?;
            fs::remove_file(path)
        }
        other => other,
    }
}

pub fn clear_readonly(path: &Path) -> io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    if perms.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

/// Total size and item count of a folder tree (used by the properties dialog).
pub fn folder_size(path: &Path, cancel: &std::sync::atomic::AtomicBool) -> (u64, u64, u64) {
    use std::sync::atomic::Ordering;
    let (mut bytes, mut files, mut dirs) = (0u64, 0u64, 0u64);
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for item in rd.flatten() {
            let Ok(ft) = item.file_type() else { continue };
            if ft.is_dir() {
                dirs += 1;
                stack.push(item.path());
            } else if let Ok(m) = item.metadata() {
                files += 1;
                bytes += m.len();
            }
        }
    }
    (bytes, files, dirs)
}

/// Open a file or folder with its default application.
pub fn open_default(path: &Path) -> io::Result<()> {
    open::that_detached(path)
}

/// Highlight the item in the system file manager (Windows Explorer on Windows).
pub fn reveal_in_system(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path.as_os_str());
        Command::new("explorer").arg(arg).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        open_default(path.parent().unwrap_or(path))
    }
}

/// Open a terminal in `dir` (Windows Terminal if installed, otherwise cmd).
pub fn open_terminal(dir: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        Command::new("wt")
            .arg("-d")
            .arg(dir)
            .spawn()
            .or_else(|_| {
                Command::new("cmd")
                    .args(["/C", "start", "cmd", "/K", "cd", "/d"])
                    .arg(dir)
                    .creation_flags_no_window()
                    .spawn()
            })
            .map(|_| ())
    }
    #[cfg(not(windows))]
    {
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
            if Command::new(term).current_dir(dir).spawn().is_ok() {
                return Ok(());
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, "no terminal found"))
    }
}

/// The command line that opens `path` in the editor: `{path}` in `template`
/// is replaced with it (and it is added in quotes if `{path}` is missing).
pub fn editor_command(template: &str, path: &Path) -> String {
    let path = path.to_string_lossy();
    if template.contains("{path}") {
        template.replace("{path}", &path)
    } else {
        format!("{} \"{path}\"", template.trim())
    }
}

/// Run the editor command for `path` (through the shell, so `code` finds
/// `code.cmd` on Windows). The process is returned so the caller can check
/// whether it failed.
pub fn open_in_editor(template: &str, path: &Path) -> io::Result<std::process::Child> {
    let line = editor_command(template, path);
    let dir = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        // /S: the line in quotes is taken as is.
        c.raw_arg(format!("/S /C \"{line}\""))
            .creation_flags_no_window();
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&line);
        c
    };
    if let Some(dir) = dir.filter(|d| d.is_dir()) {
        cmd.current_dir(dir);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
}

/// `path` with Windows short names (8.3, like `FD8DC~1.LAE`) expanded to
/// the real ones, so it can be compared with paths read from the disk. File
/// watchers report the form a program used, and `%TEMP%` is often written
/// with short names. Parts that don't exist (any more) stay as they are.
pub fn long_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if path.as_os_str().to_string_lossy().contains('~') {
        // Expand the deepest part that exists; the rest is appended.
        for existing in path.ancestors() {
            if let Some(long) = expand_short_names(existing) {
                return match path.strip_prefix(existing) {
                    Ok(rest) if !rest.as_os_str().is_empty() => long.join(rest),
                    _ => long,
                };
            }
        }
    }
    path.to_path_buf()
}

#[cfg(windows)]
fn expand_short_names(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
    if path.as_os_str().is_empty() {
        return None;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = vec![0u16; 512];
    loop {
        // SAFETY: `wide` is NUL-terminated and `buf` is as long as we say.
        let n = unsafe { GetLongPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        let n = n as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            buf.truncate(n);
            return Some(PathBuf::from(std::ffi::OsString::from_wide(&buf)));
        }
        buf.resize(n, 0); // too small: `n` is the size needed
    }
}

#[cfg(windows)]
trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl NoWindow for Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_and_copy_names() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "").unwrap();
        assert_eq!(unique_path(dir.path(), "b.txt"), dir.path().join("b.txt"));
        assert_eq!(
            unique_path(dir.path(), "a.txt"),
            dir.path().join("a (2).txt")
        );
        assert_eq!(
            copy_name_path(dir.path(), "a.txt"),
            dir.path().join("a - Copy.txt")
        );
        fs::write(dir.path().join("a - Copy.txt"), "").unwrap();
        assert_eq!(
            copy_name_path(dir.path(), "a.txt"),
            dir.path().join("a - Copy (2).txt")
        );
        assert_eq!(split_name(".gitignore"), (".gitignore", ""));
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("ok name.txt").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a:b").is_err());
        assert!(validate_name("trailing.").is_err());
        assert!(validate_name("con.txt").is_err());
    }

    #[test]
    fn create_rename_delete() {
        let dir = tempfile::tempdir().unwrap();
        let folder = create_folder(dir.path(), "New folder").unwrap();
        let folder2 = create_folder(dir.path(), "New folder").unwrap();
        assert_eq!(folder2, dir.path().join("New folder (2)"));
        let file = create_file(&folder, "New file.txt").unwrap();
        let renamed = rename(&file, "notes.txt").unwrap();
        assert!(renamed.exists() && !file.exists());
        assert!(rename(&folder2, "New folder").is_err());
        delete(&[folder.clone(), folder2.clone()], false).unwrap();
        assert!(!folder.exists() && !folder2.exists());
    }

    #[test]
    fn editor_command_lines() {
        let p = Path::new("/home/me/My Project");
        assert_eq!(
            editor_command("code \"{path}\"", p),
            "code \"/home/me/My Project\""
        );
        assert_eq!(editor_command("subl ", p), "subl \"/home/me/My Project\"");
        #[cfg(not(windows))]
        {
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("out.txt");
            let template = format!("echo {{path}} > '{}'", out.display());
            let status = open_in_editor(&template, dir.path())
                .unwrap()
                .wait()
                .unwrap();
            assert!(status.success());
            let written = std::fs::read_to_string(&out).unwrap();
            assert_eq!(written.trim(), dir.path().to_string_lossy());
        }
    }

    #[test]
    fn new_files_from_templates() {
        let dir = tempfile::tempdir().unwrap();
        let item = crate::settings::NewItem {
            label: "Python script".into(),
            file_name: "tool.py".into(),
            content: "# {name}\n".into(),
        };
        let first =
            create_file_with(dir.path(), &item.file_name, &item.content_for("tool.py")).unwrap();
        assert_eq!(fs::read_to_string(&first).unwrap(), "# tool\n");
        // A taken name gets a number, the content is written all the same.
        let second = create_file_with(dir.path(), "tool.py", "x").unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(&second).unwrap(), "x");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let dated = crate::settings::NewItem {
            content: "{date}".into(),
            ..Default::default()
        };
        assert_eq!(dated.content_for("a.txt"), today);
    }

    #[cfg(windows)]
    #[test]
    fn short_names_are_expanded() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        let dir = tempfile::tempdir().unwrap();
        let long = dir.path().join("A folder with a long name");
        fs::create_dir(&long).unwrap();
        // The temp folder itself may be written with short names too.
        let expected = expand_short_names(&long).unwrap();
        assert!(!expected.to_string_lossy().contains('~'));
        assert_eq!(long_path(&long), expected);

        let wide: Vec<u16> = long.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut buf = vec![0u16; 1024];
        // SAFETY: `wide` is NUL-terminated and `buf` is as long as we say.
        let n = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), 1024) } as usize;
        let short = PathBuf::from(std::ffi::OsString::from_wide(&buf[..n]));
        if n == 0 || !short.to_string_lossy().contains('~') {
            return; // no short names on this drive
        }
        assert_eq!(long_path(&short), expected);
        // Gone (deleted, or the old name of a rename): the rest is kept.
        assert_eq!(
            long_path(&short.join("gone.txt")),
            expected.join("gone.txt")
        );
        assert_eq!(
            long_path(Path::new(r"C:\plain")),
            PathBuf::from(r"C:\plain")
        );
    }
}
