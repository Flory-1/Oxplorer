//! The Select by pattern dialog: type `*.jpg; *.png; report*` to select
//! (or deselect) the matching items, with a live count.

use egui::{Key, RichText, Ui};
use ox_core::i18n::t as tr;
use ox_core::select::Patterns;

use crate::app::{Action, SelectAction};
use crate::tabs::FolderView;

/// The dialog's contents. Returns whether it stays open.
pub fn show(
    ui: &mut Ui,
    text: &mut String,
    select: &mut bool,
    focus: &mut bool,
    view: &FolderView,
    actions: &mut Vec<Action>,
) -> bool {
    ui.heading(if *select {
        tr("Select by pattern")
    } else {
        tr("Deselect by pattern")
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.radio_value(select, true, tr("Select"));
        ui.radio_value(select, false, tr("Deselect"));
    });
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text("*.jpg; *.png; report*")
            .desired_width(f32::INFINITY),
    );
    if *focus {
        *focus = false;
        resp.request_focus();
        // Everything selected, so typing replaces the last pattern.
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(text.chars().count()),
                )));
            state.store(ui.ctx(), resp.id);
        }
    }
    let patterns = Patterns::parse(text);
    let matching = view
        .visible
        .iter()
        .filter(|&&i| patterns.matches(&view.entries[i].name))
        .count();
    ui.label(
        RichText::new(tr(
            "* stands for any characters, ? for one. Separate several patterns with ;",
        ))
        .weak()
        .small(),
    );
    ui.label(ox_core::trf!(
        "Matching: {v1}",
        v1 = ox_core::i18n::count(matching as u64, "item")
    ));
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    ui.add_space(8.0);
    let mut keep = true;
    ui.horizontal(|ui| {
        let label = if *select {
            tr("Select")
        } else {
            tr("Deselect")
        };
        let ok = ui
            .add_enabled(!patterns.is_empty(), egui::Button::new(label))
            .clicked();
        if (ok || enter) && !patterns.is_empty() {
            actions.push(Action::Select(SelectAction::Pattern(text.clone(), *select)));
            keep = false;
        }
        if ui.button(tr("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
