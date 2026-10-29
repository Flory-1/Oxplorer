//! The Shortcuts page of the settings.

use egui::{vec2, Color32, RichText, Ui};
use ox_core::i18n::t as tr;

use crate::app::OxApp;

pub(super) fn shortcuts(ui: &mut Ui, app: &mut OxApp) {
    use crate::shortcuts::{self, COMMANDS};
    use egui::{Key, KeyboardShortcut, Modifiers};

    // While recording, the next key press (with its modifiers) is the new shortcut.
    if let Some(cmd) = app.ui.recording {
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !shortcuts::is_modifier(*key) => Some((*key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, m)) = pressed {
            if key == Key::Escape && m.is_none() {
                app.ui.recording = None;
            } else {
                let mut mods = Modifiers::NONE;
                if m.ctrl || m.command {
                    mods |= Modifiers::COMMAND;
                }
                if m.alt {
                    mods |= Modifiers::ALT;
                }
                if m.shift {
                    mods |= Modifiers::SHIFT;
                }
                let id = shortcuts::command_info(cmd).id.to_owned();
                let text = shortcuts::to_text(&KeyboardShortcut::new(mods, key));
                app.settings.shortcuts.insert(id, text);
                app.ui.recording = None;
            }
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
                ui.label(tr(c.label));
                let keys = app.keymap.keys(c.cmd).to_vec();
                let recording = app.ui.recording == Some(c.cmd);
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
                        app.ui.recording = if recording { None } else { Some(c.cmd) };
                    }
                    let changed = app.settings.shortcuts.contains_key(c.id);
                    if !keys.is_empty()
                        && ui
                            .small_button("✖")
                            .on_hover_text(tr("No shortcut"))
                            .clicked()
                    {
                        app.settings
                            .shortcuts
                            .insert(c.id.to_owned(), String::new());
                    }
                    if changed
                        && ui
                            .small_button("↺")
                            .on_hover_text(ox_core::trf!(
                                "Back to {v1}",
                                v1 = c.defaults.join(" / ")
                            ))
                            .clicked()
                    {
                        app.settings.shortcuts.remove(c.id);
                    }
                });
                let clash = keys.iter().find_map(|k| app.keymap.conflict(c.cmd, k));
                match clash {
                    Some(other) => ui.colored_label(
                        Color32::from_rgb(0xe5, 0x48, 0x4d),
                        format!(
                            "⚠ {} {}",
                            tr("also"),
                            tr(shortcuts::command_info(other).label)
                        ),
                    ),
                    None => ui.label(""),
                };
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    if !app.settings.shortcuts.is_empty() && ui.button(tr("Reset all shortcuts")).clicked() {
        app.settings.shortcuts.clear();
        app.ui.recording = None;
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
