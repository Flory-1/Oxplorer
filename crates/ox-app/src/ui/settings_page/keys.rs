//! The Shortcuts page of the settings: the built-in commands, then the
//! custom commands (their shortcut is the one in the command's form under
//! Settings → Custom commands).

use egui::{vec2, Color32, KeyboardShortcut, RichText, Ui};
use ox_core::i18n::t as tr;

use crate::app::OxApp;
use crate::shortcuts::{self, Binding, COMMANDS};

const WARN: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

pub(super) fn shortcuts(ui: &mut Ui, app: &mut OxApp) {
    // While recording, the next key press (with its modifiers) is the new shortcut.
    if let Some(target) = app.ui.recording {
        if let Some(keys) = ui.input(shortcuts::recorded) {
            if let Some(k) = keys {
                set(app, target, shortcuts::to_text(&k));
            }
            app.ui.recording = None;
        }
    }

    ui.label(RichText::new(tr("Click a shortcut and press the new keys (Esc cancels).")).weak());
    ui.add_space(8.0);
    egui::Grid::new("shortcuts")
        .striped(true)
        .num_columns(3)
        .spacing(vec2(16.0, 6.0))
        .show(ui, |ui| {
            for c in COMMANDS {
                row(ui, app, Binding::Builtin(c.cmd));
            }
        });
    ui.add_space(8.0);
    if !app.settings.shortcuts.is_empty() && ui.button(tr("Reset all shortcuts")).clicked() {
        app.settings.shortcuts.clear();
        app.ui.recording = None;
    }
    if !app.settings.commands.0.is_empty() {
        ui.add_space(12.0);
        ui.label(RichText::new(tr("⚡ Custom commands")).strong());
        ui.label(
            RichText::new(tr(
                "The same shortcuts as in Settings → Custom commands. When a built-in command has the same keys, the built-in one goes first.",
            ))
            .weak(),
        );
        egui::Grid::new("custom_shortcuts")
            .striped(true)
            .num_columns(3)
            .spacing(vec2(16.0, 6.0))
            .show(ui, |ui| {
                for i in 0..app.settings.commands.0.len() {
                    row(ui, app, Binding::Custom(i));
                }
            });
    }
    ui.add_space(12.0);
    ui.label(RichText::new(tr("Always available")).strong());
    egui::Grid::new("fixed_shortcuts")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            for (k, d) in [
                (tr("Enter"), tr("Open selection")),
                (
                    tr("Arrow keys, Home, End"),
                    tr("Move the selection (Shift to extend)"),
                ),
                (tr("Type letters"), tr("Jump to item")),
                (tr("Mouse side buttons"), tr("Back / forward")),
                (tr("Middle click"), tr("Open folder in new tab / close tab")),
            ] {
                ui.monospace(k);
                ui.label(d);
                ui.end_row();
            }
        });
}

/// What `b` is called on this page.
fn name(app: &OxApp, b: Binding) -> String {
    match b {
        Binding::Builtin(cmd) => tr(shortcuts::command_info(cmd).label).to_owned(),
        Binding::Custom(i) => app
            .settings
            .commands
            .0
            .get(i)
            .map(|c| c.name.clone())
            .unwrap_or_default(),
    }
}

/// Give `b` the keys `text` ("" for none).
fn set(app: &mut OxApp, b: Binding, text: String) {
    match b {
        Binding::Builtin(cmd) => {
            let id = shortcuts::command_info(cmd).id.to_owned();
            app.settings.shortcuts.insert(id, text);
        }
        Binding::Custom(i) => {
            if let Some(c) = app.settings.commands.0.get_mut(i) {
                c.shortcut = text;
            }
        }
    }
}

/// One line: the name, its keys (click to record new ones), *No
/// shortcut*, *Back to* the defaults and a clash with another command.
fn row(ui: &mut Ui, app: &mut OxApp, b: Binding) {
    let keys: Vec<KeyboardShortcut> = match b {
        Binding::Builtin(cmd) => app.keymap.keys(cmd).to_vec(),
        Binding::Custom(i) => app.keymap.custom_key(i).into_iter().collect(),
    };
    // Only built-in commands have defaults to go back to; a custom
    // command's go back to none with ✖.
    let changed_from = match b {
        Binding::Builtin(cmd) => {
            let c = shortcuts::command_info(cmd);
            app.settings
                .shortcuts
                .contains_key(c.id)
                .then(|| c.defaults.join(" / "))
        }
        Binding::Custom(_) => None,
    };
    ui.label(name(app, b));
    let recording = app.ui.recording == Some(b);
    let text = if recording {
        tr("Press keys…").to_owned()
    } else if keys.is_empty() {
        "—".to_owned()
    } else {
        keys.iter()
            .map(shortcuts::display)
            .collect::<Vec<_>>()
            .join(" / ")
    };
    ui.horizontal(|ui| {
        let button = egui::Button::new(RichText::new(text).monospace())
            .selected(recording)
            .min_size(vec2(170.0, 0.0));
        if ui.add(button).clicked() {
            app.ui.recording = if recording { None } else { Some(b) };
        }
        if !keys.is_empty()
            && ui
                .small_button("✖")
                .on_hover_text(tr("No shortcut"))
                .clicked()
        {
            set(app, b, String::new());
        }
        if let Some(defaults) = changed_from {
            let back = ui
                .small_button("↺")
                .on_hover_text(ox_core::trf!("Back to {v1}", v1 = defaults));
            if let (true, Binding::Builtin(cmd)) = (back.clicked(), b) {
                app.settings
                    .shortcuts
                    .remove(shortcuts::command_info(cmd).id);
            }
        }
    });
    match keys.iter().find_map(|k| app.keymap.conflict(b, k)) {
        Some(other) => ui.colored_label(WARN, format!("⚠ {} {}", tr("also"), name(app, other))),
        None => ui.label(""),
    };
    ui.end_row();
}
