//! Custom commands: their entries in the right-click menus, the question
//! before one runs and the windows with their output.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use egui::{Context, Id, RichText, Ui};
use ox_core::commands::{CustomCommand, Target};
use ox_core::i18n::t as tr;
use ox_core::trf;

use crate::app::{Action, CommandAction, OxApp};

/// Icons a command can have (all in the icon font).
pub const ICONS: &[&str] = &[
    "▶", "⚡", "🛠", "📝", "🖳", "📋", "🔐", "🔍", "🔗", "🌐", "🌿", "📦", "🖼", "🎵", "🎬", "📄",
    "📁", "💾", "🗂", "📜", "⚙", "🚀", "✨", "🗔",
];

/// The icon of `c` (the first of [`ICONS`] if it has none or another one).
pub fn icon(c: &CustomCommand) -> &'static str {
    ICONS
        .iter()
        .find(|i| **i == c.icon)
        .copied()
        .unwrap_or(ICONS[0])
}

/// Up to this many commands are listed in the menu itself, more in a submenu.
const TOP_LEVEL: usize = 3;

static CURRENT: RwLock<Vec<CustomCommand>> = RwLock::new(Vec::new());

/// Make `commands` the ones the menus offer.
pub fn set_current(commands: &[CustomCommand]) {
    *CURRENT.write().unwrap() = commands.to_vec();
}

thread_local! {
    /// The last selection asked about and which of its items are folders.
    static KINDS: RefCell<(Vec<PathBuf>, Vec<bool>)> = RefCell::default();
}

/// The items with whether each is a folder (remembered while a menu is
/// open, so it isn't looked up every frame).
fn with_kinds(paths: &[PathBuf]) -> Vec<(PathBuf, bool)> {
    KINDS.with(|k| {
        let mut k = k.borrow_mut();
        if k.0 != paths {
            *k = (paths.to_vec(), paths.iter().map(|p| p.is_dir()).collect());
        }
        k.0.iter().cloned().zip(k.1.iter().copied()).collect()
    })
}

/// The commands for `target`, with their index in the settings.
fn matching(target: Target<'_>) -> Vec<(usize, CustomCommand)> {
    CURRENT
        .read()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, c)| c.applies(target))
        .map(|(i, c)| (i, c.clone()))
        .collect()
}

fn entries(
    ui: &mut Ui,
    commands: &[(usize, CustomCommand)],
    run: impl Fn(usize) -> CommandAction,
    actions: &mut Vec<Action>,
) {
    let mut list = |ui: &mut Ui| {
        for (i, c) in commands {
            let shortcut = crate::app::shortcut_of(c)
                .map(|k| crate::shortcuts::display(&k))
                .unwrap_or_default();
            let button =
                egui::Button::new(format!("{}  {}", icon(c), c.name)).shortcut_text(shortcut);
            if ui.add(button).clicked() {
                actions.push(Action::Command(run(*i)));
            }
        }
    };
    if commands.len() <= TOP_LEVEL {
        list(ui);
    } else {
        ui.menu_button(tr("⚡ Commands"), list);
    }
}

/// The custom commands for the selected items (right-click menu).
pub fn item_menu(ui: &mut Ui, selection: &[PathBuf], actions: &mut Vec<Action>) {
    if CURRENT.read().unwrap().is_empty() {
        return;
    }
    let items = with_kinds(selection);
    let commands = matching(Target::Items(&items));
    if commands.is_empty() {
        return;
    }
    ui.separator();
    let paths = selection.to_vec();
    let run = |index| CommandAction::Run {
        index,
        items: paths.clone(),
        folder: None,
    };
    entries(ui, &commands, run, actions);
}

/// The custom commands for a folder's background (right-click menu).
pub fn background_menu(ui: &mut Ui, folder: &Path, actions: &mut Vec<Action>) {
    let commands = matching(Target::Background);
    if commands.is_empty() {
        return;
    }
    let run = |index| CommandAction::Run {
        index,
        items: Vec::new(),
        folder: Some(folder.to_path_buf()),
    };
    entries(ui, &commands, run, actions);
}

/// The question before a command runs and the output windows.
pub fn show(app: &mut OxApp, ctx: &Context) {
    // Recording a shortcut ends when the settings are left (it turns the
    // app's shortcuts off).
    if !matches!(
        app.tabs.active().location(),
        crate::tabs::Location::Settings
    ) {
        app.commands.recording = false;
    }
    confirm(app, ctx);
    outputs(app, ctx);
}

fn confirm(app: &mut OxApp, ctx: &Context) {
    let Some(pending) = &app.commands.confirm else {
        return;
    };
    let mut answer = None;
    let modal = egui::Modal::new(Id::unique("command_confirm")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.heading(trf!("Run “{name}”?", name = pending.command.name));
        ui.add_space(6.0);
        if pending.runs.len() > 1 {
            ui.label(trf!(
                "It runs {n} times, once for each item:",
                n = pending.runs.len()
            ));
        }
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                for r in pending.runs.iter().take(50) {
                    ui.label(RichText::new(&r.line).monospace());
                }
                if pending.runs.len() > 50 {
                    ui.label("…");
                }
            });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(RichText::new(tr("Run")).strong()).clicked() {
                answer = Some(true);
            }
            if ui.button(tr("Cancel")).clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    if let Some(yes) = answer {
        app.push(Action::Command(CommandAction::Confirm(yes)));
    }
}

fn outputs(app: &mut OxApp, ctx: &Context) {
    for w in &mut app.commands.outputs {
        w.poll();
        let status = match &w.done {
            None => tr("running…").to_owned(),
            Some(None) => tr("done").to_owned(),
            Some(Some(_)) => tr("failed").to_owned(),
        };
        let mut open = w.open;
        let mut close = false;
        egui::Window::new(format!("▶ {} — {status}", w.title))
            .id(Id::unique(("command_output", w.id)))
            .open(&mut open)
            .default_size([620.0, 360.0])
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                if let Some(Some(e)) = &w.done {
                    ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), format!("⚠ {e}"));
                }
                ui.horizontal(|ui| {
                    if w.done.is_none() {
                        ui.spinner();
                    }
                    if ui.button(tr("📋 Copy")).clicked() {
                        ui.ctx().copy_text(w.lines.join("\n"));
                    }
                    if w.done.is_some() && ui.button(tr("Close")).clicked() {
                        close = true;
                    }
                });
                ui.separator();
                if w.lines.is_empty() && w.done.is_some() {
                    ui.label(RichText::new(tr("No output")).weak());
                }
                let row = ui.text_style_height(&egui::TextStyle::Monospace);
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show_rows(ui, row, w.lines.len(), |ui, rows| {
                        for l in &w.lines[rows] {
                            ui.add(egui::Label::new(RichText::new(l).monospace()).extend());
                        }
                    });
            });
        w.open = open && !close;
    }
    // A closed window of a run that still goes on stays until it ends.
    app.commands.outputs.retain(|w| w.open || w.done.is_none());
}
