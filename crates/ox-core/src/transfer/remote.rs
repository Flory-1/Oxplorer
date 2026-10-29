//! Downloads and uploads: jobs with one side on a server ([`RemoteSide`]).
//!
//! They use the job's counters, pause and cancel, the speed limit, the
//! conflict decision and the error questions like local jobs, but run a
//! plan of their own, so the local fast paths stay untouched. What only
//! makes sense on local disks doesn't apply: `CopyFileExW`, renames within
//! a volume, the journal (resuming after a crash), verification, and
//! permissions/streams. Remote paths are carried as their addresses
//! ([`RemoteAddr::to_path`]).

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::SystemTime;

use super::throttle::Throttle;
use super::{ConflictPolicy, EngineConfig, ErrorChoice, Job, JobState, TransferKind};
use crate::fsops;
use crate::remote::{RemoteAddr, RemoteError, RemoteFs};

/// The server side of a job. Paths of the job on this server are its
/// addresses; all other paths are local.
#[derive(Clone)]
pub struct RemoteSide {
    pub fs: Arc<dyn RemoteFs>,
    pub server: RemoteAddr,
}

impl fmt::Debug for RemoteSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.server.server_url())
    }
}

impl RemoteSide {
    pub fn new(fs: Arc<dyn RemoteFs>, server: &RemoteAddr) -> Self {
        Self {
            fs,
            server: server.with_path(""),
        }
    }

    /// The remote path of `p`, if it is on this server.
    fn path_of(&self, p: &Path) -> Option<String> {
        RemoteAddr::from_path(p)
            .filter(|a| a.same_server(&self.server))
            .map(|a| a.path)
    }
}

#[derive(Debug, Clone, Copy)]
struct Meta {
    is_dir: bool,
    is_link: bool,
    size: u64,
    modified: Option<SystemTime>,
}

/// One side of the job: the local disk or the server.
#[derive(Clone, Copy)]
enum End<'a> {
    Local,
    Remote(&'a RemoteSide),
}

fn remote_err(e: RemoteError) -> String {
    e.to_string()
}

impl End<'_> {
    fn rpath(side: &RemoteSide, p: &Path) -> Result<String, String> {
        side.path_of(p)
            .ok_or_else(|| crate::i18n::t("This item isn't on the server.").to_owned())
    }

    fn meta(&self, p: &Path) -> Result<Option<Meta>, String> {
        match self {
            End::Local => match fs::symlink_metadata(p) {
                Ok(m) => {
                    let is_link = m.file_type().is_symlink();
                    Ok(Some(Meta {
                        is_dir: m.is_dir(),
                        is_link,
                        size: m.len(),
                        modified: m.modified().ok(),
                    }))
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.to_string()),
            },
            End::Remote(side) => {
                let path = Self::rpath(side, p)?;
                Ok(side.fs.stat(&path).map_err(remote_err)?.map(|e| Meta {
                    is_dir: e.is_dir,
                    is_link: false,
                    size: e.size,
                    modified: e.modified,
                }))
            }
        }
    }

    fn list(&self, p: &Path) -> Result<Vec<(String, Meta)>, String> {
        match self {
            End::Local => {
                let mut out = Vec::new();
                for entry in fs::read_dir(p).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if let Some(m) = End::Local.meta(&entry.path())? {
                        out.push((name, m));
                    }
                }
                Ok(out)
            }
            End::Remote(side) => {
                let path = Self::rpath(side, p)?;
                Ok(side
                    .fs
                    .list(&path)
                    .map_err(remote_err)?
                    .into_iter()
                    .filter(|e| e.name != "." && e.name != "..")
                    .map(|e| {
                        let meta = Meta {
                            is_dir: e.is_dir,
                            // Links to folders are followed on servers (they list as folders).
                            is_link: false,
                            size: e.size,
                            modified: e.modified,
                        };
                        (e.name, meta)
                    })
                    .collect())
            }
        }
    }

    fn join(&self, dir: &Path, name: &str) -> PathBuf {
        match self {
            End::Local => dir.join(name),
            End::Remote(_) => match RemoteAddr::from_path(dir) {
                Some(a) => a.join(name).to_path(),
                None => dir.join(name),
            },
        }
    }

    /// Create a folder (and the folders above it); an existing one is fine.
    fn mkdir_all(&self, p: &Path) -> Result<(), String> {
        match self {
            End::Local => fs::create_dir_all(p).map_err(|e| e.to_string()),
            End::Remote(side) => {
                let path = Self::rpath(side, p)?;
                let mut missing = Vec::new();
                let mut cur = Some(path);
                while let Some(dir) = cur.filter(|d| d != "/" && !d.is_empty()) {
                    match side.fs.stat(&dir).map_err(remote_err)? {
                        Some(e) if e.is_dir => break,
                        Some(_) => {
                            return Err(crate::i18n::t(
                                "A file with the same name as this folder already exists.",
                            )
                            .to_owned())
                        }
                        None => {
                            cur = crate::remote::url::parent_path(&dir);
                            missing.push(dir);
                        }
                    }
                }
                for dir in missing.into_iter().rev() {
                    side.fs.mkdir(&dir).map_err(remote_err)?;
                }
                Ok(())
            }
        }
    }

    fn remove_file(&self, p: &Path) -> Result<(), String> {
        match self {
            End::Local => fsops::remove_file_forced(p).map_err(|e| e.to_string()),
            End::Remote(side) => side
                .fs
                .remove_file(&Self::rpath(side, p)?)
                .map_err(remote_err),
        }
    }

    fn remove_dir(&self, p: &Path) {
        match self {
            End::Local => {
                let _ = fs::remove_dir(p);
            }
            End::Remote(side) => {
                if let Ok(path) = Self::rpath(side, p) {
                    let _ = side.fs.remove_dir(&path);
                }
            }
        }
    }
}

struct Task {
    src: PathBuf,
    dst: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    conflict: bool,
}

#[derive(Default)]
struct Plan {
    /// Folders to create, parents first.
    dirs: Vec<PathBuf>,
    files: Vec<Task>,
    /// Source folders to remove after a move, children first.
    cleanup_dirs: Vec<PathBuf>,
    /// New top-level items (source, destination), for [`Job::created`].
    roots: Vec<(PathBuf, PathBuf)>,
}

/// Builds the plan; remembers the destination folders it listed.
struct Planner<'a> {
    job: &'a Job,
    src: End<'a>,
    dst: End<'a>,
    listed: HashMap<PathBuf, HashMap<String, Meta>>,
    plan: Plan,
}

impl Planner<'_> {
    /// What is at `p` on the destination side (from one listing per folder).
    fn dst_meta(&mut self, p: &Path) -> Option<Meta> {
        if let End::Local = self.dst {
            return End::Local.meta(p).ok().flatten();
        }
        let (Some(parent), Some(name)) = (parent_of(p), name_of(p)) else {
            return None;
        };
        if !self.listed.contains_key(&parent) {
            let items = self.dst.list(&parent).unwrap_or_default();
            self.listed
                .insert(parent.clone(), items.into_iter().collect());
        }
        self.listed[&parent].get(&name).copied()
    }

    fn source(&mut self, src: &Path, dst: PathBuf) {
        let meta = match self.src.meta(src) {
            Ok(Some(m)) => m,
            Ok(None) => {
                return self
                    .job
                    .push_error(src, crate::i18n::t("The file no longer exists."))
            }
            Err(e) => return self.job.push_error(src, e),
        };
        let existing = self.dst_meta(&dst);
        if existing.is_none() {
            self.plan.roots.push((src.to_path_buf(), dst.clone()));
        }
        if meta.is_dir && meta.is_link {
            self.job.push_error(
                src,
                crate::i18n::t("Skipped a linked folder (symbolic link or junction)."),
            );
        } else if meta.is_dir {
            if existing.is_some_and(|m| !m.is_dir) {
                return self.job.push_error(
                    src,
                    crate::i18n::t("A file with the same name as this folder already exists."),
                );
            }
            self.scan_dir(src, &dst);
        } else {
            self.add_file(src.to_path_buf(), dst, meta, existing.is_some());
        }
    }

    fn scan_dir(&mut self, src: &Path, dst: &Path) {
        self.plan.dirs.push(dst.to_path_buf());
        self.job.set_current(src);
        let items = match self.src.list(src) {
            Ok(items) => items,
            Err(e) => return self.job.push_error(src, e),
        };
        for (name, meta) in items {
            if self.job.is_cancelled() {
                return;
            }
            let child = self.src.join(src, &name);
            let target = self.dst.join(dst, &name);
            if meta.is_dir && meta.is_link {
                self.job.push_error(
                    &child,
                    crate::i18n::t("Skipped a linked folder (symbolic link or junction)."),
                );
            } else if meta.is_dir {
                self.scan_dir(&child, &target);
            } else {
                let exists = self.dst_meta(&target).is_some();
                self.add_file(child, target, meta, exists);
            }
        }
        if self.job.kind == TransferKind::Move {
            self.plan.cleanup_dirs.push(src.to_path_buf());
        }
    }

    fn add_file(&mut self, src: PathBuf, dst: PathBuf, meta: Meta, conflict: bool) {
        self.job.total_bytes.fetch_add(meta.size, Ordering::Relaxed);
        self.job.total_files.fetch_add(1, Ordering::Relaxed);
        self.plan.files.push(Task {
            src,
            dst,
            size: meta.size,
            modified: meta.modified,
            conflict,
        });
    }

    /// Decide every conflicting file, like [`super::plan::Plan::apply_policy`].
    fn apply_policy(&mut self, policy: ConflictPolicy) {
        let mut files = std::mem::take(&mut self.plan.files);
        let (mut skipped_bytes, mut skipped_files) = (0, 0);
        files.retain_mut(|task| {
            if !task.conflict {
                return true;
            }
            let keep = match policy {
                ConflictPolicy::Overwrite => true,
                ConflictPolicy::Skip | ConflictPolicy::Ask => false,
                ConflictPolicy::KeepBoth => {
                    task.dst = self.free_name(&task.dst);
                    task.conflict = false;
                    true
                }
                ConflictPolicy::OverwriteIfNewer => {
                    match (
                        task.modified,
                        self.dst_meta(&task.dst).and_then(|m| m.modified),
                    ) {
                        (Some(src), Some(dst)) => src > dst,
                        _ => true,
                    }
                }
            };
            if !keep {
                skipped_bytes += task.size;
                skipped_files += 1;
            } else if policy == ConflictPolicy::KeepBoth || self.job.kind == TransferKind::Move {
                self.plan.roots.push((task.src.clone(), task.dst.clone()));
            }
            keep
        });
        self.plan.files = files;
        let job = self.job;
        job.total_bytes.fetch_sub(skipped_bytes, Ordering::Relaxed);
        job.total_files.fetch_sub(skipped_files, Ordering::Relaxed);
        job.skipped_files
            .fetch_add(skipped_files, Ordering::Relaxed);
    }

    /// `name (2).ext`, … : the first name that is free at the destination.
    fn free_name(&mut self, dst: &Path) -> PathBuf {
        let (Some(parent), Some(name)) = (parent_of(dst), name_of(dst)) else {
            return dst.to_path_buf();
        };
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name.as_str(), ""),
        };
        for i in 2.. {
            let candidate = self.dst.join(&parent, &format!("{stem} ({i}){ext}"));
            if self.dst_meta(&candidate).is_none() {
                // Taken from now on (for the next conflict in this folder).
                if let Some(list) = self.listed.get_mut(&parent) {
                    list.insert(
                        name_of(&candidate).unwrap_or_default(),
                        Meta {
                            is_dir: false,
                            is_link: false,
                            size: 0,
                            modified: None,
                        },
                    );
                }
                return candidate;
            }
        }
        unreachable!()
    }
}

fn parent_of(p: &Path) -> Option<PathBuf> {
    match RemoteAddr::from_path(p) {
        Some(a) => a.parent().map(|a| a.to_path()),
        None => p.parent().map(Path::to_path_buf),
    }
}

fn name_of(p: &Path) -> Option<String> {
    match RemoteAddr::from_path(p) {
        Some(a) => {
            Some(crate::remote::url::file_name(&a.path).to_owned()).filter(|n| !n.is_empty())
        }
        None => p.file_name().map(|n| n.to_string_lossy().into_owned()),
    }
}

/// Run a remote job (in place of the local scan/decide/transfer).
pub(super) fn run(job: &Job, side: &RemoteSide, cfg: &EngineConfig, notify: &dyn Fn()) {
    job.set_state(JobState::Scanning);
    notify();
    let upload = side.path_of(&job.destination).is_some();
    let (src, dst) = if upload {
        (End::Local, End::Remote(side))
    } else {
        (End::Remote(side), End::Local)
    };
    let mut planner = Planner {
        job,
        src,
        dst,
        listed: HashMap::new(),
        plan: Plan::default(),
    };
    for (s, d) in &job.pairs {
        if let Some(parent) = parent_of(d) {
            planner.plan.dirs.push(parent);
        }
        planner.source(s, d.clone());
    }
    for s in &job.sources {
        if job.is_cancelled() {
            break;
        }
        match name_of(s) {
            Some(name) => {
                let d = dst.join(&job.destination, &name);
                planner.source(s, d);
            }
            None => job.push_error(s, "Can't copy or move a drive root."),
        }
    }
    if job.is_cancelled() {
        job.set_state(JobState::Cancelled);
        return;
    }

    let mut policy = job.conflict_policy;
    if planner.plan.files.iter().any(|f| f.conflict) && policy == ConflictPolicy::Ask {
        *job.conflicts.lock().unwrap() = planner
            .plan
            .files
            .iter()
            .filter(|f| f.conflict)
            .map(|f| f.dst.clone())
            .collect();
        job.set_state(JobState::AwaitingDecision);
        notify();
        match job.wait_for_decision() {
            Some(p) => policy = p,
            None => {
                job.set_state(JobState::Cancelled);
                return;
            }
        }
    }
    planner.apply_policy(policy);
    let plan = planner.plan;

    let workers = side.fs.parallel().clamp(1, 8).min(plan.files.len().max(1));
    job.workers.store(workers as u64, Ordering::Relaxed);
    job.set_state(JobState::Running);
    notify();
    execute(job, &plan, src, dst, cfg, workers);
    // Downloads can be undone like local copies; uploads aren't listed
    // (undo only knows local files).
    if !upload {
        job.created
            .lock()
            .unwrap()
            .extend(plan.roots.into_iter().filter(|(_, d)| fsops::exists(d)));
    }
    job.set_state(if job.is_cancelled() {
        JobState::Cancelled
    } else {
        JobState::Completed
    });
}

fn execute(job: &Job, plan: &Plan, src: End<'_>, dst: End<'_>, cfg: &EngineConfig, workers: usize) {
    for dir in &plan.dirs {
        if job.is_cancelled() {
            return;
        }
        if let Err(e) = dst.mkdir_all(dir) {
            job.push_error(dir, e);
        }
    }
    let throttle = Throttle::new(cfg.speed_limit);
    let ctx = Ctx {
        src,
        dst,
        throttle: throttle.as_ref(),
        ask: cfg.ask_on_error,
        keep_dates: cfg.preserve_timestamps,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    for task in &plan.files {
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
                    while transfer_file(job, task, ctx) {}
                }
            });
        }
    });
    if job.kind == TransferKind::Move && !job.is_cancelled() {
        // Folders that still hold skipped files stay.
        for dir in &plan.cleanup_dirs {
            src.remove_dir(dir);
        }
    }
}

struct Ctx<'a> {
    src: End<'a>,
    dst: End<'a>,
    throttle: Option<&'a Throttle>,
    ask: bool,
    keep_dates: bool,
}

/// Counts the bytes, applies the speed limit and stops when cancelled.
struct Meter<'a, T> {
    inner: T,
    job: &'a Job,
    throttle: Option<&'a Throttle>,
    reported: u64,
}

impl<T> Meter<'_, T> {
    fn count(&mut self, n: usize) -> io::Result<()> {
        self.reported += n as u64;
        self.job.done_bytes.fetch_add(n as u64, Ordering::Relaxed);
        if let Some(t) = self.throttle {
            t.consume(n as u64);
        }
        if self.job.checkpoint() {
            Ok(())
        } else {
            // Not `Interrupted`: copy loops retry those.
            Err(io::Error::other(crate::i18n::t("Cancelled")))
        }
    }
}

impl<T: Write> Write for Meter<'_, T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count(n)?;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T: Read> Read for Meter<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count(n)?;
        Ok(n)
    }
}

/// The temporary name a file is written to before it gets its real name.
fn temp_name(end: End<'_>, dst: &Path) -> PathBuf {
    let parent = parent_of(dst).unwrap_or_default();
    let name = name_of(dst).unwrap_or_default();
    end.join(&parent, &format!(".{name}.oxpart"))
}

/// Returns `true` if the file failed and should be tried again.
fn transfer_file(job: &Job, task: &Task, ctx: &Ctx<'_>) -> bool {
    let mut reported = 0u64;
    let result = match (ctx.src, ctx.dst) {
        (End::Remote(side), End::Local) => download(job, side, task, ctx, &mut reported),
        (End::Local, End::Remote(side)) => upload(job, side, task, ctx, &mut reported),
        _ => Err("unsupported".to_owned()),
    };
    // Keep the byte counter consistent however the file ended.
    if task.size > reported {
        job.done_bytes
            .fetch_add(task.size - reported, Ordering::Relaxed);
    }
    let result = result.and_then(|()| {
        if job.kind == TransferKind::Move {
            ctx.src.remove_file(&task.src)
        } else {
            Ok(())
        }
    });
    match result {
        Ok(()) => {
            job.done_files.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) if job.is_cancelled() => {}
        Err(message) => {
            if ctx.ask && job.ask_about_error(&task.src, &task.dst, &message) == ErrorChoice::Retry
            {
                job.done_bytes
                    .fetch_sub(task.size.max(reported), Ordering::Relaxed);
                return true;
            }
            job.push_file_error(&task.src, Some(&task.dst), message);
        }
    }
    false
}

fn download(
    job: &Job,
    side: &RemoteSide,
    task: &Task,
    ctx: &Ctx<'_>,
    reported: &mut u64,
) -> Result<(), String> {
    let path = End::rpath(side, &task.src)?;
    let tmp = temp_name(End::Local, &task.dst);
    let file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut meter = Meter {
        inner: io::BufWriter::with_capacity(1024 * 1024, file),
        job,
        throttle: ctx.throttle,
        reported: 0,
    };
    let got = side.fs.read_to(&path, &mut meter).map_err(remote_err);
    *reported = meter.reported;
    let finished = got.and_then(|_| {
        let file = meter
            .inner
            .into_inner()
            .map_err(|e| e.error().to_string())?;
        if ctx.keep_dates {
            if let Some(t) = task.modified {
                let _ = file.set_modified(t);
            }
        }
        drop(file);
        if task.conflict {
            let _ = fsops::remove_file_forced(&task.dst);
        }
        fs::rename(&tmp, &task.dst).map_err(|e| e.to_string())
    });
    if finished.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    finished
}

fn upload(
    job: &Job,
    side: &RemoteSide,
    task: &Task,
    ctx: &Ctx<'_>,
    reported: &mut u64,
) -> Result<(), String> {
    let dst = End::rpath(side, &task.dst)?;
    let via_temp = side.fs.upload_via_temp();
    let target = if via_temp {
        End::rpath(side, &temp_name(ctx.dst, &task.dst))?
    } else {
        dst.clone()
    };
    let file = fs::File::open(&task.src).map_err(|e| e.to_string())?;
    let mut meter = Meter {
        inner: io::BufReader::with_capacity(1024 * 1024, file),
        job,
        throttle: ctx.throttle,
        reported: 0,
    };
    let sent = side.fs.write_from(&target, &mut meter, task.size);
    *reported = meter.reported;
    let finished = sent.and_then(|()| {
        if via_temp {
            if task.conflict {
                let _ = side.fs.remove_file(&dst);
            }
            side.fs.rename(&target, &dst)?;
        }
        if ctx.keep_dates {
            if let Some(t) = task.modified {
                let _ = side.fs.set_modified(&dst, t);
            }
        }
        Ok(())
    });
    if finished.is_err() && via_temp {
        let _ = side.fs.remove_file(&target);
    }
    finished.map_err(remote_err)
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
