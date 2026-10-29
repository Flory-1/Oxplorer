//! Disk usage view: the scans behind it, and its actions.

use ox_core::disk_usage::{self, Scan};

use super::*;

pub enum DiskUsageAction {
    /// Scan the tree the view shows again.
    Rescan,
    /// Stop the scan (what is counted so far stays).
    Stop,
}

/// The trees scanned for disk usage views. A view zoomed into a sub-folder
/// uses the scan of the folder it was opened on.
#[derive(Default)]
pub struct DiskUsageScans {
    scans: Vec<Scan>,
}

impl DiskUsageScans {
    /// The scan to show `path` from: the newest one that has it (or may
    /// still find it).
    pub fn for_path(&self, path: &Path) -> Option<&Scan> {
        self.scans.iter().rev().find(|s| {
            path.starts_with(s.root()) && (s.is_running() || s.tree().find(path).is_some())
        })
    }

    /// Files in `dir` changed: drop what is gone from the scanned trees.
    pub fn changed(&self, dir: &Path) {
        for s in &self.scans {
            s.forget_missing(dir);
        }
    }
}

fn disk_usage_path(loc: &Location) -> Option<&Path> {
    match loc {
        Location::DiskUsage(p) => Some(p),
        _ => None,
    }
}

impl OxApp {
    /// Start a scan for each shown disk usage view that has none, and stop
    /// the scans no tab shows any more.
    pub(super) fn poll_disk_usage(&mut self) {
        let shown: Vec<PathBuf> = [Some(self.tabs.active), self.tabs.split_partner()]
            .into_iter()
            .flatten()
            .filter_map(|id| self.tabs.get(id))
            .filter_map(|t| disk_usage_path(t.location()).map(Path::to_path_buf))
            .collect();
        for path in shown {
            if self.disk_usage.for_path(&path).is_none() {
                self.start_disk_scan(path);
            }
        }
        let open: Vec<&Path> = self
            .tabs
            .tabs
            .iter()
            .filter_map(|t| disk_usage_path(t.location()))
            .collect();
        self.disk_usage
            .scans
            .retain(|s| open.iter().any(|p| p.starts_with(s.root())));
    }

    fn start_disk_scan(&mut self, root: PathBuf) {
        let ctx = self.ctx.clone();
        let sizes = Some(self.folder_sizes.clone());
        let scan = disk_usage::scan(root, sizes, move || ctx.request_repaint());
        self.disk_usage.scans.push(scan);
    }

    /// The active tab shows a disk usage view.
    pub(super) fn in_disk_usage(&self) -> bool {
        disk_usage_path(self.tabs.active().location()).is_some()
    }

    pub(super) fn disk_usage_action(&mut self, action: DiskUsageAction) {
        let Some(path) = disk_usage_path(self.tabs.active().location()).map(Path::to_path_buf)
        else {
            return;
        };
        match action {
            DiskUsageAction::Rescan => {
                let root = self
                    .disk_usage
                    .for_path(&path)
                    .map_or(path, |s| s.root().to_path_buf());
                self.disk_usage.scans.retain(|s| s.root() != root);
                self.start_disk_scan(root);
            }
            DiskUsageAction::Stop => {
                if let Some(scan) = self.disk_usage.for_path(&path) {
                    scan.cancel();
                }
            }
        }
    }

    /// Up in a disk usage view shows the folder above (scanning it, if it's
    /// above the scanned one). `false` if there's no view or nothing above.
    pub(super) fn disk_usage_up(&mut self) -> bool {
        let parent = disk_usage_path(self.tabs.active().location())
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let Some(parent) = parent else {
            return false;
        };
        self.navigate(Location::DiskUsage(parent));
        true
    }
}
