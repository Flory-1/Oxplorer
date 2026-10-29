//! Mirror / sync a folder to another: the dialog's state, the preview made
//! in the background, and carrying it out (removals to the Recycle Bin
//! first, then the copies as transfer jobs). The logic is in
//! `ox_core::mirror`; the dialog is drawn by `ui::mirror`.

use ox_core::mirror::{self, Options, Plan, PlanJob, RecentPair, RecentPairs};

use super::*;

pub enum MirrorAction {
    /// Open the dialog for mirroring `source` (to `target`, if known: the
    /// preview starts right away).
    Open {
        source: PathBuf,
        target: Option<PathBuf>,
    },
    /// Make the preview for what the dialog shows.
    Preview,
    /// Carry out the previewed plan.
    Start,
    Close,
}

/// The mirror dialog.
pub struct MirrorDialog {
    pub source: PathBuf,
    /// The target as typed.
    pub target: String,
    pub options: Options,
    pub job: Option<PlanJob>,
    pub plan: Option<Plan>,
    /// Why there is no preview (refused, couldn't be read).
    pub error: Option<String>,
    /// The user confirmed removing most of the target's files.
    pub confirm_many: bool,
    /// What the preview was made for: (target, options).
    planned: Option<(PathBuf, Options)>,
    /// Put the keyboard focus on the target field.
    pub focus: bool,
}

impl MirrorDialog {
    pub fn target_path(&self) -> PathBuf {
        PathBuf::from(self.target.trim().trim_matches('"'))
    }

    /// The preview is for what the dialog shows now.
    pub fn is_current(&self) -> bool {
        self.plan.is_some()
            && self.planned.as_ref() == Some(&(self.target_path(), self.options.clone()))
    }
}

/// Removals running before the copies of a mirror can start.
struct Running {
    rx: Receiver<Result<(), String>>,
    copies: Vec<TransferRequest>,
    target: PathBuf,
}

pub struct MirrorState {
    pub dialog: Option<MirrorDialog>,
    running: Vec<Running>,
    /// Pairs mirrored recently (read at the start and when the dialog opens).
    pub recent: RecentPairs,
}

impl Default for MirrorState {
    fn default() -> Self {
        Self {
            dialog: None,
            running: Vec::new(),
            recent: RecentPairs::load(&RecentPairs::path()),
        }
    }
}

impl OxApp {
    pub(super) fn process_mirror(&mut self, action: MirrorAction) {
        match action {
            MirrorAction::Open { source, target } => {
                self.mirror.recent = RecentPairs::load(&RecentPairs::path());
                let last = self.mirror.recent.for_source(&source).cloned();
                let options = last.as_ref().map(|p| p.options.clone()).unwrap_or_default();
                let target = target.or_else(|| last.map(|p| p.target));
                let has_target = target.is_some();
                self.mirror.dialog = Some(MirrorDialog {
                    source,
                    target: target.map(|t| t.display().to_string()).unwrap_or_default(),
                    options,
                    job: None,
                    plan: None,
                    error: None,
                    confirm_many: false,
                    planned: None,
                    focus: !has_target,
                });
                if has_target {
                    self.process_mirror(MirrorAction::Preview);
                }
            }
            MirrorAction::Preview => {
                let Some(d) = &mut self.mirror.dialog else {
                    return;
                };
                let target = d.target_path();
                d.plan = None;
                d.job = None;
                d.confirm_many = false;
                d.planned = Some((target.clone(), d.options.clone()));
                if let Err(e) = mirror::check(&d.source, &target) {
                    d.error = Some(e);
                    return;
                }
                d.error = None;
                let ctx = self.ctx.clone();
                d.job = Some(PlanJob::start(
                    d.source.clone(),
                    target,
                    d.options.clone(),
                    move || ctx.request_repaint(),
                ));
            }
            MirrorAction::Start => self.start_mirror(),
            MirrorAction::Close => self.mirror.dialog = None,
        }
    }

    fn start_mirror(&mut self) {
        let Some(d) = self.mirror.dialog.take_if(|d| d.is_current()) else {
            return;
        };
        let Some(plan) = d.plan else { return };
        if plan.removes_most() && !d.confirm_many {
            self.mirror.dialog = Some(MirrorDialog {
                plan: Some(plan),
                ..d
            });
            return;
        }
        let steps = plan.steps(self.settings.transfer.default_conflict);
        if steps.trash.is_empty() && steps.copies.is_empty() {
            self.toast(tr("Nothing to do: the target is up to date."));
            return;
        }
        let pair = RecentPair {
            source: plan.source.clone(),
            target: plan.target.clone(),
            options: plan.options.clone(),
        };
        match RecentPairs::remember(&RecentPairs::path(), pair) {
            Ok(r) => self.mirror.recent = r,
            Err(e) => log::warn!("could not save the recent mirrors: {e}"),
        }
        if steps.trash.is_empty() {
            self.submit_mirror_copies(steps.copies);
            return;
        }
        self.toast(ox_core::trf!(
            "Mirror: moving {v1} to the Recycle Bin…",
            v1 = ox_core::i18n::count(steps.trash.len() as u64, "item")
        ));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let undo = self.undo.recorder();
        let ctx = self.ctx.clone();
        let trash = steps.trash;
        std::thread::spawn(move || {
            let since = ox_core::undo::now();
            let result = fsops::delete(&trash, true);
            undo.trashed(&trash, since);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.mirror.running.push(Running {
            rx,
            copies: steps.copies,
            target: plan.target,
        });
    }

    fn submit_mirror_copies(&mut self, copies: Vec<TransferRequest>) {
        if copies.is_empty() {
            return;
        }
        for req in copies {
            self.engine.submit(req);
        }
        self.ui.show_transfers = true;
    }

    /// Every frame: pick up previews and start the copies once the
    /// removals are done.
    pub(super) fn poll_mirror(&mut self) {
        if let Some(d) = &mut self.mirror.dialog {
            if let Some(result) = d.job.as_ref().and_then(PlanJob::poll) {
                d.job = None;
                match result {
                    Ok(plan) => d.plan = Some(plan),
                    Err(e) => d.error = Some(e),
                }
            } else if d.job.is_some() {
                self.ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
        let mut done = Vec::new();
        self.mirror.running.retain_mut(|r| match r.rx.try_recv() {
            Ok(result) => {
                done.push((result, std::mem::take(&mut r.copies), r.target.clone()));
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for (result, copies, target) in done {
            if let Err(e) = result {
                self.toast(ox_core::trf!(
                    "Mirror: some items couldn't be moved to the Recycle Bin: {e}",
                    e = e
                ));
            }
            self.mark_changed(Some(&target));
            self.submit_mirror_copies(copies);
        }
    }

    /// The other pane's folder in the split view (a target to offer).
    pub fn other_pane_folder(&self) -> Option<PathBuf> {
        let other = self.tabs.split_partner()?;
        let folder = self.tabs.get(other)?.location().folder()?;
        Some(folder.to_path_buf())
    }
}
