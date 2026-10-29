//! Palette entries for finding duplicates and empty folders in the current folder.

use super::*;
use ox_core::cleanup::CleanupMode;

pub(super) fn entries(app: &OxApp, v: &mut Vec<Entry>) {
    let Some(folder) = app.current_folder() else {
        return;
    };
    let detail = folder.display().to_string();
    for (mode, label) in [
        (
            CleanupMode::Duplicates,
            tr("Find duplicates in this folder"),
        ),
        (
            CleanupMode::EmptyFolders,
            tr("Find empty folders in this folder"),
        ),
    ] {
        let root = folder.clone();
        v.push(Entry {
            icon: Location::cleanup_icon(mode),
            label: label.to_owned(),
            detail: detail.clone(),
            run: act(move || Action::OpenInNewTab(Location::Cleanup { root, mode }, true)),
        });
    }
}
