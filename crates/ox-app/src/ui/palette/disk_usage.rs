//! Palette entries for the disk usage view: of the current folder, of each
//! drive, and scanning again or stopping in the view.

use super::*;
use crate::app::DiskUsageAction;

pub(super) fn entries(app: &OxApp, v: &mut Vec<Entry>) {
    let mut cmd = |label: String, detail: String, run: Run| {
        v.push(Entry {
            icon: "🖴",
            label,
            detail,
            run,
        })
    };
    if let Some(folder) = app.current_folder() {
        cmd(
            tr("Show disk usage of this folder").to_owned(),
            folder.display().to_string(),
            act(move || Action::Navigate(Location::DiskUsage(folder))),
        );
    }
    for d in app.drives.iter().filter(|d| d.remote.is_none()) {
        let mount = d.mount.clone();
        cmd(
            ox_core::trf!("Disk usage: {name}", name = d.title()),
            d.mount.display().to_string(),
            act(move || Action::Navigate(Location::DiskUsage(mount))),
        );
    }
    if let Location::DiskUsage(_) = app.tabs.active().location() {
        cmd(
            tr("Scan disk usage again").to_owned(),
            String::new(),
            act(|| Action::DiskUsage(DiskUsageAction::Rescan)),
        );
        cmd(
            tr("Stop disk usage scan").to_owned(),
            String::new(),
            act(|| Action::DiskUsage(DiskUsageAction::Stop)),
        );
    }
}
