//! Cloud files: *Always keep on this device* / *Free up space*, and keeping
//! the status of syncing items up to date.

use super::*;

/// How often folders with syncing items are listed again.
const SYNC_RECHECK: Duration = Duration::from_secs(3);
/// Larger folders wait for the next change instead.
const MAX_RECHECKED: usize = 5000;

impl OxApp {
    /// Pin or unpin items in the background; folders with everything in
    /// them, as the provider does it.
    pub(super) fn keep_on_device(&mut self, paths: Vec<PathBuf>, pin: ox_core::cloud::Pin) {
        if paths.is_empty() {
            return;
        }
        self.in_background(move || {
            let failed = ox_core::cloud::set_pin(&paths, pin);
            let toast = match failed.as_slice() {
                [] => None,
                [(p, e)] => Some(ox_core::trf!(
                    "Couldn't change {v1}: {e}",
                    v1 = entry::display_name(p),
                    e = e
                )),
                [(_, e), ..] => Some(ox_core::trf!(
                    "{v1} items couldn't be changed: {e}",
                    v1 = failed.len(),
                    e = e
                )),
            };
            let mut folders: Vec<PathBuf> = paths
                .iter()
                .filter_map(|p| p.parent())
                .map(Path::to_path_buf)
                .collect();
            folders.dedup();
            Notice::Changed(folders, toast)
        });
    }

    /// While a pane shows items that are syncing, list its folder again
    /// every few seconds so their status follows the provider.
    pub(super) fn poll_cloud(&mut self) {
        if self
            .cloud_check
            .is_some_and(|at| at.elapsed() < SYNC_RECHECK)
        {
            return;
        }
        let panes = [Some(self.tabs.active), self.tabs.split_partner()];
        let mut syncing = false;
        for id in panes.into_iter().flatten() {
            let Some(tab) = self.tabs.get_mut(id) else {
                continue;
            };
            let busy = tab
                .view
                .entries
                .iter()
                .any(|e| matches!(e.cloud, Some(ox_core::cloud::CloudStatus::Syncing)));
            let small = tab.view.entries.len() <= MAX_RECHECKED;
            if busy && small && tab.view.loading.is_none() && tab.location().folder().is_some() {
                tab.stale = true;
                syncing = true;
            }
        }
        self.cloud_check = Some(Instant::now());
        if syncing {
            self.ctx.request_repaint_after(SYNC_RECHECK);
        }
    }
}
