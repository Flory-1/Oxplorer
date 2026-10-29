//! Palette entries for mirror / sync: this folder (to the other pane's
//! folder in the split view), the split view's panes either way, and the
//! pairs mirrored recently.

use super::*;
use crate::app::mirror::MirrorAction;
use std::path::PathBuf;

fn open(source: PathBuf, target: Option<PathBuf>) -> Run {
    act(move || Action::Mirror(MirrorAction::Open { source, target }))
}

pub(super) fn entries(app: &OxApp, v: &mut Vec<Entry>) {
    let mut cmd = |label: String, detail: String, run: Run| {
        v.push(Entry {
            icon: "🔁",
            label,
            detail,
            run,
        })
    };
    let other = app.other_pane_folder();
    if let Some(folder) = app.current_folder() {
        cmd(
            tr("Mirror / sync this folder…").to_owned(),
            folder.display().to_string(),
            open(folder.clone(), None),
        );
        if let Some(other) = other.filter(|o| *o != folder) {
            cmd(
                tr("Mirror this folder to the other pane").to_owned(),
                format!("{} → {}", folder.display(), other.display()),
                open(folder, Some(other)),
            );
        }
    }
    for p in app.mirror.recent.pairs.iter().take(5) {
        cmd(
            ox_core::trf!("Mirror again: {name}", name = display_name(&p.source)),
            format!("{} → {}", p.source.display(), p.target.display()),
            open(p.source.clone(), Some(p.target.clone())),
        );
    }
}
