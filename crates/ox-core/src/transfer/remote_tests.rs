//! Downloads and uploads against the in-memory server.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::RemoteSide;
use crate::remote::memory::MemoryFs;
use crate::remote::RemoteAddr;
use crate::transfer::{
    ConflictPolicy, EngineConfig, ErrorChoice, Job, JobState, TransferEngine, TransferKind,
    TransferRequest,
};

fn server() -> RemoteAddr {
    RemoteAddr::parse("sftp://me@example.com/").unwrap()
}

fn at(path: &str) -> PathBuf {
    server().with_path(path).to_path()
}

fn mem() -> Arc<MemoryFs> {
    let fs = MemoryFs::default();
    fs.add_file("/srv/docs/a.txt", b"alpha");
    fs.add_file("/srv/docs/sub/b.txt", b"bravo!");
    fs.add_dir("/srv/docs/empty");
    fs.add_file("/srv/top.txt", b"top");
    Arc::new(fs)
}

fn request(
    fs: &Arc<MemoryFs>,
    kind: TransferKind,
    sources: Vec<PathBuf>,
    dest: PathBuf,
    conflict: ConflictPolicy,
) -> TransferRequest {
    TransferRequest {
        remote: Some(RemoteSide::new(fs.clone(), &server())),
        ..TransferRequest::new(kind, sources, dest, conflict)
    }
}

fn engine(ask_on_error: bool) -> TransferEngine {
    TransferEngine::new(EngineConfig {
        ask_on_error,
        ..EngineConfig::default()
    })
}

fn wait_until(job: &Job, done: impl Fn(&Job) -> bool) {
    let start = Instant::now();
    while !done(job) {
        assert!(start.elapsed() < Duration::from_secs(20), "job timed out");
        thread::sleep(Duration::from_millis(5));
    }
}

fn finish(job: &Job) {
    wait_until(job, |j| j.state().is_finished());
}

fn no_temp_files(dir: &Path) {
    for e in walk(dir) {
        assert!(!e.to_string_lossy().ends_with(".oxpart"), "{e:?}");
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        }
        out.push(p);
    }
    out
}

#[test]
fn downloads_a_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = mem();
    let req = request(
        &fs,
        TransferKind::Copy,
        vec![at("/srv/docs"), at("/srv/top.txt")],
        tmp.path().to_path_buf(),
        ConflictPolicy::Ask,
    );
    let job = engine(false).submit(req);
    assert!(job.is_remote());
    finish(&job);
    assert_eq!(job.state(), JobState::Completed);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    let docs = tmp.path().join("docs");
    assert_eq!(fs::read(docs.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(fs::read(docs.join("sub").join("b.txt")).unwrap(), b"bravo!");
    assert!(docs.join("empty").is_dir());
    assert_eq!(fs::read(tmp.path().join("top.txt")).unwrap(), b"top");
    assert_eq!(job.total_files.load(Ordering::Relaxed), 3);
    assert_eq!(job.done_files.load(Ordering::Relaxed), 3);
    assert_eq!(job.done_bytes.load(Ordering::Relaxed), 14);
    assert_eq!(job.progress(), 1.0);
    // The date of the original.
    let modified = fs::metadata(docs.join("a.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(modified, std::time::SystemTime::UNIX_EPOCH);
    // New top-level items can be undone (removed again).
    let created: Vec<PathBuf> = job.created().into_iter().map(|(_, d)| d).collect();
    assert!(created.contains(&docs) && created.contains(&tmp.path().join("top.txt")));
    no_temp_files(tmp.path());
}

#[test]
fn uploads_a_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("photos");
    fs::create_dir_all(src.join("2024")).unwrap();
    fs::write(src.join("2024").join("x.jpg"), b"jpeg data").unwrap();
    fs::write(src.join("notes.txt"), b"n").unwrap();
    let fs = mem();
    let req = request(
        &fs,
        TransferKind::Copy,
        vec![src.clone()],
        at("/srv"),
        ConflictPolicy::Ask,
    );
    let job = engine(false).submit(req);
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(fs.contents("/srv/photos/2024/x.jpg").unwrap(), b"jpeg data");
    assert_eq!(fs.contents("/srv/photos/notes.txt").unwrap(), b"n");
    assert!(
        fs.paths().iter().all(|p| !p.ends_with(".oxpart")),
        "{:?}",
        fs.paths()
    );
    let local = fs::metadata(src.join("notes.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(fs.modified("/srv/photos/notes.txt"), Some(local));
    // Undo only knows local files.
    assert!(job.created().is_empty());
}

#[test]
fn conflicts_wait_for_one_decision() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("top.txt"), b"old").unwrap();
    let fs = mem();
    let req = request(
        &fs,
        TransferKind::Copy,
        vec![at("/srv/top.txt")],
        tmp.path().to_path_buf(),
        ConflictPolicy::Ask,
    );
    let job = engine(false).submit(req);
    wait_until(&job, |j| j.state() == JobState::AwaitingDecision);
    assert_eq!(job.conflicts(), vec![tmp.path().join("top.txt")]);
    job.resolve_conflicts(ConflictPolicy::KeepBoth);
    finish(&job);
    assert_eq!(fs::read(tmp.path().join("top.txt")).unwrap(), b"old");
    assert_eq!(fs::read(tmp.path().join("top (2).txt")).unwrap(), b"top");

    // Uploading onto an existing remote file: replaced, or kept with a new name.
    let local = tmp.path().join("a.txt");
    fs::write(&local, b"new").unwrap();
    let dest = at("/srv/docs");
    for (policy, path, contents) in [
        (ConflictPolicy::KeepBoth, "/srv/docs/a (2).txt", &b"new"[..]),
        (ConflictPolicy::Skip, "/srv/docs/a.txt", &b"alpha"[..]),
        (ConflictPolicy::Overwrite, "/srv/docs/a.txt", &b"new"[..]),
    ] {
        let req = request(
            &fs,
            TransferKind::Copy,
            vec![local.clone()],
            dest.clone(),
            policy,
        );
        let job = engine(false).submit(req);
        finish(&job);
        assert_eq!(fs.contents(path).unwrap(), contents, "{policy:?}");
    }
}

#[test]
fn moves_remove_the_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = mem();
    let req = request(
        &fs,
        TransferKind::Move,
        vec![at("/srv/docs")],
        tmp.path().to_path_buf(),
        ConflictPolicy::Ask,
    );
    let job = engine(false).submit(req);
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert!(tmp.path().join("docs/sub/b.txt").is_file());
    assert_eq!(fs.paths(), ["/srv", "/srv/top.txt"]);

    // And back up again.
    let req = request(
        &fs,
        TransferKind::Move,
        vec![tmp.path().join("docs")],
        at("/srv"),
        ConflictPolicy::Ask,
    );
    let job = engine(false).submit(req);
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert!(!tmp.path().join("docs").exists());
    assert_eq!(fs.contents("/srv/docs/sub/b.txt").unwrap(), b"bravo!");
}

#[test]
fn failed_files_are_asked_about_and_retried() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = mem();
    *fs.fail_on.lock().unwrap() = Some("b.txt".into());
    let req = request(
        &fs,
        TransferKind::Copy,
        vec![at("/srv/docs")],
        tmp.path().to_path_buf(),
        ConflictPolicy::Ask,
    );
    let job = engine(true).submit(req);
    wait_until(&job, |j| j.is_asking() || j.state().is_finished());
    let q = job.error_question().expect("asks about the failed file");
    assert!(q.path.to_string_lossy().ends_with("b.txt"), "{:?}", q.path);
    // Fixed meanwhile: the retry works.
    *fs.fail_on.lock().unwrap() = None;
    job.answer_error(ErrorChoice::Retry);
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(
        fs::read(tmp.path().join("docs/sub/b.txt")).unwrap(),
        b"bravo!"
    );
    no_temp_files(tmp.path());
}

#[test]
fn errors_are_collected_and_can_be_retried_later() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = mem();
    *fs.fail_on.lock().unwrap() = Some("a.txt".into());
    let eng = engine(false);
    let job = eng.submit(request(
        &fs,
        TransferKind::Copy,
        vec![at("/srv/docs")],
        tmp.path().to_path_buf(),
        ConflictPolicy::Ask,
    ));
    finish(&job);
    assert_eq!(job.error_count(), 1);
    assert!(!tmp.path().join("docs/a.txt").exists());
    no_temp_files(tmp.path());
    *fs.fail_on.lock().unwrap() = None;
    let retry = job.retry_request().expect("the failed file can be retried");
    assert!(retry.remote.is_some());
    let again = eng.submit(retry);
    finish(&again);
    assert!(again.errors().is_empty(), "{:?}", again.errors());
    assert_eq!(fs::read(tmp.path().join("docs/a.txt")).unwrap(), b"alpha");
}

#[test]
fn cancelled_jobs_leave_no_partial_files() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = Arc::new(MemoryFs::default());
    fs.add_file("/big.bin", &vec![7u8; 400_000]);
    let eng = TransferEngine::new(EngineConfig {
        // Slow enough to cancel in the middle.
        speed_limit: 100_000,
        ..EngineConfig::default()
    });
    let job = eng.submit(request(
        &fs,
        TransferKind::Copy,
        vec![at("/big.bin")],
        tmp.path().to_path_buf(),
        ConflictPolicy::Ask,
    ));
    wait_until(&job, |j| j.done_bytes.load(Ordering::Relaxed) > 0);
    job.cancel();
    finish(&job);
    assert_eq!(job.state(), JobState::Cancelled);
    assert!(fs::read_dir(tmp.path()).unwrap().next().is_none());
}
