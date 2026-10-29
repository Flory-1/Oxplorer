//! Local and remote-tracking branches (`git for-each-ref`).

use std::io;
use std::path::Path;

/// One branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    /// "main", or "origin/main" for a remote-tracking branch.
    pub name: String,
    pub remote: bool,
    /// The checked out branch.
    pub head: bool,
    /// The branch it pulls from and pushes to, e.g. "origin/main".
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// The upstream was deleted on the remote.
    pub gone: bool,
    /// The newest commit's subject and time (seconds since 1970).
    pub subject: String,
    pub time: i64,
}

impl Branch {
    /// A remote branch's name without the remote ("origin/fix/x" → "fix/x").
    pub fn local_name(&self) -> &str {
        match self.remote {
            true => self.name.split_once('/').map_or(&self.name, |(_, b)| b),
            false => &self.name,
        }
    }
}

/// The `--format` of [`branches`]: fields separated by U+001F.
const FORMAT: &str = "--format=%(refname)%1f%(HEAD)%1f%(upstream:short)%1f%(upstream:track)\
%1f%(committerdate:unix)%1f%(subject)";

/// "[ahead 2, behind 1]", "[ahead 3]", "[gone]" or "": ahead, behind and
/// whether the upstream is gone.
pub fn parse_track(track: &str) -> (u32, u32, bool) {
    let inner = track.trim().trim_start_matches('[').trim_end_matches(']');
    let (mut ahead, mut behind, mut gone) = (0, 0, false);
    for part in inner.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.trim().parse().unwrap_or(0);
        } else if part == "gone" {
            gone = true;
        }
    }
    (ahead, behind, gone)
}

/// Parse `git for-each-ref` output in the [`FORMAT`] above. Local branches
/// first; remote ones without their "origin/HEAD".
pub fn parse_branches(out: &str) -> Vec<Branch> {
    let mut list: Vec<Branch> = out
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\x1f');
            let refname = f.next()?;
            let (name, remote) = if let Some(n) = refname.strip_prefix("refs/heads/") {
                (n, false)
            } else {
                (refname.strip_prefix("refs/remotes/")?, true)
            };
            if remote && (name.ends_with("/HEAD") || !name.contains('/')) {
                return None;
            }
            let head = f.next()? == "*";
            let upstream = Some(f.next()?).filter(|u| !u.is_empty()).map(str::to_owned);
            let (ahead, behind, gone) = parse_track(f.next()?);
            let time = f.next()?.parse().unwrap_or(0);
            let subject = f.next().unwrap_or_default().to_owned();
            Some(Branch {
                name: name.to_owned(),
                remote,
                head,
                upstream,
                ahead,
                behind,
                gone,
                subject,
                time,
            })
        })
        .collect();
    list.sort_by_key(|b| b.remote);
    list
}

/// The branches of the repository at `root`, newest commits first.
pub fn branches(root: &Path) -> io::Result<Vec<Branch>> {
    let out = super::run(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            FORMAT,
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    Ok(parse_branches(&String::from_utf8_lossy(&out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracking_info() {
        assert_eq!(parse_track("[ahead 2, behind 1]"), (2, 1, false));
        assert_eq!(parse_track("[ahead 3]"), (3, 0, false));
        assert_eq!(parse_track("[behind 4]"), (0, 4, false));
        assert_eq!(parse_track("[gone]"), (0, 0, true));
        assert_eq!(parse_track(""), (0, 0, false));
    }

    #[test]
    fn parses_branch_list() {
        let out = "refs/remotes/origin/main\x1f \x1f\x1f\x1f1700000300\x1fFix it\n\
                   refs/heads/main\x1f*\x1forigin/main\x1f[ahead 1, behind 2]\x1f1700000200\x1fFix it\n\
                   refs/remotes/origin/HEAD\x1f \x1f\x1f\x1f1700000300\x1fFix it\n\
                   refs/heads/feature/x\x1f \x1f\x1f\x1f1700000100\x1fStart x\n\
                   refs/heads/old\x1f \x1forigin/old\x1f[gone]\x1f1600000000\x1fOld\n\
                   refs/remotes/origin\x1f \x1f\x1f\x1f0\x1f\n";
        let b = parse_branches(out);
        let names: Vec<&str> = b.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["main", "feature/x", "old", "origin/main"]);
        assert!(b[0].head && !b[0].remote);
        assert_eq!(b[0].upstream.as_deref(), Some("origin/main"));
        assert_eq!((b[0].ahead, b[0].behind), (1, 2));
        assert_eq!(b[0].time, 1_700_000_200);
        assert_eq!(b[1].subject, "Start x");
        assert!(b[2].gone);
        assert!(b[3].remote && !b[3].head);
        assert_eq!(b[3].local_name(), "main");
        assert_eq!(b[1].local_name(), "feature/x");
    }
}
