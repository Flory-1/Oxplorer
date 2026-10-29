use std::fs::{self, File};
use std::time::{Duration, SystemTime};

use super::*;

fn write(path: &Path, text: &str, age_secs: u64) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, text).unwrap();
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 - age_secs);
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn options(mode: Mode, by: CompareBy, exclude: &str) -> Options {
    Options {
        mode,
        by,
        exclude: exclude.into(),
    }
}

fn run(src: &Path, dst: &Path, o: &Options) -> Plan {
    plan(src, dst, o, &AtomicBool::new(false), &AtomicU64::new(0))
        .unwrap()
        .unwrap()
}

fn row<'a>(p: &'a Plan, rel: &str) -> Option<&'a Row> {
    p.rows.iter().find(|r| r.rel == Path::new(rel))
}

fn group(p: &Plan, rel: &str) -> Option<(Group, Why)> {
    row(p, rel).map(|r| (r.group, r.why))
}

/// src: new.txt, changed.txt (newer), same.txt, older.txt (target newer),
/// diff.txt (same date, other size), sub/deep.txt (new), newdir/a.txt
/// dst: changed.txt, same.txt, older.txt, diff.txt, extra.txt, sub/, olddir/x.txt
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let (s, d) = (dir.path().join("src"), dir.path().join("dst"));
    write(&s.join("new.txt"), "new", 0);
    write(&s.join("changed.txt"), "v2", 10);
    write(&d.join("changed.txt"), "v1", 1000);
    write(&s.join("same.txt"), "same", 50);
    write(&d.join("same.txt"), "same", 51);
    write(&s.join("older.txt"), "old", 1000);
    write(&d.join("older.txt"), "newer", 10);
    write(&s.join("diff.txt"), "abc", 100);
    write(&d.join("diff.txt"), "abcdef", 100);
    write(&s.join("sub/deep.txt"), "deep", 0);
    fs::create_dir_all(d.join("sub")).unwrap();
    write(&s.join("newdir/a.txt"), "aaaa", 0);
    write(&s.join("newdir/b.txt"), "bb", 0);
    write(&d.join("extra.txt"), "x", 0);
    write(&d.join("olddir/x.txt"), "xx", 0);
    (dir, s, d)
}

#[test]
fn plans_a_mirror() {
    let (_dir, s, d) = setup();
    let p = run(&s, &d, &Options::default());
    assert_eq!(group(&p, "new.txt"), Some((Group::New, Why::New)));
    assert_eq!(group(&p, "changed.txt"), Some((Group::Update, Why::Newer)));
    assert_eq!(group(&p, "same.txt"), Some((Group::Same, Why::Same)));
    assert_eq!(
        group(&p, "older.txt"),
        Some((Group::Update, Why::TargetNewer))
    );
    assert_eq!(group(&p, "diff.txt"), Some((Group::Update, Why::Different)));
    assert_eq!(group(&p, "sub/deep.txt"), Some((Group::New, Why::New)));
    assert_eq!(group(&p, "extra.txt"), Some((Group::Remove, Why::Extra)));
    // Folders only one side has are one row, with their files.
    let newdir = row(&p, "newdir").unwrap();
    assert_eq!(
        (newdir.group, newdir.files, newdir.bytes),
        (Group::New, 2, 6)
    );
    let olddir = row(&p, "olddir").unwrap();
    assert_eq!((olddir.group, olddir.files), (Group::Remove, 1));
    assert!(row(&p, "olddir/x.txt").is_none());
    assert!(p
        .rows
        .iter()
        .filter(|r| r.group != Group::Same)
        .all(|r| r.include));
    assert_eq!(p.totals(Group::New, true).files, 4);
    assert_eq!(p.totals(Group::Update, true).items, 3);
    assert_eq!(p.totals(Group::Remove, true).files, 2);
    // dst: changed, same, older, diff, extra, olddir/x.
    assert_eq!(p.target_files, 6);
    assert!(!p.removes_most());
    assert!(!p.new_target);
}

#[test]
fn update_never_removes_and_keeps_newer_targets() {
    let (_dir, s, d) = setup();
    let p = run(&s, &d, &options(Mode::Update, CompareBy::SizeAndDate, ""));
    assert!(p.rows.iter().all(|r| r.group != Group::Remove));
    assert_eq!(p.kept_extras, 2);
    assert!(!row(&p, "older.txt").unwrap().include, "target is newer");
    assert!(row(&p, "changed.txt").unwrap().include);
    let steps = p.steps(ConflictPolicy::Ask);
    assert!(steps.trash.is_empty());
}

#[test]
fn compares_contents_when_asked() {
    let (_dir, s, d) = setup();
    // Same contents, other dates: the same by contents.
    write(&s.join("touched.txt"), "same bytes", 0);
    write(&d.join("touched.txt"), "same bytes", 5000);
    // Same size and date, other bytes: only contents can tell.
    write(&s.join("sneaky.txt"), "aaaa", 100);
    write(&d.join("sneaky.txt"), "bbbb", 100);
    let by_date = run(&s, &d, &Options::default());
    assert_eq!(group(&by_date, "touched.txt").unwrap().0, Group::Update);
    assert_eq!(group(&by_date, "sneaky.txt").unwrap().0, Group::Same);
    let by_contents = run(&s, &d, &options(Mode::Mirror, CompareBy::Contents, ""));
    assert_eq!(group(&by_contents, "touched.txt").unwrap().0, Group::Same);
    assert_eq!(
        group(&by_contents, "sneaky.txt"),
        Some((Group::Update, Why::Different))
    );
}

#[test]
fn two_seconds_count_as_the_same_time() {
    let dir = tempfile::tempdir().unwrap();
    let (s, d) = (dir.path().join("s"), dir.path().join("d"));
    write(&s.join("f.txt"), "x", 100);
    write(&d.join("f.txt"), "x", 102);
    let p = run(&s, &d, &Options::default());
    assert_eq!(group(&p, "f.txt").unwrap().0, Group::Same);
    write(&d.join("f.txt"), "x", 104);
    let p = run(&s, &d, &Options::default());
    assert_eq!(group(&p, "f.txt"), Some((Group::Update, Why::Newer)));
}

#[test]
fn excluded_items_are_neither_copied_nor_removed() {
    let (_dir, s, d) = setup();
    write(&s.join("junk.tmp"), "t", 0);
    write(&s.join("node_modules/lib.js"), "l", 0);
    write(&s.join("newdir/.git/HEAD"), "ref", 0);
    write(&s.join(".gitignore"), "x", 0);
    write(&d.join(".git/config"), "c", 0);
    write(&d.join("olddir/cache.tmp"), "c", 0);
    write(&s.join("docs/a.pdf"), "p", 0);
    let p = run(
        &s,
        &d,
        &options(
            Mode::Mirror,
            CompareBy::SizeAndDate,
            "*.TMP; node_modules ; .git; docs/*.pdf",
        ),
    );
    for gone in [
        "junk.tmp",
        "node_modules",
        ".git",
        "newdir/.git",
        "docs/a.pdf",
    ] {
        assert!(row(&p, gone).is_none(), "{gone} excluded");
    }
    // Exact names, not parts of them.
    assert_eq!(group(&p, ".gitignore").unwrap().0, Group::New);
    // A new folder with something excluded inside: its items one by one.
    assert!(row(&p, "newdir").is_none());
    assert_eq!(group(&p, "newdir/a.txt").unwrap().0, Group::New);
    // An extra folder holding something excluded stays; its other items go.
    assert!(row(&p, "olddir").is_none());
    assert_eq!(group(&p, "olddir/x.txt").unwrap().0, Group::Remove);
    let steps = p.steps(ConflictPolicy::Ask);
    assert!(steps.trash.iter().all(|t| !t.ends_with(".git")));
}

#[test]
fn refuses_the_same_folder_or_one_inside_the_other() {
    let (_dir, s, d) = setup();
    assert!(check(&s, &d).is_ok());
    assert!(check(&s, &s).is_err());
    assert!(check(&s, &s.join("sub")).is_err(), "target inside source");
    assert!(check(&s, &s.join("not yet/there")).is_err());
    assert!(check(&s.join("sub"), &s).is_err(), "source inside target");
    let other_case = PathBuf::from(s.to_string_lossy().to_uppercase());
    if cfg!(windows) {
        assert!(check(&s, &other_case).is_err());
    }
    let dotted = s.join("sub").join("..");
    assert!(check(&s, &dotted).is_err());
    assert!(check(&s, &s.join("new.txt")).is_err(), "a file");
    assert!(check(&s.join("missing"), &d).is_err());
    assert!(plan(
        &s,
        &s,
        &Options::default(),
        &AtomicBool::new(false),
        &AtomicU64::new(0)
    )
    .is_err());
    // A plan for a pair that is refused does nothing.
    let mut p = run(&s, &d, &Options::default());
    p.target = s.join("sub");
    let steps = p.steps(ConflictPolicy::Ask);
    assert!(steps.trash.is_empty() && steps.copies.is_empty());
}

#[test]
fn warns_when_most_of_the_target_would_go() {
    let dir = tempfile::tempdir().unwrap();
    let (s, d) = (dir.path().join("s"), dir.path().join("d"));
    write(&s.join("keep.txt"), "k", 0);
    write(&d.join("keep.txt"), "k", 0);
    for i in 0..3 {
        write(&d.join(format!("old{i}.txt")), "o", 0);
    }
    let mut p = run(&s, &d, &Options::default());
    assert_eq!((p.removed_files(), p.target_files), (3, 4));
    assert!(p.removes_most());
    // Unticking some of them: fine again.
    for r in p
        .rows
        .iter_mut()
        .filter(|r| r.group == Group::Remove)
        .take(2)
    {
        r.include = false;
    }
    assert!(!p.removes_most());
    p.tick_group(Group::Remove, true);
    assert!(p.removes_most());
}

#[test]
fn mirrors_to_a_new_folder() {
    let (_dir, s, d) = setup();
    let target = d.join("fresh");
    let p = run(&s, &target, &Options::default());
    assert!(p.new_target);
    assert!(p.rows.iter().all(|r| r.group == Group::New));
    let steps = p.steps(ConflictPolicy::Ask);
    assert_eq!(steps.copies.len(), 1);
    assert!(steps.copies[0]
        .pairs
        .iter()
        .all(|(_, to)| to.starts_with(&target)));
}

#[test]
fn a_file_replaced_by_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (s, d) = (dir.path().join("s"), dir.path().join("d"));
    write(&s.join("thing/inside.txt"), "i", 0);
    write(&d.join("thing"), "a file", 0);
    let p = run(&s, &d, &Options::default());
    let kinds: Vec<Group> = p
        .rows
        .iter()
        .filter(|r| r.why == Why::Kind)
        .map(|r| r.group)
        .collect();
    assert_eq!(kinds, [Group::Remove, Group::New]);
    let p = run(&s, &d, &options(Mode::Update, CompareBy::SizeAndDate, ""));
    assert!(p.rows.is_empty());
    assert_eq!(p.conflicts, 1);
}

#[test]
fn steps_carry_out_the_ticked_rows() {
    let (_dir, s, d) = setup();
    let mut p = run(&s, &d, &Options::default());
    p.rows
        .iter_mut()
        .find(|r| r.rel == Path::new("extra.txt"))
        .unwrap()
        .include = false;
    let steps = p.steps(ConflictPolicy::Skip);
    assert_eq!(steps.trash, [d.join("olddir")]);
    let [new, update] = &steps.copies[..] else {
        panic!("two jobs: {:?}", steps.copies);
    };
    assert_eq!(new.conflict, ConflictPolicy::Skip);
    assert_eq!(update.conflict, ConflictPolicy::Overwrite);
    assert!(new.pairs.contains(&(s.join("newdir"), d.join("newdir"))));
    assert!(new
        .pairs
        .contains(&(s.join("sub/deep.txt"), d.join("sub/deep.txt"))));
    assert_eq!(update.pairs.len(), 3);
    assert!(update
        .pairs
        .contains(&(s.join("changed.txt"), d.join("changed.txt"))));
}

#[test]
fn carrying_out_the_plan_makes_the_target_like_the_source() {
    let (_dir, s, d) = setup();
    let p = run(&s, &d, &Options::default());
    let steps = p.steps(ConflictPolicy::Overwrite);
    // The Recycle Bin isn't used in tests: remove for good here.
    crate::fsops::delete(&steps.trash, false).unwrap();
    let engine = crate::transfer::TransferEngine::new(Default::default());
    for req in steps.copies {
        let job = engine.submit(req);
        while !job.state().is_finished() {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(job.error_count(), 0);
    }
    let again = run(&s, &d, &options(Mode::Mirror, CompareBy::Contents, ""));
    let left: Vec<_> = again
        .rows
        .iter()
        .filter(|r| r.group != Group::Same)
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn exclude_patterns() {
    let e = Exclude::parse(" *.tmp ;node_modules, build\\cache/ ;; ");
    assert!(e.matches(Path::new("a/b/x.TMP")));
    assert!(e.matches(Path::new("node_modules")));
    assert!(e.matches(Path::new("deep/node_modules")));
    assert!(!e.matches(Path::new("node_modules_old")));
    assert!(e.matches(Path::new("build/cache")));
    assert!(!e.matches(Path::new("other/build/cache")));
    assert!(Exclude::parse(" ; ").is_empty());
}

#[test]
fn can_be_cancelled_and_runs_in_the_background() {
    let (_dir, s, d) = setup();
    let cancel = AtomicBool::new(true);
    assert!(
        plan(&s, &d, &Options::default(), &cancel, &AtomicU64::new(0))
            .unwrap()
            .is_none()
    );
    let job = PlanJob::start(s, d, Options::default(), || {});
    let start = std::time::Instant::now();
    loop {
        if let Some(r) = job.poll() {
            assert!(!r.unwrap().rows.is_empty());
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(job.scanned() > 0);
}

#[test]
fn remembers_recent_pairs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pairs.toml");
    let pair = |s: &str, t: &str| RecentPair {
        source: s.into(),
        target: t.into(),
        options: Options::default(),
    };
    RecentPairs::remember(&path, pair("C:\\a", "D:\\a")).unwrap();
    RecentPairs::remember(&path, pair("C:\\b", "D:\\b")).unwrap();
    let mut again = pair("c:\\A", "d:\\a");
    again.options.mode = Mode::Update;
    let r = RecentPairs::remember(&path, again).unwrap();
    assert_eq!(r.pairs.len(), 2);
    assert_eq!(r.pairs[0].options.mode, Mode::Update);
    assert_eq!(
        RecentPairs::load(&path)
            .for_source(Path::new("C:\\B"))
            .unwrap()
            .target,
        PathBuf::from("D:\\b")
    );
    for i in 0..20 {
        RecentPairs::remember(&path, pair(&format!("C:\\{i}"), "D:\\")).unwrap();
    }
    assert_eq!(RecentPairs::load(&path).pairs.len(), recent::MAX_PAIRS);
}
