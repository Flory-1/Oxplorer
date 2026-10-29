//! Resuming, verifying and copies between servers, against the in-memory
//! server.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::RemoteSide;
use crate::remote::memory::MemoryFs;
use crate::remote::RemoteAddr;
use crate::transfer::{
    ConflictPolicy, EngineConfig, Job, JobState, TransferEngine, TransferKind, TransferRequest,
};

fn server(host: &str) -> RemoteAddr {
    RemoteAddr::parse(&format!("sftp://me@{host}/")).unwrap()
}

fn at(host: &str, path: &str) -> PathBuf {
    server(host).with_path(path).to_path()
}

/// 4000 bytes that differ from block to block.
fn data() -> Vec<u8> {
    (0..4000u32).map(|i| (i % 251) as u8).collect()
}

fn side(fs: &Arc<MemoryFs>, host: &str) -> RemoteSide {
    RemoteSide::new(fs.clone(), &server(host))
}

fn request(side: RemoteSide, sources: Vec<PathBuf>, dest: PathBuf) -> TransferRequest {
    TransferRequest {
        remote: Some(side),
        ..TransferRequest::new(TransferKind::Copy, sources, dest, ConflictPolicy::Ask)
    }
}

fn engine(cfg: EngineConfig) -> TransferEngine {
    TransferEngine::new(cfg)
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out: {what}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn finish(job: &Job) {
    wait_until("job", || job.state().is_finished());
}

fn no_partials(fs: &MemoryFs) {
    assert!(
        fs.paths().iter().all(|p| !p.ends_with(".oxpart")),
        "{:?}",
        fs.paths()
    );
}

#[test]
fn downloads_continue_after_a_dropped_connection() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = Arc::new(MemoryFs::default());
    fs.add_file("/big.bin", &data());
    fs.drop_after.store(1000, Ordering::Relaxed);
    let job = engine(EngineConfig::default()).submit(request(
        side(&fs, "h"),
        vec![at("h", "/big.bin")],
        tmp.path().to_path_buf(),
    ));
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(fs::read(tmp.path().join("big.bin")).unwrap(), data());
    // Continued where it stopped, not downloaded again.
    assert_eq!(*fs.resumed_at.lock().unwrap(), [1000]);
    assert_eq!(job.done_bytes.load(Ordering::Relaxed), 4000);
    assert!(!tmp.path().join(".big.bin.oxpart").exists());
}

#[test]
fn uploads_continue_after_a_dropped_connection() {
    let tmp = tempfile::tempdir().unwrap();
    let local = tmp.path().join("big.bin");
    fs::write(&local, data()).unwrap();
    for append in [true, false] {
        let fs = Arc::new(MemoryFs::default());
        fs.add_dir("/up");
        fs.no_append.store(!append, Ordering::Relaxed);
        fs.drop_after.store(1000, Ordering::Relaxed);
        let job = engine(EngineConfig::default()).submit(request(
            side(&fs, "h"),
            vec![local.clone()],
            at("h", "/up"),
        ));
        finish(&job);
        assert!(job.errors().is_empty(), "{:?}", job.errors());
        assert_eq!(fs.contents("/up/big.bin").unwrap(), data());
        // Appended where the server can; else sent again from the start.
        let resumed = fs.resumed_at.lock().unwrap().clone();
        assert_eq!(resumed, if append { vec![1000] } else { vec![] });
        assert_eq!(job.done_bytes.load(Ordering::Relaxed), 4000);
        no_partials(&fs);
    }
}

/// The server goes away in the middle of a file and stays away: the
/// partial copy and the journal are kept, and the job continues from
/// there once it is resumed.
#[test]
fn interrupted_transfers_resume_from_the_journal() {
    for upload in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let journals = tmp.path().join("journals");
        let local_dir = tmp.path().join("local");
        fs::create_dir_all(&local_dir).unwrap();
        let fs = Arc::new(MemoryFs::default());
        fs.add_dir("/up");
        fs.add_file("/big.bin", &data());
        fs::write(local_dir.join("up.bin"), data()).unwrap();
        fs.drop_after.store(1000, Ordering::Relaxed);
        fs.stay_down.store(true, Ordering::Relaxed);
        let cfg = EngineConfig {
            journal_dir: Some(journals.clone()),
            ..EngineConfig::default()
        };
        let eng = engine(cfg);
        let req = if upload {
            request(
                side(&fs, "h"),
                vec![local_dir.join("up.bin")],
                at("h", "/up"),
            )
        } else {
            request(side(&fs, "h"), vec![at("h", "/big.bin")], local_dir.clone())
        };
        let job = eng.submit(req);
        finish(&job);
        assert_eq!(job.error_count(), 1, "{upload}");
        wait_until("journal kept", || !eng.resumable().is_empty());
        let pending = eng.resumable().remove(0);
        let servers = pending.servers();
        assert!(servers.len() == 1 && servers[0].same_server(&server("h")));
        assert!(pending.request.remote.is_none());

        // The server is back.
        fs.offline.store(false, Ordering::Relaxed);
        fs.stay_down.store(false, Ordering::Relaxed);
        let again = eng.resume_with(&pending, Some(side(&fs, "h")));
        finish(&again);
        assert!(again.errors().is_empty(), "{:?}", again.errors());
        if upload {
            assert_eq!(fs.contents("/up/up.bin").unwrap(), data());
            no_partials(&fs);
        } else {
            assert_eq!(fs::read(local_dir.join("big.bin")).unwrap(), data());
            assert!(!local_dir.join(".big.bin.oxpart").exists());
        }
        assert_eq!(*fs.resumed_at.lock().unwrap(), [1000], "{upload}");
        wait_until("journal deleted", || {
            fs::read_dir(&journals).unwrap().next().is_none()
        });
    }
}

#[test]
fn resumed_requests_get_their_servers_back() {
    let (a, b) = two_servers();
    let connect =
        |addr: &RemoteAddr| -> crate::remote::RemoteResult<Arc<dyn crate::remote::RemoteFs>> {
            Ok(if addr.host == "a" {
                a.clone()
            } else {
                b.clone()
            })
        };
    let local = PathBuf::from("C:/x");
    let make = |src: PathBuf, dst: PathBuf| {
        TransferRequest::new(TransferKind::Copy, vec![src], dst, ConflictPolicy::Ask)
    };
    let down = RemoteSide::for_request(&make(at("a", "/f"), local.clone()), &connect)
        .unwrap()
        .unwrap();
    assert!(down.server.same_server(&server("a")) && down.to.is_none());
    let up = RemoteSide::for_request(&make(local.clone(), at("b", "/d")), &connect)
        .unwrap()
        .unwrap();
    assert!(up.server.same_server(&server("b")) && up.to.is_none());
    let both = RemoteSide::for_request(&make(at("a", "/f"), at("b", "/d")), &connect)
        .unwrap()
        .unwrap();
    assert_eq!(
        both.servers(),
        [server("a").server_key(), server("b").server_key()]
    );
    assert!(
        RemoteSide::for_request(&make(local.clone(), local), &connect)
            .unwrap()
            .is_none()
    );
}

#[test]
fn failed_files_on_a_reachable_server_leave_no_partials() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = Arc::new(MemoryFs::default());
    fs.add_file("/a.bin", &data());
    *fs.fail_on.lock().unwrap() = Some("a.bin".into());
    let cfg = EngineConfig {
        journal_dir: Some(tmp.path().join("journals")),
        ..EngineConfig::default()
    };
    let eng = engine(cfg);
    let job = eng.submit(request(
        side(&fs, "h"),
        vec![at("h", "/a.bin")],
        tmp.path().to_path_buf(),
    ));
    finish(&job);
    assert_eq!(job.error_count(), 1);
    // Not retried automatically (nothing was transferred) …
    assert_eq!(fs.failures.load(Ordering::Relaxed), 1);
    // … and nothing to resume: the server is fine.
    thread::sleep(Duration::from_millis(50));
    assert!(eng.resumable().is_empty());
    assert!(!tmp.path().join(".a.bin.oxpart").exists());
}

#[test]
fn verification_uses_the_servers_checksums() {
    let tmp = tempfile::tempdir().unwrap();
    let local = tmp.path().join("up.bin");
    fs::write(&local, data()).unwrap();
    let cfg = EngineConfig {
        verify: true,
        ..EngineConfig::default()
    };
    for (checksums, bad) in [(true, false), (false, false), (true, true)] {
        let fs = Arc::new(MemoryFs::default());
        fs.add_file("/big.bin", &data());
        fs.add_dir("/up");
        fs.checksums.store(checksums, Ordering::Relaxed);
        fs.bad_checksums.store(bad, Ordering::Relaxed);
        let down = tmp.path().join(format!("down-{checksums}-{bad}"));
        fs::create_dir_all(&down).unwrap();
        let eng = engine(cfg.clone());
        let jobs = [
            eng.submit(request(
                side(&fs, "h"),
                vec![at("h", "/big.bin")],
                down.clone(),
            )),
            eng.submit(request(side(&fs, "h"), vec![local.clone()], at("h", "/up"))),
        ];
        for job in &jobs {
            finish(job);
            let verified = job.verified_files.load(Ordering::Relaxed);
            let sized = job.size_checked_files.load(Ordering::Relaxed);
            match (checksums, bad) {
                (true, false) => assert_eq!((verified, sized), (1, 0)),
                (false, _) => assert_eq!((verified, sized), (0, 1)),
                (true, true) => {
                    assert_eq!((verified, sized), (0, 0));
                    let errors = job.errors();
                    assert_eq!(errors.len(), 1);
                    assert!(errors[0].message.contains("Verification"), "{errors:?}");
                }
            }
        }
        // A bad copy is never kept.
        assert_eq!(down.join("big.bin").exists(), !bad);
        assert_eq!(fs.contents("/up/up.bin").is_some(), !bad);
        assert!(!down.join(".big.bin.oxpart").exists());
        no_partials(&fs);
    }
}

#[test]
fn verification_notices_a_changed_original() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = Arc::new(MemoryFs::default());
    fs.add_file("/big.bin", &data());
    // Slow, so the file can change on the server meanwhile.
    *fs.delay.lock().unwrap() = Some(Duration::from_millis(1));
    let eng = engine(EngineConfig {
        verify: true,
        ..EngineConfig::default()
    });
    let job = eng.submit(request(
        side(&fs, "h"),
        vec![at("h", "/big.bin")],
        tmp.path().to_path_buf(),
    ));
    wait_until("started", || job.done_bytes.load(Ordering::Relaxed) > 0);
    fs.change_file("/big.bin", b"new", std::time::SystemTime::now());
    finish(&job);
    assert_eq!(job.error_count(), 1);
    assert!(!tmp.path().join("big.bin").exists());
}

fn two_servers() -> (Arc<MemoryFs>, Arc<MemoryFs>) {
    let a = Arc::new(MemoryFs::default());
    a.add_file("/docs/a.txt", b"alpha");
    a.add_file("/docs/sub/big.bin", &data());
    let b = Arc::new(MemoryFs::default());
    b.add_dir("/backup");
    (a, b)
}

#[test]
fn copies_stream_from_one_server_to_another() {
    let (a, b) = two_servers();
    let eng = engine(EngineConfig::default());
    let both = side(&a, "a").to(side(&b, "b"));
    let job = eng.submit(request(
        both.clone(),
        vec![at("a", "/docs")],
        at("b", "/backup"),
    ));
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(b.contents("/backup/docs/a.txt").unwrap(), b"alpha");
    assert_eq!(b.contents("/backup/docs/sub/big.bin").unwrap(), data());
    assert_eq!(job.done_bytes.load(Ordering::Relaxed), 4005);
    assert_eq!(job.done_files.load(Ordering::Relaxed), 2);
    // Dates are kept; nothing is left behind on the local disk.
    assert_eq!(
        b.modified("/backup/docs/a.txt"),
        Some(std::time::SystemTime::UNIX_EPOCH)
    );
    assert!(job.created().is_empty());
    no_partials(&b);

    // A move removes the originals.
    let mut req = request(both, vec![at("a", "/docs")], at("b", "/moved"));
    req.kind = TransferKind::Move;
    b.add_dir("/moved");
    let job = eng.submit(req);
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(b.contents("/moved/docs/sub/big.bin").unwrap(), data());
    assert!(a.paths().is_empty(), "{:?}", a.paths());
}

#[test]
fn copies_within_one_server_stream_too() {
    let (a, _) = two_servers();
    let eng = engine(EngineConfig::default());
    let job = eng.submit(request(
        side(&a, "a").to(side(&a, "a")),
        vec![at("a", "/docs/a.txt")],
        at("a", "/docs/sub"),
    ));
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(a.contents("/docs/sub/a.txt").unwrap(), b"alpha");
    assert_eq!(a.contents("/docs/a.txt").unwrap(), b"alpha");
}

#[test]
fn copies_between_servers_resume_and_cancel() {
    let (a, b) = two_servers();
    let eng = engine(EngineConfig::default());
    let both = side(&a, "a").to(side(&b, "b"));
    // The upload side drops: continued at the offset.
    b.drop_after.store(1000, Ordering::Relaxed);
    let job = eng.submit(request(
        both.clone(),
        vec![at("a", "/docs/sub/big.bin")],
        at("b", "/backup"),
    ));
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(b.contents("/backup/big.bin").unwrap(), data());
    assert_eq!(*a.resumed_at.lock().unwrap(), [1000]);
    assert_eq!(*b.resumed_at.lock().unwrap(), [1000]);

    // The download side breaks: the upload fails too, no short file.
    a.drop_after.store(2000, Ordering::Relaxed);
    b.add_dir("/fresh");
    let job = eng.submit(request(
        both.clone(),
        vec![at("a", "/docs/sub/big.bin")],
        at("b", "/fresh"),
    ));
    finish(&job);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert_eq!(b.contents("/fresh/big.bin").unwrap(), data());

    // Cancelled in the middle: nothing is left on the destination.
    *a.delay.lock().unwrap() = Some(Duration::from_millis(1));
    b.add_dir("/cancel");
    let job = eng.submit(request(
        both,
        vec![at("a", "/docs/sub/big.bin")],
        at("b", "/cancel"),
    ));
    wait_until("started", || job.done_bytes.load(Ordering::Relaxed) > 0);
    job.cancel();
    finish(&job);
    assert_eq!(job.state(), JobState::Cancelled);
    no_partials(&b);
    assert!(b.contents("/cancel/big.bin").is_none());
}
