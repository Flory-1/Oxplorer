//! The changed files of a repository for the Git tab: what is staged, not
//! staged, untracked or in conflict (`git status --porcelain=v2`).

use std::io;
use std::path::{Path, PathBuf};

use super::GitState;

/// The checked out branch and how it relates to its upstream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Head {
    /// `None` when no branch is checked out (detached HEAD).
    pub branch: Option<String>,
    /// The commit checked out; `None` before the first commit.
    pub commit: Option<String>,
    /// e.g. "origin/main".
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

/// The lists of the Changes view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Conflicts,
    Staged,
    Unstaged,
    Untracked,
}

impl Group {
    pub const ALL: [Group; 4] = [
        Group::Conflicts,
        Group::Staged,
        Group::Unstaged,
        Group::Untracked,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::Conflicts => "Conflicts",
            Group::Staged => "Staged",
            Group::Unstaged => "Not staged",
            Group::Untracked => "Untracked",
        }
    }
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Relative to the repository, with `/`.
    pub path: String,
    /// The name it had before a rename or copy.
    pub orig: Option<String>,
    /// Git's state letters: in the index (`x`) and the working tree (`y`);
    /// `.` is unchanged.
    pub x: char,
    pub y: char,
    pub untracked: bool,
    pub conflict: bool,
}

impl Change {
    /// The list(s) the file is in: a file can be staged and changed again.
    pub fn in_group(&self, g: Group) -> bool {
        let tracked = !self.untracked && !self.conflict;
        match g {
            Group::Conflicts => self.conflict,
            Group::Untracked => self.untracked,
            Group::Staged => tracked && self.x != '.',
            Group::Unstaged => tracked && self.y != '.',
        }
    }

    /// The state letter shown in list `g`.
    pub fn letter(&self, g: Group) -> char {
        match g {
            Group::Conflicts => '!',
            Group::Untracked => '?',
            Group::Staged => self.x,
            Group::Unstaged => self.y,
        }
    }

    /// The state in list `g`, for its colour and name.
    pub fn state(&self, g: Group) -> GitState {
        match self.letter(g) {
            '!' => GitState::Conflicted,
            '?' => GitState::Untracked,
            'A' => GitState::Added,
            'D' => GitState::Deleted,
            'R' | 'C' => GitState::Renamed,
            _ => GitState::Modified,
        }
    }

    pub fn abs(&self, root: &Path) -> PathBuf {
        root.join(self.path.split('/').collect::<PathBuf>())
    }

    /// The paths git commands for this file are given: both names of a
    /// rename, so it is staged, unstaged and diffed as one.
    pub fn paths(&self) -> Vec<&str> {
        let mut paths = vec![self.path.as_str()];
        paths.extend(self.orig.as_deref());
        paths
    }
}

/// The branch and changed files of a repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub head: Head,
    pub files: Vec<Change>,
}

impl Changes {
    pub fn group(&self, g: Group) -> impl Iterator<Item = &Change> {
        self.files.iter().filter(move |c| c.in_group(g))
    }

    pub fn count(&self, g: Group) -> usize {
        self.group(g).count()
    }

    /// The file at `path` (relative, with `/`).
    pub fn get(&self, path: &str) -> Option<&Change> {
        self.files.iter().find(|c| c.path == path)
    }
}

/// Parse `git status --porcelain=v2 -z --branch` output.
pub fn parse_changes(out: &[u8]) -> Changes {
    let mut changes = Changes::default();
    let mut fields = out.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(f) = fields.next() {
        let line = String::from_utf8_lossy(f);
        if let Some(header) = line.strip_prefix("# ") {
            parse_header(&mut changes.head, header);
            continue;
        }
        let xy = |s: &str| {
            let mut c = s.chars();
            (c.next().unwrap_or('.'), c.next().unwrap_or('.'))
        };
        let change = match line.as_bytes().first() {
            Some(b'1') => line.splitn(9, ' ').nth(8).map(|path| {
                let (x, y) = xy(line.split(' ').nth(1).unwrap_or(".."));
                Change {
                    path: path.to_owned(),
                    orig: None,
                    x,
                    y,
                    untracked: false,
                    conflict: false,
                }
            }),
            Some(b'2') => line.splitn(10, ' ').nth(9).map(|path| {
                let (x, y) = xy(line.split(' ').nth(1).unwrap_or(".."));
                Change {
                    path: path.to_owned(),
                    // With -z the old name is the next field.
                    orig: fields
                        .next()
                        .map(|o| String::from_utf8_lossy(o).into_owned()),
                    x,
                    y,
                    untracked: false,
                    conflict: false,
                }
            }),
            Some(b'u') => line.splitn(11, ' ').nth(10).map(|path| {
                let (x, y) = xy(line.split(' ').nth(1).unwrap_or("UU"));
                Change {
                    path: path.to_owned(),
                    orig: None,
                    x,
                    y,
                    untracked: false,
                    conflict: true,
                }
            }),
            Some(b'?') => line.get(2..).map(|path| Change {
                path: path.to_owned(),
                orig: None,
                x: '?',
                y: '?',
                untracked: true,
                conflict: false,
            }),
            _ => None,
        };
        changes.files.extend(change);
    }
    changes
}

/// "branch.oid <commit>|(initial)", "branch.head <name>|(detached)",
/// "branch.upstream <name>", "branch.ab +<ahead> -<behind>".
fn parse_header(head: &mut Head, header: &str) {
    let (key, value) = header.split_once(' ').unwrap_or((header, ""));
    match key {
        "branch.oid" => head.commit = (value != "(initial)").then(|| value.to_owned()),
        "branch.head" => head.branch = (value != "(detached)").then(|| value.to_owned()),
        "branch.upstream" => head.upstream = Some(value.to_owned()),
        "branch.ab" => {
            let (ahead, behind) = parse_ab(value);
            head.ahead = ahead;
            head.behind = behind;
        }
        _ => {}
    }
}

/// "+2 -1": two commits ahead of the upstream, one behind.
fn parse_ab(value: &str) -> (u32, u32) {
    let mut ahead = 0;
    let mut behind = 0;
    for part in value.split_whitespace() {
        if let Some(n) = part.strip_prefix('+') {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix('-') {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

/// The branch and changed files of the repository at `root`, every
/// untracked file listed on its own.
pub fn changes(root: &Path) -> io::Result<Changes> {
    let out = super::run(
        root,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
        ],
    )?;
    Ok(parse_changes(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2_output() {
        let out = b"# branch.oid 1234abcd\0# branch.head main\0# branch.upstream origin/main\0\
# branch.ab +2 -1\0\
1 M. N... 100644 100644 100644 aaa bbb src/staged file.rs\0\
1 .M N... 100644 100644 100644 aaa aaa src/lib.rs\0\
1 MM N... 100644 100644 100644 aaa bbb both.rs\0\
2 R. N... 100644 100644 100644 aaa aaa R100 docs/new name.md\0docs/old.md\0\
u UU N... 100644 100644 100644 100644 aaa bbb ccc conflict.rs\0\
? notes/todo.txt\0";
        let c = parse_changes(out);
        assert_eq!(
            c.head,
            Head {
                branch: Some("main".into()),
                commit: Some("1234abcd".into()),
                upstream: Some("origin/main".into()),
                ahead: 2,
                behind: 1,
            }
        );
        let names = |g| c.group(g).map(|f| f.path.as_str()).collect::<Vec<_>>();
        assert_eq!(
            names(Group::Staged),
            ["src/staged file.rs", "both.rs", "docs/new name.md"]
        );
        assert_eq!(names(Group::Unstaged), ["src/lib.rs", "both.rs"]);
        assert_eq!(names(Group::Conflicts), ["conflict.rs"]);
        assert_eq!(names(Group::Untracked), ["notes/todo.txt"]);
        let renamed = c.get("docs/new name.md").unwrap();
        assert_eq!(renamed.orig.as_deref(), Some("docs/old.md"));
        assert_eq!(renamed.letter(Group::Staged), 'R');
        assert_eq!(renamed.paths(), ["docs/new name.md", "docs/old.md"]);
        assert_eq!(c.get("src/lib.rs").unwrap().letter(Group::Unstaged), 'M');
        assert_eq!(
            c.get("conflict.rs").unwrap().state(Group::Conflicts),
            GitState::Conflicted
        );
        assert_eq!(
            c.get("notes/todo.txt").unwrap().abs(Path::new("/r")),
            Path::new("/r").join("notes").join("todo.txt")
        );
    }

    #[test]
    fn parses_branch_headers_and_ahead_behind() {
        let c = parse_changes(b"# branch.oid (initial)\0# branch.head main\0");
        assert_eq!(c.head.branch.as_deref(), Some("main"));
        assert_eq!(c.head.commit, None);
        assert_eq!(c.head.upstream, None);
        let c = parse_changes(b"# branch.oid abc\0# branch.head (detached)\0");
        assert_eq!(c.head.branch, None);
        assert_eq!(parse_ab("+0 -0"), (0, 0));
        assert_eq!(parse_ab("+12 -3"), (12, 3));
        assert_eq!(parse_ab(""), (0, 0));
    }
}
