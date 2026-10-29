//! Resumable transfers: every job keeps a journal in the config folder
//! (`transfers/<name>.journal`), so a copy cut off by a crash, sleep or a
//! pulled USB stick can continue where it stopped.
//!
//! The journal is plain text, one record per line, fields separated by tabs
//! (`%`, tab, CR and LF inside a field are percent-escaped). First comes the
//! request, written and synced in one go and closed by a `begin` line. After
//! that records are only appended:
//!
//! - `root  src  dst`: where a top-level source goes (a copy into the same
//!   folder gets a "name - Copy" target that must not change on resume),
//! - `policy  name`: the answer to the conflict question,
//! - `start  src  dst`: a file is about to be written,
//! - `done  src  dst  size  modified  size  modified`: a file is completely
//!   transferred (for a move: the original is gone), with the size and
//!   modified time of the original and of the copy.
//!
//! A record counts only once its newline is written, so a torn last line
//! after a crash is ignored. Lines are written straight to the OS (they
//! survive the app crashing) and synced to the disk at most once a second.
//!
//! Resuming re-plans the request. A file with a `done` record is skipped
//! when the original and the copy still have the recorded size and modified
//! time. A file with only a `start` record was being written when the job
//! stopped: it is copied again **from the start**, over the partial copy and
//! without asking. A partial copy is never continued from an offset: Windows
//! pre-allocates the full size, so its length says nothing about how much is
//! valid. Moves stay safe because the original is only deleted after its copy
//! is complete, and `done` is written after that.
//!
//! The journal is deleted when the job completes or the user cancels it. A
//! job that ends with errors while its destination (or a source drive) is
//! gone keeps it, so it can be resumed once the drive is back.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::plan::Plan;
use super::{ConflictPolicy, Job, JobState, TransferEngine, TransferKind, TransferRequest};
use crate::fsops;

const MAGIC: &str = "oxplorer-transfer-journal 1";
const EXTENSION: &str = "journal";

/// Where the app keeps its journals.
pub fn default_dir() -> PathBuf {
    crate::config_dir().join("transfers")
}

/// Size and modified time of a file, as recorded in the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    /// Nanoseconds since 1970.
    modified: Option<u128>,
}

impl Stamp {
    fn new(size: u64, modified: Option<SystemTime>) -> Self {
        Self {
            size,
            modified: modified
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos()),
        }
    }

    fn of(path: &Path) -> Option<Self> {
        let meta = fs::symlink_metadata(path).ok()?;
        Some(Self::new(meta.len(), meta.modified().ok()))
    }

    fn fields(self) -> [String; 2] {
        [
            self.size.to_string(),
            self.modified.map_or("-".into(), |m| m.to_string()),
        ]
    }

    fn parse(size: &str, modified: &str) -> Option<Self> {
        Some(Self {
            size: size.parse().ok()?,
            modified: match modified {
                "-" => None,
                m => Some(m.parse().ok()?),
            },
        })
    }
}

#[derive(Debug, Clone)]
struct Entry {
    dst: PathBuf,
    /// `(original, copy)` when the file was completely transferred.
    done: Option<(Stamp, Stamp)>,
}

/// An interrupted transfer that can be resumed (or discarded).
#[derive(Debug)]
pub struct Pending {
    pub path: PathBuf,
    pub request: TransferRequest,
    /// Files completely transferred before the interruption.
    pub done_files: u64,
    pub done_bytes: u64,
    /// When the journal was last written.
    pub saved: Option<SystemTime>,
    policy: Option<ConflictPolicy>,
    roots: HashMap<PathBuf, PathBuf>,
    files: HashMap<PathBuf, Entry>,
    /// The last line was torn: start appending on a new line.
    torn: bool,
    /// The job of this session that left the journal behind.
    job_id: Option<u64>,
}

// ---- text format -------------------------------------------------------

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' => out.push_str("%25"),
            '\t' => out.push_str("%09"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> Option<String> {
    if !s.contains('%') {
        return Some(s.to_string());
    }
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// One record: escaped fields joined by tabs, ending with a newline.
fn record(fields: &[&str]) -> String {
    let mut line = fields
        .iter()
        .map(|f| escape(f))
        .collect::<Vec<_>>()
        .join("\t");
    line.push('\n');
    line
}

fn kind_name(kind: TransferKind) -> &'static str {
    match kind {
        TransferKind::Copy => "Copy",
        TransferKind::Move => "Move",
    }
}

fn policy_name(p: ConflictPolicy) -> &'static str {
    match p {
        ConflictPolicy::Ask => "Ask",
        ConflictPolicy::Overwrite => "Overwrite",
        ConflictPolicy::Skip => "Skip",
        ConflictPolicy::KeepBoth => "KeepBoth",
        ConflictPolicy::OverwriteIfNewer => "OverwriteIfNewer",
    }
}

fn parse_policy(s: &str) -> Option<ConflictPolicy> {
    ConflictPolicy::ALL
        .into_iter()
        .find(|p| policy_name(*p) == s)
}

/// The request part of a journal, or `None` if a path isn't valid Unicode.
fn header(req: &TransferRequest) -> Option<String> {
    let mut out = format!("{MAGIC}\n");
    out.push_str(&record(&["kind", kind_name(req.kind)]));
    out.push_str(&record(&["conflict", policy_name(req.conflict)]));
    out.push_str(&record(&["dest", req.destination.to_str()?]));
    for src in &req.sources {
        out.push_str(&record(&["src", src.to_str()?]));
    }
    for (src, dst) in &req.pairs {
        out.push_str(&record(&["pair", src.to_str()?, dst.to_str()?]));
    }
    out.push_str(&record(&["begin"]));
    Some(out)
}

/// Read a journal. `None` if it is not a journal or its request is incomplete.
fn parse(path: &Path, bytes: &[u8]) -> Option<Pending> {
    let text = String::from_utf8_lossy(bytes);
    // Only complete lines count: whatever follows the last newline is torn.
    let end = text.rfind('\n')?;
    let torn = end + 1 < text.len();
    let mut lines = text[..end].split('\n');
    if lines.next()?.trim_end_matches('\r') != MAGIC {
        return None;
    }
    let (mut kind, mut conflict, mut dest) = (None, None, None);
    let (mut sources, mut pairs) = (Vec::new(), Vec::new());
    let mut begun = false;
    for line in lines.by_ref() {
        let f: Vec<String> = line.split('\t').map(unescape).collect::<Option<_>>()?;
        match (f[0].as_str(), f.len()) {
            ("kind", 2) => {
                kind = Some(match f[1].as_str() {
                    "Copy" => TransferKind::Copy,
                    "Move" => TransferKind::Move,
                    _ => return None,
                })
            }
            ("conflict", 2) => conflict = Some(parse_policy(&f[1])?),
            ("dest", 2) => dest = Some(PathBuf::from(&f[1])),
            ("src", 2) => sources.push(PathBuf::from(&f[1])),
            ("pair", 3) => pairs.push((PathBuf::from(&f[1]), PathBuf::from(&f[2]))),
            ("begin", 1) => {
                begun = true;
                break;
            }
            _ => return None,
        }
    }
    if !begun {
        return None;
    }
    let mut request = TransferRequest::new(kind?, sources, dest?, conflict?);
    request.pairs = pairs;

    let mut policy = None;
    let mut roots = HashMap::new();
    let mut files: HashMap<PathBuf, Entry> = HashMap::new();
    for line in lines {
        // Damaged records (a crash can leave garbage) are ignored.
        let Some(f) = line.split('\t').map(unescape).collect::<Option<Vec<_>>>() else {
            continue;
        };
        match (f[0].as_str(), f.len()) {
            ("policy", 2) => policy = parse_policy(&f[1]).or(policy),
            ("root", 3) => {
                roots.insert(PathBuf::from(&f[1]), PathBuf::from(&f[2]));
            }
            ("start", 3) => {
                files.insert(
                    PathBuf::from(&f[1]),
                    Entry {
                        dst: PathBuf::from(&f[2]),
                        done: None,
                    },
                );
            }
            ("done", 7) => {
                if let (Some(src), Some(dst)) =
                    (Stamp::parse(&f[3], &f[4]), Stamp::parse(&f[5], &f[6]))
                {
                    files.insert(
                        PathBuf::from(&f[1]),
                        Entry {
                            dst: PathBuf::from(&f[2]),
                            done: Some((src, dst)),
                        },
                    );
                }
            }
            _ => {}
        }
    }
    let done = files.values().filter_map(|e| e.done);
    let (done_files, done_bytes) = done.fold((0, 0), |(n, b), (s, _)| (n + 1, b + s.size));
    Some(Pending {
        path: path.to_path_buf(),
        request,
        done_files,
        done_bytes,
        saved: fs::metadata(path).and_then(|m| m.modified()).ok(),
        policy,
        roots,
        files,
        torn,
        job_id: None,
    })
}

/// Open a journal that no running job holds. `None` if another job (of this
/// or another Oxplorer window) is using it.
fn open_unlocked(path: &Path) -> Option<File> {
    let file = OpenOptions::new().read(true).append(true).open(path).ok()?;
    file.try_lock().ok()?;
    Some(file)
}

fn read_pending(path: &Path) -> Option<Pending> {
    let mut file = open_unlocked(path)?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes).ok()?;
    let pending = parse(path, &bytes);
    drop(file);
    if pending.is_none() {
        // The request was never completely written, so nothing was transferred.
        let _ = fs::remove_file(path);
    }
    pending
}

/// All interrupted transfers in `dir`, oldest first.
pub fn list(dir: Option<&Path>) -> Vec<Arc<Pending>> {
    let Some(entries) = dir.and_then(|d| fs::read_dir(d).ok()) else {
        return Vec::new();
    };
    let mut found: Vec<Arc<Pending>> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXTENSION))
        .filter_map(|p| read_pending(&p).map(Arc::new))
        .collect();
    found.sort_by_key(|p| p.saved);
    found
}

// ---- the journal of a running job ---------------------------------------

struct Writer {
    file: File,
    path: PathBuf,
    synced: Instant,
}

impl Writer {
    fn write(&mut self, text: &str) {
        if let Err(e) = self.file.write_all(text.as_bytes()) {
            log::warn!("transfer journal {}: {e}", self.path.display());
        }
        if self.synced.elapsed().as_secs() >= 1 {
            let _ = self.file.sync_data();
            self.synced = Instant::now();
        }
    }
}

type Shared = Arc<Mutex<Vec<Arc<Pending>>>>;

#[derive(Default)]
struct Setup {
    resume: Option<Arc<Pending>>,
    /// The engine's list of resumable transfers, for a journal that is kept.
    pending: Option<Shared>,
}

/// The journal part of a [`Job`].
#[derive(Default)]
pub(super) struct JobJournal {
    setup: OnceLock<Setup>,
    writer: Mutex<Option<Writer>>,
}

impl JobJournal {
    fn resume(&self) -> Option<&Pending> {
        self.setup.get()?.resume.as_deref()
    }

    fn write(&self, fields: &[&str]) {
        if let Some(w) = self.writer.lock().unwrap().as_mut() {
            w.write(&record(fields));
        }
    }

    fn active(&self) -> bool {
        self.writer.lock().unwrap().is_some()
    }

    fn open(&self, job: &Job, dir: Option<&Path>) {
        let writer = if let Some(p) = self.resume() {
            let Some(mut file) = open_unlocked(&p.path) else {
                job.push_error(
                    &p.path,
                    crate::i18n::t("This transfer is already being resumed in another window."),
                );
                job.cancel();
                return;
            };
            if p.torn {
                let _ = file.write_all(b"\n");
            }
            Writer {
                file,
                path: p.path.clone(),
                synced: Instant::now(),
            }
        } else {
            let Some(dir) = dir else { return };
            match create(job, dir) {
                Ok(Some(w)) => w,
                Ok(None) => return,
                Err(e) => {
                    log::warn!(
                        "could not create a transfer journal in {}: {e}",
                        dir.display()
                    );
                    return;
                }
            }
        };
        *self.writer.lock().unwrap() = Some(writer);
    }

    /// Finish the journal: delete it, or keep it for resuming later.
    fn close(&self, job: &Job) {
        let Some(writer) = self.writer.lock().unwrap().take() else {
            return;
        };
        let keep = match job.state() {
            JobState::Cancelled => false,
            JobState::Completed => job.error_count() > 0 && unreachable(job),
            // The job didn't end normally (a panic): keep everything.
            _ => true,
        };
        let path = writer.path.clone();
        let _ = writer.file.sync_data();
        drop(writer);
        if !keep {
            if let Err(e) = fs::remove_file(&path) {
                log::warn!("could not delete transfer journal {}: {e}", path.display());
            }
            return;
        }
        let shared = self.setup.get().and_then(|s| s.pending.clone());
        if let (Some(shared), Some(mut pending)) = (shared, read_pending(&path)) {
            pending.job_id = Some(job.id);
            let mut list = shared.lock().unwrap();
            list.retain(|p| p.path != path);
            list.push(Arc::new(pending));
        }
    }

    /// Where a top-level source goes. On resume the recorded target wins.
    pub(super) fn root(&self, src: &Path, dst: &mut PathBuf) {
        if let Some(recorded) = self.resume().and_then(|p| p.roots.get(src)) {
            *dst = recorded.clone();
            return;
        }
        if let (Some(s), Some(d)) = (src.to_str(), dst.to_str()) {
            self.write(&["root", s, d]);
        }
    }

    /// When resuming a move, a top-level source that is gone and already
    /// sits at its destination was moved before the interruption.
    pub(super) fn already_moved(&self, job: &Job, src: &Path) -> bool {
        let Some(p) = self.resume() else {
            return false;
        };
        if job.kind != TransferKind::Move || fsops::exists(src) {
            return false;
        }
        let target = p
            .roots
            .get(src)
            .cloned()
            .or_else(|| Some(job.destination.join(src.file_name()?)));
        target.is_some_and(|t| fsops::exists(&t))
    }

    /// The conflict decision of an earlier run of this job.
    pub(super) fn decided_policy(&self) -> Option<ConflictPolicy> {
        self.resume()?.policy.filter(|p| *p != ConflictPolicy::Ask)
    }

    pub(super) fn record_policy(&self, policy: ConflictPolicy) {
        self.write(&["policy", policy_name(policy)]);
    }

    pub(super) fn started(&self, src: &Path, dst: &Path) {
        if let (Some(s), Some(d)) = (src.to_str(), dst.to_str()) {
            self.write(&["start", s, d]);
        }
    }

    /// A file is completely transferred.
    pub(super) fn finished(&self, src: &Path, dst: &Path, size: u64, modified: Option<SystemTime>) {
        if !self.active() {
            return;
        }
        let (Some(s), Some(d), Some(copy)) = (src.to_str(), dst.to_str(), Stamp::of(dst)) else {
            return;
        };
        let [ss, sm] = Stamp::new(size, modified).fields();
        let [ds, dm] = copy.fields();
        self.write(&["done", s, d, &ss, &sm, &ds, &dm]);
    }
}

fn create(job: &Job, dir: &Path) -> std::io::Result<Option<Writer>> {
    let mut req = TransferRequest::new(
        job.kind,
        job.sources.clone(),
        job.destination.clone(),
        job.conflict_policy,
    );
    req.pairs = job.pairs.clone();
    let Some(head) = header(&req) else {
        // A path that isn't valid Unicode can't be written down: no journal.
        return Ok(None);
    };
    fs::create_dir_all(dir)?;
    let name = format!(
        "{}-{}-{}.{EXTENSION}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        std::process::id(),
        job.id
    );
    let path = dir.join(name);
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create_new(true)
        .open(&path)?;
    let _ = file.try_lock();
    file.write_all(head.as_bytes())?;
    file.sync_data()?;
    Ok(Some(Writer {
        file,
        path,
        synced: Instant::now(),
    }))
}

/// The destination or a source drive is gone (e.g. a pulled USB stick).
fn unreachable(job: &Job) -> bool {
    let root_gone = |p: &Path| p.ancestors().last().is_some_and(|r| !r.exists());
    !job.destination.exists()
        || job.sources.iter().any(|s| root_gone(s))
        || job.pairs.iter().any(|(s, d)| root_gone(s) || root_gone(d))
}

/// Opens the journal when a job starts and finishes it when the job ends
/// (however `run_job` returns).
pub(super) struct Guard<'a>(&'a Job);

impl<'a> Guard<'a> {
    pub fn open(job: &'a Job, dir: Option<&Path>) -> Self {
        job.journal.open(job, dir);
        Self(job)
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.journal.close(self.0);
    }
}

/// On resume: drop the files that were completely transferred and still
/// match, and send partly written ones back to their recorded target.
pub(super) fn apply_resume(job: &Job, plan: &mut Plan) {
    let Some(p) = job.journal.resume() else {
        return;
    };
    let (mut files, mut bytes) = (0, 0);
    plan.files.retain_mut(|task| {
        let Some(entry) = p.files.get(&task.src) else {
            return true;
        };
        task.dst = entry.dst.clone();
        match entry.done {
            Some((src, copy)) => {
                let unchanged = Stamp::new(task.size, task.modified) == src
                    && Stamp::of(&entry.dst) == Some(copy);
                if unchanged {
                    files += 1;
                    bytes += task.size;
                    return false;
                }
                // Changed since: copied again, an existing copy is a normal conflict.
                task.conflict = fsops::exists(&task.dst);
            }
            None => {
                // Our own partial copy: replace it without asking.
                let _ = fsops::clear_readonly(&task.dst);
                task.conflict = false;
            }
        }
        true
    });
    job.done_files.fetch_add(files, Ordering::Relaxed);
    job.done_bytes.fetch_add(bytes, Ordering::Relaxed);
}

/// Hand a job its resume state and the engine's list before it starts.
pub(super) fn setup(job: &Job, resume: Option<Arc<Pending>>, pending: &Shared) {
    let _ = job.journal.setup.set(Setup {
        resume,
        pending: Some(pending.clone()),
    });
}

impl TransferEngine {
    /// Interrupted transfers that can be resumed: from journals found at
    /// start-up and from jobs of this session that lost their drive.
    pub fn resumable(&self) -> Vec<Arc<Pending>> {
        self.journals.lock().unwrap().clone()
    }

    /// Continue an interrupted transfer.
    pub fn resume(&self, pending: &Arc<Pending>) -> Arc<Job> {
        self.forget_journal(pending);
        if let Some(id) = pending.job_id {
            self.remove(id);
        }
        self.submit_job(pending.request.clone(), false, Some(pending.clone()))
    }

    /// Forget an interrupted transfer. Files already copied stay where they are.
    pub fn discard(&self, pending: &Pending) {
        self.forget_journal(pending);
        if open_unlocked(&pending.path).is_some() {
            let _ = fs::remove_file(&pending.path);
        }
    }

    fn forget_journal(&self, pending: &Pending) {
        self.journals
            .lock()
            .unwrap()
            .retain(|p| p.path != pending.path);
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
