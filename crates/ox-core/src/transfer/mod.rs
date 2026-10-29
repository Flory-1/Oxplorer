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
pub mod history;
mod plan;
mod throttle;
mod verify;

use std::collections::{HashMap, VecDeque};
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
        crate::i18n::t(match self {
            Self::Copy => "Copying",
            Self::Move => "Moving",
        })
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
        crate::i18n::t(match self {
            Self::Ask => "Ask me",
            Self::Overwrite => "Replace",
            Self::Skip => "Skip",
            Self::KeepBoth => "Keep both",
            Self::OverwriteIfNewer => "Replace if newer",
        })
    }
}

#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub kind: TransferKind,
    pub sources: Vec<PathBuf>,
    /// Folder the sources are copied/moved into.
    pub destination: PathBuf,
    pub conflict: ConflictPolicy,
    /// Individual files with their exact target path (used to retry failed
    /// files). Copied in addition to `sources`.
    pub pairs: Vec<(PathBuf, PathBuf)>,
}

impl TransferRequest {
    pub fn new(
        kind: TransferKind,
        sources: Vec<PathBuf>,
        destination: PathBuf,
        conflict: ConflictPolicy,
    ) -> Self {
        Self {
            kind,
            sources,
            destination,
            conflict,
            pairs: Vec::new(),
        }
    }
}

/// Tunables for the engine. Can be changed at runtime; new jobs pick up the
/// current values.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Number of files copied in parallel within one job.
    /// `0` = automatic: one at a time on HDDs, more on SSDs.
    pub workers: usize,
    /// Buffer size for the portable copy loop.
    pub buffer_size: usize,
    /// Files at least this large use unbuffered I/O on Windows.
    pub unbuffered_threshold: u64,
    /// How many jobs may run at the same time; extra jobs wait in a queue.
    pub max_concurrent_jobs: usize,
    /// Jobs touching the same drive run one after another; jobs on
    /// different drives run in parallel (up to `max_concurrent_jobs`).
    pub per_disk_queue: bool,
    pub preserve_timestamps: bool,
    /// Compare source and copy (xxh3 hash) after each file. Moves only delete
    /// the source when the copy is verified.
    pub verify: bool,
    /// Bytes per second per job, `0` = unlimited.
    pub speed_limit: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            workers: 0,
            buffer_size: 4 * 1024 * 1024,
            unbuffered_threshold: 256 * 1024 * 1024,
            max_concurrent_jobs: 4,
            per_disk_queue: true,
            preserve_timestamps: true,
            verify: false,
            speed_limit: 0,
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
    /// Where the file should have gone, for errors that can be retried.
    pub dest: Option<PathBuf>,
    pub message: String,
}

/// A running or finished transfer job. Shared between the UI and the workers.
pub struct Job {
    pub id: u64,
    pub kind: TransferKind,
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
    conflict_policy: ConflictPolicy,
    pairs: Vec<(PathBuf, PathBuf)>,
    /// Volumes this job reads from or writes to (for per-disk queueing).
    volumes: Vec<String>,
    /// Parallel file copies actually used (after drive detection).
    pub workers: AtomicU64,
    /// A spinning disk was detected, so files are copied one at a time.
    pub hdd: AtomicBool,
    pub verified_files: AtomicU64,

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
    /// See [`Job::created`].
    created: Mutex<Vec<(PathBuf, PathBuf)>>,
}

impl Job {
    fn new(id: u64, req: &TransferRequest) -> Self {
        Self {
            id,
            kind: req.kind,
            sources: req.sources.clone(),
            destination: req.destination.clone(),
            conflict_policy: req.conflict,
            pairs: req.pairs.clone(),
            volumes: job_volumes(req),
            workers: AtomicU64::new(0),
            hdd: AtomicBool::new(false),
            verified_files: AtomicU64::new(0),
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
            created: Mutex::new(Vec::new()),
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

    /// A new request that copies/moves just the files that failed, or `None`
    /// if no failed file can be retried.
    pub fn retry_request(&self) -> Option<TransferRequest> {
        let pairs: Vec<(PathBuf, PathBuf)> = self
            .errors
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| Some((e.path.clone(), e.dest.clone()?)))
            .collect();
        (!pairs.is_empty()).then(|| TransferRequest {
            kind: self.kind,
            sources: Vec::new(),
            destination: self.destination.clone(),
            // The failed copies were removed, so remaining conflicts are real ones.
            conflict: ConflictPolicy::Ask,
            pairs,
        })
    }

    /// What the job put in place, as (source, destination) pairs: each new
    /// top-level item (with its final name, e.g. after "keep both"), new
    /// items inside folders that were merged into existing ones and, for
    /// moves, replaced files. Skipped items are not listed. Complete once
    /// the job is finished (used to undo it).
    pub fn created(&self) -> Vec<(PathBuf, PathBuf)> {
        self.created.lock().unwrap().clone()
    }

    /// Total number of items the job was started with.
    pub fn item_count(&self) -> usize {
        self.sources.len() + self.pairs.len()
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
        self.push_file_error(path, None, message);
    }

    fn push_file_error(&self, path: &Path, dest: Option<&Path>, message: impl Into<String>) {
        self.errors.lock().unwrap().push(TransferError {
            path: path.to_path_buf(),
            dest: dest.map(Path::to_path_buf),
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

fn job_volumes(req: &TransferRequest) -> Vec<String> {
    let mut paths: Vec<&Path> = vec![req.destination.as_path()];
    paths.extend(req.sources.iter().map(PathBuf::as_path));
    for (src, dst) in &req.pairs {
        paths.push(src);
        paths.push(dst);
    }
    let mut vols: Vec<String> = paths
        .into_iter()
        .filter_map(crate::disk::volume_id)
        .collect();
    vols.sort();
    vols.dedup();
    vols
}

/// Which jobs are running, and on which volumes.
#[derive(Default)]
struct Scheduler {
    running: usize,
    busy: HashMap<String, usize>,
}

impl Scheduler {
    fn can_start(&self, job: &Job, cfg: &EngineConfig) -> bool {
        self.running < cfg.max_concurrent_jobs.max(1)
            && (!cfg.per_disk_queue || job.volumes.iter().all(|v| !self.busy.contains_key(v)))
    }

    fn start(&mut self, job: &Job) {
        self.running += 1;
        for v in &job.volumes {
            *self.busy.entry(v.clone()).or_default() += 1;
        }
    }

    fn finish(&mut self, job: &Job) {
        self.running -= 1;
        for v in &job.volumes {
            if let Some(n) = self.busy.get_mut(v) {
                *n -= 1;
                if *n == 0 {
                    self.busy.remove(v);
                }
            }
        }
    }
}

/// Owns all transfer jobs. Cheap to share: wrap it in an `Arc` or keep one per app.
pub struct TransferEngine {
    config: Arc<RwLock<EngineConfig>>,
    jobs: Mutex<Vec<Arc<Job>>>,
    next_id: AtomicU64,
    scheduler: Arc<(Mutex<Scheduler>, Condvar)>,
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
            scheduler: Arc::new((Mutex::new(Scheduler::default()), Condvar::new())),
            notifier: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_config(&self, config: EngineConfig) {
        *self.config.write().unwrap() = config;
        self.scheduler.1.notify_all();
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
        let scheduler = self.scheduler.clone();
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
                // Wait in the queue until a slot (and our drives) are free.
                {
                    let (lock, cv) = &*scheduler;
                    let mut sched = lock.lock().unwrap();
                    loop {
                        if worker_job.is_cancelled() {
                            worker_job.set_state(JobState::Cancelled);
                            notify();
                            return;
                        }
                        if sched.can_start(&worker_job, &config.read().unwrap()) {
                            sched.start(&worker_job);
                            break;
                        }
                        sched = cv
                            .wait_timeout(sched, Duration::from_millis(200))
                            .unwrap()
                            .0;
                    }
                }
                let cfg = config.read().unwrap().clone();
                run_job(&worker_job, &cfg, &notify);
                {
                    let (lock, cv) = &*scheduler;
                    lock.lock().unwrap().finish(&worker_job);
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
    let roots = std::mem::take(&mut plan.roots);

    let workers = worker_count(job, cfg);
    job.workers.store(workers as u64, Ordering::Relaxed);
    job.set_state(JobState::Running);
    notify();
    plan::execute(job, plan, cfg, workers);
    // Whatever reached its destination (also when cancelled or partly failed).
    job.created.lock().unwrap().extend(
        roots
            .into_iter()
            .filter(|(_, dst)| crate::fsops::exists(dst)),
    );

    job.set_state(if job.is_cancelled() {
        JobState::Cancelled
    } else {
        JobState::Completed
    });
}

/// How many files to copy in parallel. In automatic mode HDDs get one
/// worker (parallel reads mostly add seeking), SSDs get eight.
fn worker_count(job: &Job, cfg: &EngineConfig) -> usize {
    if cfg.workers > 0 {
        return cfg.workers.min(64);
    }
    let mut paths = vec![job.destination.clone()];
    paths.extend(job.sources.iter().take(1).cloned());
    paths.extend(job.pairs.iter().take(1).map(|(s, _)| s.clone()));
    let kinds: Vec<Option<bool>> = paths
        .iter()
        .map(|p| crate::disk::is_rotational(p))
        .collect();
    if kinds.contains(&Some(true)) {
        job.hdd.store(true, Ordering::Relaxed);
        1
    } else if kinds.iter().all(|k| *k == Some(false)) {
        8
    } else {
        4 // unknown (network share, virtual drive, …)
    }
}

#[cfg(test)]
mod tests;
