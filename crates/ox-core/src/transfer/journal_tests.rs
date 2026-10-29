use super::*;
use crate::transfer::EngineConfig;
use std::time::Duration;

fn wait_finished(job: &Job) -> JobState {
    let start = Instant::now();
    loop {
        let s = job.state();
        if s.is_finished() || s == JobState::AwaitingDecision {
            return s;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "job timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The journal is finished right after the job (see `Guard`).
fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn journals(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn engine(dir: &Path) -> TransferEngine {
    TransferEngine::new(EngineConfig {
        buffer_size: 64 * 1024,
        journal_dir: Some(dir.to_path_buf()),
        ..EngineConfig::default()
    })
}

fn tree(root: &Path) -> PathBuf {
    let src = root.join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), "hello").unwrap();
    fs::write(src.join("sub/b.bin"), vec![7u8; 3 * 1024 * 1024 + 5]).unwrap();
    fs::write(src.join("sub/c.txt"), "c").unwrap();
    src
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn done_record(src: &Path, dst: &Path) -> String {
    let meta = fs::metadata(src).unwrap();
    let [ss, sm] = Stamp::new(meta.len(), meta.modified().ok()).fields();
    let [ds, dm] = Stamp::of(dst).unwrap().fields();
    record(&["done", s(src), s(dst), &ss, &sm, &ds, &dm])
}

/// Give `dst` the modified time of `src` (as a finished copy has).
fn same_time(src: &Path, dst: &Path) {
    let modified = fs::metadata(src).unwrap().modified().unwrap();
    let f = OpenOptions::new().write(true).open(dst).unwrap();
    f.set_modified(modified).unwrap();
}

#[test]
fn escaping_round_trips() {
    for text in [
        "plain",
        "50% off",
        "tab\there",
        "line\nbreak\r",
        "%09",
        "ä/ß\\x",
    ] {
        assert_eq!(unescape(&escape(text)).as_deref(), Some(text), "{text:?}");
    }
    assert_eq!(unescape("bad%G0"), None);
    assert_eq!(unescape("cut%2"), None);
}

#[test]
fn reads_what_it_writes_and_ignores_a_torn_line() {
    let mut req = TransferRequest::new(
        TransferKind::Move,
        vec!["C:/from/a".into(), "C:/from/b\tc".into()],
        "D:/to".into(),
        ConflictPolicy::Ask,
    );
    req.pairs = vec![("C:/x.txt".into(), "D:/y.txt".into())];
    let mut text = header(&req).unwrap();
    text.push_str(&record(&["root", "C:/from/a", "D:/to/a"]));
    text.push_str(&record(&["policy", "KeepBoth"]));
    text.push_str(&record(&["start", "C:/from/a/1", "D:/to/a/1"]));
    text.push_str(&record(&[
        "done",
        "C:/from/a/1",
        "D:/to/a/1",
        "5",
        "100",
        "5",
        "100",
    ]));
    text.push_str(&record(&["start", "C:/from/a/2", "D:/to/a/2 (2)"]));
    text.push_str("garbage line\u{0}\u{0}\n");
    text.push_str(&record(&[
        "done",
        "C:/from/a/3",
        "D:/to/a/3",
        "x",
        "-",
        "1",
        "-",
    ]));
    // A crash in the middle of this record:
    text.push_str("done\tC:/from/a/2\tD:/to/a/2 (2)\t9");

    let p = parse(Path::new("j.journal"), text.as_bytes()).unwrap();
    assert_eq!(p.request.kind, TransferKind::Move);
    assert_eq!(p.request.sources, req.sources);
    assert_eq!(p.request.destination, PathBuf::from("D:/to"));
    assert_eq!(p.request.pairs, req.pairs);
    assert_eq!(p.request.conflict, ConflictPolicy::Ask);
    assert_eq!(p.policy, Some(ConflictPolicy::KeepBoth));
    assert_eq!(p.roots[Path::new("C:/from/a")], PathBuf::from("D:/to/a"));
    assert!(p.torn);
    assert_eq!((p.done_files, p.done_bytes), (1, 5));
    let one = &p.files[Path::new("C:/from/a/1")];
    let stamp = Stamp {
        size: 5,
        modified: Some(100),
    };
    assert_eq!(one.done, Some((stamp, stamp)));
    // The torn `done` doesn't count: the file is only started.
    let two = &p.files[Path::new("C:/from/a/2")];
    assert_eq!(two.dst, PathBuf::from("D:/to/a/2 (2)"));
    assert!(two.done.is_none());
    // The damaged record was skipped.
    assert!(!p.files.contains_key(Path::new("C:/from/a/3")));
}

#[test]
fn an_incomplete_request_is_not_a_journal() {
    let req = TransferRequest::new(
        TransferKind::Copy,
        vec!["C:/a".into()],
        "D:/".into(),
        ConflictPolicy::Skip,
    );
    let full = header(&req).unwrap();
    let cut = &full[..full.len() - "begin\n".len()];
    assert!(parse(Path::new("j"), cut.as_bytes()).is_none());
    assert!(parse(Path::new("j"), b"something else\nbegin\n").is_none());
    assert!(parse(Path::new("j"), full.as_bytes()).is_some());

    // Listing removes such leftovers.
    let tmp = tempfile::tempdir().unwrap();
    let bad = tmp.path().join("bad.journal");
    fs::write(&bad, cut).unwrap();
    fs::write(tmp.path().join("good.journal"), &full).unwrap();
    fs::write(tmp.path().join("other.txt"), &full).unwrap();
    let found = list(Some(tmp.path()));
    assert_eq!(found.len(), 1);
    assert!(found[0].path.ends_with("good.journal"));
    assert!(!bad.exists());
}

#[test]
fn a_journal_in_use_is_not_offered() {
    let tmp = tempfile::tempdir().unwrap();
    let req = TransferRequest::new(
        TransferKind::Copy,
        vec!["C:/a".into()],
        "D:/".into(),
        ConflictPolicy::Ask,
    );
    let path = tmp.path().join("busy.journal");
    fs::write(&path, header(&req).unwrap()).unwrap();
    let held = open_unlocked(&path).unwrap();
    assert!(list(Some(tmp.path())).is_empty());
    assert!(path.exists(), "a journal in use must not be deleted");
    drop(held);
    assert_eq!(list(Some(tmp.path())).len(), 1);
}

#[test]
fn a_finished_job_deletes_its_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tree(tmp.path());
    let dest = tmp.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let dir = tmp.path().join("journals");
    let engine = engine(&dir);
    let job = engine.submit(TransferRequest::new(
        TransferKind::Copy,
        vec![src],
        dest.clone(),
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait_finished(&job), JobState::Completed);
    assert!(dest.join("src/sub/b.bin").exists());
    wait_for("journal deleted", || journals(&dir).is_empty());
    assert!(engine.resumable().is_empty());
}

#[test]
fn resume_skips_finished_files_and_redoes_the_partial_one() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tree(tmp.path());
    let dest = tmp.path().join("dest");
    let target = dest.join("src");
    fs::create_dir_all(target.join("sub")).unwrap();
    // a.txt was copied completely. Different contents with the same size and
    // date prove that it isn't copied again.
    fs::write(target.join("a.txt"), "HELLO").unwrap();
    same_time(&src.join("a.txt"), &target.join("a.txt"));
    // b.bin was being copied when the power went out.
    fs::write(target.join("sub/b.bin"), vec![0u8; 3 * 1024 * 1024 + 5]).unwrap();

    let dir = tmp.path().join("journals");
    fs::create_dir(&dir).unwrap();
    let req = TransferRequest::new(
        TransferKind::Copy,
        vec![src.clone()],
        dest.clone(),
        ConflictPolicy::Ask,
    );
    let mut text = header(&req).unwrap();
    text.push_str(&record(&["root", s(&src), s(&target)]));
    text.push_str(&record(&["policy", "Ask"]));
    text.push_str(&record(&[
        "start",
        s(&src.join("a.txt")),
        s(&target.join("a.txt")),
    ]));
    text.push_str(&done_record(&src.join("a.txt"), &target.join("a.txt")));
    text.push_str(&record(&[
        "start",
        s(&src.join("sub/b.bin")),
        s(&target.join("sub/b.bin")),
    ]));
    let path = dir.join("old.journal");
    fs::write(&path, text).unwrap();

    let engine = engine(&dir);
    let pending = engine.resumable();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].done_files, 1);
    let job = engine.resume(&pending[0]);
    assert!(engine.resumable().is_empty());
    // No conflict question: the partial file is our own.
    assert_eq!(wait_finished(&job), JobState::Completed);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(fs::read_to_string(target.join("a.txt")).unwrap(), "HELLO");
    assert_eq!(
        fs::read(target.join("sub/b.bin")).unwrap(),
        fs::read(src.join("sub/b.bin")).unwrap()
    );
    assert_eq!(fs::read_to_string(target.join("sub/c.txt")).unwrap(), "c");
    assert_eq!(job.done_files.load(Ordering::Relaxed), 3);
    assert_eq!(job.progress(), 1.0);
    wait_for("journal deleted", || !path.exists());
}

#[test]
fn resume_copies_again_what_changed_since() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tree(tmp.path());
    let dest = tmp.path().join("dest");
    let target = dest.join("src");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("a.txt"), "hello").unwrap();
    same_time(&src.join("a.txt"), &target.join("a.txt"));
    let dir = tmp.path().join("journals");
    fs::create_dir(&dir).unwrap();
    let req = TransferRequest::new(
        TransferKind::Copy,
        vec![src.clone()],
        dest.clone(),
        ConflictPolicy::Ask,
    );
    let mut text = header(&req).unwrap();
    text.push_str(&record(&["policy", "Overwrite"]));
    text.push_str(&done_record(&src.join("a.txt"), &target.join("a.txt")));
    fs::write(dir.join("old.journal"), text).unwrap();
    // The original changed after it was copied.
    fs::write(src.join("a.txt"), "hello, world").unwrap();

    let engine = engine(&dir);
    let job = engine.resume(&engine.resumable()[0]);
    // The recorded answer "Replace" is used again instead of asking.
    assert_eq!(wait_finished(&job), JobState::Completed);
    assert_eq!(
        fs::read_to_string(target.join("a.txt")).unwrap(),
        "hello, world"
    );
}

#[test]
fn resuming_a_move_skips_what_was_already_moved() {
    let tmp = tempfile::tempdir().unwrap();
    let from = tmp.path().join("from");
    let dest = tmp.path().join("dest");
    fs::create_dir_all(&from).unwrap();
    fs::create_dir_all(&dest).unwrap();
    fs::write(from.join("y.txt"), "y").unwrap();
    // x.txt was moved before the interruption.
    fs::write(dest.join("x.txt"), "x").unwrap();
    let dir = tmp.path().join("journals");
    fs::create_dir(&dir).unwrap();
    let req = TransferRequest::new(
        TransferKind::Move,
        vec![from.join("x.txt"), from.join("y.txt")],
        dest.clone(),
        ConflictPolicy::Ask,
    );
    fs::write(dir.join("old.journal"), header(&req).unwrap()).unwrap();

    let engine = engine(&dir);
    let job = engine.resume(&engine.resumable()[0]);
    assert_eq!(wait_finished(&job), JobState::Completed);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert!(!from.join("y.txt").exists());
    assert_eq!(fs::read_to_string(dest.join("y.txt")).unwrap(), "y");
    assert_eq!(fs::read_to_string(dest.join("x.txt")).unwrap(), "x");
}

#[test]
fn a_job_that_lost_its_destination_keeps_its_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.txt");
    fs::write(&file, "a").unwrap();
    // Like a USB stick that was pulled: the destination isn't there.
    let dest = tmp.path().join("stick");
    let dir = tmp.path().join("journals");
    let engine = engine(&dir);
    let job = engine.submit(TransferRequest::new(
        TransferKind::Copy,
        vec![file.clone()],
        dest.clone(),
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait_finished(&job), JobState::Completed);
    assert_eq!(job.error_count(), 1);
    wait_for("journal kept", || engine.resumable().len() == 1);
    let pending = engine.resumable().remove(0);
    assert_eq!(pending.job_id, Some(job.id));
    assert_eq!(pending.request.destination, dest);
    assert_eq!(journals(&dir).len(), 1);

    // The stick is back.
    fs::create_dir(&dest).unwrap();
    let again = engine.resume(&pending);
    assert!(
        engine.jobs().iter().all(|j| j.id != job.id),
        "the failed job makes room for the resumed one"
    );
    assert_eq!(wait_finished(&again), JobState::Completed);
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "a");
    wait_for("journal deleted", || journals(&dir).is_empty());
}

#[test]
fn discarding_deletes_the_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let req = TransferRequest::new(
        TransferKind::Copy,
        vec!["C:/a".into()],
        "D:/".into(),
        ConflictPolicy::Ask,
    );
    let path = tmp.path().join("old.journal");
    fs::write(&path, header(&req).unwrap()).unwrap();
    let engine = engine(tmp.path());
    let pending = engine.resumable();
    assert_eq!(pending.len(), 1);
    engine.discard(&pending[0]);
    assert!(engine.resumable().is_empty());
    assert!(!path.exists());
}
