//! Compare folders: the comparison of the two panes of the split view, kept
//! up to date while the panes change, and copying what's missing or newer.

use std::collections::HashMap;

use ox_core::compare::{CompareJob, Comparison, CopyPlan, Mark, Options};

use super::*;

pub enum CompareAction {
    /// Compare the panes' folders (opening the split view first if needed),
    /// or compare again if a comparison is open.
    Start,
    Close,
    /// Copy what only one side has to the other side (`true`: to the right).
    CopyMissing(bool),
    /// Replace older files with the newer ones, both ways.
    CopyNewer,
    /// Start the copies of the confirmed plan.
    ConfirmCopy,
}

/// What a comparison was made for.
#[derive(Clone, PartialEq)]
struct Key {
    tabs: [TabId; 2],
    roots: [PathBuf; 2],
    options: Options,
}

/// Copies waiting for the user to confirm them.
pub struct PendingCopy {
    pub plan: CopyPlan,
    /// Replacing older files (`true`) or copying missing ones.
    pub newer: bool,
}

#[derive(Default)]
pub struct CompareState {
    /// Look into sub-folders, compare contents (hidden items follow the settings).
    pub options: Options,
    /// Hide the items that are the same on both sides.
    pub only_differences: bool,
    key: Option<Key>,
    /// Listing versions of the panes the comparison was made with.
    versions: [u64; 2],
    pub job: Option<CompareJob>,
    pub result: Option<Comparison>,
    /// Marks of the left and right pane's items.
    marks: [HashMap<PathBuf, Mark>; 2],
    pub error: Option<String>,
    /// Why nothing is compared (e.g. both panes show the same folder).
    pub hint: Option<&'static str>,
    pub confirm: Option<PendingCopy>,
    /// The pane tabs whose listings are filtered to differences.
    filtered: Option<[TabId; 2]>,
    /// The result changed: filter the listings anew.
    refilter: bool,
    /// Compare again even if nothing changed.
    force: bool,
}

impl CompareState {
    fn clear(&mut self, hint: Option<&'static str>) {
        self.key = None;
        self.job = None;
        self.result = None;
        self.marks = Default::default();
        self.error = None;
        self.hint = hint;
        self.confirm = None;
        self.refilter = true;
    }

    /// Marks for the items of the pane showing `tab`.
    pub fn marks_for(&self, tab: TabId) -> Option<&HashMap<PathBuf, Mark>> {
        let key = self.key.as_ref().filter(|_| self.result.is_some())?;
        let i = key.tabs.iter().position(|&t| t == tab)?;
        Some(&self.marks[i])
    }
}

impl OxApp {
    pub(super) fn process_compare(&mut self, action: CompareAction) {
        match action {
            CompareAction::Start => {
                if self.tabs.panes.is_none() {
                    self.tabs.toggle_split(&self.settings);
                    // The new pane shows the same folder: ask for the other one.
                    self.tabs.switch_pane();
                    self.ui.address_editing = true;
                    self.ui.focus_address = true;
                }
                // Starts (again) with the next frame.
                self.compare.get_or_insert_with(Default::default).force = true;
            }
            CompareAction::Close => self.stop_comparing(),
            CompareAction::CopyMissing(to_right) => {
                let conflict = self.settings.transfer.default_conflict;
                self.ask_to_copy(|c| c.copy_missing(to_right, conflict), false);
            }
            CompareAction::CopyNewer => self.ask_to_copy(Comparison::copy_newer, true),
            CompareAction::ConfirmCopy => {
                let Some(pending) = self.compare.as_mut().and_then(|c| c.confirm.take()) else {
                    return;
                };
                for req in pending.plan.requests {
                    self.engine.submit(req);
                }
                self.ui.show_transfers = true;
            }
        }
    }

    fn ask_to_copy(&mut self, make: impl FnOnce(&Comparison) -> CopyPlan, newer: bool) {
        let Some(state) = &mut self.compare else {
            return;
        };
        let Some(plan) = state.result.as_ref().map(make) else {
            return;
        };
        if plan.is_empty() {
            self.toast(tr("Nothing to copy."));
        } else {
            state.confirm = Some(PendingCopy { plan, newer });
        }
    }

    /// Close the comparison and show everything in the panes again.
    fn stop_comparing(&mut self) {
        if let Some(state) = &mut self.compare {
            state.only_differences = false;
            self.filter_compared_panes(self.tabs.panes);
        }
        self.compare = None;
    }

    /// Keep the comparison in step with the panes: compare again when a pane
    /// shows another folder or its files changed, and pick up results.
    pub(super) fn poll_compare(&mut self) {
        self.poll_mirror();
        let Some(state) = &mut self.compare else {
            return;
        };
        let Some(panes) = self.tabs.panes else {
            self.stop_comparing();
            return;
        };
        let tabs = panes.map(|id| self.tabs.get(id));
        let roots: Vec<PathBuf> = tabs
            .iter()
            .filter_map(|t| t.and_then(|t| t.location().folder()).map(Path::to_path_buf))
            .collect();
        let hint = match roots.as_slice() {
            [a, b] if a == b => Some(tr("Open the folder to compare with in the other pane.")),
            [_, _] => None,
            _ => Some(tr("Both panes must show a folder to compare them.")),
        };
        if hint.is_some() {
            if state.hint != hint {
                state.clear(hint);
            }
        } else {
            let loading = tabs
                .iter()
                .any(|t| t.is_some_and(|t| t.view.loading.is_some()));
            let versions = tabs.map(|t| t.map_or(0, |t| t.view.version));
            let key = Key {
                tabs: panes,
                roots: [roots[0].clone(), roots[1].clone()],
                options: Options {
                    hidden: self.settings.browsing.show_hidden,
                    ..state.options
                },
            };
            let moved = state
                .key
                .as_ref()
                .is_none_or(|k| k.tabs != key.tabs || k.roots != key.roots);
            if moved && state.key.is_some() {
                state.clear(None);
            }
            state.hint = None;
            // Files that changed during a comparison are looked at once it's done.
            let changed = state.key.as_ref() != Some(&key)
                || (state.versions != versions && state.job.is_none());
            if !loading && (changed || std::mem::take(&mut state.force)) {
                let ctx = self.ctx.clone();
                state.job = Some(CompareJob::start(
                    key.roots[0].clone(),
                    key.roots[1].clone(),
                    key.options,
                    move || ctx.request_repaint(),
                ));
                state.key = Some(key);
                state.versions = versions;
            }
        }
        if let Some(result) = state.job.as_ref().and_then(CompareJob::poll) {
            state.job = None;
            match result {
                Ok(c) => {
                    state.marks = [c.marks(true), c.marks(false)];
                    state.result = Some(c);
                    state.error = None;
                }
                Err(e) => {
                    state.result = None;
                    state.marks = Default::default();
                    state.error = Some(e);
                }
            }
            state.confirm = None;
            state.refilter = true;
        } else if state.job.is_some() {
            self.ctx.request_repaint_after(Duration::from_millis(200));
        }
        self.filter_compared_panes(Some(panes));
    }

    /// "Only differences": hide what's the same on both sides.
    fn filter_compared_panes(&mut self, panes: Option<[TabId; 2]>) {
        let Some(state) = &mut self.compare else {
            return;
        };
        let wanted = panes.filter(|_| state.only_differences && state.result.is_some());
        // Show everything again in tabs no longer filtered, or before filtering anew.
        let mut refresh: Vec<TabId> = Vec::new();
        if let Some(old) = state.filtered.take() {
            if wanted != Some(old) || state.refilter {
                refresh.extend(old);
            }
        }
        if std::mem::take(&mut state.refilter) {
            refresh.extend(panes.into_iter().flatten());
        }
        state.filtered = wanted;
        let (folders_first, case) = (
            self.settings.browsing.folders_first,
            self.settings.search.case_sensitive,
        );
        for id in refresh {
            if let Some(t) = self.tabs.get_mut(id) {
                t.refresh_visible(folders_first, case);
            }
        }
        let Some(state) = &self.compare else { return };
        let Some(panes) = wanted else { return };
        for (i, id) in panes.into_iter().enumerate() {
            let Some(tab) = self.tabs.get_mut(id) else {
                continue;
            };
            let marks = &state.marks[i];
            let view = &mut tab.view;
            let same = |e: &usize| marks.get(&view.entries[*e].path) == Some(&Mark::Same);
            if view.visible.iter().any(same) {
                let keep: Vec<usize> = view.visible.iter().copied().filter(|e| !same(e)).collect();
                view.visible = keep;
                let max = view.visible.len();
                if view.cursor.is_some_and(|c| c >= max) {
                    view.cursor = max.checked_sub(1);
                }
            }
        }
    }
}
