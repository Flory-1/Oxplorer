//! Find duplicates and empty folders: the scan of each tab showing a
//! clean-up page, the items ticked in it, and moving them to the Recycle Bin.

use std::collections::HashMap;

use ox_core::cleanup::{CleanupMode, Outcome, ScanHandle, ScanOptions};

use super::*;

pub enum CleanupAction {
    /// Move the ticked items of this tab's page to the Recycle Bin (the
    /// page has asked already).
    DeleteConfirmed(TabId),
    /// Open the folder an item is in, in a new tab, with the item selected.
    ShowInFolder(PathBuf),
}

/// The delete question of a clean-up page.
#[derive(Default)]
pub struct ConfirmCleanup {
    /// Every copy of some files is ticked, and the user said that's fine.
    pub all_copies_ok: bool,
}

/// A clean-up page: its scan, what it found and what is ticked.
pub struct CleanupView {
    pub root: PathBuf,
    pub mode: CleanupMode,
    /// The scan in progress.
    pub scan: Option<ScanHandle>,
    pub outcome: Option<Outcome>,
    pub elapsed: Duration,
    pub selected: HashSet<PathBuf>,
    pub confirm: Option<ConfirmCleanup>,
}

impl CleanupView {
    /// The ticked items that are in the list, in its order.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        let listed: Vec<&PathBuf> = match &self.outcome {
            Some(Outcome::Duplicates(r)) => r
                .groups
                .iter()
                .flat_map(|g| g.files.iter().map(|f| &f.path))
                .collect(),
            Some(Outcome::EmptyFolders(r)) => r.folders.iter().collect(),
            None => Vec::new(),
        };
        listed
            .into_iter()
            .filter(|p| self.selected.contains(*p))
            .cloned()
            .collect()
    }
}

/// The clean-up pages of all tabs.
#[derive(Default)]
pub struct Cleanups(HashMap<TabId, CleanupView>);

impl Cleanups {
    /// (Re)start the scan of the page in `tab`.
    pub(super) fn start(
        &mut self,
        tab: TabId,
        root: PathBuf,
        mode: CleanupMode,
        show_hidden: bool,
        ctx: Context,
    ) {
        let opts = ScanOptions {
            show_hidden,
            ..Default::default()
        };
        let scan = ox_core::cleanup::start(root.clone(), mode, opts, move || ctx.request_repaint());
        self.0.insert(
            tab,
            CleanupView {
                root,
                mode,
                scan: Some(scan),
                outcome: None,
                elapsed: Duration::ZERO,
                selected: HashSet::new(),
                confirm: None,
            },
        );
    }

    pub fn get_mut(&mut self, tab: TabId) -> Option<&mut CleanupView> {
        self.0.get_mut(&tab)
    }
}

impl OxApp {
    /// Take in finished scans and forget (stopping) those of tabs that
    /// were closed or show something else now.
    pub(super) fn poll_cleanups(&mut self) {
        let tabs = &self.tabs;
        self.cleanups.0.retain(|&id, view| {
            tabs.get(id).is_some_and(|t| {
                matches!(t.location(), Location::Cleanup { root, mode }
                    if *root == view.root && *mode == view.mode)
            })
        });
        let mut running = false;
        for view in self.cleanups.0.values_mut() {
            let Some(scan) = &view.scan else { continue };
            match scan.result.try_recv() {
                Ok((outcome, elapsed)) => {
                    view.outcome = Some(outcome);
                    view.elapsed = elapsed;
                    view.scan = None;
                }
                Err(_) => running = true,
            }
        }
        if running {
            // For the progress counters.
            self.ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    /// Up: from a clean-up page to its folder; elsewhere as tabs go up.
    pub(super) fn cleanup_aware_up(&mut self) {
        let tab = self.tabs.active_mut();
        match tab.location().clone() {
            Location::Cleanup { root, .. } => tab.navigate(Location::Folder(root)),
            _ => tab.up(),
        }
    }

    pub(super) fn process_cleanup(&mut self, action: CleanupAction) {
        match action {
            CleanupAction::DeleteConfirmed(tab) => {
                let Some(view) = self.cleanups.get_mut(tab) else {
                    return;
                };
                let paths = view.selected_paths();
                if paths.is_empty() {
                    return;
                }
                let gone: HashSet<PathBuf> = paths.iter().cloned().collect();
                match &mut view.outcome {
                    Some(Outcome::Duplicates(r)) => r.remove(&gone),
                    Some(Outcome::EmptyFolders(r)) => r.folders.retain(|p| !gone.contains(p)),
                    None => {}
                }
                view.selected.clear();
                view.confirm = None;
                // Always to the Recycle Bin (whatever the delete setting), with undo.
                self.process(Action::MoveToBin(paths));
            }
            CleanupAction::ShowInFolder(path) => {
                let Some(parent) = path.parent().map(Path::to_path_buf) else {
                    return;
                };
                let after = Some(self.tabs.active);
                let id = self
                    .tabs
                    .open(Location::Folder(parent), after, true, &self.settings);
                if let Some(tab) = self.tabs.get_mut(id) {
                    tab.view.select_after_load = Some(path);
                }
            }
        }
    }
}
