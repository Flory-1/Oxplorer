//! Running a picture task on many pictures: a few threads, progress, cancel,
//! and the safe way of replacing originals.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use super::{convert, transform, Destination, Task};
use crate::fsops;
use crate::undo::{self, Bin, Placed, Replacement};

/// A running task: how far it is, and its [`Summary`] once it's done.
pub struct Job {
    pub total: usize,
    done: AtomicUsize,
    cancel: AtomicBool,
    summary: Mutex<Option<Summary>>,
    finished: AtomicBool,
}

impl Job {
    pub(super) fn new(total: usize) -> Self {
        Self {
            total,
            done: AtomicUsize::new(0),
            cancel: AtomicBool::new(false),
            summary: Mutex::new(None),
            finished: AtomicBool::new(false),
        }
    }

    /// Pictures finished (done or failed).
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    /// Stop after the pictures being worked on now.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// What was done, once the job has finished (only the first call gets it).
    pub fn take_summary(&self) -> Option<Summary> {
        lock(&self.summary).take()
    }
}

/// What a job did.
#[derive(Debug, Default)]
pub struct Summary {
    /// New files (converted or resized copies).
    pub created: Vec<Placed>,
    /// Files that took the place of their originals.
    pub replaced: Vec<Replacement>,
    /// Pictures that couldn't be done, and why.
    pub failed: Vec<(PathBuf, String)>,
    /// It was cancelled before every picture was done.
    pub cancelled: bool,
    /// When it started (Unix seconds): the replaced originals went to the
    /// Recycle Bin after that.
    pub since: i64,
}

/// Run `task` on `paths` in the background. `notify` is called whenever a
/// picture is done and when the job has finished.
pub fn start(
    task: Task,
    paths: Vec<PathBuf>,
    bin: Arc<dyn Bin + Send + Sync>,
    notify: impl Fn() + Send + Sync + 'static,
) -> Arc<Job> {
    let job = Arc::new(Job::new(paths.len()));
    let threads = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 4);
    let j = job.clone();
    std::thread::spawn(move || {
        let summary = run(&j, &task, &paths, &*bin, threads, &notify);
        *lock(&j.summary) = Some(summary);
        j.finished.store(true, Ordering::Release);
        notify();
    });
    job
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Shared by the threads of a job.
struct Ctx<'a> {
    bin: &'a (dyn Bin + Sync),
    since: i64,
    /// Held while names are chosen and files replaced, so two threads never
    /// pick the same name.
    names: Mutex<()>,
}

enum Done {
    Created(Placed),
    Replaced(Replacement),
}

pub(super) fn run(
    job: &Job,
    task: &Task,
    paths: &[PathBuf],
    bin: &(dyn Bin + Sync),
    threads: usize,
    notify: &(dyn Fn() + Sync),
) -> Summary {
    let ctx = Ctx {
        bin,
        since: undo::now(),
        names: Mutex::new(()),
    };
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, paths.len().max(1)) {
            s.spawn(|| loop {
                if job.is_cancelled() {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(i) else {
                    break;
                };
                let result = one(task, path, &ctx);
                lock(&results).push((i, result));
                job.done.fetch_add(1, Ordering::Relaxed);
                notify();
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_else(|p| p.into_inner());
    results.sort_by_key(|(i, _)| *i);
    let mut summary = Summary {
        cancelled: results.len() < paths.len(),
        since: ctx.since,
        ..Default::default()
    };
    for (i, result) in results {
        match result {
            Ok(Done::Created(p)) => summary.created.push(p),
            Ok(Done::Replaced(r)) => summary.replaced.push(r),
            Err(e) => summary.failed.push((paths[i].clone(), e)),
        }
    }
    summary
}

/// Do `task` to one picture.
fn one(task: &Task, path: &Path, ctx: &Ctx) -> Result<Done, String> {
    match task {
        Task::Transform(tf) => {
            let bytes = transform(path, *tf)?;
            replace(path, None, &bytes, ctx).map(Done::Replaced)
        }
        Task::Convert(c) => {
            let (bytes, plain, with_suffix) = convert(path, c)?;
            let dir = path.parent().unwrap_or(Path::new(""));
            let created = match &c.destination {
                Destination::Replace => {
                    return replace(path, Some(&plain), &bytes, ctx).map(Done::Replaced)
                }
                Destination::Beside => write_new(dir, &with_suffix, &bytes, ctx)?,
                Destination::Folder(folder) => {
                    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
                    write_new(folder, &with_suffix, &bytes, ctx)?
                }
            };
            Placed::now(created)
                .map(Done::Created)
                .map_err(|e| e.to_string())
        }
    }
}

/// Write `bytes` to a new file `dir/name` (or `name (2)`, … if taken).
fn write_new(dir: &Path, name: &str, bytes: &[u8], ctx: &Ctx) -> Result<PathBuf, String> {
    let (path, mut file) = {
        let _names = lock(&ctx.names);
        let mut tries = 0;
        loop {
            let path = fsops::unique_path(dir, name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => break (path, f),
                // Made by someone else just now.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && tries < 10 => tries += 1,
                Err(e) => return Err(e.to_string()),
            }
        }
    };
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(e.to_string());
    }
    Ok(path)
}

/// Put `bytes` in the place of `original`, named `new_name` (the original's
/// name if `None`): written next to it first, then the original goes to the
/// Recycle Bin and the new file is renamed into place.
fn replace(
    original: &Path,
    new_name: Option<&str>,
    bytes: &[u8],
    ctx: &Ctx,
) -> Result<Replacement, String> {
    let dir = original.parent().unwrap_or(Path::new(""));
    let name = original.file_name().unwrap_or_default().to_string_lossy();
    let created = fs::metadata(original).and_then(|m| m.created()).ok();
    let tmp = write_new(dir, &format!("{name}.oxtmp"), bytes, ctx)?;
    let names = lock(&ctx.names);
    let target = match new_name {
        Some(n) if !same_name(n, &name) => fsops::unique_path(dir, n),
        _ => original.to_path_buf(),
    };
    if let Err(e) = ctx.bin.delete(&[original.to_path_buf()]) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, &target) {
        if ctx
            .bin
            .restore(&[original.to_path_buf()], ctx.since)
            .is_ok()
        {
            let _ = fs::remove_file(&tmp);
        }
        return Err(e.to_string());
    }
    drop(names);
    if let Some(c) = created {
        let _ = set_created(&target, c);
    }
    let new = Placed::now(target).map_err(|e| e.to_string())?;
    Ok(Replacement {
        original: original.to_path_buf(),
        new,
    })
}

fn same_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

/// Give the file at `path` the created date `when`.
#[cfg(windows)]
pub(super) fn set_created(path: &Path, when: SystemTime) -> io::Result<()> {
    use std::os::windows::fs::FileTimesExt;
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_times(fs::FileTimes::new().set_created(when))
}

#[cfg(not(windows))]
pub(super) fn set_created(_path: &Path, _when: SystemTime) -> io::Result<()> {
    Ok(())
}
