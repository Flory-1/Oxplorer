//! The Oxplorer delete engine: deletes items as jobs that report progress
//! and can be paused or cancelled, next to the copy/move engine in
//! [`crate::transfer`].
//!
//! How a job runs:
//!
//! 1. **End programs** – processes the user chose to end (because they held
//!    the items open) are ended first.
//! 2. **Scan** – for permanent deletes the trees are walked once to count
//!    files, folders and bytes, so progress has a total. Links and junctions
//!    are counted as one item and never followed.
//! 3. **Delete** – permanently: files are removed by a pool of worker threads
//!    (one on HDDs, more on SSDs), read-only ones included, then the folders
//!    from the deepest up. To the Recycle Bin: the items go in small batches,
//!    so a large selection shows progress and can be cancelled in between.
//!
//! What couldn't be deleted is collected per path; a folder that only stays
//! because something inside it failed is not reported a second time.

mod run;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

/// Jobs that run at least this long are worth showing with progress.
const SHOW_AFTER: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteMode {
    /// Move to the Recycle Bin (can be undone).
    Trash,
    Permanent,
}

#[derive(Debug, Clone)]
pub struct DeleteRequest {
    pub paths: Vec<PathBuf>,
    pub mode: DeleteMode,
    /// Process ids of programs to end before deleting (they held the items open).
    pub end: Vec<u32>,
}

impl DeleteRequest {
    pub fn new(paths: Vec<PathBuf>, mode: DeleteMode) -> Self {
        Self {
            paths,
            mode,
            end: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DeleteState {
    Scanning = 0,
    Running = 1,
    Completed = 2,
    Cancelled = 3,
}

impl DeleteState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Scanning,
            1 => Self::Running,
            2 => Self::Completed,
            _ => Self::Cancelled,
        }
    }

    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Debug, Clone)]
pub struct DeleteError {
    /// What couldn't be deleted (may be a file deep inside `item`).
    pub path: PathBuf,
    /// The selected item it belongs to (deleting that again retries it).
    pub item: PathBuf,
    pub message: String,
}

/// A running or finished delete job. Shared between the UI and the workers.
pub struct DeleteJob {
    pub id: u64,
    pub paths: Vec<PathBuf>,
    pub mode: DeleteMode,
    end: Vec<u32>,
    /// When the job started, in Unix seconds (see [`crate::undo::now`]);
    /// Recycle Bin items deleted since then can be put back.
    pub since: i64,
    state: AtomicU8,
    /// Files, folders and links to delete (top-level items for the Recycle Bin).
    pub total_items: AtomicU64,
    pub done_items: AtomicU64,
    /// Bytes of the files to delete (permanent deletes only).
    pub total_bytes: AtomicU64,
    pub done_bytes: AtomicU64,
    /// Parallel deletes actually used (after drive detection).
    pub workers: AtomicU64,
    paused: AtomicBool,
    cancelled: AtomicBool,
    current: Mutex<PathBuf>,
    errors: Mutex<Vec<DeleteError>>,
    /// Programs that couldn't be ended.
    notes: Mutex<Vec<String>>,
    started: Instant,
    finished: Mutex<Option<Instant>>,
}

impl DeleteJob {
    fn new(id: u64, req: DeleteRequest) -> Self {
        Self {
            id,
            paths: req.paths,
            mode: req.mode,
            end: req.end,
            since: crate::undo::now(),
            state: AtomicU8::new(DeleteState::Scanning as u8),
            total_items: AtomicU64::new(0),
            done_items: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            done_bytes: AtomicU64::new(0),
            workers: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            current: Mutex::new(PathBuf::new()),
            errors: Mutex::new(Vec::new()),
            notes: Mutex::new(Vec::new()),
            started: Instant::now(),
            finished: Mutex::new(None),
        }
    }

    pub fn state(&self) -> DeleteState {
        DeleteState::from_u8(self.state.load(Ordering::Acquire))
    }

    fn set_state(&self, s: DeleteState) {
        self.state.store(s as u8, Ordering::Release);
        if s.is_finished() {
            *self.finished.lock().unwrap() = Some(Instant::now());
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn errors(&self) -> Vec<DeleteError> {
        self.errors.lock().unwrap().clone()
    }

    pub fn error_count(&self) -> usize {
        self.errors.lock().unwrap().len()
    }

    /// Why programs couldn't be ended.
    pub fn notes(&self) -> Vec<String> {
        self.notes.lock().unwrap().clone()
    }

    /// The selected items that are gone now.
    pub fn removed(&self) -> Vec<PathBuf> {
        self.paths
            .iter()
            .filter(|p| !crate::fsops::exists(p))
            .cloned()
            .collect()
    }

    /// The file being deleted.
    pub fn current_item(&self) -> PathBuf {
        self.current.lock().unwrap().clone()
    }

    /// Progress from 0.0 to 1.0, by item count.
    pub fn progress(&self) -> f32 {
        let total = self.total_items.load(Ordering::Relaxed);
        if total == 0 {
            return if self.state().is_finished() { 1.0 } else { 0.0 };
        }
        let done = self.done_items.load(Ordering::Relaxed);
        (done as f64 / total as f64).clamp(0.0, 1.0) as f32
    }

    /// Time since the job started (until it finished).
    pub fn elapsed(&self) -> Duration {
        match *self.finished.lock().unwrap() {
            Some(end) => end.saturating_duration_since(self.started),
            None => self.started.elapsed(),
        }
    }

    /// Still running after a moment: worth showing with progress (quick
    /// deletes would only flash by).
    pub fn is_slow(&self) -> bool {
        !self.state().is_finished() && self.elapsed() >= SHOW_AFTER
    }

    // ---- worker side -------------------------------------------------------

    fn set_current(&self, path: &Path) {
        if let Ok(mut c) = self.current.try_lock() {
            c.clear();
            c.push(path);
        }
    }

    fn push_error(&self, path: &Path, item: &Path, message: impl Into<String>) {
        self.errors.lock().unwrap().push(DeleteError {
            path: path.to_path_buf(),
            item: item.to_path_buf(),
            message: message.into(),
        });
    }

    /// Block while paused. Returns `false` if the job was cancelled.
    fn checkpoint(&self) -> bool {
        while self.is_paused() && !self.is_cancelled() {
            thread::sleep(Duration::from_millis(50));
        }
        !self.is_cancelled()
    }
}

type Notifier = Arc<dyn Fn() + Send + Sync>;

/// Owns all delete jobs. Each job runs on its own thread right away.
#[derive(Default)]
pub struct DeleteEngine {
    jobs: Mutex<Vec<Arc<DeleteJob>>>,
    next_id: AtomicU64,
    notifier: Arc<RwLock<Option<Notifier>>>,
}

impl DeleteEngine {
    /// Called whenever a job changes state (e.g. to request a UI repaint).
    pub fn set_notifier(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.notifier.write().unwrap() = Some(Arc::new(f));
    }

    /// Start a new job and return a handle to it.
    pub fn submit(&self, req: DeleteRequest) -> Arc<DeleteJob> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let job = Arc::new(DeleteJob::new(id, req));
        self.jobs.lock().unwrap().push(job.clone());
        let notifier = self.notifier.clone();
        let worker_job = job.clone();
        thread::Builder::new()
            .name(format!("delete-{id}"))
            .spawn(move || {
                let notify = || {
                    if let Some(n) = notifier.read().unwrap().as_ref() {
                        n();
                    }
                };
                run::run(&worker_job, &notify);
                worker_job.set_state(if worker_job.is_cancelled() {
                    DeleteState::Cancelled
                } else {
                    DeleteState::Completed
                });
                notify();
            })
            .expect("failed to spawn delete thread");
        job
    }

    pub fn jobs(&self) -> Vec<Arc<DeleteJob>> {
        self.jobs.lock().unwrap().clone()
    }

    pub fn has_active(&self) -> bool {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .any(|j| !j.state().is_finished())
    }

    /// Forget a finished job.
    pub fn remove(&self, id: u64) {
        self.jobs
            .lock()
            .unwrap()
            .retain(|j| j.id != id || !j.state().is_finished());
    }
}

#[cfg(test)]
mod tests;
