//! Phones and cameras on a USB cable under This PC, after the drives.

use super::*;
use crate::app::RemoteAction;

pub(super) fn show(
    app: &OxApp,
    ui: &mut Ui,
    current: &Location,
    accent: Color32,
    actions: &mut Vec<Action>,
) {
    for device in &app.phones.devices {
        let addr = device.addr();
        let selected = matches!(current, Location::Remote(a) if a.same_server(&addr));
        let resp = item(ui, Icon::Glyph("📱"), &device.name, None, selected, accent)
            .on_hover_text(tr("Phone or camera on a USB cable (read-only)"));
        if resp.clicked() {
            actions.push(Action::Remote(RemoteAction::Open(addr.clone(), false)));
        }
        if resp.middle_clicked() {
            actions.push(Action::Remote(RemoteAction::Open(addr.clone(), true)));
        }
        resp.context_menu(|ui| {
            if ui.button(tr("📑 Open in new tab")).clicked() {
                actions.push(Action::Remote(RemoteAction::Open(addr.clone(), true)));
            }
        });
    }
}
