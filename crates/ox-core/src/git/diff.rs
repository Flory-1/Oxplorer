//! Diffs of changed files: `git diff` output parsed into files, hunks and
//! lines, untracked files shown as all added, and what changed within a
//! line.

use std::io;
use std::ops::Range;
use std::path::Path;

/// More than this isn't shown.
pub const MAX_BYTES: usize = 4 << 20;
pub const MAX_LINES: usize = 20_000;

/// What a diff compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffOf {
    /// The working tree against the index (`git diff`).
    Unstaged,
    /// The index against HEAD (`git diff --cached`).
    Staged,
    /// The working tree against HEAD (both).
    Head,
    /// A file Git doesn't track yet: all of it is new.
    Untracked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// Line numbers in the old and the new file.
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
    /// "\ No newline at end of file" follows the line.
    pub no_newline: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    /// What git shows after the range: the function or section around it.
    pub section: String,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// "@@ -1,3 +1,4 @@ fn main()".
    pub fn header(&self) -> String {
        let mut s = format!(
            "@@ -{},{} +{},{} @@",
            self.old_start, self.old_count, self.new_start, self.new_count
        );
        if !self.section.is_empty() {
            s.push(' ');
            s.push_str(&self.section);
        }
        s
    }
}

/// The diff of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// `None` for a new file.
    pub old_path: Option<String>,
    /// `None` for a deleted file.
    pub new_path: Option<String>,
    pub renamed: bool,
    /// "similarity index 90%" of renames and copies.
    pub similarity: Option<u8>,
    /// File modes when they changed ("100644" → "100755").
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    pub fn added(&self) -> usize {
        self.count(LineKind::Added)
    }

    pub fn removed(&self) -> usize {
        self.count(LineKind::Removed)
    }

    fn count(&self, kind: LineKind) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == kind)
            .count()
    }
}

/// A file's diff as loaded for the Git tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub files: Vec<FileDiff>,
    /// Too big to show (more than [`MAX_BYTES`] or [`MAX_LINES`]).
    pub too_large: bool,
}

impl Diff {
    /// Nothing differs (e.g. only the other side changed).
    pub fn is_empty(&self) -> bool {
        !self.too_large && self.files.iter().all(|f| f.hunks.is_empty() && !f.binary)
    }
}

/// A path in a "---"/"+++" or "rename from" line: `a/` and `b/` dropped,
/// quoted paths ("a/\303\244.txt") unquoted, `/dev/null` is none.
fn diff_path(s: &str, strip_prefix: bool) -> Option<String> {
    let s = s.trim_end_matches('\t'); // git adds a tab after names with spaces
    if s == "/dev/null" {
        return None;
    }
    let s = if s.starts_with('"') {
        unquote(s)
    } else {
        s.to_owned()
    };
    // `diff` asks for these prefixes, whatever git is configured to use.
    match s.strip_prefix("a/").or_else(|| s.strip_prefix("b/")) {
        Some(rest) if strip_prefix => Some(rest.to_owned()),
        _ => Some(s),
    }
}

/// Undo git's C-style quoting of a path: `"tab\there \303\244"`.
fn unquote(s: &str) -> String {
    let inner = s.strip_prefix('"').and_then(|s| s.strip_suffix('"'));
    let Some(inner) = inner else {
        return s.to_owned();
    };
    let b = inner.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            let c = b[i + 1];
            let oct = inner
                .get(i + 1..i + 4)
                .and_then(|o| u8::from_str_radix(o, 8).ok());
            if let (true, Some(v)) = (c.is_ascii_digit(), oct) {
                out.push(v);
                i += 4;
                continue;
            }
            out.push(match c {
                b'n' => b'\n',
                b't' => b'\t',
                b'r' => b'\r',
                b'a' => 7,
                b'b' => 8,
                b'f' => 12,
                b'v' => 11,
                other => other,
            });
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// "@@ -1,3 +1,4 @@ section": the hunk without lines. A missing count is 1.
fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ ")?;
    let (ranges, section) = rest.split_once(" @@").unwrap_or((rest, ""));
    let mut parts = ranges.split_whitespace();
    let range = |r: &str| -> Option<(u32, u32)> {
        let (start, count) = r.split_once(',').unwrap_or((r, "1"));
        Some((start.parse().ok()?, count.parse().ok()?))
    };
    let (old_start, old_count) = range(parts.next()?.strip_prefix('-')?)?;
    let (new_start, new_count) = range(parts.next()?.strip_prefix('+')?)?;
    Some(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
        section: section.trim().to_owned(),
        lines: Vec::new(),
    })
}

/// Parse unified `git diff` output (one or more files).
pub fn parse(out: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    // Lines left in the current hunk (old, new): past them, lines are headers.
    let (mut old_left, mut new_left) = (0u32, 0u32);
    let (mut old_no, mut new_no) = (0u32, 0u32);
    for line in out.split('\n') {
        let in_hunk = old_left > 0 || new_left > 0;
        if line.starts_with("diff --git ") || (files.is_empty() && line.starts_with("--- ")) {
            files.push(FileDiff::default());
            (old_left, new_left) = (0, 0);
            if let Some(p) = line.strip_prefix("--- ") {
                files.last_mut().expect("pushed").old_path = diff_path(p, true);
            }
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if in_hunk {
            if let Some(hunk) = file.hunks.last_mut() {
                let (kind, text) = match line.as_bytes().first() {
                    Some(b'+') => (LineKind::Added, &line[1..]),
                    Some(b'-') => (LineKind::Removed, &line[1..]),
                    Some(b' ') => (LineKind::Context, &line[1..]),
                    // An empty context line, if something dropped its space.
                    None => (LineKind::Context, ""),
                    Some(b'\\') => {
                        if let Some(last) = hunk.lines.last_mut() {
                            last.no_newline = true;
                        }
                        continue;
                    }
                    Some(_) => {
                        (old_left, new_left) = (0, 0);
                        continue;
                    }
                };
                let (old, new) = match kind {
                    LineKind::Added => (None, Some(new_no)),
                    LineKind::Removed => (Some(old_no), None),
                    LineKind::Context => (Some(old_no), Some(new_no)),
                };
                if kind != LineKind::Added {
                    old_no += 1;
                    old_left = old_left.saturating_sub(1);
                }
                if kind != LineKind::Removed {
                    new_no += 1;
                    new_left = new_left.saturating_sub(1);
                }
                hunk.lines.push(DiffLine {
                    kind,
                    old,
                    new,
                    text: text.to_owned(),
                    no_newline: false,
                });
                continue;
            }
        }
        if line.starts_with("\\") {
            // "\ No newline at end of file" after the hunk's last line.
            if let Some(last) = file.hunks.last_mut().and_then(|h| h.lines.last_mut()) {
                last.no_newline = true;
            }
        } else if line.starts_with("@@ ") {
            if let Some(h) = parse_hunk_header(line) {
                (old_no, new_no) = (h.old_start, h.new_start);
                (old_left, new_left) = (h.old_count, h.new_count);
                file.hunks.push(h);
            }
        } else if let Some(p) = line.strip_prefix("--- ") {
            file.old_path = diff_path(p, true);
        } else if let Some(p) = line.strip_prefix("+++ ") {
            file.new_path = diff_path(p, true);
        } else if let Some(p) = line
            .strip_prefix("rename from ")
            .or_else(|| line.strip_prefix("copy from "))
        {
            file.renamed = true;
            file.old_path = diff_path(p, false);
        } else if let Some(p) = line
            .strip_prefix("rename to ")
            .or_else(|| line.strip_prefix("copy to "))
        {
            file.renamed = true;
            file.new_path = diff_path(p, false);
        } else if let Some(s) = line
            .strip_prefix("similarity index ")
            .and_then(|s| s.strip_suffix('%'))
        {
            file.similarity = s.parse().ok();
        } else if let Some(m) = line.strip_prefix("old mode ") {
            file.old_mode = Some(m.to_owned());
        } else if let Some(m) = line.strip_prefix("new mode ") {
            file.new_mode = Some(m.to_owned());
        } else if line.starts_with("new file mode ") {
            file.old_path = None;
        } else if line.starts_with("deleted file mode ") {
            file.new_path = None;
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            file.binary = true;
        }
    }
    files
}

/// Git's own test for binary content: a NUL byte near the start.
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(8000)].contains(&0)
}

/// An untracked file as a diff: every line added.
pub fn all_added(path: &str, bytes: &[u8]) -> Diff {
    let mut file = FileDiff {
        new_path: Some(path.to_owned()),
        ..Default::default()
    };
    if bytes.len() > MAX_BYTES {
        return Diff {
            files: vec![file],
            too_large: true,
        };
    }
    if looks_binary(bytes) {
        file.binary = true;
        return Diff {
            files: vec![file],
            too_large: false,
        };
    }
    let text = String::from_utf8_lossy(bytes);
    let no_newline = !text.is_empty() && !text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(&text);
    let mut lines: Vec<DiffLine> = if text.is_empty() {
        Vec::new()
    } else {
        body.split('\n')
            .enumerate()
            .map(|(i, l)| DiffLine {
                kind: LineKind::Added,
                old: None,
                new: Some(i as u32 + 1),
                text: l.to_owned(),
                no_newline: false,
            })
            .collect()
    };
    if lines.len() > MAX_LINES {
        return Diff {
            files: vec![file],
            too_large: true,
        };
    }
    if let Some(last) = lines.last_mut() {
        last.no_newline = no_newline;
    }
    if !lines.is_empty() {
        file.hunks.push(Hunk {
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: lines.len() as u32,
            section: String::new(),
            lines,
        });
    }
    Diff {
        files: vec![file],
        too_large: false,
    }
}

/// The diff of the file `paths[0]` (relative, with `/`; a renamed file's
/// old name second) in the repository at `root`.
pub fn diff(root: &Path, paths: &[&str], of: DiffOf) -> io::Result<Diff> {
    if of == DiffOf::Untracked {
        let rel = paths.first().copied().unwrap_or_default();
        let path = root.join(rel.split('/').collect::<std::path::PathBuf>());
        let len = std::fs::metadata(&path)?.len() as usize;
        if len > MAX_BYTES {
            return Ok(Diff {
                files: vec![FileDiff {
                    new_path: Some(rel.to_owned()),
                    ..Default::default()
                }],
                too_large: true,
            });
        }
        return Ok(all_added(rel, &std::fs::read(&path)?));
    }
    let args = |of: DiffOf| {
        let mut args = vec![
            "-c",
            "core.quotePath=false",
            "--literal-pathspecs",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--find-renames",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ];
        match of {
            DiffOf::Staged => args.push("--cached"),
            DiffOf::Head => args.push("HEAD"),
            _ => {}
        }
        args.push("--");
        args.extend_from_slice(paths);
        args
    };
    let out = match super::run(root, &args(of)) {
        // Before the first commit there is no HEAD: everything is staged.
        Err(e) if of == DiffOf::Head && e.to_string().contains("HEAD") => {
            super::run(root, &args(DiffOf::Staged))?
        }
        r => r?,
    };
    if out.len() > MAX_BYTES {
        return Ok(Diff {
            files: Vec::new(),
            too_large: true,
        });
    }
    let files = parse(&String::from_utf8_lossy(&out));
    let lines: usize = files
        .iter()
        .flat_map(|f| &f.hunks)
        .map(|h| h.lines.len())
        .sum();
    Ok(Diff {
        too_large: lines > MAX_LINES,
        files,
    })
}

/// For each line of `lines`, the line it replaces or is replaced by: in a
/// run of removed lines followed by added ones, the first removed goes
/// with the first added and so on.
pub fn partners(lines: &[DiffLine]) -> Vec<Option<usize>> {
    let mut out = vec![None; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Removed {
            i += 1;
            continue;
        }
        let removed = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let added = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        for k in 0..(added - removed).min(i - added) {
            out[removed + k] = Some(added + k);
            out[added + k] = Some(removed + k);
        }
    }
    out
}

/// The rows of a side-by-side view of `lines`: old on the left, new on the
/// right, as indexes into `lines`. Context lines fill both sides; removed
/// and added lines sit next to the ones they replace.
pub fn side_by_side(lines: &[DiffLine]) -> Vec<(Option<usize>, Option<usize>)> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind == LineKind::Context {
            rows.push((Some(i), Some(i)));
            i += 1;
            continue;
        }
        let removed = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let added = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        let (r, a) = (added - removed, i - added);
        for k in 0..r.max(a) {
            rows.push(((k < r).then_some(removed + k), (k < a).then_some(added + k)));
        }
    }
    rows
}

/// What differs between a line and the one replacing it: the byte ranges
/// left when their common start and end are taken off. `None` when they
/// have nothing in common (the whole line changed).
pub fn changed_ranges(old: &str, new: &str) -> Option<(Range<usize>, Range<usize>)> {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let (o, n) = (&old[prefix..], &new[prefix..]);
    let suffix: usize = o
        .chars()
        .rev()
        .zip(n.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    if prefix + suffix == 0 {
        return None;
    }
    Some((prefix..old.len() - suffix, prefix..new.len() - suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines of `git diff` output.
    fn out(lines: &[&str]) -> String {
        lines.join("\n") + "\n"
    }

    fn modified() -> String {
        out(&[
            "diff --git a/src/lib.rs b/src/lib.rs",
            "index 1111111..2222222 100644",
            "--- a/src/lib.rs",
            "+++ b/src/lib.rs",
            "@@ -1,4 +1,4 @@ mod x",
            " one",
            "-two",
            "+TWO",
            " three",
            "-four",
            "\\ No newline at end of file",
            "+four",
            "@@ -10 +10,2 @@",
            " ten",
            "+eleven",
        ])
    }

    #[test]
    fn parses_hunks_and_line_numbers() {
        let files = parse(&modified());
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.old_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(f.new_path.as_deref(), Some("src/lib.rs"));
        assert!(!f.binary && !f.renamed);
        assert_eq!(f.hunks.len(), 2);
        let h = &f.hunks[0];
        assert_eq!(h.header(), "@@ -1,4 +1,4 @@ mod x");
        let kinds: Vec<LineKind> = h.lines.iter().map(|l| l.kind).collect();
        use LineKind::*;
        assert_eq!(kinds, [Context, Removed, Added, Context, Removed, Added]);
        let nums: Vec<(Option<u32>, Option<u32>)> =
            h.lines.iter().map(|l| (l.old, l.new)).collect();
        assert_eq!(
            nums,
            [
                (Some(1), Some(1)),
                (Some(2), None),
                (None, Some(2)),
                (Some(3), Some(3)),
                (Some(4), None),
                (None, Some(4)),
            ]
        );
        // The old file had no newline at its end; the new one has.
        assert!(h.lines[4].no_newline);
        assert!(!h.lines[5].no_newline);
        // "@@ -10 +10,2 @@": a missing count is 1.
        let h = &f.hunks[1];
        assert_eq!((h.old_start, h.old_count, h.new_count), (10, 1, 2));
        assert_eq!(h.lines[1].new, Some(11));
        assert_eq!((f.added(), f.removed()), (3, 2));
    }

    #[test]
    fn a_missing_newline_at_the_very_end() {
        let out = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\n\\ No newline at end of file\n";
        let h = &parse(out)[0].hunks[0];
        assert!(!h.lines[0].no_newline);
        assert!(h.lines[1].no_newline);
    }

    #[test]
    fn parses_renames_new_deleted_and_binary_files() {
        let out = "diff --git a/old name.txt b/new name.txt\n\
similarity index 90%\n\
rename from old name.txt\n\
rename to new name.txt\n\
index 1..2 100644\n\
--- a/old name.txt\t\n\
+++ b/new name.txt\t\n\
@@ -1 +1 @@\n\
-a\n\
+b\n\
diff --git a/new.txt b/new.txt\n\
new file mode 100644\n\
index 0000000..3333333\n\
--- /dev/null\n\
+++ b/new.txt\n\
@@ -0,0 +1 @@\n\
+hello\n\
diff --git a/gone.txt b/gone.txt\n\
deleted file mode 100644\n\
--- a/gone.txt\n\
+++ /dev/null\n\
@@ -1 +0,0 @@\n\
-bye\n\
diff --git a/logo.png b/logo.png\n\
index 4..5 100644\n\
Binary files a/logo.png and b/logo.png differ\n\
diff --git a/run.sh b/run.sh\n\
old mode 100644\n\
new mode 100755\n\
diff --git \"a/\\303\\244.txt\" \"b/\\303\\244.txt\"\n\
--- \"a/\\303\\244.txt\"\n\
+++ \"b/\\303\\244.txt\"\n\
@@ -1 +1 @@\n\
-1\n\
+2\n";
        let f = parse(out);
        assert_eq!(f.len(), 6);
        assert!(f[0].renamed);
        assert_eq!(f[0].similarity, Some(90));
        assert_eq!(f[0].old_path.as_deref(), Some("old name.txt"));
        assert_eq!(f[0].new_path.as_deref(), Some("new name.txt"));
        assert_eq!(f[0].hunks[0].lines.len(), 2);
        assert_eq!(f[1].old_path, None);
        assert_eq!(f[1].new_path.as_deref(), Some("new.txt"));
        assert_eq!(f[1].hunks[0].lines[0].new, Some(1));
        assert_eq!(f[2].new_path, None);
        assert_eq!(f[2].hunks[0].lines[0].old, Some(1));
        assert!(f[3].binary && f[3].hunks.is_empty());
        assert_eq!(
            (f[4].old_mode.as_deref(), f[4].new_mode.as_deref()),
            (Some("100644"), Some("100755"))
        );
        assert!(f[4].hunks.is_empty());
        assert_eq!(f[5].new_path.as_deref(), Some("ä.txt"));
    }

    #[test]
    fn lines_that_look_like_headers_inside_hunks() {
        // A removed "-- x" line is "--- x"; an added "++ y" is "+++ y".
        let out = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n--- x\n+++ y\n same\n";
        let f = &parse(out)[0];
        assert_eq!(f.old_path.as_deref(), Some("a"));
        let h = &f.hunks[0];
        assert_eq!(h.lines.len(), 3);
        assert_eq!(h.lines[0].text, "-- x");
        assert_eq!(h.lines[1].text, "++ y");
    }

    #[test]
    fn untracked_files_are_all_added() {
        let d = all_added("n.txt", b"a\r\nb\nc");
        let h = &d.files[0].hunks[0];
        assert_eq!(h.lines.len(), 3);
        assert!(h.lines.iter().all(|l| l.kind == LineKind::Added));
        assert_eq!(h.lines[2].new, Some(3));
        assert!(h.lines[2].no_newline);
        assert_eq!(h.lines[0].text, "a\r");
        assert!(all_added("e.txt", b"").is_empty());
        assert!(all_added("b.bin", b"PK\0\x01").files[0].binary);
        assert!(!all_added("x", b"x\n").files[0].hunks[0].lines[0].no_newline);
    }

    fn line(kind: LineKind, text: &str) -> DiffLine {
        DiffLine {
            kind,
            old: None,
            new: None,
            text: text.into(),
            no_newline: false,
        }
    }

    #[test]
    fn pairs_replaced_lines() {
        use LineKind::*;
        let lines = [
            line(Context, "a"),
            line(Removed, "b"),
            line(Removed, "c"),
            line(Added, "B"),
            line(Context, "d"),
            line(Added, "e"),
        ];
        assert_eq!(partners(&lines), [None, Some(3), None, Some(1), None, None]);
        assert_eq!(
            side_by_side(&lines),
            [
                (Some(0), Some(0)),
                (Some(1), Some(3)),
                (Some(2), None),
                (Some(4), Some(4)),
                (None, Some(5)),
            ]
        );
    }

    #[test]
    fn finds_what_changed_within_a_line() {
        let (o, n) = changed_ranges("let x = 1;", "let x = 42;").unwrap();
        assert_eq!((o, n), (8..9, 8..10));
        // Several bytes per character.
        let (o, n) = changed_ranges("größe: ä", "größe: ü").unwrap();
        assert_eq!(&"größe: ä"[o], "ä");
        assert_eq!(&"größe: ü"[n], "ü");
        assert_eq!(changed_ranges("abc", "xyz"), None);
        // Only something added.
        let (o, n) = changed_ranges("ab", "aXb").unwrap();
        assert_eq!((o, n), (1..1, 1..2));
    }
}
