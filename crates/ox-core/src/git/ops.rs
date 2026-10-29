//! Commands of the Git tab that change the repository: stage, unstage,
//! discard, commit, switch and create branches, fetch, pull and push.
//!
//! Each returns what git printed (hooks print there too); a failure's
//! error is git's message as it is. Paths are relative to the repository,
//! with `/`, and taken literally (`[id].tsx` is a name, not a pattern).

use std::io;
use std::path::Path;

use super::{cmd, Branch, Head};

/// Run `git` with `args` (after `--literal-pathspecs`) in the repository
/// at `root`; what it printed, or that as the error when it failed.
fn run_op(root: &Path, args: &[&str]) -> io::Result<String> {
    let mut cmd = cmd::command(root);
    cmd.arg("--literal-pathspecs").args(args);
    let out = cmd::output(&mut cmd, cmd::WRITE_TIMEOUT)?;
    let text = [&out.stdout, &out.stderr]
        .iter()
        .map(|b| String::from_utf8_lossy(b).trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if out.status.success() {
        Ok(text)
    } else if text.is_empty() {
        Err(io::Error::other(format!("git: {}", out.status)))
    } else {
        Err(io::Error::other(text))
    }
}

/// Run `args -- paths`, a hundred paths at a time (a command line has a
/// length limit on Windows).
fn run_with_paths(root: &Path, args: &[&str], paths: &[&str]) -> io::Result<String> {
    let mut printed = Vec::new();
    for chunk in paths.chunks(100) {
        let mut all = args.to_vec();
        all.push("--");
        all.extend_from_slice(chunk);
        printed.push(run_op(root, &all)?);
    }
    printed.retain(|p| !p.is_empty());
    Ok(printed.join("\n"))
}

/// Stage files (deleted ones too): `git add -A -- paths`.
pub fn stage(root: &Path, paths: &[&str]) -> io::Result<String> {
    run_with_paths(root, &["add", "-A"], paths)
}

/// Stage every change, untracked files included.
pub fn stage_all(root: &Path) -> io::Result<String> {
    run_op(root, &["add", "-A"])
}

/// Take files out of the next commit again (works before the first commit
/// too): `git reset -q -- paths`.
pub fn unstage(root: &Path, paths: &[&str]) -> io::Result<String> {
    run_with_paths(root, &["reset", "-q"], paths)
}

pub fn unstage_all(root: &Path) -> io::Result<String> {
    run_op(root, &["reset", "-q"])
}

/// Throw away the changes of files that aren't staged: they become what is
/// staged (or committed) again. Untracked files aren't touched.
pub fn discard(root: &Path, paths: &[&str]) -> io::Result<String> {
    run_with_paths(root, &["restore", "--worktree"], paths)
}

/// Commit what is staged with the message `summary` (and `description` as
/// its body). `amend` replaces the last commit instead; with no summary it
/// keeps that commit's message. Hooks run as usual.
pub fn commit(root: &Path, summary: &str, description: &str, amend: bool) -> io::Result<String> {
    let (summary, description) = (summary.trim(), description.trim());
    let mut args = vec!["commit"];
    if amend {
        args.push("--amend");
    }
    if summary.is_empty() {
        if !amend {
            return Err(io::Error::other(crate::i18n::t(
                "Write a commit message first.",
            )));
        }
        args.push("--no-edit");
    } else {
        args.extend(["-m", summary]);
        if !description.is_empty() {
            args.extend(["-m", description]);
        }
    }
    run_op(root, &args)
}

/// The full message of the last commit (to amend it).
pub fn last_message(root: &Path) -> io::Result<String> {
    let out = super::run(root, &["log", "-1", "--format=%B"])?;
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}

/// The local branch to check out for `branch` and, when it has to be
/// created, the remote branch it tracks. A remote branch whose local one
/// exists already goes to that.
pub fn switch_target<'a>(branch: &'a Branch, all: &[Branch]) -> (&'a str, Option<&'a str>) {
    if !branch.remote {
        return (&branch.name, None);
    }
    let local = branch.local_name();
    match all.iter().any(|b| !b.remote && b.name == local) {
        true => (local, None),
        false => (local, Some(&branch.name)),
    }
}

/// Check out `branch` (of the list `all`): a remote one gets a local
/// branch tracking it. Fails, saying so, when local changes would be
/// overwritten (see [`would_overwrite`]).
pub fn switch(root: &Path, branch: &Branch, all: &[Branch]) -> io::Result<String> {
    match switch_target(branch, all) {
        (local, None) => run_op(root, &["switch", local]),
        (local, Some(remote)) => run_op(root, &["switch", "-c", local, "--track", remote]),
    }
}

/// Put the local changes (untracked files too) aside with `git stash`,
/// then switch. The stash stays for `git stash pop` later.
pub fn stash_and_switch(root: &Path, branch: &Branch, all: &[Branch]) -> io::Result<String> {
    let message = format!("Oxplorer: before switching to {}", branch.name);
    let stashed = run_op(
        root,
        &["stash", "push", "--include-untracked", "-m", &message],
    )?;
    let switched = switch(root, branch, all)?;
    Ok(format!("{stashed}\n{switched}").trim().to_owned())
}

/// A new branch from HEAD, checked out.
pub fn create_branch(root: &Path, name: &str) -> io::Result<String> {
    run_op(root, &["switch", "-c", name.trim()])
}

/// Git refused to switch because local changes would be overwritten.
pub fn would_overwrite(error: &str) -> bool {
    // Git may speak the user's language; English and German are known.
    error.contains("would be overwritten") || error.contains("überschrieben")
}

pub fn fetch(root: &Path) -> io::Result<String> {
    run_op(root, &["fetch"])
}

/// `git pull` as configured (merge or rebase), without an editor.
pub fn pull(root: &Path) -> io::Result<String> {
    run_op(root, &["pull", "--no-edit"])
}

/// `git push`; a branch without an upstream is pushed to the default
/// remote (`origin`, or the only one) and tracks it from then on.
pub fn push(root: &Path, head: &Head) -> io::Result<String> {
    if head.upstream.is_some() || head.branch.is_none() {
        return run_op(root, &["push"]);
    }
    let remotes = super::run(root, &["remote"])?;
    let remotes = String::from_utf8_lossy(&remotes);
    let remote = remotes
        .lines()
        .map(str::trim)
        .find(|r| *r == "origin")
        .or_else(|| remotes.lines().map(str::trim).find(|r| !r.is_empty()));
    match remote {
        Some(remote) => run_op(root, &["push", "--set-upstream", remote, "HEAD"]),
        None => Err(io::Error::other(crate::i18n::t(
            "This repository has no remote to push to.",
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::super::diff::{self, DiffOf, LineKind};
    use super::super::{branches, changes, Group};
    use super::*;
    use std::process::Command;

    #[test]
    fn remote_branches_switch_to_their_local_branch() {
        let branch = |name: &str, remote: bool| Branch {
            name: name.into(),
            remote,
            head: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            gone: false,
            subject: String::new(),
            time: 0,
        };
        let all = [
            branch("main", false),
            branch("origin/main", true),
            branch("origin/fix/x", true),
        ];
        assert_eq!(switch_target(&all[0], &all), ("main", None));
        assert_eq!(switch_target(&all[1], &all), ("main", None));
        assert_eq!(
            switch_target(&all[2], &all),
            ("fix/x", Some("origin/fix/x"))
        );
        assert!(would_overwrite(
            "error: Your local changes to the following files would be overwritten by checkout:"
        ));
        assert!(!would_overwrite("fatal: invalid reference: x"));
    }

    #[test]
    fn stages_commits_switches_and_diffs_in_a_real_repository() {
        let tmp = tempfile::tempdir().unwrap();
        let root = &tmp.path().join("repo");
        std::fs::create_dir_all(root).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !git(root, &["init", "-q", "-b", "main"]) {
            eprintln!("git is not installed; skipping");
            return;
        }
        assert!(git(root, &["config", "user.name", "Ada"]));
        assert!(git(root, &["config", "user.email", "ada@example.com"]));
        assert!(git(root, &["config", "commit.gpgsign", "false"]));
        let write = |rel: &str, text: &str| std::fs::write(root.join(rel), text).unwrap();
        // Git may check files out with Windows line endings.
        let read = |rel: &str| {
            std::fs::read_to_string(root.join(rel))
                .unwrap()
                .replace('\r', "")
        };
        write("a.txt", "one\ntwo\n");
        write("[id].txt", "x");

        // Stage, unstage, stage again and commit.
        let c = changes(root).unwrap();
        assert_eq!(c.head.branch.as_deref(), Some("main"));
        assert_eq!(c.head.commit, None);
        assert_eq!(c.count(Group::Untracked), 2);
        let d = diff::diff(root, &["a.txt"], DiffOf::Untracked).unwrap();
        assert_eq!(d.files[0].added(), 2);
        stage(root, &["a.txt", "[id].txt"]).unwrap();
        assert_eq!(changes(root).unwrap().count(Group::Staged), 2);
        unstage(root, &["[id].txt"]).unwrap();
        let c = changes(root).unwrap();
        assert_eq!((c.count(Group::Staged), c.count(Group::Untracked)), (1, 1));
        // Before the first commit, the diff against HEAD is what is staged.
        let d = diff::diff(root, &["a.txt"], DiffOf::Head).unwrap();
        assert_eq!(d.files[0].added(), 2);
        assert!(commit(root, "", "", false).is_err());
        commit(root, "Add a", "The first file.", false).unwrap();
        assert_eq!(last_message(root).unwrap(), "Add a\n\nThe first file.");
        commit(root, "Add a.txt", "", true).unwrap();
        assert_eq!(last_message(root).unwrap(), "Add a.txt");

        // A change, its diff and discarding it.
        write("a.txt", "one\nTWO\n");
        let c = changes(root).unwrap();
        assert_eq!(c.get("a.txt").unwrap().letter(Group::Unstaged), 'M');
        let d = diff::diff(root, &["a.txt"], DiffOf::Unstaged).unwrap();
        let lines = &d.files[0].hunks[0].lines;
        let removed = lines.iter().find(|l| l.kind == LineKind::Removed).unwrap();
        assert_eq!((removed.text.as_str(), removed.old), ("two", Some(2)));
        assert!(diff::diff(root, &["a.txt"], DiffOf::Staged)
            .unwrap()
            .is_empty());
        discard(root, &["a.txt"]).unwrap();
        assert_eq!(read("a.txt"), "one\ntwo\n");

        // Branches: create one, commit there, switch back.
        create_branch(root, "feature").unwrap();
        write("a.txt", "one\nfeature\n");
        stage_all(root).unwrap();
        commit(root, "On feature", "", false).unwrap();
        let all = branches(root).unwrap();
        let names: Vec<&str> = all.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["feature", "main"]);
        assert!(all[0].head);
        let main = all.iter().find(|b| b.name == "main").unwrap();
        // A change the switch would overwrite: refused, then stashed.
        write("a.txt", "local\n");
        let err = switch(root, main, &all).unwrap_err().to_string();
        assert!(would_overwrite(&err), "{err}");
        stash_and_switch(root, main, &all).unwrap();
        assert_eq!(changes(root).unwrap().head.branch.as_deref(), Some("main"));
        assert_eq!(read("a.txt"), "one\ntwo\n");

        // A clone: the remote branch "origin/feature" becomes a local
        // branch tracking it, and is ahead of nothing.
        let clone = &tmp.path().join("clone");
        assert!(git(
            tmp.path(),
            &["clone", "-q", &root.to_string_lossy(), "clone"]
        ));
        let all = branches(clone).unwrap();
        let remote = all.iter().find(|b| b.name == "origin/feature").unwrap();
        switch(clone, remote, &all).unwrap();
        let c = changes(clone).unwrap();
        assert_eq!(c.head.branch.as_deref(), Some("feature"));
        assert_eq!(c.head.upstream.as_deref(), Some("origin/feature"));
        assert_eq!((c.head.ahead, c.head.behind), (0, 0));
        std::fs::write(clone.join("b.txt"), "b").unwrap();
        stage_all(clone).unwrap();
        assert!(git(clone, &["config", "user.name", "Ada"]));
        assert!(git(clone, &["config", "user.email", "ada@example.com"]));
        assert!(git(clone, &["config", "commit.gpgsign", "false"]));
        commit(clone, "Add b", "", false).unwrap();
        assert_eq!(changes(clone).unwrap().head.ahead, 1);
    }
}
