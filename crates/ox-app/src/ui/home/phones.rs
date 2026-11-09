//! Phones and cameras on a USB cable on the Home page, as cards after the drives.

use super::*;
use crate::app::RemoteAction;

pub(super) fn cards(app: &OxApp, ui: &mut Ui, accent: Color32, actions: &mut Vec<Action>) {
    for device in &app.phones.devices {
        let addr = device.addr();
        let resp = card(
            ui,
            Icon::Glyph("📱"),
            &device.name,
            tr("Phone (read-only)"),
            None,
            accent,
        );
        if resp.clicked() {
            actions.push(Action::Remote(RemoteAction::Open(addr.clone(), false)));
        }
        if resp.middle_clicked() {
            actions.push(Action::Remote(RemoteAction::Open(addr, true)));
        }
    }
}
