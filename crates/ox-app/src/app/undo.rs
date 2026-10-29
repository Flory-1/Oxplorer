//! Undo and redo (Ctrl+Z, Ctrl+Y): file operations are recorded as they
//! finish and reversed on a helper thread. The logic is in `ox_core::undo`.
//! The history is saved as it changes and loaded again when the main window
//! starts (Settings → Browsing → Keep undo history after restart).

use super::*;
use ox_core::transfer::Job;
use ox_core::undo::{self, Entry, History, Kind, Op, Outcome, Placed, Store};

/// Records operations that finish on other threads (deletes, copies).
#[derive(Clone)]
pub struct Recorder(Sender<Entry>);

impl Recorder {
    /// Items were moved to the Recycle Bin at or after `since` (see
    /// [`undo::now`]); those that are gone now can be put back.
    pub fn trashed(&self, paths: &[PathBuf], since: i64) {
        let paths: Vec<PathBuf> = paths
            .iter()
            .filter(|p| !fsops::exists(p))
            .cloned()
            .collect();
        if !paths.is_empty() {
            let _ = self
                .0
                .send(Entry::new(Kind::Delete, Op::Trashed { paths, since }));
        }
    }
}

/// How far an undo or redo is.
enum Running {
    /// Checking and reversing on a helper thread.
    Thread(Receiver<Result<Outcome, String>>),
    /// Moving items back with the transfer engine: the job, and what was
    /// moved back already.
    Job(u64, Vec<(PathBuf, PathBuf)>),
}

pub struct UndoState {
    pub history: History,
    records: (Sender<Entry>, Receiver<Entry>),
    /// The entry being undone (`false`) or redone (`true`).
    running: Option<(bool, Entry, Running)>,
    /// Transfer jobs started by an undo or redo (not recorded themselves).
    own_jobs: HashSet<u64>,
    /// Where the history is kept across restarts.
    store: Store,
    /// Whether it is kept, as of the last frame (`None`: first frame).
    keeping: Option<bool>,
}

impl Default for UndoState {
    fn default() -> Self {
        Self {
            history: History::default(),
            records: crossbeam_channel::unbounded(),
            running: None,
            own_jobs: HashSet::new(),
            store: Store::new(Store::default_path()),
            keeping: None,
        }
    }
}

impl UndoState {
    pub fn record(&mut self, kind: Kind, op: Op) {
        self.history.record(Entry::new(kind, op));
    }

    pub fn recorder(&self) -> Recorder {
        Recorder(self.records.0.clone())
    }

    /// A new folder or file was created.
    pub fn created(&mut self, path: &Path) {
        if path.is_dir() {
            self.record(Kind::NewFolder, Op::NewFolder(path.to_path_buf()));
        } else if let Ok(item) = Placed::now(path.to_path_buf()) {
            self.record(Kind::NewFile, Op::Placed(vec![item]));
        }
    }

    fn take_records(&mut self) {
        while let Ok(entry) = self.records.1.try_recv() {
            self.history.record(entry);
        }
    }

    /// Keep the history across restarts (`keep`): the main window loads it
    /// once, every window saves its changes. Turned off: the file goes.
    fn persist(&mut self, keep: bool, main_window: bool) {
        let first = self.keeping.is_none();
        if !keep {
            if self.keeping != Some(false) {
                if let Err(e) = self.store.remove() {
                    log::warn!("could not remove the undo history: {e}");
                }
            }
            self.keeping = Some(false);
            return;
        }
        let empty = !self.history.can_undo() && !self.history.can_redo();
        if self.keeping != Some(true) && first && main_window && empty {
            self.history = self.store.load();
        }
        // Turned on later: what is there now is saved at once.
        let turned_on = !first && self.keeping == Some(false);
        self.keeping = Some(true);
        if self.history.take_changed() || turned_on {
            if let Err(e) = self.store.save(&mut self.history) {
                log::warn!("could not save the undo history: {e}");
            }
        }
    }
}

impl OxApp {
    /// Undo (`redo` false) or redo the last operation.
    pub fn undo_redo(&mut self, redo: bool) {
        self.undo.take_records();
        if self.undo.running.is_some() {
            self.toast(tr("Wait until the last undo or redo has finished."));
            return;
        }
        let Some(entry) = self.undo.history.take(redo) else {
            self.toast(if redo {
                tr("Nothing to redo")
            } else {
                tr("Nothing to undo")
            });
            return;
        };
        let op = entry.op.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(undo::reverse(&op, &undo::RecycleBin));
            ctx.request_repaint();
        });
        self.undo.running = Some((redo, entry, Running::Thread(rx)));
    }

    /// A transfer job finished: remember what it did.
    pub(super) fn record_job(&mut self, job: &Job) {
        if self.undo.own_jobs.remove(&job.id) {
            return;
        }
        let created = job.created();
        if created.is_empty() {
            return;
        }
        match job.kind {
            TransferKind::Move => self.undo.record(Kind::Move, Op::Moved(created)),
            TransferKind::Copy => {
                // Stamping big folders takes a while.
                let tx = self.undo.records.0.clone();
                std::thread::spawn(move || {
                    let items: Vec<Placed> = created
                        .into_iter()
                        .filter_map(|(_, dst)| Placed::now(dst).ok())
                        .collect();
                    if !items.is_empty() {
                        let _ = tx.send(Entry::new(Kind::Copy, Op::Placed(items)));
                    }
                });
            }
        }
    }

    /// Every frame: take in recorded operations and finish undos and redos.
    pub(super) fn poll_undo(&mut self) {
        self.undo.take_records();
        let (keep, main) = (
            self.settings.browsing.keep_undo_history,
            self.instance.owns_session(),
        );
        self.undo.persist(keep, main);
        let Some((redo, entry, running)) = self.undo.running.take() else {
            return;
        };
        match running {
            Running::Thread(rx) => match rx.try_recv() {
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    self.undo.running = Some((redo, entry, Running::Thread(rx)));
                }
                Ok(Ok(Outcome::Done(op))) => self.reversed(redo, entry, op),
                Ok(Ok(Outcome::Transfer { done, pending })) => {
                    let dest = pending[0].1.parent().map(Path::to_path_buf);
                    let mut req = TransferRequest::new(
                        TransferKind::Move,
                        Vec::new(),
                        dest.unwrap_or_default(),
                        self.settings.transfer.default_conflict,
                    );
                    req.pairs = pending;
                    let job = self.engine.submit(req);
                    self.ui.show_transfers = true;
                    self.undo.own_jobs.insert(job.id);
                    self.undo.running = Some((redo, entry, Running::Job(job.id, done)));
                }
                Ok(Err(e)) => self.not_reversed(redo, entry, e),
                Err(_) => self.not_reversed(redo, entry, tr("It stopped unexpectedly.").into()),
            },
            Running::Job(id, mut moved) => {
                let job = self.engine.jobs().into_iter().find(|j| j.id == id);
                if job.as_ref().is_some_and(|j| !j.state().is_finished()) {
                    self.undo.running = Some((redo, entry, Running::Job(id, moved)));
                    return;
                }
                moved.extend(job.map(|j| j.created()).unwrap_or_default());
                if moved.is_empty() {
                    // Nothing changed (cancelled or failed): it can be tried again.
                    self.undo.history.put_back(redo, entry);
                } else {
                    self.reversed(redo, entry, Op::Moved(moved));
                }
            }
        }
    }

    fn reversed(&mut self, redo: bool, entry: Entry, op: Op) {
        for dir in op.folders() {
            self.mark_changed(Some(&dir));
        }
        let moved = op.moved_pairs();
        if !moved.is_empty() {
            self.push(Action::PathsMoved(moved));
        }
        let what = entry.describe();
        self.toast(if redo {
            ox_core::trf!("Redid: {what}", what = what)
        } else {
            ox_core::trf!("Undid: {what}", what = what)
        });
        self.undo.history.reversed(redo, Entry { op, ..entry });
    }

    fn not_reversed(&mut self, redo: bool, entry: Entry, error: String) {
        let what = entry.describe();
        self.undo.history.put_back(redo, entry);
        self.toast(if redo {
            ox_core::trf!("Couldn't redo {what}: {e}", what = what, e = error)
        } else {
            ox_core::trf!("Couldn't undo {what}: {e}", what = what, e = error)
        });
    }
}
