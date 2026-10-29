//! "Copy path as": paths in the forms developers paste into code, shells,
//! browsers and documentation.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFormat {
    /// `C:/Users/me/file.txt`
    ForwardSlashes,
    /// `"C:\Users\me\file.txt"`
    Quoted,
    /// `src/main.rs`, from the root of the Git repository.
    GitRelative,
    /// `file:///C:/Users/me/file.txt`
    FileUrl,
    /// `file.txt`
    Name,
    /// `/home/me/file.txt` for `\\wsl.localhost\Ubuntu\home\me\file.txt`
    /// (`/mnt/c/…` for `C:\…`), see [`crate::wsl::to_linux`].
    Linux,
}

impl PathFormat {
    pub const ALL: [PathFormat; 6] = [
        PathFormat::ForwardSlashes,
        PathFormat::Quoted,
        PathFormat::GitRelative,
        PathFormat::FileUrl,
        PathFormat::Name,
        PathFormat::Linux,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PathFormat::ForwardSlashes => crate::i18n::t("With forward slashes"),
            PathFormat::Quoted => crate::i18n::t("In quotes"),
            PathFormat::GitRelative => crate::i18n::t("Relative to the Git repository"),
            PathFormat::FileUrl => crate::i18n::t("As file:// URL"),
            PathFormat::Name => crate::i18n::t("Name only"),
            PathFormat::Linux => crate::i18n::t("As Linux path"),
        }
    }
}

/// `path` in format `fmt`. `GitRelative` needs the repository's root (the
/// full path is used outside one).
pub fn format(path: &Path, fmt: PathFormat, git_root: Option<&Path>) -> String {
    let full = path.to_string_lossy();
    match fmt {
        PathFormat::ForwardSlashes => full.replace('\\', "/"),
        PathFormat::Quoted => format!("\"{full}\""),
        PathFormat::GitRelative => match git_root.and_then(|r| path.strip_prefix(r).ok()) {
            Some(rel) if rel.as_os_str().is_empty() => ".".into(),
            Some(rel) => rel.to_string_lossy().replace('\\', "/"),
            None => full.into_owned(),
        },
        PathFormat::FileUrl => file_url(&full),
        PathFormat::Name => path
            .file_name()
            .map_or_else(|| full.clone().into_owned(), |n| n.to_string_lossy().into()),
        PathFormat::Linux => crate::wsl::to_linux(path).unwrap_or_else(|| full.into_owned()),
    }
}

/// `C:\a b\c.txt` → `file:///C:/a%20b/c.txt`, `\\server\share\x` →
/// `file://server/share/x`, `/home/me` → `file:///home/me`.
pub fn file_url(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    let (prefix, rest) = if let Some(unc) = slashed.strip_prefix("//") {
        ("file://", unc.to_owned())
    } else if slashed.starts_with('/') {
        ("file://", slashed)
    } else {
        ("file:///", slashed)
    };
    let mut out = String::from(prefix);
    for b in rest.bytes() {
        if b.is_ascii_alphanumeric() || b"/:-._~!$&'()*+,;=@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn formats() {
        let p = PathBuf::from("/home/me/My Project/src/main.rs");
        let root = PathBuf::from("/home/me/My Project");
        let f = |fmt| format(&p, fmt, Some(&root));
        assert_eq!(f(PathFormat::Quoted), "\"/home/me/My Project/src/main.rs\"");
        assert_eq!(f(PathFormat::GitRelative), "src/main.rs");
        assert_eq!(f(PathFormat::Name), "main.rs");
        assert_eq!(
            f(PathFormat::FileUrl),
            "file:///home/me/My%20Project/src/main.rs"
        );
        assert_eq!(format(&root, PathFormat::GitRelative, Some(&root)), ".");
        // Outside the repository: the full path.
        assert_eq!(
            format(Path::new("/tmp/x"), PathFormat::GitRelative, Some(&root)),
            "/tmp/x"
        );
    }

    #[test]
    fn windows_paths_as_urls() {
        assert_eq!(
            format(
                Path::new(r"C:\Users\me\file.txt"),
                PathFormat::ForwardSlashes,
                None
            ),
            "C:/Users/me/file.txt"
        );
        assert_eq!(
            file_url(r"C:\a b\Ä#1.txt"),
            "file:///C:/a%20b/%C3%84%231.txt"
        );
        assert_eq!(file_url(r"\\nas\share\x"), "file://nas/share/x");
    }

    #[test]
    fn linux_paths() {
        let f = |p: &str| format(Path::new(p), PathFormat::Linux, None);
        assert_eq!(f(r"\\wsl.localhost\Ubuntu\home\me\a.txt"), "/home/me/a.txt");
        assert_eq!(f(r"C:\Users\me"), "/mnt/c/Users/me");
        // No Linux form: the path as it is.
        assert_eq!(f(r"\\nas\share"), r"\\nas\share");
    }
}
