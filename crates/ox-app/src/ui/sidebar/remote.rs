//! The sidebar's "Remote locations" section: saved SFTP/FTP/WebDAV places,
//! servers open in tabs that aren't saved, and "Add…".

use super::*;
use crate::app::RemoteAction;
use ox_core::remote::RemoteAddr;

pub(super) fn show(
    app: &OxApp,
    ui: &mut Ui,
    current: &Location,
    accent: Color32,
    actions: &mut Vec<Action>,
) {
    let places = &app.settings.remote.0;
    if !app.settings.sidebar.show_drives && places.is_empty() {
        return;
    }
    section(ui, tr("Remote locations"));
    let here = match current {
        Location::Remote(a) => Some(a),
        _ => None,
    };
    for (i, place) in places.iter().enumerate() {
        let addr = &place.addr;
        let selected = here.is_some_and(|h| h.same_server(addr) && h.path == addr.path);
        let connected = app.remote.conns.is_connected(addr);
        let resp = item(ui, Icon::Server, &place.title(), None, selected, accent)
            .on_hover_text(hover(addr, connected));
        if connected {
            dot(ui, &resp, accent);
        }
        if resp.clicked() {
            actions.push(Action::Remote(RemoteAction::Open(addr.clone(), false)));
        }
        if resp.middle_clicked() {
            actions.push(Action::Remote(RemoteAction::Open(addr.clone(), true)));
        }
        if connected {
            let hovering = super::super::drop_target(actions, &resp, &addr.to_path());
            highlight_drop(ui, &resp, hovering, accent);
        }
        resp.context_menu(|ui| {
            if ui.button(tr("📑 Open in new tab")).clicked() {
                actions.push(Action::Remote(RemoteAction::Open(addr.clone(), true)));
            }
            if connected && ui.button(tr("✖ Disconnect")).clicked() {
                actions.push(Action::Remote(RemoteAction::Disconnect(addr.clone())));
            }
            ui.separator();
            if ui.button(tr("✏ Edit…")).clicked() {
                actions.push(Action::Remote(RemoteAction::Edit(i)));
            }
            if ui.button(tr("✖ Forget the password")).clicked() {
                actions.push(Action::Remote(RemoteAction::ForgetPassword(addr.clone())));
            }
            if ui.button(tr("🗑 Remove")).clicked() {
                actions.push(Action::Remote(RemoteAction::Remove(i)));
            }
        });
    }
    // Servers open in a tab without a saved place.
    let mut unsaved: Vec<RemoteAddr> = Vec::new();
    for tab in app.tabs.tabs.iter() {
        if let Location::Remote(a) = tab.location() {
            let saved = places.iter().any(|p| p.addr.same_server(a));
            if !saved && !unsaved.iter().any(|u| u.same_server(a)) {
                unsaved.push(a.with_path("/"));
            }
        }
    }
    for addr in unsaved {
        let selected = here.is_some_and(|h| h.same_server(&addr));
        let resp = item(ui, Icon::Server, &addr.title(), None, selected, accent)
            .on_hover_text(hover(&addr, true));
        dot(ui, &resp, accent);
        if resp.clicked() {
            actions.push(Action::Remote(RemoteAction::Open(addr.clone(), false)));
        }
        if resp.middle_clicked() {
            actions.push(Action::Remote(RemoteAction::Open(addr.clone(), true)));
        }
        resp.context_menu(|ui| {
            if ui.button(tr("⭐ Save as remote location")).clicked() {
                actions.push(Action::Remote(RemoteAction::Add(Some(addr.clone()))));
            }
            if ui.button(tr("✖ Disconnect")).clicked() {
                actions.push(Action::Remote(RemoteAction::Disconnect(addr.clone())));
            }
        });
    }
    let add = item(ui, Icon::Glyph("➕"), tr("Add…"), None, false, accent)
        .on_hover_text(tr("Connect to an SFTP, FTP or WebDAV server"));
    if add.clicked() {
        actions.push(Action::Remote(RemoteAction::Add(None)));
    }
}

fn hover(addr: &RemoteAddr, connected: bool) -> String {
    let state = if connected {
        tr("Connected")
    } else {
        tr("Not connected")
    };
    format!("{}\n{state}", addr.to_url())
}

/// A small dot at the right: the server is connected.
fn dot(ui: &Ui, resp: &egui::Response, accent: Color32) {
    let at = resp.rect.right_center() - vec2(12.0, 0.0);
    ui.painter().circle_filled(at, 3.0, accent);
}
