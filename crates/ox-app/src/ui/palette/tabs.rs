//! Palette entries for pinned tabs and named sessions.

use super::*;
use crate::app::SessionAction;

pub(super) fn entries(app: &OxApp, v: &mut Vec<Entry>) {
    let mut cmd = |icon, label: String, detail: String, run: Run| {
        v.push(Entry {
            icon,
            label,
            detail,
            run,
        })
    };
    let tab = app.tabs.active();
    let id = tab.id;
    if !app.tabs.is_home(id) {
        if tab.pinned {
            cmd(
                "📍",
                tr("Unpin tab").into(),
                String::new(),
                act(move || Action::Tab(TabAction::Pin(id, false))),
            );
        } else {
            cmd(
                "📌",
                tr("Pin tab").into(),
                String::new(),
                act(move || Action::Tab(TabAction::Pin(id, true))),
            );
        }
    }
    cmd(
        "💾",
        tr("Save tabs as session…").into(),
        String::new(),
        act(|| Action::Session(SessionAction::SaveAs)),
    );
    for s in app.sessions.list() {
        let tabs = ox_core::i18n::count(s.session.tabs.len().saturating_sub(1) as u64, "tab");
        let (n1, n2) = (s.name.clone(), s.name.clone());
        cmd(
            "🗂",
            ox_core::trf!("Open session: {name}", name = s.name),
            tabs.clone(),
            act(move || Action::Session(SessionAction::Open(n1, false))),
        );
        cmd(
            "🗂",
            ox_core::trf!("Add session to the open tabs: {name}", name = s.name),
            tabs,
            act(move || Action::Session(SessionAction::Open(n2, true))),
        );
    }
}
