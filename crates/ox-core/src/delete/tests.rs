use super::*;
use std::fs;

fn wait_finished(job: &DeleteJob) -> DeleteState {
    let start = Instant::now();
    loop {
        let s = job.state();
        if s.is_finished() {
            return s;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "job timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

/// root/a.txt, root/sub/b.bin, root/sub/deeper/c.txt (read-only).
fn tree(root: &Path) {
    fs::create_dir_all(root.join("sub/deeper")).unwrap();
    fs::write(root.join("a.txt"), "hello").unwrap();
    fs::write(root.join("sub/b.bin"), vec![1u8; 1000]).unwrap();
    let c = root.join("sub/deeper/c.txt");
    fs::write(&c, "c").unwrap();
    let mut perms = fs::metadata(&c).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&c, perms).unwrap();
}

fn permanent(paths: Vec<PathBuf>) -> DeleteJob {
    DeleteJob::new(1, DeleteRequest::new(paths, DeleteMode::Permanent))
}

#[test]
fn deletes_trees_and_files_for_good() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    tree(&root);
    let single = tmp.path().join("single.txt");
    fs::write(&single, "12345").unwrap();

    let engine = DeleteEngine::default();
    let job = engine.submit(DeleteRequest::new(
        vec![root.clone(), single.clone()],
        DeleteMode::Permanent,
    ));
    assert_eq!(wait_finished(&job), DeleteState::Completed);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert!(!root.exists() && !single.exists());
    // 4 files + 3 folders.
    assert_eq!(job.total_items.load(Ordering::Relaxed), 7);
    assert_eq!(job.done_items.load(Ordering::Relaxed), 7);
    assert_eq!(job.total_bytes.load(Ordering::Relaxed), 5 + 1000 + 1 + 5);
    assert_eq!(job.progress(), 1.0);
    assert_eq!(job.removed(), vec![root, single]);
    assert!(!engine.has_active());
    engine.remove(job.id);
    assert!(engine.jobs().is_empty());
}

#[test]
fn missing_items_are_no_error() {
    let tmp = tempfile::tempdir().unwrap();
    let job = permanent(vec![tmp.path().join("gone")]);
    run::run(&job, &|| {});
    assert!(job.errors().is_empty());
}

#[test]
fn cancelled_job_deletes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    tree(&root);
    let job = permanent(vec![root.clone()]);
    job.cancel();
    run::run(&job, &|| {});
    assert!(root.join("sub/deeper/c.txt").exists());
    assert_eq!(job.done_items.load(Ordering::Relaxed), 0);
}

#[test]
fn pause_holds_the_job_until_resumed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    tree(&root);
    let job = Arc::new(permanent(vec![root.clone()]));
    job.pause();
    let worker = {
        let job = job.clone();
        thread::spawn(move || run::run(&job, &|| {}))
    };
    let start = Instant::now();
    while job.state() != DeleteState::Running {
        assert!(start.elapsed() < Duration::from_secs(20), "never started");
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(100));
    assert_eq!(job.done_items.load(Ordering::Relaxed), 0);
    assert!(root.join("a.txt").exists());
    job.resume();
    worker.join().unwrap();
    assert!(!root.exists());
}

#[cfg(windows)]
#[test]
fn locked_file_is_reported_once_and_the_rest_goes() {
    use std::os::windows::fs::OpenOptionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    tree(&root);
    let locked = root.join("sub/deeper/locked.txt");
    fs::write(&locked, "x").unwrap();
    // No sharing at all: deleting it fails while it is open.
    let _handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked)
        .unwrap();

    let job = permanent(vec![root.clone()]);
    run::run(&job, &|| {});
    let errors = job.errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].path, locked);
    assert_eq!(errors[0].item, root);
    // Everything else is gone; the folders holding the locked file stay.
    assert!(!root.join("a.txt").exists() && !root.join("sub/b.bin").exists());
    assert!(locked.exists());
}

#[cfg(windows)]
#[test]
fn junctions_are_removed_without_touching_their_target() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep.txt"), "keep").unwrap();
    let root = tmp.path().join("root");
    fs::create_dir(&root).unwrap();
    let link = root.join("link");
    let made = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(&link)
        .arg(&target)
        .output()
        .is_ok_and(|o| o.status.success());
    if !made {
        return; // no junctions here (e.g. not NTFS)
    }

    let job = permanent(vec![root.clone()]);
    run::run(&job, &|| {});
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    assert!(!root.exists());
    assert!(target.join("keep.txt").exists());
}
