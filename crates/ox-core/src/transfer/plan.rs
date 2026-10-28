//! Scanning sources into a transfer plan, resolving conflicts and executing it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use super::copy::{self, CopyError, CopyOptions};
use super::throttle::Throttle;
use super::{verify, ConflictPolicy, EngineConfig, Job, TransferKind};
use crate::fsops;

#[derive(Debug)]
pub(super) struct FileTask {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Destination already existed during the scan.
    pub conflict: bool,
}

#[derive(Debug, Default)]
pub(super) struct Plan {
    /// Folders to create, parents before children.
    pub dirs: Vec<PathBuf>,
    pub files: Vec<FileTask>,
    /// Source folders to remove after a cross-volume move, children before parents.
    pub cleanup_dirs: Vec<PathBuf>,
}

impl Plan {
    pub fn has_conflicts(&self) -> bool {
        self.files.iter().any(|f| f.conflict)
    }

    pub fn conflict_paths(&self) -> Vec<PathBuf> {
        self.files
            .iter()
            .filter(|f| f.conflict)
            .map(|f| f.dst.clone())
            .collect()
    }

    /// Decide what happens to every conflicting file.
    pub fn apply_policy(&mut self, job: &Job, policy: ConflictPolicy) {
        let mut skipped_bytes = 0;
        let mut skipped_files = 0;
        self.files.retain_mut(|task| {
            if !task.conflict {
                return true;
            }
            let keep = match policy {
                ConflictPolicy::Overwrite => true,
                ConflictPolicy::Skip | ConflictPolicy::Ask => false,
                ConflictPolicy::KeepBoth => {
                    let name = task.dst.file_name().unwrap_or_default().to_string_lossy();
                    let parent = task.dst.parent().unwrap_or(Path::new(""));
                    task.dst = fsops::unique_path(parent, &name);
                    task.conflict = false;
                    true
                }
                ConflictPolicy::OverwriteIfNewer => {
                    let existing = fs::metadata(&task.dst).and_then(|m| m.modified()).ok();
                    match (task.modified, existing) {
                        (Some(src), Some(dst)) => src > dst,
                        _ => true,
                    }
                }
            };
            if !keep {
                skipped_bytes += task.size;
                skipped_files += 1;
            }
            keep
        });
        job.total_bytes.fetch_sub(skipped_bytes, Ordering::Relaxed);
        job.total_files.fetch_sub(skipped_files, Ordering::Relaxed);
        job.skipped_files
            .fetch_add(skipped_files, Ordering::Relaxed);
    }
}

/// Walk all sources and build the plan. Same-volume moves happen right here.
pub(super) fn build(job: &Job) -> Plan {
    let mut plan = Plan::default();
    // Explicit files (a retry of failed files): each with its exact target.
    for (src, dst) in &job.pairs {
        match fs::symlink_metadata(src) {
            Ok(meta) if !meta.is_dir() => {
                if let Some(parent) = dst.parent() {
                    plan.dirs.push(parent.to_path_buf());
                }
                add_file(job, src.clone(), dst.clone(), &meta, &mut plan);
            }
            Ok(_) => job.push_error(src, crate::i18n::t("Expected a file but found a folder.")),
            Err(e) => job.push_error(src, e.to_string()),
        }
    }
    for src in &job.sources {
        if job.is_cancelled() {
            break;
        }
        let meta = match fs::symlink_metadata(src) {
            Ok(m) => m,
            Err(e) => {
                job.push_error(src, e.to_string());
                continue;
            }
        };
        let Some(name) = src.file_name() else {
            job.push_error(src, "Can't copy or move a drive root.");
            continue;
        };
        let is_dir = meta.is_dir() && !is_link_dir(&meta);
        if is_dir && job.destination.starts_with(src) {
            job.push_error(
                src,
                crate::i18n::t("The destination folder is inside the source folder."),
            );
            continue;
        }

        let mut dst = job.destination.join(name);
        if same_path(src, &dst) {
            match job.kind {
                // Pasting into the same folder makes "name - Copy".
                TransferKind::Copy => {
                    dst = fsops::copy_name_path(&job.destination, &name.to_string_lossy())
                }
                TransferKind::Move => continue,
            }
        }

        // Fast path: a move to a free name on the same volume is just a rename.
        if job.kind == TransferKind::Move && !fsops::exists(&dst) && fs::rename(src, &dst).is_ok() {
            job.total_files.fetch_add(1, Ordering::Relaxed);
            job.done_files.fetch_add(1, Ordering::Relaxed);
            continue;
        }

        if is_dir {
            if let Ok(m) = fs::symlink_metadata(&dst) {
                if !m.is_dir() {
                    job.push_error(
                        src,
                        crate::i18n::t("A file with the same name as this folder already exists."),
                    );
                    continue;
                }
            }
            scan_dir(job, src, &dst, &mut plan);
        } else if meta.is_dir() {
            job.push_error(
                src,
                crate::i18n::t("Skipped a linked folder (symbolic link or junction)."),
            );
        } else {
            add_file(job, src.clone(), dst, &meta, &mut plan);
        }
    }
    plan
}

fn scan_dir(job: &Job, src: &Path, dst: &Path, plan: &mut Plan) {
    plan.dirs.push(dst.to_path_buf());
    job.set_current(src);
    let entries = match fs::read_dir(src) {
        Ok(e) => e,
        Err(e) => {
            job.push_error(src, e.to_string());
            return;
        }
    };
    for entry in entries {
        if job.is_cancelled() {
            return;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                job.push_error(src, e.to_string());
                continue;
            }
        };
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                job.push_error(&path, e.to_string());
                continue;
            }
        };
        let target = dst.join(entry.file_name());
        if meta.is_dir() {
            if is_link_dir(&meta) {
                job.push_error(
                    &path,
                    crate::i18n::t("Skipped a linked folder (symbolic link or junction)."),
                );
            } else {
                scan_dir(job, &path, &target, plan);
            }
        } else {
            add_file(job, path, target, &meta, plan);
        }
    }
    if job.kind == TransferKind::Move {
        plan.cleanup_dirs.push(src.to_path_buf());
    }
}

fn add_file(job: &Job, src: PathBuf, dst: PathBuf, meta: &fs::Metadata, plan: &mut Plan) {
    let size = meta.len();
    job.total_bytes.fetch_add(size, Ordering::Relaxed);
    job.total_files.fetch_add(1, Ordering::Relaxed);
    let conflict = fsops::exists(&dst);
    plan.files.push(FileTask {
        src,
        dst,
        size,
        modified: meta.modified().ok(),
        conflict,
    });
}

/// Symlinked folders and NTFS junctions are not followed.
fn is_link_dir(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        if meta.is_dir() && meta.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// Run the plan: create folders, then copy files in parallel.
pub(super) fn execute(job: &Job, plan: Plan, cfg: &EngineConfig, workers: usize) {
    for dir in &plan.dirs {
        if let Err(e) = fs::create_dir_all(dir) {
            job.push_error(dir, e.to_string());
        }
    }

    let opts = CopyOptions {
        buffer_size: cfg.buffer_size.max(64 * 1024),
        unbuffered_threshold: cfg.unbuffered_threshold,
        preserve_timestamps: cfg.preserve_timestamps,
    };
    let workers = workers.clamp(1, 64).min(plan.files.len().max(1));
    let throttle = Throttle::new(cfg.speed_limit);
    let ctx = FileCtx {
        opts: &opts,
        throttle: throttle.as_ref(),
        verify: cfg.verify,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    for task in plan.files {
        let _ = tx.send(task);
    }
    drop(tx);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let rx = rx.clone();
            let ctx = &ctx;
            scope.spawn(move || {
                for task in rx.iter() {
                    if !job.checkpoint() {
                        break;
                    }
                    job.set_current(&task.src);
                    transfer_file(job, &task, ctx);
                }
            });
        }
    });

    if job.kind == TransferKind::Move && !job.is_cancelled() {
        // Children were pushed before parents, so plain iteration removes bottom-up.
        // Folders that still hold skipped files are left alone.
        for dir in &plan.cleanup_dirs {
            let _ = fs::remove_dir(dir);
        }
    }
}

/// Settings shared by all workers of a job.
struct FileCtx<'a> {
    opts: &'a CopyOptions,
    throttle: Option<&'a Throttle>,
    verify: bool,
}

fn transfer_file(job: &Job, task: &FileTask, ctx: &FileCtx<'_>) {
    let opts = ctx.opts;
    let mut reported = 0u64;
    let result = (|| -> Result<(), CopyError> {
        if task.conflict {
            // We're overwriting: make sure a read-only target doesn't block us.
            let _ = fsops::clear_readonly(&task.dst);
        }
        if job.kind == TransferKind::Move {
            if task.conflict {
                let _ = fsops::remove_file_forced(&task.dst);
            }
            if fs::rename(&task.src, &task.dst).is_ok() {
                return Ok(());
            }
        }
        copy::copy_file(&task.src, &task.dst, task.size, opts, &mut |delta| {
            reported += delta;
            job.done_bytes.fetch_add(delta, Ordering::Relaxed);
            if let Some(t) = ctx.throttle {
                t.consume(delta);
            }
            job.checkpoint()
        })?;
        // The OS may report little or no progress (e.g. copy offload on
        // Windows); still apply the speed limit to the whole file.
        if let Some(t) = ctx.throttle {
            if task.size > reported {
                t.consume(task.size - reported);
            }
        }
        if ctx.verify {
            let checkpoint = || job.checkpoint();
            if !verify::same_contents(&task.src, &task.dst, opts.buffer_size, &checkpoint)? {
                // Never keep (or move away the original of) a bad copy.
                let _ = fsops::remove_file_forced(&task.dst);
                return Err(CopyError::Io(io::Error::other(crate::i18n::t(
                    "Verification failed: the copy differs from the original.",
                ))));
            }
            job.verified_files.fetch_add(1, Ordering::Relaxed);
        }
        if job.kind == TransferKind::Move {
            fsops::remove_file_forced(&task.src).map_err(CopyError::Io)?;
        }
        Ok(())
    })();

    // Keep the byte counter consistent no matter how the file ended.
    if task.size > reported {
        job.done_bytes
            .fetch_add(task.size - reported, Ordering::Relaxed);
    }
    match result {
        Ok(()) => {
            job.done_files.fetch_add(1, Ordering::Relaxed);
        }
        Err(CopyError::Cancelled) => {}
        Err(CopyError::Io(e)) => job.push_file_error(&task.src, Some(&task.dst), describe(&e)),
    }
}

fn describe(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => crate::i18n::t("Access denied.").into(),
        io::ErrorKind::NotFound => crate::i18n::t("The file no longer exists.").into(),
        _ => e.to_string(),
    }
}
