//! Custom commands: their entries in the right-click menus, the question
//! before one runs and the windows with their output.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use egui::{Context, Id, RichText, Ui};
use ox_core::commands::{menu_groups, CustomCommand, MenuGroup, MenuPlace, Target};
use ox_core::i18n::t as tr;
use ox_core::trf;

use crate::app::{Action, CommandAction, OxApp};

/// Icons a command can have (all in the icon font).
pub const ICONS: &[&str] = &[
    "▶", "⚡", "🚀", "✨", "🛠", "⚙", "🖳", "⌨", "📝", "✏", "📄", "📜", "📋", "📥", "📤", "📁", "📂",
    "🗂", "🗃", "📦", "💾", "🖴", "🔐", "🔍", "👁", "🔗", "🌐", "☁", "🖧", "🖥", "🗔", "🗗", "🌿", "🖼",
    "🎞", "🎵", "🎬", "🔊", "🔤", "🏷", "⭐", "📌", "🔄", "⇄", "✂", "🗑", "🎨", "ℹ", "⚠", "✔",
];

/// The icon of `c` (the first of [`ICONS`] if it has none or another one).
pub fn icon(c: &CustomCommand) -> &'static str {
    ICONS
        .iter()
        .find(|i| **i == c.icon)
        .copied()
        .unwrap_or(ICONS[0])
}

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

/// The commands for `target` listed at `place` of the item menu (or of the
/// background's, which lacks some places), by their index in the settings.
fn at_place(target: Target<'_>, place: MenuPlace) -> Vec<MenuGroup<(usize, CustomCommand)>> {
    let background = matches!(target, Target::Background);
    let shown: Vec<(usize, CustomCommand)> = CURRENT
        .read()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, c)| c.applies(target))
        .map(|(i, c)| {
            let mut c = c.clone();
            if background {
                c.menu_place = c.menu_place.on_background();
            }
            (i, c)
        })
        .collect();
    let list = shown.iter().map(|(i, c)| (c, (*i, c.clone())));
    menu_groups(list, place, tr("Commands"))
}

fn entries(
    ui: &mut Ui,
    groups: &[MenuGroup<(usize, CustomCommand)>],
    run: impl Fn(usize) -> CommandAction,
    actions: &mut Vec<Action>,
) {
    let entry = |ui: &mut Ui, (i, c): &(usize, CustomCommand), actions: &mut Vec<Action>| {
        let shortcut = crate::app::shortcut_of(c)
            .map(|k| crate::shortcuts::display(&k))
            .unwrap_or_default();
        let button = egui::Button::new(format!("{}  {}", icon(c), c.name)).shortcut_text(shortcut);
        if ui.add(button).clicked() {
            actions.push(Action::Command(run(*i)));
        }
    };
    for g in groups {
        match g {
            MenuGroup::Entry(c) => entry(ui, c, actions),
            MenuGroup::Submenu(name, list) => {
                ui.menu_button(format!("⚡ {name}"), |ui| {
                    for c in list {
                        entry(ui, c, actions);
                    }
                });
            }
        }
    }
}

/// The commands at `place`, set apart where they aren't under an entry.
fn with_separators(ui: &mut Ui, place: MenuPlace, add: impl FnOnce(&mut Ui)) {
    if matches!(place, MenuPlace::Tools | MenuPlace::Bottom) {
        ui.separator();
    }
    add(ui);
    if place == MenuPlace::Top {
        ui.separator();
    }
}

/// The custom commands for the selected items that are listed at `place` of
/// the right-click menu.
pub fn item_menu(ui: &mut Ui, selection: &[PathBuf], place: MenuPlace, actions: &mut Vec<Action>) {
    if CURRENT.read().unwrap().is_empty() {
        return;
    }
    let items = with_kinds(selection);
    let groups = at_place(Target::Items(&items), place);
    if groups.is_empty() {
        return;
    }
    let paths = selection.to_vec();
    let run = |index| CommandAction::Run {
        index,
        items: paths.clone(),
        folder: None,
    };
    with_separators(ui, place, |ui| entries(ui, &groups, run, actions));
}

/// The custom commands for a folder's background that are listed at
/// `place` of its right-click menu.
pub fn background_menu(ui: &mut Ui, folder: &Path, place: MenuPlace, actions: &mut Vec<Action>) {
    if CURRENT.read().unwrap().is_empty() {
        return;
    }
    let groups = at_place(Target::Background, place);
    if groups.is_empty() {
        return;
    }
    let run = |index| CommandAction::Run {
        index,
        items: Vec::new(),
        folder: Some(folder.to_path_buf()),
    };
    with_separators(ui, place, |ui| entries(ui, &groups, run, actions));
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
