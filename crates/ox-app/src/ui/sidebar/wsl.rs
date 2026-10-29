//! The WSL distributions under This PC, after the drives.

use super::*;

pub(super) fn show(
    app: &OxApp,
    ui: &mut Ui,
    current: &Location,
    accent: Color32,
    actions: &mut Vec<Action>,
) {
    for distro in &app.wsl.distros {
        let root = &distro.root;
        let selected = current.folder() == Some(root.as_path());
        let resp = item(ui, Icon::Glyph("🖳"), &distro.name, None, selected, accent);
        ui.painter().text(
            resp.rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            format!("WSL {}", distro.version),
            FontId::proportional(11.5),
            ui.visuals().weak_text_color(),
        );
        if distro.running == Some(true) {
            // A small green dot on the icon while it runs.
            ui.painter().circle_filled(
                egui::pos2(resp.rect.left() + 27.0, resp.rect.center().y + 6.0),
                3.0,
                Color32::from_rgb(0x30, 0xa4, 0x6c),
            );
        }
        let resp = resp.on_hover_text(super::super::wsl::tooltip(distro));
        super::super::open_link(actions, &resp, root);
        highlight_drop(
            ui,
            &resp,
            super::super::drop_target(actions, &resp, root),
            accent,
        );
        let pinned = app.quick.is_favorite(root);
        resp.context_menu(|ui| super::super::folder_link_menu(ui, root, pinned, actions));
    }
}
