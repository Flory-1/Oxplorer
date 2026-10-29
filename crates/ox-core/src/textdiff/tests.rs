use super::*;
use std::fs;

/// `edits` applied to `old`, giving what should be `new`.
fn apply<'a>(old: &[&'a str], new: &[&'a str], edits: &[Edit]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let (mut o, mut n) = (0, 0);
    for e in edits {
        match *e {
            Edit::Same(a, b) => {
                assert_eq!((a, b), (o, n), "in order");
                assert_eq!(old[a], new[b]);
                out.push(old[a]);
                o += 1;
                n += 1;
            }
            Edit::Removed(a) => {
                assert_eq!(a, o);
                o += 1;
            }
            Edit::Added(b) => {
                assert_eq!(b, n);
                out.push(new[b]);
                n += 1;
            }
        }
    }
    assert_eq!((o, n), (old.len(), new.len()), "everything used");
    out
}

fn changes(edits: &[Edit]) -> usize {
    edits
        .iter()
        .filter(|e| !matches!(e, Edit::Same(..)))
        .count()
}

fn d(old: &[&str], new: &[&str]) -> Vec<Edit> {
    myers::diff(old, new, None)
}

#[test]
fn myers_finds_the_shortest_script() {
    // The paper's example: ABCABBA → CBABAC takes 5 edits.
    let old: Vec<&str> = "A B C A B B A".split(' ').collect();
    let new: Vec<&str> = "C B A B A C".split(' ').collect();
    let e = d(&old, &new);
    assert_eq!(apply(&old, &new, &e), new);
    assert_eq!(changes(&e), 5);
}

#[test]
fn insertions_deletions_and_replacements() {
    let e = d(&["a", "b"], &["a", "x", "b", "y"]);
    assert_eq!(
        e,
        [
            Edit::Same(0, 0),
            Edit::Added(1),
            Edit::Same(1, 2),
            Edit::Added(3)
        ]
    );
    let e = d(&["a", "b", "c"], &["c"]);
    assert_eq!(e, [Edit::Removed(0), Edit::Removed(1), Edit::Same(2, 0)]);
    // A replaced line: removed first, then added.
    let e = d(&["a", "b", "c"], &["a", "B", "c"]);
    assert_eq!(
        e,
        [
            Edit::Same(0, 0),
            Edit::Removed(1),
            Edit::Added(1),
            Edit::Same(2, 2)
        ]
    );
    // Empty on either side.
    assert_eq!(d(&[], &["a"]), [Edit::Added(0)]);
    assert_eq!(d(&["a"], &[]), [Edit::Removed(0)]);
    assert!(d(&[], &[]).is_empty());
}

#[test]
fn a_moved_block_is_removed_in_one_place_and_added_in_another() {
    let old = ["1", "2", "3", "x", "y", "4", "5"];
    let new = ["1", "x", "y", "2", "3", "4", "5"];
    let e = d(&old, &new);
    assert_eq!(apply(&old, &new, &e), new);
    assert_eq!(changes(&e), 4);
}

#[test]
fn random_sequences_round_trip() {
    // A small deterministic generator: no test dependency needed.
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let words = ["a", "b", "c", "d"];
    for _ in 0..300 {
        let old: Vec<&str> = (0..next() % 30)
            .map(|_| words[next() as usize % 4])
            .collect();
        let new: Vec<&str> = (0..next() % 30)
            .map(|_| words[next() as usize % 4])
            .collect();
        let e = d(&old, &new);
        assert_eq!(apply(&old, &new, &e), new, "{old:?} → {new:?}");
        // Never worse than the longest-common-subsequence bound.
        assert_eq!(
            changes(&e),
            old.len() + new.len() - 2 * lcs(&old, &new),
            "{old:?} → {new:?}"
        );
    }
}

/// Length of the longest common subsequence (dynamic programming).
fn lcs(a: &[&str], b: &[&str]) -> usize {
    let mut t = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            t[i][j] = if a[i] == b[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    t[0][0]
}

#[test]
fn a_deadline_still_gives_a_valid_script() {
    let old: Vec<String> = (0..200).map(|i| format!("{}", i * 7 % 13)).collect();
    let new: Vec<String> = (0..200).map(|i| format!("{}", i * 5 % 11)).collect();
    let e = myers::diff(&old, &new, Some(Instant::now()));
    let old: Vec<&str> = old.iter().map(String::as_str).collect();
    let new: Vec<&str> = new.iter().map(String::as_str).collect();
    assert_eq!(apply(&old, &new, &e), new);
}

#[test]
fn lines_are_split_at_every_line_ending() {
    let l = lines("a\r\nb\nc\rd");
    let texts: Vec<&str> = l.iter().map(|l| l.text).collect();
    assert_eq!(texts, ["a", "b", "c", "d"]);
    assert!(l[3].no_newline && !l[2].no_newline);
    assert!(lines("").is_empty());
    assert_eq!(lines("\n").len(), 1);
    assert!(!lines("x\n")[0].no_newline);
}

fn kinds(h: &Hunk) -> String {
    h.lines
        .iter()
        .map(|l| match l.kind {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        })
        .collect()
}

/// Lines 1–20, with line `i` given by `f` (`None`: left out).
fn numbered(f: impl Fn(u32) -> Option<String>) -> String {
    (1..=20).filter_map(|i| f(i).map(|l| l + "\n")).collect()
}

#[test]
fn hunks_have_context_and_git_style_ranges() {
    let old = numbered(|i| Some(i.to_string()));
    let new = numbered(|i| match i {
        5 => Some("five".into()),
        16 => None,
        i => Some(i.to_string()),
    });
    let f = compare(&old, &new, Options::default());
    assert_eq!(f.hunks.len(), 2);
    let h = &f.hunks[0];
    assert_eq!(h.header(), "@@ -2,7 +2,7 @@");
    assert_eq!(kinds(h), "   -+   ");
    assert_eq!(h.lines[3].text, "5");
    assert_eq!((h.lines[3].old, h.lines[3].new), (Some(5), None));
    assert_eq!((h.lines[4].old, h.lines[4].new), (None, Some(5)));
    assert_eq!((h.lines[0].old, h.lines[0].new), (Some(2), Some(2)));
    let h = &f.hunks[1];
    assert_eq!(h.header(), "@@ -13,7 +13,6 @@");
    assert_eq!(kinds(h), "   -   ");
    assert_eq!((f.added(), f.removed()), (1, 2));
}

#[test]
fn close_changes_share_a_hunk() {
    let old = numbered(|i| Some(i.to_string()));
    let changed = |a: u32, b: u32| {
        numbered(move |i| {
            Some(if i == a || i == b {
                "x".into()
            } else {
                i.to_string()
            })
        })
    };
    // Six unchanged lines between: still one hunk; seven: two.
    let six = changed(5, 12);
    assert_eq!(compare(&old, &six, Options::default()).hunks.len(), 1);
    let seven = changed(5, 13);
    assert_eq!(compare(&old, &seven, Options::default()).hunks.len(), 2);
}

#[test]
fn empty_files() {
    let f = compare("", "a\nb\n", Options::default());
    assert_eq!(f.hunks.len(), 1);
    assert_eq!(f.hunks[0].header(), "@@ -0,0 +1,2 @@");
    assert_eq!(kinds(&f.hunks[0]), "++");
    let f = compare("a\n", "", Options::default());
    assert_eq!(f.hunks[0].header(), "@@ -1,1 +0,0 @@");
    assert!(compare("", "", Options::default()).hunks.is_empty());
}

#[test]
fn a_missing_newline_at_the_end_is_a_change() {
    let f = compare("a\nb\n", "a\nb", Options::default());
    assert_eq!(kinds(&f.hunks[0]), " -+");
    let added = &f.hunks[0].lines[2];
    assert!(added.no_newline && !f.hunks[0].lines[1].no_newline);
    // Ignoring whitespace, it isn't.
    let opts = Options {
        ignore_whitespace: true,
    };
    assert!(compare("a\nb\n", "a\nb", opts).hunks.is_empty());
}

#[test]
fn whitespace_and_line_endings() {
    let old = "fn main() {\n    x();\n}\n";
    let new = "fn main() {\r\n\tx( );\r\n}\r\n";
    // Line endings alone don't count; the indentation does.
    let f = compare(old, new, Options::default());
    assert_eq!(kinds(&f.hunks[0]), " -+ ");
    let opts = Options {
        ignore_whitespace: true,
    };
    assert!(compare(old, new, opts).hunks.is_empty());
    assert!(compare("a\n", "a\r\n", Options::default()).hunks.is_empty());
}

#[test]
fn bytes_are_compared_to_the_first_difference() {
    let fd = |a: &[u8], b: &[u8]| first_difference(a, b).unwrap();
    assert_eq!(fd(b"abc", b"abc"), None);
    assert_eq!(fd(b"abc", b"abd"), Some(2));
    assert_eq!(fd(b"abc", b"abcd"), Some(3));
    assert_eq!(fd(b"", b""), None);
    let big = vec![7u8; 200_000];
    let mut other = big.clone();
    other[150_000] = 8;
    assert_eq!(fd(&big, &other), Some(150_000));
}

#[test]
fn files_are_compared() {
    let dir = tempfile::tempdir().unwrap();
    let path = |n: &str| dir.path().join(n);
    fs::write(path("a.txt"), "one\ntwo\nthree\n").unwrap();
    fs::write(path("b.txt"), "one\r\n2\r\nthree\r\n").unwrap();
    let c = compare_files(&path("a.txt"), &path("b.txt"), Options::default()).unwrap();
    assert!(!c.identical && !c.binary());
    assert_eq!(
        (c.old.endings, c.new.endings),
        (LineEndings::Lf, LineEndings::Crlf)
    );
    assert_eq!(c.diff.files.len(), 1);
    assert_eq!(kinds(&c.diff.files[0].hunks[0]), " -+ ");

    // UTF-16 against UTF-8: decoded each in its own encoding.
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(
        "one\ntwo\nthree\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    fs::write(path("c.txt"), utf16).unwrap();
    let c = compare_files(&path("a.txt"), &path("c.txt"), Options::default()).unwrap();
    assert_eq!(c.new.encoding, Encoding::Utf16Le);
    assert!(!c.identical && !c.binary());
    assert!(c.diff.is_empty(), "{:?}", c.diff);

    // The same bytes.
    fs::write(path("d.txt"), "one\ntwo\nthree\n").unwrap();
    let c = compare_files(&path("a.txt"), &path("d.txt"), Options::default()).unwrap();
    assert!(c.identical && c.diff.files.is_empty());

    // Binary: no lines, but where they differ.
    fs::write(path("e.bin"), [0u8, 1, 2, 3]).unwrap();
    fs::write(path("f.bin"), [0u8, 1, 9, 3]).unwrap();
    let c = compare_files(&path("e.bin"), &path("f.bin"), Options::default()).unwrap();
    assert!(c.binary() && c.diff.files.is_empty());
    assert_eq!(c.first_difference, Some(2));

    // Too many lines.
    let long: String = "x\n".repeat(MAX_LINES + 1);
    fs::write(path("g.txt"), &long).unwrap();
    let c = compare_files(&path("a.txt"), &path("g.txt"), Options::default()).unwrap();
    assert!(c.diff.too_large);
}
