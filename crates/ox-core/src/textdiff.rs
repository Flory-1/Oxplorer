//! Comparing two text files line by line, in-process: Myers' diff
//! ([`myers`]) turned into the files/hunks/lines model of Git's diffs
//! ([`crate::git::diff`]), so the same view draws both.
//!
//! Lines are compared without their line endings (CRLF and LF lines are
//! the same; that the endings differ is told separately), each file
//! decoded in its own encoding. A missing newline at the end counts as a
//! difference, as in Git, unless whitespace is ignored.

mod myers;

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::git::diff::{looks_binary, Diff, DiffLine, FileDiff, Hunk, LineKind};
use crate::textinfo::{self, Encoding, LineEndings};

pub use myers::Edit;

/// Larger files aren't compared line by line.
pub const MAX_BYTES: u64 = crate::git::diff::MAX_BYTES as u64;
/// Nor files with more lines.
pub const MAX_LINES: usize = crate::git::diff::MAX_LINES;
/// Unchanged lines shown around each change.
pub const CONTEXT: usize = 3;
/// After this long the diff stops looking for the shortest result.
const TIME_LIMIT: Duration = Duration::from_secs(3);

/// How lines are compared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Lines that differ only in spaces and tabs are the same.
    pub ignore_whitespace: bool,
}

/// A line of a text, without its line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line<'a> {
    pub text: &'a str,
    /// The text ends with this line, without a line ending.
    pub no_newline: bool,
}

/// The lines of `text`, split at CRLF, LF and CR.
pub fn lines(text: &str) -> Vec<Line<'_>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < b.len() {
        let end = match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => Some(2),
            b'\r' | b'\n' => Some(1),
            _ => None,
        };
        if let Some(len) = end {
            out.push(Line {
                text: &text[start..i],
                no_newline: false,
            });
            i += len;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < b.len() {
        out.push(Line {
            text: &text[start..],
            no_newline: true,
        });
    }
    out
}

/// The edit script between two lists of lines.
pub fn diff_lines<'a>(old: &[Line<'a>], new: &[Line<'a>], opts: Options) -> Vec<Edit> {
    // Each distinct line becomes a number: comparing those is quick.
    let mut ids: HashMap<(Cow<'a, str>, bool), u32> = HashMap::new();
    let mut id = |l: &Line<'a>| -> u32 {
        let key = if opts.ignore_whitespace {
            let text: String = l.text.split_whitespace().collect();
            (Cow::Owned(text), false)
        } else {
            (Cow::Borrowed(l.text), l.no_newline)
        };
        let next = ids.len() as u32;
        *ids.entry(key).or_insert(next)
    };
    let a: Vec<u32> = old.iter().map(&mut id).collect();
    let b: Vec<u32> = new.iter().map(&mut id).collect();
    myers::diff(&a, &b, Some(Instant::now() + TIME_LIMIT))
}

fn diff_line(kind: LineKind, old: Option<usize>, new: Option<usize>, line: &Line) -> DiffLine {
    DiffLine {
        kind,
        old: old.map(|i| i as u32 + 1),
        new: new.map(|i| i as u32 + 1),
        text: line.text.to_owned(),
        no_newline: line.no_newline,
    }
}

/// The hunks of an edit script: each change with `context` unchanged lines
/// around it; changes closer together than that share a hunk.
pub fn hunks(old: &[Line], new: &[Line], edits: &[Edit], context: usize) -> Vec<Hunk> {
    let changes: Vec<usize> = (0..edits.len())
        .filter(|&i| !matches!(edits[i], Edit::Same(..)))
        .collect();
    // Lines of each file before each edit.
    let mut before = Vec::with_capacity(edits.len() + 1);
    let (mut o, mut n) = (0usize, 0usize);
    for e in edits {
        before.push((o, n));
        match e {
            Edit::Same(..) => (o, n) = (o + 1, n + 1),
            Edit::Removed(_) => o += 1,
            Edit::Added(_) => n += 1,
        }
    }
    before.push((o, n));
    let mut out = Vec::new();
    let mut i = 0;
    while i < changes.len() {
        let mut last = changes[i];
        let mut j = i + 1;
        while j < changes.len() && changes[j] - last <= 2 * context + 1 {
            last = changes[j];
            j += 1;
        }
        let from = changes[i].saturating_sub(context);
        let to = (last + context + 1).min(edits.len());
        let lines: Vec<DiffLine> = edits[from..to]
            .iter()
            .map(|e| match *e {
                Edit::Same(a, b) => diff_line(LineKind::Context, Some(a), Some(b), &new[b]),
                Edit::Removed(a) => diff_line(LineKind::Removed, Some(a), None, &old[a]),
                Edit::Added(b) => diff_line(LineKind::Added, None, Some(b), &new[b]),
            })
            .collect();
        let (o0, n0) = before[from];
        let (o1, n1) = before[to];
        let (old_count, new_count) = ((o1 - o0) as u32, (n1 - n0) as u32);
        // As in unified diffs: an empty range starts at the line before it.
        let start = |at: usize, count: u32| at as u32 + u32::from(count > 0);
        out.push(Hunk {
            old_start: start(o0, old_count),
            old_count,
            new_start: start(n0, new_count),
            new_count,
            section: String::new(),
            lines,
        });
        i = j;
    }
    out
}

/// The diff of two texts, as one file of a [`Diff`].
pub fn compare(old: &str, new: &str, opts: Options) -> FileDiff {
    let (a, b) = (lines(old), lines(new));
    let edits = diff_lines(&a, &b, opts);
    FileDiff {
        hunks: hunks(&a, &b, &edits, CONTEXT),
        ..Default::default()
    }
}

/// What was found out about one of the files compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Side {
    pub size: u64,
    pub encoding: Encoding,
    pub endings: LineEndings,
    pub binary: bool,
}

/// Two files compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub old: Side,
    pub new: Side,
    /// The files have exactly the same bytes.
    pub identical: bool,
    /// Where the bytes first differ (also for binary files).
    pub first_difference: Option<u64>,
    /// The lines that differ; `too_large` when a file is too big to compare
    /// line by line. Empty for binary files.
    pub diff: Diff,
}

impl Comparison {
    pub fn binary(&self) -> bool {
        self.old.binary || self.new.binary
    }
}

/// Where the contents of two readers first differ (`None`: they're the same).
pub fn first_difference(mut a: impl Read, mut b: impl Read) -> io::Result<Option<u64>> {
    let (mut ba, mut bb) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    let mut at = 0u64;
    loop {
        let na = read_full(&mut a, &mut ba)?;
        let nb = read_full(&mut b, &mut bb)?;
        let n = na.min(nb);
        if let Some(i) = (0..n).find(|&i| ba[i] != bb[i]) {
            return Ok(Some(at + i as u64));
        }
        if na != nb {
            return Ok(Some(at + n as u64));
        }
        if na == 0 {
            return Ok(None);
        }
        at += n as u64;
    }
}

/// Fill `buf` as far as the reader goes.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// The start of a file (up to [`MAX_BYTES`] and one byte more) and its size.
fn read_start(path: &Path) -> io::Result<(Vec<u8>, u64)> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut buf = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut buf)?;
    let size = size.max(buf.len() as u64);
    Ok((buf, size))
}

/// What a file's start says about it, and its text.
fn side(bytes: &[u8], size: u64) -> (Side, String) {
    let encoding = textinfo::encoding(bytes);
    let utf16 = matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be);
    let binary = !utf16 && looks_binary(bytes);
    let text = if binary {
        String::new()
    } else {
        textinfo::decode(bytes, encoding)
    };
    let side = Side {
        size,
        encoding,
        endings: textinfo::line_endings(&text),
        binary,
    };
    (side, text)
}

/// Compare the files `old` and `new`.
pub fn compare_files(old: &Path, new: &Path, opts: Options) -> io::Result<Comparison> {
    let first_difference = first_difference(File::open(old)?, File::open(new)?)?;
    let (a, a_size) = read_start(old)?;
    let (b, b_size) = read_start(new)?;
    let too_big = a_size > MAX_BYTES || b_size > MAX_BYTES;
    let (old_side, old_text) = side(&a[..a.len().min(MAX_BYTES as usize)], a_size);
    let (new_side, new_text) = side(&b[..b.len().min(MAX_BYTES as usize)], b_size);
    let mut diff = Diff::default();
    if !(old_side.binary || new_side.binary) {
        let too_long = |t: &str| t.lines().nth(MAX_LINES).is_some();
        if too_big || too_long(&old_text) || too_long(&new_text) {
            diff.too_large = true;
        } else if first_difference.is_some() {
            diff.files.push(compare(&old_text, &new_text, opts));
        }
    }
    Ok(Comparison {
        old: old_side,
        new: new_side,
        identical: first_difference.is_none(),
        first_difference,
        diff,
    })
}

#[cfg(test)]
mod tests;
