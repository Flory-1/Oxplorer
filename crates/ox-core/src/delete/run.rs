//! Running one delete job: ending programs, scanning, deleting.

use std::collections::HashSet;
use std::fs;
use std::io;

use super::*;

/// Items sent to the Recycle Bin in one go (between batches the job can be
/// paused or cancelled).
const TRASH_BATCH: usize = 16;

pub(super) fn run(job: &DeleteJob, notify: &dyn Fn()) {
    end_programs(job);
    match job.mode {
        DeleteMode::Trash => trash(job, notify),
        DeleteMode::Permanent => permanent(job, notify),
    }
}

fn end_programs(job: &DeleteJob) {
    let notes: Vec<String> = job
        .end
        .iter()
        .filter_map(|&pid| {
            crate::locks::kill(pid)
                .err()
                .map(|e| crate::trf!("Couldn't end process {pid}: {e}", pid = pid, e = e))
        })
        .collect();
    *job.notes.lock().unwrap() = notes;
}

fn trash(job: &DeleteJob, notify: &dyn Fn()) {
    job.total_items
        .store(job.paths.len() as u64, Ordering::Relaxed);
    job.workers.store(1, Ordering::Relaxed);
    job.set_state(DeleteState::Running);
    notify();
    for batch in job.paths.chunks(TRASH_BATCH) {
        if !job.checkpoint() {
            return;
        }
        job.set_current(&batch[0]);
        for (path, error) in crate::fsops::delete_each(batch, true) {
            job.push_error(&path, &path, error);
        }
        job.done_items
            .fetch_add(batch.len() as u64, Ordering::Relaxed);
    }
}

/// A file or link to remove.
struct FileTask {
    path: PathBuf,
    size: u64,
    /// Index of the selected item it is in.
    item: usize,
    /// A link to a folder or a junction: removed like a folder, never entered.
    dir_link: bool,
}

#[derive(Default)]
struct Plan {
    files: Vec<FileTask>,
    /// Folders with the index of their selected item, parents before children.
    dirs: Vec<(PathBuf, usize)>,
}

impl Plan {
    fn add_file(&mut self, job: &DeleteJob, task: FileTask) {
        job.total_items.fetch_add(1, Ordering::Relaxed);
        job.total_bytes.fetch_add(task.size, Ordering::Relaxed);
        self.files.push(task);
    }

    fn add_dir(&mut self, job: &DeleteJob, dir: PathBuf, item: usize) {
        job.total_items.fetch_add(1, Ordering::Relaxed);
        self.dirs.push((dir, item));
    }
}

fn permanent(job: &DeleteJob, notify: &dyn Fn()) {
    notify();
    let plan = scan(job);
    if job.is_cancelled() {
        return;
    }
    let workers = worker_count(job).min(plan.files.len().max(1));
    job.workers.store(workers as u64, Ordering::Relaxed);
    job.set_state(DeleteState::Running);
    notify();
    remove_files(job, plan.files, workers);
    if !job.checkpoint() {
        return;
    }
    remove_dirs(job, &plan.dirs);
}

/// Walk the selected items once: what to remove, and how much.
fn scan(job: &DeleteJob) -> Plan {
    let mut plan = Plan::default();
    for (item, root) in job.paths.iter().enumerate() {
        if job.is_cancelled() {
            break;
        }
        match fs::symlink_metadata(root) {
            // Already gone: nothing to do.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => job.push_error(root, root, e.to_string()),
            Ok(m) if m.is_dir() => walk(job, root, item, &mut plan),
            Ok(m) => plan.add_file(
                job,
                FileTask {
                    path: root.clone(),
                    size: m.len(),
                    item,
                    dir_link: is_dir_link(&m.file_type()),
                },
            ),
        }
    }
    plan
}

fn walk(job: &DeleteJob, root: &Path, item: usize, plan: &mut Plan) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if job.is_cancelled() {
            return;
        }
        job.set_current(&dir);
        let entries = fs::read_dir(&dir);
        plan.add_dir(job, dir.clone(), item);
        let entries = match entries {
            Ok(rd) => rd,
            Err(e) => {
                job.push_error(&dir, root, e.to_string());
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                // `is_dir` is false for links and junctions: they aren't entered.
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) => {
                    let size = if t.is_symlink() {
                        0
                    } else {
                        entry.metadata().map_or(0, |m| m.len())
                    };
                    plan.add_file(
                        job,
                        FileTask {
                            path,
                            size,
                            item,
                            dir_link: is_dir_link(&t),
                        },
                    );
                }
                Err(e) => job.push_error(&path, root, e.to_string()),
            }
        }
    }
}

fn remove_files(job: &DeleteJob, files: Vec<FileTask>, workers: usize) {
    let (tx, rx) = crossbeam_channel::unbounded();
    for task in files {
        let _ = tx.send(task);
    }
    drop(tx);
    thread::scope(|scope| {
        for _ in 0..workers {
            let rx = rx.clone();
            scope.spawn(move || {
                for task in rx.iter() {
                    if !job.checkpoint() {
                        break;
                    }
                    job.set_current(&task.path);
                    let result = if task.dir_link {
                        remove_dir_forced(&task.path)
                    } else {
                        crate::fsops::remove_file_forced(&task.path)
                    };
                    match result {
                        Err(e) if e.kind() != io::ErrorKind::NotFound => {
                            job.push_error(&task.path, &job.paths[task.item], e.to_string());
                        }
                        _ => {
                            job.done_items.fetch_add(1, Ordering::Relaxed);
                            job.done_bytes.fetch_add(task.size, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });
}

/// Remove the (now empty) folders, deepest first. A folder that still holds
/// something that failed stays without a second error.
fn remove_dirs(job: &DeleteJob, dirs: &[(PathBuf, usize)]) {
    let mut blocked: HashSet<PathBuf> = HashSet::new();
    let block = |blocked: &mut HashSet<PathBuf>, path: &Path| {
        for parent in path.ancestors().skip(1) {
            if !blocked.insert(parent.to_path_buf()) {
                break; // its parents are in already
            }
        }
    };
    for e in job.errors.lock().unwrap().iter() {
        block(&mut blocked, &e.path);
    }
    for (dir, item) in dirs.iter().rev() {
        if !job.checkpoint() {
            return;
        }
        job.set_current(dir);
        match remove_dir_forced(dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                if !blocked.contains(dir) {
                    job.push_error(dir, &job.paths[*item], e.to_string());
                }
                block(&mut blocked, dir);
            }
            _ => {
                job.done_items.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Remove an empty folder (or a link to one), clearing the read-only flag
/// first if needed.
fn remove_dir_forced(dir: &Path) -> io::Result<()> {
    match fs::remove_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            // Not for links: setting permissions would change their target.
            let meta = fs::symlink_metadata(dir)?;
            if !meta.permissions().readonly() || meta.file_type().is_symlink() {
                return Err(e);
            }
            let mut perms = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            fs::set_permissions(dir, perms)?;
            fs::remove_dir(dir)
        }
        other => other,
    }
}

fn is_dir_link(t: &fs::FileType) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        t.is_symlink_dir()
    }
    #[cfg(not(windows))]
    {
        let _ = t;
        false
    }
}

/// How many files to delete in parallel: one on HDDs (parallel deletes
/// mostly add seeking), eight on SSDs, four when unknown (network shares).
fn worker_count(job: &DeleteJob) -> usize {
    match job
        .paths
        .first()
        .and_then(|p| crate::disk::is_rotational(p))
    {
        Some(true) => 1,
        Some(false) => 8,
        None => 4,
    }
}
