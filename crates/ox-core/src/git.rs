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

/// Run `git` with `args` in the repository at `root`; its output.
fn run(root: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .args(args)
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
    Ok(out.stdout)
}

/// Run `git status` for the repository at `root`.
pub fn status(root: &Path) -> io::Result<RepoStatus> {
    let out = run(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "-b",
            "--untracked-files=normal",
            "--ignored=matching",
        ],
    )?;
    Ok(parse(root, &out))
}

/// The checked out branch ("main"), or "HEAD" when none is.
pub fn current_branch(root: &Path) -> io::Result<String> {
    let out = run(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}

/// The URL of the repository's `origin` remote (or its first remote).
pub fn remote_url(root: &Path) -> io::Result<Option<String>> {
    let name = match run(root, &["remote", "get-url", "origin"]) {
        Ok(out) => return Ok(Some(String::from_utf8_lossy(&out).trim().to_owned())),
        Err(_) => {
            let remotes = run(root, &["remote"])?;
            match String::from_utf8_lossy(&remotes).lines().next() {
                Some(n) => n.trim().to_owned(),
                None => return Ok(None),
            }
        }
    };
    let out = run(root, &["remote", "get-url", &name])?;
    Ok(Some(String::from_utf8_lossy(&out).trim().to_owned()))
}

/// The web page of a remote: `git@github.com:me/repo.git`,
/// `ssh://git@host:22/me/repo.git` and `https://user@host/me/repo.git` all
/// become `https://host/me/repo`. Local remotes have none.
pub fn web_url(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let (host, path) = if let Some(rest) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
        .or_else(|| remote.strip_prefix("ssh://"))
        .or_else(|| remote.strip_prefix("git://"))
    {
        let (host, path) = rest.split_once('/')?;
        // Drop "user:password@" and ":port".
        let host = host.rsplit('@').next()?;
        let host = host.split(':').next()?;
        (host.to_owned(), path.to_owned())
    } else {
        // scp-like: [user@]host:path
        let (host, path) = remote.split_once(':')?;
        if host.contains('/') || host.contains('\\') || host.len() < 2 {
            return None; // a local path ("C:\repo", "/srv/repo")
        }
        (host.rsplit('@').next()?.to_owned(), path.to_owned())
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

/// The web page of `path` (a file or folder in the repository at `root`)
/// on the branch `branch`, in the URL style of GitHub, GitLab or Bitbucket.
pub fn web_url_of(repo_url: &str, root: &Path, path: &Path, branch: &str, is_dir: bool) -> String {
    let rel: Vec<String> = path
        .strip_prefix(root)
        .map(|r| {
            r.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    if rel.is_empty() {
        return repo_url.to_owned();
    }
    let rel = rel.join("/");
    let kind = if is_dir { "tree" } else { "blob" };
    if repo_url.contains("gitlab") {
        format!("{repo_url}/-/{kind}/{branch}/{rel}")
    } else if repo_url.contains("bitbucket") {
        format!("{repo_url}/src/{branch}/{rel}")
    } else {
        format!("{repo_url}/{kind}/{branch}/{rel}")
    }
}

/// The last commit that changed `path` (a file or folder in the repository
/// at `root`), if any did.
pub fn last_commit(root: &Path, path: &Path) -> io::Result<Option<Commit>> {
    let out = run(
        root,
        &[
            "log",
            "-1",
            "--format=%H%x1f%P%x1f%an%x1f%ct%x1f%x1f%s%x1e",
            "--",
            &path.to_string_lossy(),
        ],
    )?;
    Ok(parse_log(&String::from_utf8_lossy(&out)).into_iter().next())
}

/// One commit of the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Commit time in seconds since 1970.
    pub time: i64,
    /// Branches and tags pointing here.
    pub refs: Vec<GitRef>,
    pub subject: String,
}

/// A branch or tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRef {
    /// The checked out branch.
    Head(String),
    /// HEAD when no branch is checked out.
    Detached,
    Branch(String),
    /// A remote branch ("origin/main").
    Remote(String),
    Tag(String),
}

impl GitRef {
    /// One entry of `%D` with `--decorate=full`, e.g.
    /// "HEAD -> refs/heads/main" or "tag: refs/tags/v1.0".
    fn parse(r: &str) -> Option<GitRef> {
        Some(if let Some(head) = r.strip_prefix("HEAD -> ") {
            GitRef::Head(head.strip_prefix("refs/heads/").unwrap_or(head).to_owned())
        } else if r == "HEAD" {
            GitRef::Detached
        } else if let Some(tag) = r.strip_prefix("tag: ") {
            GitRef::Tag(tag.strip_prefix("refs/tags/").unwrap_or(tag).to_owned())
        } else if let Some(b) = r.strip_prefix("refs/heads/") {
            GitRef::Branch(b.to_owned())
        } else {
            // Stashes, notes, … aren't shown.
            let b = r.strip_prefix("refs/remotes/")?;
            // "origin/HEAD" only says which branch the remote defaults to.
            if b.ends_with("/HEAD") {
                return None;
            }
            GitRef::Remote(b.to_owned())
        })
    }

    pub fn name(&self) -> &str {
        match self {
            GitRef::Head(n) | GitRef::Branch(n) | GitRef::Remote(n) | GitRef::Tag(n) => n,
            GitRef::Detached => "HEAD",
        }
    }
}

impl Commit {
    pub fn short(&self) -> &str {
        &self.hash[..self.hash.len().min(7)]
    }
}

/// Separates the fields of a commit in `git log` output, and the commits.
const FIELD: char = '\x1f';
const RECORD: char = '\x1e';

/// Parse `git log --format=%H%x1f%P%x1f%an%x1f%ct%x1f%D%x1f%s%x1e` output.
pub fn parse_log(out: &str) -> Vec<Commit> {
    out.split(RECORD)
        .filter_map(|rec| {
            let mut f = rec.trim_start_matches('\n').split(FIELD);
            let hash = f.next().filter(|h| !h.is_empty())?.to_owned();
            let parents = f.next()?.split_whitespace().map(str::to_owned).collect();
            let author = f.next()?.to_owned();
            let time = f.next()?.parse().unwrap_or(0);
            let refs = f.next()?.split(", ").filter_map(GitRef::parse).collect();
            let subject = f.next().unwrap_or_default().to_owned();
            Some(Commit {
                hash,
                parents,
                author,
                time,
                refs,
                subject,
            })
        })
        .collect()
}

/// The newest `limit` commits of every branch, newest first and children
/// before their parents (`git log --all --topo-order`).
pub fn log(root: &Path, limit: usize) -> io::Result<Vec<Commit>> {
    let out = run(
        root,
        &[
            "log",
            "--all",
            "--topo-order",
            &format!("--max-count={limit}"),
            "--decorate=full",
            "--format=%H%x1f%P%x1f%an%x1f%ct%x1f%D%x1f%s%x1e",
        ],
    )?;
    Ok(parse_log(&String::from_utf8_lossy(&out)))
}

/// How one row of the commit graph is drawn. Columns ("lanes") are counted
/// from the left; each line keeps the colour of the lane it starts in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphRow {
    /// The commit's lane.
    pub lane: usize,
    /// Lanes passing by from the row above to the row below.
    pub through: Vec<usize>,
    /// Lanes coming from the row above into the commit (its children).
    pub from_above: Vec<usize>,
    /// Lanes leaving the commit to the row below (its parents).
    pub to_below: Vec<usize>,
}

/// Lay out the commit graph of `commits` (children before parents, as
/// `log` returns them). Also returns the number of lanes used.
pub fn graph(commits: &[Commit]) -> (Vec<GraphRow>, usize) {
    // The commit each lane waits for.
    let mut lanes: Vec<Option<&str>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let mut width = 0;
    let free_lane = |lanes: &mut Vec<Option<&str>>| {
        lanes.iter().position(Option::is_none).unwrap_or_else(|| {
            lanes.push(None);
            lanes.len() - 1
        })
    };
    for c in commits {
        let waiting: Vec<usize> = (0..lanes.len())
            .filter(|&i| lanes[i] == Some(c.hash.as_str()))
            .collect();
        let lane = match waiting.first() {
            Some(&l) => l,
            // A branch tip: nothing above leads here.
            None => free_lane(&mut lanes),
        };
        let through = (0..lanes.len())
            .filter(|&i| lanes[i].is_some() && !waiting.contains(&i))
            .collect();
        for &i in &waiting {
            lanes[i] = None;
        }
        let mut to_below = Vec::new();
        for (n, parent) in c.parents.iter().enumerate() {
            // The first parent continues the commit's lane (lanes leading to
            // the same commit meet there); merged ones join a lane already
            // leading to them, or start a new one.
            let target = if n == 0 {
                lane
            } else if let Some(i) = lanes.iter().position(|l| *l == Some(parent.as_str())) {
                i
            } else {
                free_lane(&mut lanes)
            };
            lanes[target] = Some(parent.as_str());
            to_below.push(target);
        }
        while lanes.last() == Some(&None) {
            lanes.pop();
        }
        width = width.max(lanes.len()).max(lane + 1);
        rows.push(GraphRow {
            lane,
            through,
            from_above: waiting,
            to_below,
        });
    }
    (rows, width)
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

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: "A".into(),
            time: 0,
            refs: Vec::new(),
            subject: String::new(),
        }
    }

    #[test]
    fn parses_log_output() {
        let out = "aaa\x1fbbb ccc\x1fAda\x1f1700000000\x1fHEAD -> refs/heads/main, \
                   refs/remotes/origin/main, refs/remotes/origin/HEAD, refs/heads/dev, \
                   tag: refs/tags/v1, refs/stash\x1fMerge it\x1e\n\
                   bbb\x1f\x1fBob\x1f1600000000\x1f\x1fFirst\x1e\n";
        let log = parse_log(out);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].parents, ["bbb", "ccc"]);
        assert_eq!(
            log[0].refs,
            [
                GitRef::Head("main".into()),
                GitRef::Remote("origin/main".into()),
                GitRef::Branch("dev".into()),
                GitRef::Tag("v1".into())
            ]
        );
        assert_eq!(log[0].subject, "Merge it");
        assert_eq!(log[0].time, 1_700_000_000);
        assert_eq!(log[1].author, "Bob");
        assert!(log[1].parents.is_empty() && log[1].refs.is_empty());
    }

    #[test]
    fn lays_out_branches_and_merges() {
        // m merges feature (f) into main (b); both started at a.
        //   m        lane 0
        //   |\
        //   f |      lane 1
        //   | b      lane 0
        //   |/
        //   a
        let commits = [
            commit("m", &["b", "f"]),
            commit("f", &["a"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ];
        let (rows, width) = graph(&commits);
        assert_eq!(width, 2);
        let lanes: Vec<usize> = rows.iter().map(|r| r.lane).collect();
        assert_eq!(lanes, [0, 1, 0, 0]);
        assert_eq!(rows[0].to_below, [0, 1]);
        assert!(rows[0].from_above.is_empty());
        assert_eq!(rows[1].through, [0]);
        assert_eq!(rows[1].to_below, [1]);
        assert_eq!(rows[2].through, [1]);
        assert_eq!(rows[2].to_below, [0]);
        // Both lanes end in a.
        assert_eq!(rows[3].from_above, [0, 1]);
        assert!(rows[3].through.is_empty() && rows[3].to_below.is_empty());

        // A merge of a commit already shown in another lane joins that lane.
        let commits = [
            commit("m", &["b", "f"]),
            commit("x", &["f"]),
            commit("b", &["f"]),
            commit("f", &[]),
        ];
        let (rows, _) = graph(&commits);
        assert_eq!(rows[0].to_below, [0, 1]);
        assert_eq!(rows[1].lane, 2);
        assert_eq!(rows[3].from_above, [0, 1, 2]);
    }

    #[test]
    fn web_pages_of_remotes() {
        let want = Some("https://github.com/me/repo".to_owned());
        assert_eq!(web_url("git@github.com:me/repo.git"), want);
        assert_eq!(web_url("https://github.com/me/repo.git"), want);
        assert_eq!(web_url("https://me:token@github.com/me/repo"), want);
        assert_eq!(web_url("ssh://git@github.com:22/me/repo.git/"), want);
        assert_eq!(
            web_url("git@gitlab.com:group/sub/repo.git").as_deref(),
            Some("https://gitlab.com/group/sub/repo")
        );
        assert_eq!(web_url("/srv/git/repo.git"), None);
        assert_eq!(web_url(r"C:\repos\x"), None);
        assert_eq!(web_url("../other"), None);

        let root = Path::new("/r");
        let gh = "https://github.com/me/repo";
        assert_eq!(
            web_url_of(gh, root, &root.join("src/main.rs"), "main", false),
            "https://github.com/me/repo/blob/main/src/main.rs"
        );
        assert_eq!(web_url_of(gh, root, root, "main", true), gh);
        assert_eq!(
            web_url_of(
                "https://gitlab.com/g/r",
                root,
                &root.join("docs"),
                "dev",
                true
            ),
            "https://gitlab.com/g/r/-/tree/dev/docs"
        );
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

        let commit = |msg: &str| {
            git(&[
                "-c",
                "user.name=Ada",
                "-c",
                "user.email=ada@example.com",
                "commit",
                "-q",
                "-m",
                msg,
            ])
        };
        assert!(commit("Add a"));
        std::fs::write(root.join("b.txt"), "b").unwrap();
        assert!(git(&["add", "b.txt"]));
        assert!(commit("Add b"));
        assert_eq!(current_branch(root).unwrap(), "main");
        let last = last_commit(root, &root.join("src/a.txt")).unwrap().unwrap();
        assert_eq!(
            (last.subject.as_str(), last.author.as_str()),
            ("Add a", "Ada")
        );
        let log = log(root, 10).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].refs, [GitRef::Head("main".into())]);
        assert_eq!(remote_url(root).unwrap(), None);
        assert!(git(&["remote", "add", "origin", "git@github.com:me/r.git"]));
        assert_eq!(
            remote_url(root)
                .unwrap()
                .and_then(|u| web_url(&u))
                .as_deref(),
            Some("https://github.com/me/r")
        );
    }
}
