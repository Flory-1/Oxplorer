//! The Oxplorer transfer engine: a custom copy/move implementation that
//! replaces the (slow) Windows shell copy dialog.
//!
//! How a job runs:
//!
//! 1. **Scan** – the source trees are walked once to count bytes/files and to
//!    find every conflict up front. Same-volume moves are done right here with a
//!    single `rename` and never touch file contents.
//! 2. **Decide** – if there are conflicts and the policy is
//!    [`ConflictPolicy::Ask`], the job waits for a *single* decision for all of
//!    them instead of interrupting the user once per file.
//! 3. **Transfer** – folders are created, then files are copied by a pool of
//!    worker threads. On Windows each file goes through `CopyFileExW` (the same
//!    kernel fast path Explorer uses, without the shell UI overhead), with
//!    unbuffered I/O for very large files. Elsewhere a large-buffer copy loop
//!    is used.
//!
//! Jobs can be paused, resumed and cancelled at any time and report live
//! progress through lock-free counters, so the UI can poll them every frame.

mod copy;
mod plan;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub use copy::CopyOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferKind {
    Copy,
    Move,
}

impl TransferKind {
    pub fn verb(self) -> &'static str {
        match self {
            Self::Copy => "Copying",
            Self::Move => "Moving",
        }
    }
}

/// What to do when a file already exists at the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConflictPolicy {
    /// Pause the job and ask the user once for all conflicts.
    #[default]
    Ask,
    Overwrite,
    Skip,
    /// Keep both files by giving the new one a numbered name.
    KeepBoth,
    /// Overwrite only if the source is newer than the existing file.
    OverwriteIfNewer,
}

impl ConflictPolicy {
    pub const ALL: [ConflictPolicy; 5] = [
        Self::Ask,
        Self::Overwrite,
        Self::Skip,
        Self::KeepBoth,
        Self::OverwriteIfNewer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "Ask me",
            Self::Overwrite => "Replace",
            Self::Skip => "Skip",
            Self::KeepBoth => "Keep both",
            Self::OverwriteIfNewer => "Replace if newer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub kind: TransferKind,
    pub sources: Vec<PathBuf>,
    /// Folder the sources are copied/moved into.
    pub destination: PathBuf,
    pub conflict: ConflictPolicy,
}

/// Tunables for the engine. Can be changed at runtime; new jobs pick up the
/// current values.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Number of files copied in parallel within one job.
    pub workers: usize,
    /// Buffer size for the portable copy loop.
    pub buffer_size: usize,
    /// Files at least this large use unbuffered I/O on Windows.
    pub unbuffered_threshold: u64,
    /// How many jobs may run at the same time; extra jobs wait in a queue.
    pub max_concurrent_jobs: usize,
    pub preserve_timestamps: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            workers: 4,
            buffer_size: 4 * 1024 * 1024,
            unbuffered_threshold: 256 * 1024 * 1024,
            max_concurrent_jobs: 2,
            preserve_timestamps: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum JobState {
    Queued = 0,
    Scanning = 1,
    AwaitingDecision = 2,
    Running = 3,
    Completed = 4,
    Cancelled = 5,
}

impl JobState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Queued,
            1 => Self::Scanning,
            2 => Self::AwaitingDecision,
            3 => Self::Running,
            4 => Self::Completed,
            _ => Self::Cancelled,
        }
    }

    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Debug, Clone)]
pub struct TransferError {
    pub path: PathBuf,
    pub message: String,
}

/// A running or finished transfer job. Shared between the UI and the workers.
pub struct Job {
    pub id: u64,
    pub kind: TransferKind,
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
    conflict_policy: ConflictPolicy,

    state: AtomicU8,
    pub total_bytes: AtomicU64,
    pub done_bytes: AtomicU64,
    pub total_files: AtomicU64,
    pub done_files: AtomicU64,
    pub skipped_files: AtomicU64,
    paused: AtomicBool,
    cancelled: AtomicBool,

    current: Mutex<PathBuf>,
    errors: Mutex<Vec<TransferError>>,
    conflicts: Mutex<Vec<PathBuf>>,
    decision: Mutex<Option<ConflictPolicy>>,
    started: Mutex<Instant>,
    finished: Mutex<Option<Instant>>,
    speed: Mutex<SpeedMeter>,
}

impl Job {
    fn new(id: u64, req: &TransferRequest) -> Self {
        Self {
            id,
            kind: req.kind,
            sources: req.sources.clone(),
            destination: req.destination.clone(),
            conflict_policy: req.conflict,
            state: AtomicU8::new(JobState::Queued as u8),
            total_bytes: AtomicU64::new(0),
            done_bytes: AtomicU64::new(0),
            total_files: AtomicU64::new(0),
            done_files: AtomicU64::new(0),
            skipped_files: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            current: Mutex::new(PathBuf::new()),
            errors: Mutex::new(Vec::new()),
            conflicts: Mutex::new(Vec::new()),
            decision: Mutex::new(None),
            started: Mutex::new(Instant::now()),
            finished: Mutex::new(None),
            speed: Mutex::new(SpeedMeter::default()),
        }
    }

    pub fn state(&self) -> JobState {
        JobState::from_u8(self.state.load(Ordering::Acquire))
    }

    fn set_state(&self, s: JobState) {
        if s == JobState::Running {
            // Don't count the time spent waiting in the queue or for a conflict decision.
            *self.started.lock().unwrap() = Instant::now();
        }
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

    /// Answer the conflict prompt of a job in [`JobState::AwaitingDecision`].
    pub fn resolve_conflicts(&self, policy: ConflictPolicy) {
        *self.decision.lock().unwrap() = Some(policy);
    }

    /// Destination paths that already existed when the job was scanned.
    pub fn conflicts(&self) -> Vec<PathBuf> {
        self.conflicts.lock().unwrap().clone()
    }

    pub fn errors(&self) -> Vec<TransferError> {
        self.errors.lock().unwrap().clone()
    }

    pub fn error_count(&self) -> usize {
        self.errors.lock().unwrap().len()
    }

    /// The file currently being scanned or copied.
    pub fn current_item(&self) -> PathBuf {
        self.current.lock().unwrap().clone()
    }

    /// Progress from 0.0 to 1.0.
    pub fn progress(&self) -> f32 {
        let total = self.total_bytes.load(Ordering::Relaxed);
        let done = self.done_bytes.load(Ordering::Relaxed);
        if total == 0 {
            let files = self.total_files.load(Ordering::Relaxed);
            if files == 0 {
                return if self.state().is_finished() { 1.0 } else { 0.0 };
            }
            return self.done_files.load(Ordering::Relaxed) as f32 / files as f32;
        }
        (done as f64 / total as f64).clamp(0.0, 1.0) as f32
    }

    /// Current throughput in bytes per second (averaged over the last seconds).
    pub fn speed(&self) -> f64 {
        if self.state() != JobState::Running || self.is_paused() {
            return 0.0;
        }
        self.speed
            .lock()
            .unwrap()
            .sample(self.done_bytes.load(Ordering::Relaxed))
    }

    /// Estimated time remaining.
    pub fn eta(&self) -> Option<Duration> {
        let speed = self.speed();
        if speed < 1.0 {
            return None;
        }
        let left = self
            .total_bytes
            .load(Ordering::Relaxed)
            .saturating_sub(self.done_bytes.load(Ordering::Relaxed));
        Some(Duration::from_secs_f64(left as f64 / speed))
    }

    /// Time spent transferring (excludes queueing and waiting for a decision).
    pub fn elapsed(&self) -> Duration {
        let started = *self.started.lock().unwrap();
        match *self.finished.lock().unwrap() {
            Some(end) => end.saturating_duration_since(started),
            None => started.elapsed(),
        }
    }

    // ---- worker side -------------------------------------------------------

    fn set_current(&self, path: &Path) {
        if let Ok(mut c) = self.current.try_lock() {
            c.clear();
            c.push(path);
        }
    }

    fn push_error(&self, path: &Path, message: impl Into<String>) {
        self.errors.lock().unwrap().push(TransferError {
            path: path.to_path_buf(),
            message: message.into(),
        });
    }

    /// Block while paused. Returns `false` if the job was cancelled.
    fn checkpoint(&self) -> bool {
        while self.paused.load(Ordering::Relaxed) && !self.is_cancelled() {
            thread::sleep(Duration::from_millis(50));
        }
        !self.is_cancelled()
    }

    fn wait_for_decision(&self) -> Option<ConflictPolicy> {
        loop {
            if self.is_cancelled() {
                return None;
            }
            if let Some(d) = self.decision.lock().unwrap().take() {
                return Some(d);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Moving-window throughput estimate.
#[derive(Default)]
struct SpeedMeter {
    samples: VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    const WINDOW: Duration = Duration::from_secs(3);

    fn sample(&mut self, bytes: u64) -> f64 {
        let now = Instant::now();
        if self
            .samples
            .back()
            .is_none_or(|(t, _)| now - *t > Duration::from_millis(100))
        {
            self.samples.push_back((now, bytes));
        }
        while self.samples.len() > 2 && now - self.samples[0].0 > Self::WINDOW {
            self.samples.pop_front();
        }
        match (self.samples.front(), self.samples.back()) {
            (Some((t0, b0)), Some((t1, b1))) if t1 > t0 => {
                (b1 - b0) as f64 / (*t1 - *t0).as_secs_f64()
            }
            _ => 0.0,
        }
    }
}

type Notifier = Arc<dyn Fn() + Send + Sync>;

/// Owns all transfer jobs. Cheap to share: wrap it in an `Arc` or keep one per app.
pub struct TransferEngine {
    config: Arc<RwLock<EngineConfig>>,
    jobs: Mutex<Vec<Arc<Job>>>,
    next_id: AtomicU64,
    running: Arc<(Mutex<usize>, Condvar)>,
    notifier: Arc<RwLock<Option<Notifier>>>,
}

impl Default for TransferEngine {
    fn default() -> Self {
        Self::new(EngineConfig::default())
    }
}

impl TransferEngine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            jobs: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            running: Arc::new((Mutex::new(0), Condvar::new())),
            notifier: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_config(&self, config: EngineConfig) {
        *self.config.write().unwrap() = config;
        self.running.1.notify_all();
    }

    /// Called whenever a job changes state (e.g. to request a UI repaint).
    pub fn set_notifier(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.notifier.write().unwrap() = Some(Arc::new(f));
    }

    /// Start a new job and return a handle to it.
    pub fn submit(&self, req: TransferRequest) -> Arc<Job> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let job = Arc::new(Job::new(id, &req));
        self.jobs.lock().unwrap().push(job.clone());

        let config = self.config.clone();
        let running = self.running.clone();
        let notifier = self.notifier.clone();
        let worker_job = job.clone();
        thread::Builder::new()
            .name(format!("transfer-{id}"))
            .spawn(move || {
                let notify = || {
                    if let Some(n) = notifier.read().unwrap().as_ref() {
                        n();
                    }
                };
                // Wait for a free job slot (queue).
                {
                    let (lock, cv) = &*running;
                    let mut count = lock.lock().unwrap();
                    loop {
                        if worker_job.is_cancelled() {
                            worker_job.set_state(JobState::Cancelled);
                            notify();
                            return;
                        }
                        let max = config.read().unwrap().max_concurrent_jobs.max(1);
                        if *count < max {
                            *count += 1;
                            break;
                        }
                        count = cv
                            .wait_timeout(count, Duration::from_millis(200))
                            .unwrap()
                            .0;
                    }
                }
                let cfg = config.read().unwrap().clone();
                run_job(&worker_job, &cfg, &notify);
                {
                    let (lock, cv) = &*running;
                    *lock.lock().unwrap() -= 1;
                    cv.notify_all();
                }
                notify();
            })
            .expect("failed to spawn transfer thread");
        job
    }

    pub fn jobs(&self) -> Vec<Arc<Job>> {
        self.jobs.lock().unwrap().clone()
    }

    pub fn has_active(&self) -> bool {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .any(|j| !j.state().is_finished())
    }

    /// Forget all finished jobs.
    pub fn clear_finished(&self) {
        self.jobs
            .lock()
            .unwrap()
            .retain(|j| !j.state().is_finished());
    }

    pub fn remove(&self, id: u64) {
        self.jobs
            .lock()
            .unwrap()
            .retain(|j| j.id != id || !j.state().is_finished());
    }
}

fn run_job(job: &Job, cfg: &EngineConfig, notify: &dyn Fn()) {
    job.set_state(JobState::Scanning);
    notify();
    let mut plan = plan::build(job);
    if job.is_cancelled() {
        job.set_state(JobState::Cancelled);
        return;
    }

    let mut policy = job.conflict_policy;
    if plan.has_conflicts() && policy == ConflictPolicy::Ask {
        *job.conflicts.lock().unwrap() = plan.conflict_paths();
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
    plan.apply_policy(job, policy);

    job.set_state(JobState::Running);
    notify();
    plan::execute(job, plan, cfg);

    job.set_state(if job.is_cancelled() {
        JobState::Cancelled
    } else {
        JobState::Completed
    });
}

#[cfg(test)]
mod tests;
