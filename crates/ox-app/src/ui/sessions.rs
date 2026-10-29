//! Named sessions: the menu by the tab strip and the dialog that names one.

use egui::{Key, RichText, Ui};
use ox_core::i18n::t as tr;

use crate::app::{Action, SavedSessions, SessionAction};

/// "Save tabs as session…" and the saved sessions, each with a submenu to
/// open, add, rename or delete it.
pub fn sessions_menu(ui: &mut Ui, sessions: &mut SavedSessions, actions: &mut Vec<Action>) {
    sessions.refresh();
    let act = |a: SessionAction| Action::Session(a);
    if ui.button(tr("💾 Save tabs as session…")).clicked() {
        actions.push(act(SessionAction::SaveAs));
    }
    ui.separator();
    if sessions.list().is_empty() {
        ui.label(RichText::new(tr("No saved sessions yet.")).weak());
        return;
    }
    for s in sessions.list() {
        let name = &s.name;
        let tabs = ox_core::i18n::count(s.session.tabs.len().saturating_sub(1) as u64, "tab");
        ui.menu_button(format!("🗂 {name}"), |ui| {
            ui.label(RichText::new(tabs).weak().small());
            if ui.button(tr("📂 Open instead of the open tabs")).clicked() {
                actions.push(act(SessionAction::Open(name.clone(), false)));
            }
            if ui.button(tr("➕ Add to the open tabs")).clicked() {
                actions.push(act(SessionAction::Open(name.clone(), true)));
            }
            if ui.button(tr("💾 Save the open tabs here")).clicked() {
                actions.push(act(SessionAction::Save(name.clone())));
            }
            ui.separator();
            if ui.button(tr("✏ Rename…")).clicked() {
                actions.push(act(SessionAction::Rename(name.clone())));
            }
            if ui.button(tr("🗑 Delete")).clicked() {
                actions.push(act(SessionAction::Delete(name.clone())));
            }
        });
    }
}

/// The body of the dialog that names a new session (`old: None`) or renames
/// one. Returns whether the dialog stays open.
pub fn name_dialog(
    ui: &mut Ui,
    sessions: &SavedSessions,
    old: &Option<String>,
    text: &mut String,
    focus: &mut bool,
    actions: &mut Vec<Action>,
) -> bool {
    ui.heading(if old.is_some() {
        tr("Rename session")
    } else {
        tr("Save tabs as session")
    });
    ui.add_space(6.0);
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(tr("e.g. Work, Photos, Project X"))
            .desired_width(f32::INFINITY),
    );
    if std::mem::take(focus) {
        resp.request_focus();
    }
    let name = text.trim();
    let same_as_old = old
        .as_deref()
        .is_some_and(|o| o.trim().to_lowercase() == name.to_lowercase());
    let taken = !name.is_empty() && sessions.contains(name) && !same_as_old;
    // Saving under a taken name replaces that session; renaming can't.
    let ok = !name.is_empty() && !(taken && old.is_some());
    if taken {
        ui.add_space(4.0);
        ui.label(
            RichText::new(if old.is_some() {
                tr("A session with this name already exists.")
            } else {
                tr("A session with this name already exists; saving replaces it.")
            })
            .weak(),
        );
    }
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let mut keep = true;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let label = if old.is_some() {
            tr("Rename")
        } else {
            tr("Save")
        };
        if ui.add_enabled(ok, egui::Button::new(label)).clicked() || (enter && ok) {
            actions.push(Action::Session(match old {
                Some(o) => SessionAction::RenameTo(o.clone(), name.to_owned()),
                None => SessionAction::Save(name.to_owned()),
            }));
            keep = false;
        }
        if ui.button(tr("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
