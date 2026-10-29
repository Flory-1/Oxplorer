//! Stashes: local changes put aside with `git stash` (the Git tab's
//! Branches view lists them to apply, pop or drop).

use std::io;
use std::path::Path;

use super::ops::run_stash;

/// One entry of `git stash list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    /// Its place in the list: `stash@{index}`, 0 the newest.
    pub index: usize,
    /// The branch it was made on (`None` for a detached HEAD).
    pub branch: Option<String>,
    /// The message: given when stashing, or "abc1234 subject" of the
    /// commit it was made on.
    pub message: String,
    /// When it was made (seconds since 1970).
    pub time: i64,
    /// Its commit, to make sure `stash@{index}` is still this one.
    pub hash: String,
}

impl Stash {
    /// `stash@{2}`.
    pub fn name(&self) -> String {
        format!("stash@{{{}}}", self.index)
    }
}

/// The `--format` of [`stash_list`]: fields separated by U+001F.
const FORMAT: &str = "--format=%gd%x1f%gs%x1f%ct%x1f%H";

/// "WIP on main: abc1234 Fix x" or "On main: my message" → (branch,
/// message). Git writes "(no branch)" for a detached HEAD.
fn parse_subject(subject: &str) -> (Option<String>, String) {
    let rest = subject
        .strip_prefix("WIP on ")
        .or_else(|| subject.strip_prefix("On "));
    let Some((branch, message)) = rest.and_then(|r| r.split_once(": ")) else {
        return (None, subject.to_owned());
    };
    let branch = (branch != "(no branch)").then(|| branch.to_owned());
    (branch, message.to_owned())
}

/// Parse `git stash list` output in the [`FORMAT`] above.
pub fn parse_stashes(out: &str) -> Vec<Stash> {
    out.lines()
        .filter_map(|line| {
            let mut f = line.split('\u{1f}');
            let (name, subject, time, hash) = (f.next()?, f.next()?, f.next()?, f.next()?);
            let index = name
                .strip_prefix("stash@{")?
                .strip_suffix('}')?
                .parse()
                .ok()?;
            let (branch, message) = parse_subject(subject);
            Some(Stash {
                index,
                branch,
                message,
                time: time.trim().parse().unwrap_or(0),
                hash: hash.trim().to_owned(),
            })
        })
        .collect()
}

/// The stashes of the repository at `root`, the newest first.
pub fn stash_list(root: &Path) -> io::Result<Vec<Stash>> {
    let out = super::run(root, &["stash", "list", FORMAT])?;
    Ok(parse_stashes(&String::from_utf8_lossy(&out)))
}

/// Put the local changes aside (untracked files too), with `message` if
/// one is given.
pub fn stash_push(root: &Path, message: &str) -> io::Result<String> {
    let mut args = vec!["stash", "push", "--include-untracked"];
    let message = message.trim();
    if !message.is_empty() {
        args.extend(["-m", message]);
    }
    run_stash(root, &args)
}

/// `stash@{index}`, after checking that it is still `stash` (the list may
/// have changed since it was read).
fn current_name(root: &Path, stash: &Stash) -> io::Result<String> {
    let name = stash.name();
    let out = super::run(root, &["rev-parse", "--verify", "-q", &name]).unwrap_or_default();
    if String::from_utf8_lossy(&out).trim() == stash.hash {
        Ok(name)
    } else {
        Err(io::Error::other(crate::i18n::t(
            "The list of stashes changed in the meantime. Look at it again and try once more.",
        )))
    }
}

/// Bring the stash's changes back; the stash stays.
pub fn stash_apply(root: &Path, stash: &Stash) -> io::Result<String> {
    let name = current_name(root, stash)?;
    run_stash(root, &["stash", "apply", &name])
}

/// Bring the stash's changes back and remove it (it stays when they
/// conflict).
pub fn stash_pop(root: &Path, stash: &Stash) -> io::Result<String> {
    let name = current_name(root, stash)?;
    run_stash(root, &["stash", "pop", &name])
}

/// Throw the stash away (can't be undone).
pub fn stash_drop(root: &Path, stash: &Stash) -> io::Result<String> {
    let name = current_name(root, stash)?;
    run_stash(root, &["stash", "drop", &name])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_stash_list() {
        let out = "stash@{0}\u{1f}On main: before the release\u{1f}1700000100\u{1f}aaa\n\
                   stash@{1}\u{1f}WIP on fix/x: 1a2b3c4 Fix the thing\u{1f}1700000000\u{1f}bbb\n\
                   stash@{12}\u{1f}WIP on (no branch): 9f8e7d6 Detached\u{1f}1690000000\u{1f}ccc\n\
                   stash@{13}\u{1f}autostash\u{1f}1680000000\u{1f}ddd\n\
                   broken line\n";
        let list = parse_stashes(out);
        assert_eq!(list.len(), 4);
        assert_eq!(
            list[0],
            Stash {
                index: 0,
                branch: Some("main".into()),
                message: "before the release".into(),
                time: 1_700_000_100,
                hash: "aaa".into(),
            }
        );
        assert_eq!(list[1].branch.as_deref(), Some("fix/x"));
        assert_eq!(list[1].message, "1a2b3c4 Fix the thing");
        assert_eq!(list[2].index, 12);
        assert_eq!(list[2].branch, None);
        assert_eq!(list[2].name(), "stash@{12}");
        assert_eq!(
            (list[3].branch.as_deref(), list[3].message.as_str()),
            (None, "autostash")
        );
        assert!(parse_stashes("").is_empty());
    }
}
