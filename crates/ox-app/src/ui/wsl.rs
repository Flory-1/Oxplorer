//! WSL distributions in the interface: the menu entry for their folders and
//! the tooltip of their sidebar rows and Home page cards.

use ox_core::i18n::t as tr;
use std::path::Path;

use egui::Ui;
use ox_core::wsl::Distro;

use crate::app::Action;

/// "Open in WSL terminal", next to "Open in Terminal" (only inside a
/// distribution).
pub(super) fn terminal_button(ui: &mut Ui, dir: &Path, actions: &mut Vec<Action>) {
    let Some(distro) = ox_core::wsl::distro_of(dir) else {
        return;
    };
    if ui
        .button(tr("🖳 Open in WSL terminal"))
        .on_hover_text(ox_core::trf!("A {v1} shell in this folder", v1 = distro))
        .clicked()
    {
        actions.push(Action::OpenWslTerminal(dir.to_path_buf()));
    }
}

/// "Linux distribution (WSL 2)", whether it is the default and runs, and
/// its path.
pub(super) fn tooltip(d: &Distro) -> String {
    let mut lines = vec![sub_title(d)];
    if d.default {
        lines.push(tr("Default distribution").to_owned());
    }
    match d.running {
        Some(true) => lines.push(tr("Running now").to_owned()),
        Some(false) => lines.push(tr("Not running (opening it starts WSL)").to_owned()),
        None => {}
    }
    lines.push(d.root.display().to_string());
    lines.join("\n")
}

/// "Linux distribution (WSL 2)"
pub(super) fn sub_title(d: &Distro) -> String {
    ox_core::trf!("Linux distribution (WSL {v1})", v1 = d.version)
}
