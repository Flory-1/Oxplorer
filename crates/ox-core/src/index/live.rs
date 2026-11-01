//! Live index updates. Changes come from the NTFS change journal where it
//! can be read (Windows: through the index service or as administrator),
//! otherwise from a recursive file watcher. A background thread coalesces bursts into a
//! [`Batch`] and applies it to the index (see `patch`). Changes that arrive
//! while a build runs are kept and replayed on the new index; lost changes
//! (watcher overflow or errors, journal wrap) trigger a full rebuild. The
//! scheduled rebuild stays as a safety net: the Windows file watcher can
//! drop changes silently when a lot happens at once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::patch::{Batch, Signal};
use super::service::{self, IndexState, Shared};
use super::{FileIndex, IndexConfig};

/// Apply a batch once no change came for this long…
const QUIET: Duration = Duration::from_millis(300);
/// …or this long after its first change.
const MAX_WAIT: Duration = Duration::from_secs(2);
/// After lost changes, rebuild at most this often.
const RESCAN_EVERY: Duration = Duration::from_secs(120);
/// Save a live-updated index at most this often.
const SAVE_EVERY: Duration = Duration::from_secs(600);

/// What live updates are doing (for the settings page).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveInfo {
    /// Drives followed through the NTFS change journal.
    pub journal_drives: usize,
    /// Indexed folders followed with a file watcher.
    pub watched_roots: usize,
    /// Items added, changed or removed since live updates started.
    pub changes: u64,
}

#[derive(Default)]
struct Status {
    journal_drives: AtomicUsize,
    watched_roots: AtomicUsize,
    changes: AtomicU64,
}

/// The running live updater; dropping it stops it.
pub(super) struct Live {
    cfg: IndexConfig,
    /// Whether the index service could be used when this started.
    service: bool,
    stop: Arc<AtomicBool>,
    status: Arc<Status>,
}

impl Live {
    pub fn start(shared: &Arc<Shared>, cfg: IndexConfig) -> Live {
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Status::default());
        let (weak, run_cfg) = (Arc::downgrade(shared), cfg.clone());
        let (run_stop, run_status) = (stop.clone(), status.clone());
        // Setting up watchers can take a while (a recursive watch on Linux
        // walks the folders), so it happens on the thread.
        std::thread::Builder::new()
            .name("index-live".into())
            .spawn(move || run(weak, run_cfg, run_stop, run_status))
            .expect("failed to spawn index thread");
        Live {
            cfg,
            service: super::helper::enabled(),
            stop,
            status,
        }
    }

    pub fn config(&self) -> &IndexConfig {
        &self.cfg
    }

    pub fn service(&self) -> bool {
        self.service
    }

    pub fn info(&self) -> LiveInfo {
        LiveInfo {
            journal_drives: self.status.journal_drives.load(Ordering::Relaxed),
            watched_roots: self.status.watched_roots.load(Ordering::Relaxed),
            changes: self.status.changes.load(Ordering::Relaxed),
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What a watcher event means for the index.
fn signals(res: notify::Result<notify::Event>, roots: &[PathBuf]) -> Vec<Signal> {
    let mut ev = match res {
        Ok(ev) if !ev.need_rescan() => ev,
        _ => return vec![Signal::Rescan],
    };
    // The index has the real names: a short one (`%TEMP%` is often written
    // as `C:\Users\ABCDEF~1\…`) would look like a new folder, and the whole
    // real folder would be added a second time.
    for p in &mut ev.paths {
        *p = real_names(p, roots);
    }
    match ev.kind {
        EventKind::Access(_) => Vec::new(),
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            ev.paths.into_iter().map(Signal::RenamedFrom).collect()
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            ev.paths.into_iter().map(Signal::RenamedTo).collect()
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if ev.paths.len() == 2 => {
            let mut p = ev.paths.into_iter();
            vec![
                Signal::RenamedFrom(p.next().unwrap()),
                Signal::RenamedTo(p.next().unwrap()),
            ]
        }
        _ => ev.paths.into_iter().map(Signal::Changed).collect(),
    }
}

/// `path` with short names below its root expanded; the root keeps the
/// form it was given in (the index uses that one).
fn real_names(path: &Path, roots: &[PathBuf]) -> PathBuf {
    if !path.as_os_str().to_string_lossy().contains('~') {
        return path.to_path_buf();
    }
    let long = crate::fsops::long_path(path);
    for root in roots {
        if let Ok(rest) = long.strip_prefix(crate::fsops::long_path(root)) {
            return if rest.as_os_str().is_empty() {
                root.clone()
            } else {
                root.join(rest)
            };
        }
    }
    long
}

/// Watch `roots` recursively. Returns the watcher and how many roots it watches.
fn watch(roots: &[PathBuf], tx: &Sender<Signal>) -> (Option<RecommendedWatcher>, usize) {
    if roots.is_empty() {
        return (None, 0);
    }
    let tx = tx.clone();
    let watched = roots.to_vec();
    let watcher = notify::recommended_watcher(move |res| {
        for s in signals(res, &watched) {
            let _ = tx.send(s);
        }
    });
    let Ok(mut w) = watcher else {
        return (None, 0);
    };
    let n = roots
        .iter()
        .filter(|r| match w.watch(r, RecursiveMode::Recursive) {
            Ok(()) => true,
            Err(e) => {
                log::info!("index: can't watch {} for changes: {e}", r.display());
                false
            }
        })
        .count();
    ((n > 0).then_some(w), n)
}

/// Wait for changes and collect them until things calm down.
fn collect(rx: &Receiver<Signal>) -> Batch {
    let mut batch = Batch::default();
    let Ok(first) = rx.recv_timeout(Duration::from_secs(1)) else {
        return batch;
    };
    batch.push(first);
    let started = Instant::now();
    while let Some(left) = MAX_WAIT.checked_sub(started.elapsed()) {
        match rx.recv_timeout(left.min(QUIET)) {
            Ok(s) => batch.push(s),
            Err(_) => break,
        }
    }
    batch
}

fn run(weak: Weak<Shared>, cfg: IndexConfig, stop: Arc<AtomicBool>, status: Arc<Status>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    // A saved index is caught up on from the change journals while it
    // loads; the journals are followed from where that ended.
    let loading = || {
        weak.upgrade()
            .is_some_and(|s| *s.state.lock().unwrap() == IndexState::Loading)
    };
    while loading() && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }
    #[cfg(windows)]
    let journal = weak
        .upgrade()
        .map(|s| s.journal.lock().unwrap().clone())
        .unwrap_or_default();
    #[cfg(windows)]
    let (journals, rest) = super::journal::follow_journals(&cfg.roots, &tx, &stop, &journal);
    #[cfg(not(windows))]
    let (journals, rest) = (0, cfg.roots.clone());
    let (_watcher, watched) = watch(&rest, &tx);
    status.journal_drives.store(journals, Ordering::Relaxed);
    status.watched_roots.store(watched, Ordering::Relaxed);
    if let Some(shared) = weak.upgrade() {
        (shared.notify)();
    }

    let exclude: Vec<String> = cfg.exclude.iter().map(|e| e.to_lowercase()).collect();
    let mut backlog = Batch::default();
    let mut last_rescan: Option<Instant> = None;
    let mut last_save = Instant::now();
    let mut unsaved = false;
    while !stop.load(Ordering::Relaxed) {
        let batch = collect(&rx);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Some(shared) = weak.upgrade() else { break };
        backlog.merge(batch);
        let building = shared.building.load(Ordering::SeqCst);
        if backlog.needs_rescan() {
            if !building && last_rescan.is_none_or(|t| t.elapsed() >= RESCAN_EVERY) {
                log::info!("index: file changes were lost, rebuilding");
                service::rebuild(&shared, cfg.clone());
                last_rescan = Some(Instant::now());
                backlog = Batch::default();
            }
        } else if !backlog.is_empty() && !building {
            let batch = std::mem::take(&mut backlog);
            let n = apply(&shared, &batch, &exclude);
            {
                let mut journal = shared.journal.lock().unwrap();
                for &cp in batch.checkpoints() {
                    journal.set(cp);
                }
            }
            // A build that started meanwhile may have read the folders before
            // they changed: replay on the new index.
            if shared.building.load(Ordering::SeqCst) {
                backlog.merge(batch);
            }
            if n > 0 {
                // Not announced: a drive changes all the time, and only the
                // count in the settings shows it (they look it up themselves).
                status.changes.fetch_add(n as u64, Ordering::Relaxed);
                unsaved = true;
            }
        }
        if unsaved && !building && last_save.elapsed() >= SAVE_EVERY {
            if let Err(e) = service::save(&shared) {
                log::warn!("index: couldn't save: {e}");
            }
            unsaved = false;
            last_save = Instant::now();
        }
    }
}

/// Apply `batch` to the current index. Planning (reading the disk, looking up
/// paths) runs on a snapshot while queries continue; the index is then
/// patched in place, or as a copy when a search is still using it. Returns
/// how many items changed.
pub(super) fn apply(shared: &Shared, batch: &Batch, exclude: &[String]) -> usize {
    let current = || shared.index.read().unwrap().clone();
    let mut changed = 0;
    for step in batch.steps() {
        loop {
            let Some(snap) = current() else {
                return changed;
            };
            let ops = snap.plan(&step, exclude);
            if ops.is_empty() {
                break;
            }
            let mut guard = shared.index.write().unwrap();
            let Some(cur) = guard.as_mut() else {
                return changed;
            };
            if !Arc::ptr_eq(cur, &snap) {
                continue; // a build replaced it: plan again
            }
            if Arc::strong_count(cur) == 2 {
                drop(snap);
                if let Some(idx) = Arc::get_mut(cur) {
                    changed += idx.apply(ops);
                    break;
                }
                continue;
            }
            drop(guard);
            let mut copy = FileIndex::clone(&snap);
            let n = copy.apply(ops);
            let mut guard = shared.index.write().unwrap();
            if guard.as_ref().is_some_and(|cur| Arc::ptr_eq(cur, &snap)) {
                *guard = Some(Arc::new(copy));
                changed += n;
                break;
            }
        }
    }
    if let Some(snap) = current().filter(|idx| idx.needs_compaction()) {
        let compact = Arc::new(snap.compact());
        let mut guard = shared.index.write().unwrap();
        if guard.as_ref().is_some_and(|cur| Arc::ptr_eq(cur, &snap)) {
            *guard = Some(compact);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn signals_(res: notify::Result<notify::Event>) -> Vec<Signal> {
        signals(res, &[])
    }

    #[test]
    fn watcher_events_become_signals() {
        use notify::event::{AccessKind, CreateKind, Event, Flag};
        let ev = |kind, paths: &[&str]| {
            let mut e = Event::new(kind);
            for p in paths {
                e = e.add_path(PathBuf::from(p));
            }
            Ok(e)
        };
        assert_eq!(
            signals_(ev(EventKind::Create(CreateKind::File), &["a"])),
            vec![Signal::Changed("a".into())]
        );
        assert!(signals_(ev(EventKind::Access(AccessKind::Any), &["a"])).is_empty());
        assert_eq!(
            signals_(ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                &["a", "b"]
            )),
            vec![
                Signal::RenamedFrom("a".into()),
                Signal::RenamedTo("b".into())
            ]
        );
        assert_eq!(
            signals_(ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                &["a"]
            )),
            vec![Signal::RenamedFrom("a".into())]
        );
        let overflow = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        assert_eq!(signals(Ok(overflow), &[]), vec![Signal::Rescan]);
        assert_eq!(
            signals(Err(notify::Error::generic("boom")), &[]),
            vec![Signal::Rescan]
        );
    }

    /// A change reported with a short (8.3) name must update the folder the
    /// index has, not add the whole folder again under the short name.
    #[cfg(windows)]
    #[test]
    fn short_names_dont_add_folders_twice() {
        use notify::event::{CreateKind, Event};
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use std::sync::atomic::{AtomicBool, AtomicU64};
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

        let tmp = tempfile::tempdir().unwrap();
        // As given, which may be in short form itself (`%TEMP%`).
        let root = tmp.path().to_path_buf();
        let folder = root.join("A folder with a long name");
        std::fs::create_dir(&folder).unwrap();
        for i in 0..20 {
            std::fs::write(folder.join(format!("f{i}.txt")), "x").unwrap();
        }
        let cfg = super::super::IndexConfig {
            roots: vec![root.clone()],
            exclude: Vec::new(),
            threads: 1,
        };
        let build = || FileIndex::build(&cfg, &AtomicBool::new(false), &AtomicU64::new(0));
        let mut idx = build();

        let wide: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut buf = vec![0u16; 1024];
        // SAFETY: `wide` is NUL-terminated and `buf` is as long as we say.
        let n = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), 1024) } as usize;
        let short = PathBuf::from(std::ffi::OsString::from_wide(&buf[..n]));
        if n == 0 || !short.to_string_lossy().contains('~') {
            return; // no short names on this drive
        }
        std::fs::write(folder.join("new.txt"), "n").unwrap();
        let ev = Event::new(EventKind::Create(CreateKind::File)).add_path(short.join("new.txt"));
        let mut batch = Batch::default();
        for s in signals(Ok(ev), std::slice::from_ref(&root)) {
            batch.push(s);
        }
        assert_eq!(idx.update(&batch, &[]), 1, "only the new file");
        assert_eq!(idx.len(), build().len());
    }
}
