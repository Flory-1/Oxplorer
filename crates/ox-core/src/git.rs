//! Git status for folders inside repositories (the Git column).
//!
//! Uses the `git` command line (`git status --porcelain -z`), so nothing is
//! shown when Git isn't installed. On Windows it runs without a console
//! window.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// State of one file or folder, most important first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitState {
    Conflicted,
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Ignored,
}

impl GitState {
    /// One-letter badge for the Git column.
    pub fn letter(self) -> &'static str {
        match self {
            GitState::Conflicted => "!",
            GitState::Modified => "M",
            GitState::Added => "A",
            GitState::Deleted => "D",
            GitState::Renamed => "R",
            GitState::Untracked => "?",
            GitState::Ignored => "I",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GitState::Conflicted => "Conflict",
            GitState::Modified => "Modified",
            GitState::Added => "Added",
            GitState::Deleted => "Deleted",
            GitState::Renamed => "Renamed",
            GitState::Untracked => "Untracked",
            GitState::Ignored => "Ignored",
        }
    }

    fn from_xy(x: u8, y: u8) -> Option<Self> {
        Some(match (x, y) {
            (b'?', b'?') => GitState::Untracked,
            (b'!', b'!') => GitState::Ignored,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => GitState::Conflicted,
            (b'R', _) | (b'C', _) => GitState::Renamed,
            (b'A', _) => GitState::Added,
            (b'D', _) | (_, b'D') => GitState::Deleted,
            (b'M', _) | (_, b'M') | (b'T', _) | (_, b'T') => GitState::Modified,
            _ => return None,
        })
    }
}

/// Status of a whole repository.
#[derive(Debug, Clone, Default)]
pub struct RepoStatus {
    pub root: PathBuf,
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Changed files, relative to the root.
    files: HashMap<PathBuf, GitState>,
    /// Folders reported as a whole (untracked or ignored), relative.
    whole_dirs: HashMap<PathBuf, GitState>,
    /// Most important state of anything inside each folder, relative.
    summary: HashMap<PathBuf, GitState>,
}

impl RepoStatus {
    /// State of `path` (absolute). Folders get the most important state of
    /// what's inside them; files and folders inside an untracked or ignored
    /// folder share its state.
    pub fn state(&self, path: &Path) -> Option<GitState> {
        let rel = path.strip_prefix(&self.root).ok()?;
        if let Some(s) = self.files.get(rel).or_else(|| self.summary.get(rel)) {
            return Some(*s);
        }
        rel.ancestors()
            .find_map(|a| self.whole_dirs.get(a))
            .copied()
    }

    /// e.g. "main ↑2 ↓1".
    pub fn branch_label(&self) -> Option<String> {
        let mut s = self.branch.clone()?;
        if self.ahead > 0 {
            s.push_str(&format!(" ↑{}", self.ahead));
        }
        if self.behind > 0 {
            s.push_str(&format!(" ↓{}", self.behind));
        }
        Some(s)
    }

    pub fn changed_files(&self) -> usize {
        self.files
            .values()
            .filter(|s| **s != GitState::Ignored)
            .count()
    }

    fn add(&mut self, path: &str, state: GitState) {
        let is_dir = path.ends_with('/');
        // Git always uses `/`; build the path from its parts so it matches
        // native paths on Windows too.
        let rel: PathBuf = path.split('/').filter(|p| !p.is_empty()).collect();
        if is_dir {
            self.whole_dirs.insert(rel.clone(), state);
        } else {
            self.files.insert(rel.clone(), state);
        }
        // Ignored items don't make their folders look changed.
        if state == GitState::Ignored {
            return;
        }
        for dir in rel.ancestors().skip(1) {
            let e = self.summary.entry(dir.to_path_buf()).or_insert(state);
            *e = (*e).min(state);
        }
    }
}

/// The repository `dir` belongs to (the folder holding `.git`), if any.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|a| a.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Parse `git status --porcelain=v1 -z -b` output.
pub fn parse(root: &Path, out: &[u8]) -> RepoStatus {
    let mut status = RepoStatus {
        root: root.to_path_buf(),
        ..Default::default()
    };
    let mut fields = out.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(f) = fields.next() {
        let line = String::from_utf8_lossy(f);
        if let Some(head) = line.strip_prefix("## ") {
            parse_branch(&mut status, head);
            continue;
        }
        if f.len() < 4 {
            continue;
        }
        let (x, y) = (f[0], f[1]);
        let path = String::from_utf8_lossy(&f[3..]).into_owned();
        if x == b'R' || x == b'C' {
            fields.next(); // the old name
        }
        if let Some(state) = GitState::from_xy(x, y) {
            status.add(&path, state);
        }
    }
    status
}

/// "main...origin/main [ahead 2, behind 1]", "No commits yet on main",
/// "HEAD (no branch)".
fn parse_branch(status: &mut RepoStatus, head: &str) {
    let (name, rest) = match head.split_once(" [") {
        Some((n, r)) => (n, r.trim_end_matches(']')),
        None => (head, ""),
    };
    let name = name
        .strip_prefix("No commits yet on ")
        .or_else(|| name.strip_prefix("Initial commit on "))
        .unwrap_or(name);
    let name = name.split("...").next().unwrap_or(name);
    status.branch = Some(if name.starts_with("HEAD") {
        "detached HEAD".into()
    } else {
        name.to_owned()
    });
    for part in rest.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            status.ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            status.behind = n.parse().unwrap_or(0);
        }
    }
}

/// Run `git status` for the repository at `root`.
pub fn status(root: &Path) -> io::Result<RepoStatus> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "-b",
            "--untracked-files=normal",
            "--ignored=matching",
        ])
        .env("GIT_OPTIONAL_LOCKS", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(parse(root, &out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_output() {
        let root = Path::new("/repo");
        let out = b"## main...origin/main [ahead 2, behind 1]\0 M src/lib.rs\0A  src/new.rs\0R  docs/b.md\0docs/a.md\0?? notes/\0!! target/\0UU src/conflict.rs\0 D old.txt\0";
        let s = parse(root, out);
        assert_eq!(s.branch_label().as_deref(), Some("main ↑2 ↓1"));
        let st = |p: &str| s.state(&root.join(p));
        assert_eq!(st("src/lib.rs"), Some(GitState::Modified));
        assert_eq!(st("src/new.rs"), Some(GitState::Added));
        assert_eq!(st("docs/b.md"), Some(GitState::Renamed));
        assert_eq!(st("docs/a.md"), None);
        assert_eq!(st("old.txt"), Some(GitState::Deleted));
        // Folders show the most important state inside them.
        assert_eq!(st("src"), Some(GitState::Conflicted));
        assert_eq!(st("docs"), Some(GitState::Renamed));
        // Whole untracked / ignored folders and what's inside them.
        assert_eq!(st("notes"), Some(GitState::Untracked));
        assert_eq!(st("notes/deep/x.txt"), Some(GitState::Untracked));
        assert_eq!(st("target/debug"), Some(GitState::Ignored));
        assert_eq!(st("README.md"), None);
        assert_eq!(s.changed_files(), 5);
        assert!(s.state(Path::new("/elsewhere/x")).is_none());
    }

    #[test]
    fn parses_branch_headers() {
        let s = parse(Path::new("/r"), b"## No commits yet on main\0");
        assert_eq!(s.branch_label().as_deref(), Some("main"));
        let s = parse(Path::new("/r"), b"## HEAD (no branch)\0");
        assert_eq!(s.branch.as_deref(), Some("detached HEAD"));
    }

    #[test]
    fn runs_git_in_a_real_repository() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !git(&["init", "-q", "-b", "main"]) {
            eprintln!("git is not installed; skipping");
            return;
        }
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.txt"), "a").unwrap();
        std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join("build/out.bin"), "x").unwrap();
        assert!(git(&["add", "src/a.txt"]));

        assert_eq!(repo_root(&root.join("src")).as_deref(), Some(root));
        let s = status(root).unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.state(&root.join("src/a.txt")), Some(GitState::Added));
        assert_eq!(s.state(&root.join("src")), Some(GitState::Added));
        assert_eq!(s.state(&root.join(".gitignore")), Some(GitState::Untracked));
        assert_eq!(s.state(&root.join("build")), Some(GitState::Ignored));
    }
}
