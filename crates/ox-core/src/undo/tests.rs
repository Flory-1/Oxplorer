use super::*;
use std::sync::Mutex;

/// A Recycle Bin in a folder: (original path, where it is kept, deletion time).
struct FakeBin {
    dir: PathBuf,
    items: Mutex<Vec<(PathBuf, PathBuf, i64)>>,
}

impl FakeBin {
    fn new(root: &Path) -> Self {
        let dir = root.join("bin");
        fs::create_dir_all(&dir).unwrap();
        Self {
            dir,
            items: Mutex::new(Vec::new()),
        }
    }

    fn len(&self) -> usize {
        self.items.lock().unwrap().len()
    }
}

impl Bin for FakeBin {
    fn delete(&self, paths: &[PathBuf]) -> Result<(), String> {
        let mut items = self.items.lock().unwrap();
        for p in paths {
            let kept = self.dir.join(items.len().to_string());
            fs::rename(p, &kept).map_err(|e| e.to_string())?;
            items.push((p.clone(), kept, now()));
        }
        Ok(())
    }

    fn restore(&self, paths: &[PathBuf], since: i64) -> Result<(), String> {
        let mut items = self.items.lock().unwrap();
        for p in paths {
            let i = items
                .iter()
                .position(|(orig, _, at)| orig == p && *at >= since)
                .ok_or("not in the bin")?;
            let (orig, kept, _) = items.remove(i);
            fs::rename(kept, orig).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn done(outcome: Result<Outcome, String>) -> Op {
    match outcome {
        Ok(Outcome::Done(op)) => op,
        other => panic!("not done: {other:?}"),
    }
}

/// Undo the newest entry like the app does; returns the error, if any.
fn undo_redo(h: &mut History, redo: bool, bin: &dyn Bin) -> Result<(), String> {
    let entry = h.take(redo).expect("something to do");
    match reverse(&entry.op, bin) {
        Ok(Outcome::Done(op)) => {
            h.reversed(redo, Entry { op, ..entry });
            Ok(())
        }
        Ok(other) => panic!("unexpected {other:?}"),
        Err(e) => {
            h.put_back(redo, entry);
            Err(e)
        }
    }
}

#[test]
fn rename_is_undone_and_redone() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let a = tmp.path().join("a.txt");
    let b = tmp.path().join("b.txt");
    fs::write(&a, "a").unwrap();
    fs::rename(&a, &b).unwrap();
    let mut h = History::default();
    h.record(Entry::new(
        Kind::Rename,
        Op::Renamed(vec![(a.clone(), b.clone())]),
    ));
    assert_eq!(h.undo.back().unwrap().subject, "a.txt");

    undo_redo(&mut h, false, &bin).unwrap();
    assert!(a.exists() && !b.exists());
    assert!(!h.can_undo() && h.can_redo());
    undo_redo(&mut h, true, &bin).unwrap();
    assert!(!a.exists() && b.exists());
    assert!(h.can_undo() && !h.can_redo());
}

#[test]
fn swapped_names_are_swapped_back() {
    let tmp = tempfile::tempdir().unwrap();
    let x = tmp.path().join("x");
    let y = tmp.path().join("y");
    fs::write(&x, "was y").unwrap();
    fs::write(&y, "was x").unwrap();
    // A bulk rename swapped x and y.
    let op = Op::Renamed(vec![(x.clone(), y.clone()), (y.clone(), x.clone())]);
    let back = done(reverse(&op, &FakeBin::new(tmp.path())));
    assert_eq!(fs::read_to_string(&x).unwrap(), "was x");
    assert_eq!(fs::read_to_string(&y).unwrap(), "was y");
    assert_eq!(back, Op::Renamed(vec![(y.clone(), x.clone()), (x, y)]));
}

#[test]
fn rename_is_refused_when_things_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let a = tmp.path().join("a.txt");
    let b = tmp.path().join("b.txt");
    // The renamed item is gone.
    let op = Op::Renamed(vec![(a.clone(), b.clone())]);
    let err = reverse(&op, &bin).unwrap_err();
    assert!(err.contains("b.txt"), "{err}");
    // Its old name is taken by something else.
    fs::write(&a, "new").unwrap();
    fs::write(&b, "renamed").unwrap();
    assert!(reverse(&op, &bin).is_err());
    assert_eq!(fs::read_to_string(&a).unwrap(), "new");
    assert_eq!(fs::read_to_string(&b).unwrap(), "renamed");
}

#[test]
fn failed_undo_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let mut h = History::default();
    let gone = tmp.path().join("gone");
    h.record(Entry::new(Kind::NewFolder, Op::NewFolder(gone)));
    assert!(undo_redo(&mut h, false, &bin).is_err());
    assert!(h.can_undo() && !h.can_redo());
}

#[test]
fn move_goes_back_also_into_removed_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let src = tmp.path().join("src");
    let dest = tmp.path().join("dest");
    fs::create_dir_all(dest.join("src/sub")).unwrap();
    fs::write(dest.join("src/sub/f.txt"), "f").unwrap();
    fs::write(dest.join("top.txt"), "t").unwrap();
    // `src/sub` was merged into the existing `dest/src`, and `src` removed.
    let op = Op::Moved(vec![
        (src.join("sub"), dest.join("src/sub")),
        (tmp.path().join("top.txt"), dest.join("top.txt")),
    ]);
    let back = done(reverse(&op, &bin));
    assert!(src.join("sub/f.txt").exists());
    assert!(tmp.path().join("top.txt").exists());
    assert!(dest.join("src").exists(), "the merged-into folder stays");
    assert!(!dest.join("src/sub").exists());
    // Redo moves them again.
    done(reverse(&back, &bin));
    assert!(dest.join("src/sub/f.txt").exists());
    assert!(dest.join("top.txt").exists());
}

#[test]
fn move_back_into_a_folder_that_is_there_again_needs_a_transfer() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let from = tmp.path().join("photos");
    let to = tmp.path().join("dest/photos");
    fs::create_dir_all(&from).unwrap();
    fs::write(from.join("left behind.jpg"), "x").unwrap();
    fs::create_dir_all(&to).unwrap();
    fs::write(to.join("moved.jpg"), "y").unwrap();
    let op = Op::Moved(vec![(from.clone(), to.clone())]);
    assert_eq!(
        reverse(&op, &bin).unwrap(),
        Outcome::Transfer {
            done: Vec::new(),
            pending: vec![(to.clone(), from.clone())],
        }
    );
    // A file in the way is refused.
    fs::write(tmp.path().join("a"), "in the way").unwrap();
    fs::write(tmp.path().join("b"), "moved").unwrap();
    let op = Op::Moved(vec![(tmp.path().join("a"), tmp.path().join("b"))]);
    assert!(reverse(&op, &bin).is_err());
}

#[test]
fn copy_goes_to_the_bin_and_comes_back() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let copy = tmp.path().join("copy");
    fs::create_dir_all(copy.join("sub")).unwrap();
    fs::write(copy.join("sub/f.txt"), "f").unwrap();
    let file = tmp.path().join("copy.txt");
    fs::write(&file, "c").unwrap();
    let mut h = History::default();
    let items = vec![
        Placed::now(copy.clone()).unwrap(),
        Placed::now(file.clone()).unwrap(),
    ];
    h.record(Entry::new(Kind::Copy, Op::Placed(items)));
    assert_eq!(h.undo.back().unwrap().describe(), "Copy (2 items)");

    undo_redo(&mut h, false, &bin).unwrap();
    assert!(!copy.exists() && !file.exists());
    assert_eq!(bin.len(), 2);
    undo_redo(&mut h, true, &bin).unwrap();
    assert!(copy.join("sub/f.txt").exists() && file.exists());
    assert_eq!(bin.len(), 0);
    // And once more: the restored items can be undone again.
    undo_redo(&mut h, false, &bin).unwrap();
    assert!(!copy.exists());
}

#[test]
fn changed_copies_are_not_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let copy = tmp.path().join("copy");
    fs::create_dir_all(copy.join("sub")).unwrap();
    let op = Op::Placed(vec![Placed::now(copy.clone()).unwrap()]);
    // Something new deep inside the copied folder.
    fs::write(copy.join("sub/new.txt"), "new").unwrap();
    let err = reverse(&op, &bin).unwrap_err();
    assert!(err.contains("changed"), "{err}");
    assert!(copy.join("sub/new.txt").exists());

    let file = tmp.path().join("f.txt");
    fs::write(&file, "1").unwrap();
    let op = Op::Placed(vec![Placed::now(file.clone()).unwrap()]);
    fs::write(&file, "12").unwrap();
    assert!(reverse(&op, &bin).is_err());
    assert_eq!(bin.len(), 0);
}

#[test]
fn new_folder_is_removed_only_while_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let dir = tmp.path().join("New folder");
    fs::create_dir(&dir).unwrap();
    let mut h = History::default();
    h.record(Entry::new(Kind::NewFolder, Op::NewFolder(dir.clone())));
    undo_redo(&mut h, false, &bin).unwrap();
    assert!(!dir.exists());
    undo_redo(&mut h, true, &bin).unwrap();
    assert!(dir.is_dir());
    fs::write(dir.join("x"), "x").unwrap();
    let err = undo_redo(&mut h, false, &bin).unwrap_err();
    assert!(err.contains("isn't empty"), "{err}");
    assert!(dir.join("x").exists());
}

#[test]
fn new_links_are_removed_but_never_their_targets() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let target = tmp.path().join("data.txt");
    fs::write(&target, "keep").unwrap();
    let link = tmp.path().join("again.txt");
    crate::links::create(LinkKind::Hard, &link, &target).unwrap();
    let l = Link {
        path: link.clone(),
        target: target.clone(),
        kind: LinkKind::Hard,
    };
    let mut h = History::default();
    h.record(Entry::new(Kind::NewLink, Op::Linked(vec![l])));
    undo_redo(&mut h, false, &bin).unwrap();
    assert!(!link.exists());
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    assert_eq!(bin.len(), 0);
    undo_redo(&mut h, true, &bin).unwrap();
    assert_eq!(fs::read_to_string(&link).unwrap(), "keep");

    // Replaced by an ordinary file: left alone.
    fs::remove_file(&link).unwrap();
    fs::write(&link, "other").unwrap();
    let err = undo_redo(&mut h, false, &bin).unwrap_err();
    assert!(err.contains("changed"), "{err}");
    assert!(link.exists());
}

#[cfg(windows)]
#[test]
fn undoing_a_junction_keeps_the_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let target = tmp.path().join("real");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("inside.txt"), "x").unwrap();
    let link = tmp.path().join("jump");
    crate::links::create(LinkKind::Junction, &link, &target).unwrap();
    let mut h = History::default();
    let l = Link {
        path: link.clone(),
        target: target.clone(),
        kind: LinkKind::Junction,
    };
    h.record(Entry::new(Kind::NewLink, Op::Linked(vec![l])));
    undo_redo(&mut h, false, &bin).unwrap();
    assert!(!crate::fsops::exists(&link));
    assert!(target.join("inside.txt").exists());
    undo_redo(&mut h, true, &bin).unwrap();
    assert!(link.join("inside.txt").exists());
}

#[test]
fn delete_is_undone_by_restoring() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = FakeBin::new(tmp.path());
    let f = tmp.path().join("f.txt");
    fs::write(&f, "f").unwrap();
    let since = now();
    bin.delete(std::slice::from_ref(&f)).unwrap();
    let mut h = History::default();
    h.record(Entry::new(
        Kind::Delete,
        Op::Trashed {
            paths: vec![f.clone()],
            since,
        },
    ));
    // Something new with the same name blocks the restore.
    fs::write(&f, "new").unwrap();
    assert!(undo_redo(&mut h, false, &bin).is_err());
    fs::remove_file(&f).unwrap();
    undo_redo(&mut h, false, &bin).unwrap();
    assert_eq!(fs::read_to_string(&f).unwrap(), "f");
    // Redo deletes it again.
    undo_redo(&mut h, true, &bin).unwrap();
    assert!(!f.exists());
    assert_eq!(bin.len(), 1);
}

fn trash_item(parent: &str, name: &str, time: i64) -> TrashItem {
    TrashItem {
        id: format!("{parent}/{name}@{time}").into(),
        name: name.into(),
        original_parent: parent.into(),
        time_deleted: time,
    }
}

#[test]
fn finds_the_newest_matching_bin_item() {
    let items = vec![
        trash_item("/home/me", "a.txt", 50),
        trash_item("/home/me", "a.txt", 120),
        trash_item("/home/me", "a.txt", 110),
        trash_item("/home/other", "a.txt", 130),
        trash_item("/home/me", "b", 101),
    ];
    let found = find_deleted(
        &items,
        &["/home/me/a.txt".into(), "/home/me/b.txt".into()],
        100,
    )
    .unwrap();
    assert_eq!(found[0].time_deleted, 120);
    // Found by its name without the (hidden) extension.
    assert_eq!(found[1].name, "b");
    // Deleted before the operation: not the one.
    let err = find_deleted(&items, &["/home/me/a.txt".into()], 200).unwrap_err();
    assert!(err.contains("a.txt"), "{err}");
}

#[test]
fn history_is_bounded_and_new_operations_end_redo() {
    let mut h = History::default();
    for i in 0..LIMIT + 5 {
        h.record(Entry::new(
            Kind::NewFolder,
            Op::NewFolder(PathBuf::from(i.to_string())),
        ));
    }
    assert_eq!(h.undo.len(), LIMIT);
    assert_eq!(h.undo.front().unwrap().subject, "5");

    let e = h.take(false).unwrap();
    h.reversed(false, e);
    assert!(h.can_redo());
    h.record(Entry::new(Kind::NewFolder, Op::NewFolder("new".into())));
    assert!(!h.can_redo());

    // Something recorded while an undo was running: that undo can't be redone.
    let e = h.take(false).unwrap();
    h.record(Entry::new(Kind::NewFolder, Op::NewFolder("other".into())));
    h.reversed(false, e);
    assert!(!h.can_redo());
}

#[test]
fn folders_and_subjects() {
    let op = Op::Moved(vec![("/a/x".into(), "/b/x".into())]);
    assert_eq!(op.folders(), vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    assert_eq!(Entry::new(Kind::Move, op).describe(), "Move (x)");
}
