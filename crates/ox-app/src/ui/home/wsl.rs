//! The WSL distributions on the Home page, as cards after the drives.

use super::*;

pub(super) fn cards(app: &OxApp, ui: &mut Ui, accent: Color32, actions: &mut Vec<Action>) {
    for d in &app.wsl.distros {
        let resp = card(
            ui,
            Icon::Glyph("🖳"),
            &d.name,
            &super::super::wsl::sub_title(d),
            None,
            accent,
        )
        .on_hover_text(super::super::wsl::tooltip(d));
        super::super::open_link(actions, &resp, &d.root);
        super::super::drop_target(actions, &resp, &d.root);
        let pinned = app.quick.is_favorite(&d.root);
        resp.context_menu(|ui| super::super::folder_link_menu(ui, &d.root, pinned, actions));
    }
}
