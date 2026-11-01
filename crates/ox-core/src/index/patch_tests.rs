use std::sync::atomic::{AtomicBool, AtomicU64};

use super::super::IndexConfig;
use super::*;

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[test]
fn batch_coalesces_and_pairs_renames() {
    let mut b = Batch::default();
    assert!(b.is_empty());
    b.push(Signal::Changed(p("a")));
    b.push(Signal::Changed(p("a")));
    b.push(Signal::RenamedFrom(p("b")));
    b.push(Signal::RenamedTo(p("c")));
    // An old name without a new one (moved out of view) is a plain change.
    b.push(Signal::RenamedFrom(p("d")));
    b.push(Signal::Changed(p("e")));
    // A new name without an old one (moved in) too.
    b.push(Signal::RenamedTo(p("f")));
    assert_eq!(b.paths, vec![p("a"), p("d"), p("e"), p("f")]);
    assert_eq!(b.renames, vec![(p("b"), p("c"))]);
    assert!(!b.needs_rescan());

    let steps = b.steps();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].renames.len(), 1);
    assert_eq!(steps[1].paths.len(), 6, "all paths plus both rename ends");

    let mut other = Batch::default();
    other.push(Signal::Changed(p("a")));
    other.push(Signal::Changed(p("g")));
    b.merge(other);
    assert_eq!(b.paths.len(), 5);

    let mut lost = Batch::default();
    lost.push(Signal::Rescan);
    b.merge(lost);
    assert!(b.needs_rescan());
    assert!(!b.is_empty());
    b.push(Signal::Changed(p("h")));
    assert!(
        b.paths.is_empty(),
        "nothing to patch when rebuilding anyway"
    );

    let mut flood = Batch::default();
    for i in 0..=MAX_PATHS {
        flood.push(Signal::Changed(PathBuf::from(i.to_string())));
    }
    assert!(flood.needs_rescan());
}

fn hide(path: &Path) {
    #[cfg(windows)]
    std::process::Command::new("attrib")
        .arg("+h")
        .arg(path)
        .status()
        .unwrap();
    #[cfg(not(windows))]
    let _ = path;
}

fn config(root: &Path) -> IndexConfig {
    IndexConfig {
        roots: vec![root.to_path_buf()],
        exclude: vec!["node_modules".into()],
        threads: 2,
    }
}

fn build(cfg: &IndexConfig) -> FileIndex {
    FileIndex::build(cfg, &AtomicBool::new(false), &AtomicU64::new(0))
}

/// Every live item: path, kind and hidden flags, size and (for files) date.
/// Folder dates change with their contents and are only refreshed when the
/// folder itself is reported, so they are left out.
fn snapshot(idx: &FileIndex) -> Vec<(String, u8, u64, u32)> {
    let mut v: Vec<_> = (0..idx.entries.len() as u32)
        .filter(|&i| idx.entries[i as usize].flags & F_DELETED == 0)
        .map(|i| {
            let e = &idx.entries[i as usize];
            let mtime = if e.flags & F_DIR == 0 { e.mtime } else { 0 };
            (
                idx.path_of(i).to_string_lossy().into_owned(),
                e.flags & (F_DIR | F_HIDDEN),
                e.size,
                mtime,
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn live_changes_match_a_fresh_build() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("r");
    fs::create_dir_all(root.join("a/sub")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    fs::create_dir_all(root.join("gone/x")).unwrap();
    fs::create_dir_all(root.join("node_modules")).unwrap();
    fs::create_dir_all(root.join(".cache")).unwrap();
    fs::write(root.join("a/one.txt"), "1").unwrap();
    fs::write(root.join("a/sub/two.txt"), "22").unwrap();
    fs::write(root.join("gone/x/y.txt"), "y").unwrap();
    fs::write(root.join("keep.txt"), "k").unwrap();
    fs::write(root.join("old.txt"), "old").unwrap();
    hide(&root.join(".cache"));
    let cfg = config(&root);
    let exclude = vec!["node_modules".to_owned()];
    let mut idx = build(&cfg);

    fs::write(root.join("new.txt"), "hello").unwrap();
    fs::create_dir_all(root.join("newdir/deep")).unwrap();
    fs::write(root.join("newdir/deep/f.txt"), "f").unwrap();
    fs::write(root.join("newdir/g.txt"), "g").unwrap();
    fs::remove_dir_all(root.join("gone")).unwrap();
    fs::write(root.join("keep.txt"), "longer content").unwrap();
    fs::rename(root.join("old.txt"), root.join("b/renamed.txt")).unwrap();
    fs::rename(root.join("a"), root.join("b/a2")).unwrap();
    fs::write(root.join("node_modules/pkg.js"), "x").unwrap();
    fs::write(root.join(".cache/thumb.jpg"), "t").unwrap();

    let mut batch = Batch::default();
    for s in [
        Signal::Changed(root.join("new.txt")),
        // Only the deepest file: the new folders are found from it.
        Signal::Changed(root.join("newdir/deep/f.txt")),
        Signal::Changed(root.join("gone/x/y.txt")),
        Signal::Changed(root.join("gone")),
        Signal::Changed(root.join("keep.txt")),
        Signal::RenamedFrom(root.join("old.txt")),
        Signal::RenamedTo(root.join("b/renamed.txt")),
        Signal::RenamedFrom(root.join("a")),
        Signal::RenamedTo(root.join("b/a2")),
        Signal::Changed(root.join("node_modules/pkg.js")),
        Signal::Changed(root.join(".cache/thumb.jpg")),
        Signal::Changed(root.clone()),
        Signal::Changed(dir.path().join("outside.txt")),
    ] {
        batch.push(s);
    }
    assert!(idx.update(&batch, &exclude) > 0);
    assert_eq!(snapshot(&idx), snapshot(&build(&cfg)));

    // Applying the same changes again changes nothing.
    assert_eq!(idx.update(&batch, &exclude), 0);

    let thumb = (0..idx.entries.len() as u32)
        .find(|&i| idx.path_of(i) == root.join(".cache/thumb.jpg"))
        .unwrap();
    assert!(idx.file_entry(thumb).hidden, "inherits the hidden folder");
}

#[test]
fn live_additions_dont_double_the_buffers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("r");
    fs::create_dir_all(&root).unwrap();
    for i in 0..400 {
        fs::write(root.join(format!("file-with-a-long-name-{i}.txt")), "x").unwrap();
    }
    let cfg = config(&root);
    let mut idx = build(&cfg);
    assert_eq!(idx.entries.capacity(), idx.entries.len(), "built exactly");
    assert_eq!(idx.names.capacity(), idx.names.len(), "built exactly");

    fs::write(root.join("new.txt"), "n").unwrap();
    let mut batch = Batch::default();
    batch.push(Signal::Changed(root.join("new.txt")));
    assert_eq!(idx.update(&batch, &[]), 1);
    let (len, names) = (idx.entries.len(), idx.names.len());
    assert!(idx.entries.capacity() <= len + len / 32 + 1);
    assert!(idx.names.capacity() <= names + names / 32 + 8);
    assert_eq!(snapshot(&idx), snapshot(&build(&cfg)));
}

#[test]
fn renames_are_patched_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("r");
    fs::create_dir_all(root.join("big/inner")).unwrap();
    for i in 0..50 {
        fs::write(root.join(format!("big/inner/f{i}.txt")), "x").unwrap();
    }
    let cfg = config(&root);
    let mut idx = build(&cfg);
    let records = idx.entries.len();

    fs::rename(root.join("big"), root.join("bigger")).unwrap();
    let mut batch = Batch::default();
    batch.push(Signal::RenamedFrom(root.join("big")));
    batch.push(Signal::RenamedTo(root.join("bigger")));
    idx.update(&batch, &[]);

    assert_eq!(idx.entries.len(), records, "nothing was re-read");
    assert_eq!(idx.garbage, 1);
    assert_eq!(snapshot(&idx), snapshot(&build(&cfg)));
}

#[test]
fn compaction_and_saving_keep_a_patched_index() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("r");
    fs::create_dir_all(root.join("old")).unwrap();
    for i in 0..20 {
        fs::write(root.join(format!("old/f{i}.txt")), "x").unwrap();
    }
    fs::write(root.join("stay.txt"), "s").unwrap();
    let cfg = config(&root);
    let mut idx = build(&cfg);

    fs::remove_dir_all(root.join("old")).unwrap();
    fs::write(root.join("added.txt"), "a").unwrap();
    let mut batch = Batch::default();
    batch.push(Signal::Changed(root.join("old")));
    batch.push(Signal::Changed(root.join("added.txt")));
    idx.update(&batch, &[]);
    assert_eq!(idx.deleted, 21);
    assert_eq!(idx.len(), 3, "root, stay.txt, added.txt");
    let want = snapshot(&idx);

    let file = dir.path().join("index.bin");
    idx.save(&file).unwrap();
    let loaded = FileIndex::load(&file).unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(snapshot(&loaded), want);

    let compact = idx.compact();
    assert_eq!(compact.entries.len(), 3);
    assert_eq!(compact.deleted, 0);
    assert_eq!(snapshot(&compact), want);
    assert!(compact.names.len() < idx.names.len());
}
