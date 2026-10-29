use super::*;
use std::fs;
use std::time::Duration;

/// Wait until the job finishes or needs a conflict decision.
fn wait(job: &Job) -> JobState {
    wait_until(job, |s| s.is_finished() || s == JobState::AwaitingDecision)
}

fn wait_finished(job: &Job) -> JobState {
    wait_until(job, JobState::is_finished)
}

fn wait_until(job: &Job, done: impl Fn(JobState) -> bool) -> JobState {
    let start = Instant::now();
    loop {
        let s = job.state();
        if done(s) {
            return s;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "job timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

fn tree(root: &Path) {
    fs::create_dir_all(root.join("src/sub")).unwrap();
    fs::write(root.join("src/a.txt"), "hello").unwrap();
    fs::write(root.join("src/sub/b.bin"), vec![7u8; 5 * 1024 * 1024 + 3]).unwrap();
    fs::write(root.join("src/sub/c.txt"), "c").unwrap();
}

fn engine() -> TransferEngine {
    TransferEngine::new(EngineConfig {
        buffer_size: 64 * 1024,
        ..EngineConfig::default()
    })
}

fn req(
    kind: TransferKind,
    sources: Vec<PathBuf>,
    dest: &Path,
    conflict: ConflictPolicy,
) -> TransferRequest {
    TransferRequest {
        kind,
        sources,
        destination: dest.to_path_buf(),
        conflict,
        pairs: Vec::new(),
    }
}

#[test]
fn copies_a_tree() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let dest = tmp.path().join("dest");
    fs::create_dir(&dest).unwrap();

    let job = engine().submit(req(
        TransferKind::Copy,
        vec![tmp.path().join("src")],
        &dest,
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait(&job), JobState::Completed);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(fs::read_to_string(dest.join("src/a.txt")).unwrap(), "hello");
    assert_eq!(
        fs::read(dest.join("src/sub/b.bin")).unwrap().len(),
        5 * 1024 * 1024 + 3
    );
    assert_eq!(job.done_files.load(Ordering::Relaxed), 3);
    assert_eq!(
        job.done_bytes.load(Ordering::Relaxed),
        job.total_bytes.load(Ordering::Relaxed)
    );
    assert_eq!(job.progress(), 1.0);
    // Source untouched.
    assert!(tmp.path().join("src/a.txt").exists());
}

#[test]
fn moves_a_tree() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let dest = tmp.path().join("dest");
    fs::create_dir(&dest).unwrap();

    let job = engine().submit(req(
        TransferKind::Move,
        vec![tmp.path().join("src")],
        &dest,
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait(&job), JobState::Completed);
    assert!(!tmp.path().join("src").exists());
    assert_eq!(fs::read_to_string(dest.join("src/sub/c.txt")).unwrap(), "c");
}

#[test]
fn move_merges_into_existing_folder() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let dest = tmp.path().join("dest");
    fs::create_dir_all(dest.join("src")).unwrap();
    fs::write(dest.join("src/existing.txt"), "keep").unwrap();

    let job = engine().submit(req(
        TransferKind::Move,
        vec![tmp.path().join("src")],
        &dest,
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait(&job), JobState::Completed);
    assert!(!tmp.path().join("src").exists());
    assert!(dest.join("src/existing.txt").exists());
    assert!(dest.join("src/sub/b.bin").exists());
}

#[test]
fn copy_into_same_folder_makes_a_copy() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("doc.txt"), "x").unwrap();
    let job = engine().submit(req(
        TransferKind::Copy,
        vec![tmp.path().join("doc.txt")],
        tmp.path(),
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait(&job), JobState::Completed);
    assert!(tmp.path().join("doc - Copy.txt").exists());
}

#[test]
fn refuses_to_copy_folder_into_itself() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let job = engine().submit(req(
        TransferKind::Copy,
        vec![tmp.path().join("src")],
        &tmp.path().join("src/sub"),
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait(&job), JobState::Completed);
    assert_eq!(job.error_count(), 1);
    assert!(!tmp.path().join("src/sub/src").exists());
}

fn conflict_setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("same.txt"), "new").unwrap();
    fs::write(a.join("other.txt"), "other").unwrap();
    fs::write(b.join("same.txt"), "old").unwrap();
    (tmp, a, b)
}

#[test]
fn asks_once_then_applies_decision() {
    for (policy, expect_same, expect_copy) in [
        (ConflictPolicy::Overwrite, "new", false),
        (ConflictPolicy::Skip, "old", false),
        (ConflictPolicy::KeepBoth, "old", true),
    ] {
        let (_tmp, a, b) = conflict_setup();
        let job = engine().submit(req(
            TransferKind::Copy,
            vec![a.join("same.txt"), a.join("other.txt")],
            &b,
            ConflictPolicy::Ask,
        ));
        assert_eq!(wait(&job), JobState::AwaitingDecision);
        assert_eq!(job.conflicts(), vec![b.join("same.txt")]);
        job.resolve_conflicts(policy);
        assert_eq!(wait_finished(&job), JobState::Completed);
        assert_eq!(
            fs::read_to_string(b.join("same.txt")).unwrap(),
            expect_same,
            "{policy:?}"
        );
        assert_eq!(b.join("same (2).txt").exists(), expect_copy, "{policy:?}");
        assert!(b.join("other.txt").exists());
    }
}

#[test]
fn overwrite_if_newer_keeps_newer_target() {
    let (_tmp, a, b) = conflict_setup();
    // Make the target newer than the source.
    let f = fs::OpenOptions::new()
        .write(true)
        .open(b.join("same.txt"))
        .unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
    drop(f);
    let job = engine().submit(req(
        TransferKind::Copy,
        vec![a.join("same.txt")],
        &b,
        ConflictPolicy::OverwriteIfNewer,
    ));
    assert_eq!(wait(&job), JobState::Completed);
    assert_eq!(fs::read_to_string(b.join("same.txt")).unwrap(), "old");
    assert_eq!(job.skipped_files.load(Ordering::Relaxed), 1);
}

#[test]
fn cancel_while_waiting_for_decision() {
    let (_tmp, a, b) = conflict_setup();
    let job = engine().submit(req(
        TransferKind::Copy,
        vec![a.join("same.txt")],
        &b,
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait(&job), JobState::AwaitingDecision);
    job.cancel();
    assert_eq!(wait_finished(&job), JobState::Cancelled);
    assert_eq!(fs::read_to_string(b.join("same.txt")).unwrap(), "old");
}

#[test]
fn queue_limits_concurrent_jobs() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let engine = TransferEngine::new(EngineConfig {
        max_concurrent_jobs: 1,
        ..EngineConfig::default()
    });
    let jobs: Vec<_> = (0..3)
        .map(|i| {
            let dest = tmp.path().join(format!("d{i}"));
            fs::create_dir(&dest).unwrap();
            engine.submit(req(
                TransferKind::Copy,
                vec![tmp.path().join("src")],
                &dest,
                ConflictPolicy::Ask,
            ))
        })
        .collect();
    for job in &jobs {
        assert_eq!(wait(job), JobState::Completed);
    }
    assert!(!engine.has_active());
    engine.clear_finished();
    assert!(engine.jobs().is_empty());
}

use std::time::SystemTime;

fn engine_with(cfg: EngineConfig) -> TransferEngine {
    TransferEngine::new(EngineConfig {
        buffer_size: 64 * 1024,
        ..cfg
    })
}

#[test]
fn verify_counts_checked_files_and_moves_after_check() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let dest = tmp.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let engine = engine_with(EngineConfig {
        verify: true,
        ..EngineConfig::default()
    });

    let job = engine.submit(req(
        TransferKind::Copy,
        vec![tmp.path().join("src")],
        &dest,
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait_finished(&job), JobState::Completed);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(job.verified_files.load(Ordering::Relaxed), 3);

    // Moving to another folder of the same volume is a rename: nothing to verify.
    let moved = tmp.path().join("moved");
    fs::create_dir(&moved).unwrap();
    let job = engine.submit(req(
        TransferKind::Move,
        vec![dest.join("src")],
        &moved,
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait_finished(&job), JobState::Completed);
    assert!(moved.join("src/sub/b.bin").exists());
    assert!(!dest.join("src").exists());
}

#[test]
fn retry_copies_failed_files_to_their_exact_target() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("a.txt");
    fs::write(&src, "retry me").unwrap();
    let target = tmp.path().join("out/nested/renamed.txt");

    // A finished job whose only file failed.
    let failed = Job::new(
        1,
        &req(
            TransferKind::Copy,
            vec![src.clone()],
            tmp.path(),
            ConflictPolicy::Ask,
        ),
    );
    failed.push_file_error(&src, Some(&target), "disk full");
    failed.push_error(tmp.path(), "a scan error can't be retried");
    let retry = failed.retry_request().expect("one retryable file");
    assert_eq!(retry.pairs, vec![(src.clone(), target.clone())]);

    let job = engine().submit(retry);
    assert_eq!(wait_finished(&job), JobState::Completed);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(fs::read_to_string(&target).unwrap(), "retry me");

    let ok = Job::new(
        2,
        &req(TransferKind::Copy, vec![], tmp.path(), ConflictPolicy::Ask),
    );
    assert!(ok.retry_request().is_none());
}

#[test]
fn speed_limit_slows_the_copy() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("f.bin"), vec![1u8; 3 * 1024 * 1024 / 2]).unwrap();
    let dest = tmp.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let engine = engine_with(EngineConfig {
        speed_limit: 1024 * 1024,
        ..EngineConfig::default()
    });
    let start = Instant::now();
    let job = engine.submit(req(
        TransferKind::Copy,
        vec![tmp.path().join("f.bin")],
        &dest,
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait_finished(&job), JobState::Completed);
    // 1.5 MiB at 1 MiB/s.
    assert!(
        start.elapsed() >= Duration::from_millis(1200),
        "{:?}",
        start.elapsed()
    );
}

/// Two slow jobs on the same drive: with the per-disk queue the second waits.
fn second_job_waits(per_disk_queue: bool) -> bool {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("f.bin"), vec![1u8; 1024 * 1024]).unwrap();
    let engine = engine_with(EngineConfig {
        speed_limit: 1024 * 1024,
        max_concurrent_jobs: 8,
        per_disk_queue,
        ..EngineConfig::default()
    });
    let jobs: Vec<_> = (0..2)
        .map(|i| {
            let dest = tmp.path().join(format!("d{i}"));
            fs::create_dir(&dest).unwrap();
            engine.submit(req(
                TransferKind::Copy,
                vec![tmp.path().join("f.bin")],
                &dest,
                ConflictPolicy::Ask,
            ))
        })
        .collect();
    // Either job may grab the drive first; wait until one is copying.
    let start = Instant::now();
    while !jobs.iter().any(|j| j.state() == JobState::Running) {
        assert!(start.elapsed() < Duration::from_secs(10), "no job started");
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(200));
    // Each copy takes ~1 s, so the other job is still queued if it had to wait.
    let one_waiting = jobs.iter().any(|j| j.state() == JobState::Queued);
    for j in &jobs {
        assert_eq!(wait_finished(j), JobState::Completed);
    }
    one_waiting
}

#[test]
fn per_disk_queue_serializes_jobs_on_the_same_drive() {
    assert!(second_job_waits(true));
    assert!(!second_job_waits(false));
}

#[test]
fn explicit_worker_count_is_used() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let dest = tmp.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let engine = engine_with(EngineConfig {
        workers: 3,
        ..EngineConfig::default()
    });
    let job = engine.submit(req(
        TransferKind::Copy,
        vec![tmp.path().join("src")],
        &dest,
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait_finished(&job), JobState::Completed);
    assert_eq!(job.workers.load(Ordering::Relaxed), 3);
}

fn sorted(mut v: Vec<(PathBuf, PathBuf)>) -> Vec<(PathBuf, PathBuf)> {
    v.sort();
    v
}

#[test]
fn records_what_was_created() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let dest = tmp.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let src = tmp.path().join("src");
    let job = engine().submit(req(
        TransferKind::Copy,
        vec![src.clone()],
        &dest,
        ConflictPolicy::Ask,
    ));
    assert_eq!(wait(&job), JobState::Completed);
    assert_eq!(job.created(), vec![(src.clone(), dest.join("src"))]);

    // Copying again into the same folder merges: only the new file is listed.
    fs::write(src.join("new.txt"), "n").unwrap();
    let job = engine().submit(req(
        TransferKind::Copy,
        vec![src.clone()],
        &dest,
        ConflictPolicy::Skip,
    ));
    assert_eq!(wait(&job), JobState::Completed);
    assert_eq!(
        job.created(),
        vec![(src.join("new.txt"), dest.join("src/new.txt"))]
    );
}

#[test]
fn records_renamed_copies_but_not_skipped_or_replaced_ones() {
    for (policy, expect) in [
        (ConflictPolicy::KeepBoth, Some("same (2).txt")),
        (ConflictPolicy::Skip, None),
        (ConflictPolicy::Overwrite, None),
    ] {
        let (_tmp, a, b) = conflict_setup();
        let job = engine().submit(req(
            TransferKind::Copy,
            vec![a.join("same.txt"), a.join("other.txt")],
            &b,
            policy,
        ));
        assert_eq!(wait(&job), JobState::Completed);
        let mut want = vec![(a.join("other.txt"), b.join("other.txt"))];
        if let Some(name) = expect {
            want.push((a.join("same.txt"), b.join(name)));
        }
        assert_eq!(sorted(job.created()), sorted(want), "{policy:?}");
    }
}

#[test]
fn records_moves_including_replaced_files() {
    let (_tmp, a, b) = conflict_setup();
    let job = engine().submit(req(
        TransferKind::Move,
        vec![a.join("same.txt"), a.join("other.txt")],
        &b,
        ConflictPolicy::Overwrite,
    ));
    assert_eq!(wait(&job), JobState::Completed);
    assert_eq!(
        sorted(job.created()),
        sorted(vec![
            (a.join("same.txt"), b.join("same.txt")),
            (a.join("other.txt"), b.join("other.txt")),
        ])
    );
}

#[test]
fn folder_pairs_move_whole_folders_to_an_exact_target() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let target = tmp.path().join("elsewhere/renamed");
    let mut r = req(
        TransferKind::Move,
        Vec::new(),
        tmp.path(),
        ConflictPolicy::Ask,
    );
    r.pairs = vec![(tmp.path().join("src"), target.clone())];
    let job = engine().submit(r);
    assert_eq!(wait(&job), JobState::Completed);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert!(target.join("sub/b.bin").exists());
    assert!(!tmp.path().join("src").exists());
    assert_eq!(job.created(), vec![(tmp.path().join("src"), target)]);
}
