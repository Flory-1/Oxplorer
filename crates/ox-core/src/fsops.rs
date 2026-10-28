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
    if to_trash {
        return trash::delete_all(paths).map_err(|e| e.to_string());
    }
    let mut errors = Vec::new();
    for p in paths {
        let result = match fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(p),
            Ok(_) => remove_file_forced(p),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            errors.push(format!("{}: {e}", p.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
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
}
