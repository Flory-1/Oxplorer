use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::duplicates::{self, DupFile, DupGroup, Keep};
use super::*;

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

fn dups(root: &Path, opts: ScanOptions) -> (Vec<(u64, Vec<String>)>, duplicates::Report) {
    let progress = Progress::default();
    let report = duplicates::find(root, opts, &AtomicBool::new(false), &progress);
    let groups = report
        .groups
        .iter()
        .map(|g| (g.size, g.files.iter().map(|f| rel(root, &f.path)).collect()))
        .collect();
    (groups, report)
}

fn empties(root: &Path, opts: ScanOptions) -> Vec<String> {
    let progress = Progress::default();
    let report = empty_folders::find(root, opts, &AtomicBool::new(false), &progress);
    assert!(!report.cancelled);
    report.folders.iter().map(|p| rel(root, p)).collect()
}

/// `n` bytes that differ from file to file only where `marker` says.
fn data(n: usize, marker: Option<(usize, u8)>) -> Vec<u8> {
    let mut v: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
    if let Some((at, b)) = marker {
        v[at] = b;
    }
    v
}

#[test]
fn finds_files_with_the_same_contents() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::create_dir_all(p.join("a/b")).unwrap();
    fs::write(p.join("one.txt"), "hello").unwrap();
    fs::write(p.join("a/copy.txt"), "hello").unwrap();
    fs::write(p.join("a/b/again.txt"), "hello").unwrap();
    // Same size, other contents.
    fs::write(p.join("a/other.txt"), "world").unwrap();
    fs::write(p.join("unique.bin"), "something else").unwrap();
    let (groups, report) = dups(p, ScanOptions::default());
    assert_eq!(
        groups,
        [(
            5,
            vec![
                "a/b/again.txt".into(),
                "a/copy.txt".into(),
                "one.txt".into()
            ]
        )]
    );
    assert_eq!(report.wasted(), 10);
    assert!(!report.cancelled);
}

#[test]
fn big_files_that_differ_in_the_middle_only_are_not_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let n = 200 * 1024;
    fs::write(p.join("a.bin"), data(n, None)).unwrap();
    fs::write(p.join("b.bin"), data(n, None)).unwrap();
    // Same start and end, so only the full hash tells them apart.
    fs::write(p.join("c.bin"), data(n, Some((n / 2, 0xff)))).unwrap();
    // Files between one and two parts long are compared in full at once.
    let m = 20 * 1024;
    fs::write(p.join("d.bin"), data(m, None)).unwrap();
    fs::write(p.join("e.bin"), data(m, Some((17 * 1024, 0xff)))).unwrap();
    let (groups, report) = dups(p, ScanOptions::default());
    assert_eq!(groups, [(n as u64, vec!["a.bin".into(), "b.bin".into()])]);
    assert_eq!(report.wasted(), n as u64);
}

#[test]
fn unique_sizes_are_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("a"), data(100_000, None)).unwrap();
    fs::write(p.join("b"), data(100_001, None)).unwrap();
    let progress = Progress::default();
    let report = duplicates::find(
        p,
        ScanOptions::default(),
        &AtomicBool::new(false),
        &progress,
    );
    assert!(report.groups.is_empty());
    assert_eq!(Progress::get(&progress.files), 2);
    assert_eq!(Progress::get(&progress.hashed), 0);
}

#[test]
fn empty_files_only_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("a"), "").unwrap();
    fs::write(p.join("b"), "").unwrap();
    assert!(dups(p, ScanOptions::default()).0.is_empty());
    let opts = ScanOptions {
        include_empty_files: true,
        ..Default::default()
    };
    assert_eq!(dups(p, opts).0, [(0, vec!["a".into(), "b".into()])]);
}

#[test]
fn hard_links_are_not_copies() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("a.txt"), "hello").unwrap();
    fs::hard_link(p.join("a.txt"), p.join("b.txt")).unwrap();
    assert!(dups(p, ScanOptions::default()).0.is_empty());
    // A real copy next to them: one of the links and the copy.
    fs::write(p.join("c.txt"), "hello").unwrap();
    let (groups, _) = dups(p, ScanOptions::default());
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].1.len(), 2);
    assert!(groups[0].1.contains(&"c.txt".to_string()));
}

#[cfg(windows)]
#[test]
fn hidden_files_follow_the_setting() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("a"), "same").unwrap();
    fs::write(p.join("b"), "same").unwrap();
    crate::details::set_attributes(&p.join("b"), None, Some(true)).unwrap();
    assert!(dups(p, ScanOptions::default()).0.is_empty());
    let opts = ScanOptions {
        show_hidden: true,
        ..Default::default()
    };
    assert_eq!(dups(p, opts).0.len(), 1);
}

#[cfg(windows)]
#[test]
fn junctions_are_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::create_dir(p.join("real")).unwrap();
    fs::write(p.join("real/file"), "contents").unwrap();
    crate::links::create(
        crate::links::LinkKind::Junction,
        &p.join("link"),
        &p.join("real"),
    )
    .unwrap();
    // Through the junction the same file would show up twice.
    assert!(dups(p, ScanOptions::default()).0.is_empty());
    // A folder with only a junction in it isn't empty.
    fs::create_dir(p.join("holder")).unwrap();
    crate::links::create(
        crate::links::LinkKind::Junction,
        &p.join("holder/link"),
        &p.join("real"),
    )
    .unwrap();
    assert!(empties(p, ScanOptions::default()).is_empty());
}

#[test]
fn a_set_cancel_flag_stops_the_scan() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a"), "x").unwrap();
    fs::write(dir.path().join("b"), "x").unwrap();
    let progress = Progress::default();
    let cancel = AtomicBool::new(true);
    let report = duplicates::find(dir.path(), ScanOptions::default(), &cancel, &progress);
    assert!(report.cancelled);
    assert!(report.groups.is_empty());
    let report = empty_folders::find(dir.path(), ScanOptions::default(), &cancel, &progress);
    assert!(report.cancelled);
}

#[test]
fn the_scan_runs_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a"), "x").unwrap();
    fs::write(dir.path().join("b"), "x").unwrap();
    let handle = start(
        dir.path().to_path_buf(),
        CleanupMode::Duplicates,
        ScanOptions::default(),
        || {},
    );
    let (outcome, _) = handle.result.recv().unwrap();
    let Outcome::Duplicates(report) = outcome else {
        panic!("not a duplicates report")
    };
    assert_eq!(report.groups.len(), 1);
    assert_eq!(Progress::get(&handle.progress.candidates), 2);
}

fn group(size: u64, files: &[(&str, Option<u64>)]) -> DupGroup {
    DupGroup {
        size,
        files: files
            .iter()
            .map(|&(p, secs)| DupFile {
                path: p.into(),
                modified: secs.map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s)),
            })
            .collect(),
    }
}

#[test]
fn select_all_but_the_newest_or_oldest() {
    let report = duplicates::Report {
        groups: vec![
            group(10, &[("a", Some(5)), ("b", Some(9)), ("c", None)]),
            // A tie keeps the first.
            group(3, &[("d", Some(1)), ("e", Some(1))]),
        ],
        ..Default::default()
    };
    let mut newest = report.all_but(Keep::Newest);
    newest.sort();
    assert_eq!(newest, [Path::new("a"), Path::new("c"), Path::new("e")]);
    let mut oldest = report.all_but(Keep::Oldest);
    oldest.sort();
    assert_eq!(oldest, [Path::new("a"), Path::new("b"), Path::new("e")]);
    let selection: HashSet<_> = newest.into_iter().collect();
    assert_eq!(report.selected(&selection), (3, 23));
    assert_eq!(report.fully_selected(&selection), 0);
}

#[test]
fn every_copy_selected_and_removing_deleted_files() {
    let mut report = duplicates::Report {
        groups: vec![
            group(10, &[("a", None), ("b", None), ("c", None)]),
            group(3, &[("d", None), ("e", None)]),
        ],
        ..Default::default()
    };
    let selection: HashSet<_> = ["d", "e", "a"].map(PathBuf::from).into_iter().collect();
    assert_eq!(report.fully_selected(&selection), 1);
    report.remove(&selection);
    // "d" and "e" are gone, and a group of one is no group.
    assert_eq!(report.groups, [group(10, &[("b", None), ("c", None)])]);
}

#[test]
fn finds_the_top_most_empty_folders() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::create_dir_all(p.join("empty/nested/deeper")).unwrap();
    fs::create_dir_all(p.join("only_junk/sub")).unwrap();
    fs::write(p.join("only_junk/desktop.ini"), "[x]").unwrap();
    fs::write(p.join("only_junk/sub/Thumbs.db"), "x").unwrap();
    fs::create_dir_all(p.join("full/empty_inside")).unwrap();
    fs::create_dir_all(p.join("full/deep/a")).unwrap();
    fs::write(p.join("full/deep/a/file.txt"), "x").unwrap();
    fs::write(p.join("top.txt"), "x").unwrap();
    assert_eq!(
        empties(p, ScanOptions::default()),
        ["empty", "full/empty_inside", "only_junk"]
    );
}

#[test]
fn the_root_itself_is_never_listed() {
    let dir = tempfile::tempdir().unwrap();
    assert!(empties(dir.path(), ScanOptions::default()).is_empty());
    fs::create_dir(dir.path().join("a")).unwrap();
    assert_eq!(empties(dir.path(), ScanOptions::default()), ["a"]);
}

#[cfg(windows)]
#[test]
fn hidden_files_keep_a_folder_and_hidden_folders_are_listed_when_shown() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::create_dir(p.join("has_hidden")).unwrap();
    fs::write(p.join("has_hidden/secret"), "x").unwrap();
    crate::details::set_attributes(&p.join("has_hidden/secret"), None, Some(true)).unwrap();
    fs::create_dir(p.join("hidden_empty")).unwrap();
    crate::details::set_attributes(&p.join("hidden_empty"), None, Some(true)).unwrap();
    assert!(empties(p, ScanOptions::default()).is_empty());
    let opts = ScanOptions {
        show_hidden: true,
        ..Default::default()
    };
    assert_eq!(empties(p, opts), ["hidden_empty"]);
}
