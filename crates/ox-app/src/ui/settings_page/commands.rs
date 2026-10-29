//! The Custom commands page of the settings: the list, the form of the one
//! being edited with a preview of its command line, and examples.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use egui::{vec2, Color32, RichText, Ui};
use ox_core::commands::line::PLACEHOLDERS;
use ox_core::commands::run::{self, Launch};
use ox_core::commands::{examples, CustomCommand, Invocation, Many, Target, TypeFilter, Window};
use ox_core::i18n::t as tr;
use ox_core::trf;

use crate::app::{shortcut_of, OxApp};
use crate::shortcuts::{self, Binding};
use crate::ui::commands::{icon, ICONS};

const WARN: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

pub(super) fn commands(ui: &mut Ui, app: &mut OxApp) {
    ui.label(
        RichText::new(tr(
            "Your own commands in the right-click menu and the command palette. In the command line, the placeholders become the selected items; paths are put in quotes where needed. A line with |, &, > or a command of cmd (echo, start, …) runs through cmd /C, other programs are started directly.",
        ))
        .weak(),
    );
    ui.add_space(8.0);
    let preview = preview_items(app);
    let mut remove = None;
    let mut swap = None;
    let mut duplicate = None;
    let count = app.settings.commands.0.len();
    for i in 0..count {
        let editing = app.commands.editing == Some(i);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let c = &app.settings.commands.0[i];
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}  {}", icon(c), c.name)).strong());
                ui.label(RichText::new(summary(c)).small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("🗑").on_hover_text(tr("Remove")).clicked() {
                        remove = Some(i);
                    }
                    if ui
                        .small_button("📑")
                        .on_hover_text(tr("Duplicate"))
                        .clicked()
                    {
                        duplicate = Some(i);
                    }
                    if ui
                        .add_enabled(i + 1 < count, egui::Button::new("⬇").small())
                        .on_hover_text(tr("Move down"))
                        .clicked()
                    {
                        swap = Some((i, i + 1));
                    }
                    if ui
                        .add_enabled(i > 0, egui::Button::new("⬆").small())
                        .on_hover_text(tr("Move up"))
                        .clicked()
                    {
                        swap = Some((i - 1, i));
                    }
                    let edit = egui::Button::new("✏").small().selected(editing);
                    if ui.add(edit).on_hover_text(tr("Edit")).clicked() {
                        app.commands.editing = (!editing).then_some(i);
                        app.commands.recording = false;
                    }
                });
            });
            if editing {
                ui.separator();
                form(ui, app, i, &preview);
            }
        });
        ui.add_space(4.0);
    }
    let list = &mut app.settings.commands.0;
    if let Some(i) = remove {
        list.remove(i);
        app.commands.editing = None;
        app.commands.recording = false;
    }
    if let Some(i) = duplicate {
        let mut copy = list[i].clone();
        copy.name = trf!("{name} (copy)", name = copy.name);
        copy.shortcut.clear();
        list.insert(i + 1, copy);
        app.commands.editing = Some(i + 1);
    }
    if let Some((a, b)) = swap {
        list.swap(a, b);
        app.commands.editing = match app.commands.editing {
            Some(e) if e == a => Some(b),
            Some(e) if e == b => Some(a),
            e => e,
        };
    }
    ui.horizontal(|ui| {
        if ui.button(tr("➕ Add command")).clicked() {
            list.push(CustomCommand {
                name: tr("New command").to_owned(),
                ..Default::default()
            });
            app.commands.editing = Some(list.len() - 1);
        }
    });
    ui.add_space(8.0);
    ui.label(RichText::new(tr("Examples to add")).strong());
    ui.horizontal_wrapped(|ui| {
        for example in examples() {
            let added = list.iter().any(|c| c.command == example.command);
            let button = egui::Button::new(format!("{}  {}", icon(&example), example.name));
            if ui
                .add_enabled(!added, button)
                .on_hover_text(RichText::new(&example.command).monospace())
                .clicked()
            {
                list.push(example);
                app.commands.editing = Some(list.len() - 1);
            }
        }
    });
}

/// "Files, folders · Ctrl+Alt+H" under the name.
fn summary(c: &CustomCommand) -> String {
    let mut what: Vec<&str> = Vec::new();
    if c.files {
        what.push(tr("files"));
    }
    if c.folders {
        what.push(tr("folders"));
    }
    if c.background {
        what.push(tr("folder background"));
    }
    let mut s = what.join(", ");
    if c.files && !c.types.trim().is_empty() {
        s.push_str(&format!(" ({})", c.types.trim()));
    }
    if let Some(k) = shortcut_of(c) {
        s.push_str(" · ");
        s.push_str(&shortcuts::display(&k));
    }
    s
}

/// What the preview of a command line is shown for: the selection (or
/// folder) of a folder tab, else made-up items.
struct PreviewItems {
    items: Vec<(PathBuf, bool)>,
    folder: Option<PathBuf>,
}

fn preview_items(app: &OxApp) -> PreviewItems {
    let tabs = &app.tabs.tabs;
    let with_folder = || tabs.iter().find(|t| t.location().folder().is_some());
    let tab = tabs
        .iter()
        .find(|t| t.location().folder().is_some() && !t.view.selection.is_empty())
        .or_else(with_folder);
    PreviewItems {
        items: tab
            .map(|t| {
                t.view
                    .selected_entries()
                    .into_iter()
                    .map(|e| (e.path.clone(), e.is_dir))
                    .collect()
            })
            .unwrap_or_default(),
        folder: tab.and_then(|t| t.location().folder().map(Path::to_path_buf)),
    }
}

/// The runs `c` would have for the preview, and whether they are for
/// made-up items.
fn preview_runs(c: &CustomCommand, p: &PreviewItems) -> (Vec<Invocation>, bool) {
    if !p.items.is_empty() && c.applies(Target::Items(&p.items)) {
        let items: Vec<PathBuf> = p.items.iter().map(|(p, _)| p.clone()).collect();
        return (c.invocations(&items, p.folder.as_deref()), false);
    }
    if p.items.is_empty() && p.folder.is_some() && c.background {
        return (c.invocations(&[], p.folder.as_deref()), false);
    }
    let folder = p
        .folder
        .clone()
        .unwrap_or_else(|| ox_core::places::home_dir().join("Documents"));
    let types = TypeFilter::parse(&c.types);
    let ext = types.extensions.first().map_or("txt", String::as_str);
    let n = if c.many == Many::One { 1 } else { 2 };
    let items: Vec<PathBuf> = if c.files || !c.folders {
        (1..=n)
            .map(|i| folder.join(format!("{} {i}.{ext}", tr("Example"))))
            .collect()
    } else {
        (1..=n)
            .map(|i| folder.join(format!("{} {i}", tr("Example"))))
            .collect()
    };
    (c.invocations(&items, Some(&folder)), true)
}

/// A program (empty for the shell) and working folder, and how a line with
/// them starts.
type LaunchCache = Option<(String, PathBuf, Result<Launch, String>)>;

thread_local! {
    /// The last line whose start was looked up.
    static LAUNCH: RefCell<LaunchCache> = const { RefCell::new(None) };
}

/// How `inv` would start (remembered, so `PATH` isn't searched every frame).
fn launch_of(inv: &Invocation) -> Result<Launch, String> {
    let (program, _) = ox_core::commands::line::split_program(&inv.line);
    let key = if inv.shell_syntax {
        String::new()
    } else {
        program
    };
    LAUNCH.with(|l| {
        let mut l = l.borrow_mut();
        match &*l {
            Some((k, d, r)) if *k == key && *d == inv.dir => r.clone(),
            _ => {
                let r = run::launch(inv);
                *l = Some((key, inv.dir.clone(), r.clone()));
                r
            }
        }
    })
}

fn form(ui: &mut Ui, app: &mut OxApp, i: usize, preview: &PreviewItems) {
    record_shortcut(ui, app, i);
    let keymap = app.keymap.clone();
    // The names of all commands, for a clash with another one.
    let others: Vec<String> = app
        .settings
        .commands
        .0
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let recording = app.commands.recording;
    let mut toggle_recording = false;
    let c = &mut app.settings.commands.0[i];
    egui::Grid::new(("command_form", i))
        .num_columns(2)
        .spacing(vec2(12.0, 8.0))
        .show(ui, |ui| {
            ui.label(tr("Name"));
            ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(260.0));
            ui.end_row();

            ui.label(tr("Icon"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let current = icon(c);
                for sym in ICONS {
                    if ui.selectable_label(current == *sym, *sym).clicked() {
                        c.icon = (*sym).to_owned();
                    }
                }
            });
            ui.end_row();

            ui.label(tr("Command line"));
            ui.vertical(|ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut c.command)
                        .code_editor()
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("code {paths}"),
                );
                ui.horizontal_wrapped(|ui| {
                    for p in PLACEHOLDERS {
                        if ui
                            .small_button(RichText::new(*p).monospace())
                            .on_hover_text(tr(placeholder_help(p)))
                            .clicked()
                        {
                            if !c.command.is_empty() && !c.command.ends_with(' ') {
                                c.command.push(' ');
                            }
                            c.command.push_str(p);
                        }
                    }
                });
                ui.label(
                    RichText::new(tr(
                        "Also in the environment: OX_PATH, OX_PATHS (one per line) and OX_FOLDER, e.g. $env:OX_PATH in PowerShell.",
                    ))
                    .small()
                    .weak(),
                );
            });
            ui.end_row();

            ui.label(tr("Working folder"));
            ui.add(
                egui::TextEdit::singleline(&mut c.working_dir)
                    .font(egui::TextStyle::Monospace)
                    .hint_text(tr("the current folder"))
                    .desired_width(260.0),
            );
            ui.end_row();

            ui.label(tr("For"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut c.files, tr("Files"));
                ui.checkbox(&mut c.folders, tr("Folders"));
                ui.checkbox(&mut c.background, tr("Folder background"));
            });
            ui.end_row();

            ui.label(tr("File types"));
            ui.vertical(|ui| {
                ui.add_enabled(
                    c.files,
                    egui::TextEdit::singleline(&mut c.types)
                        .hint_text(tr("all files"))
                        .desired_width(260.0),
                );
                let kinds = ox_core::commands::KINDS
                    .iter()
                    .map(|(w, _)| *w)
                    .collect::<Vec<_>>()
                    .join(", ");
                ui.label(
                    RichText::new(trf!(
                        "Extensions (jpg, png) or kinds: {kinds}",
                        kinds = kinds
                    ))
                    .small()
                    .weak(),
                );
            });
            ui.end_row();

            ui.label(tr("Selection"));
            egui::ComboBox::from_id_salt(("command_many", i))
                .selected_text(tr(c.many.label()))
                .width(260.0)
                .show_ui(ui, |ui| {
                    for m in Many::ALL {
                        ui.selectable_value(&mut c.many, m, tr(m.label()));
                    }
                });
            ui.end_row();

            ui.label(tr("Window"));
            ui.vertical(|ui| {
                for w in Window::ALL {
                    ui.radio_value(&mut c.window, w, tr(w.label()));
                }
                if c.window == Window::Console {
                    ui.label(
                        RichText::new(tr(
                            "The console closes when the command ends; add & pause to keep it open.",
                        ))
                        .small()
                        .weak(),
                    );
                }
            });
            ui.end_row();

            ui.label(tr("Ask first"));
            ui.checkbox(&mut c.confirm, tr("Ask before running it"));
            ui.end_row();

            ui.label(tr("Shortcut"));
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let text = if recording {
                        tr("Press keys…").to_owned()
                    } else {
                        shortcut_of(c).map_or("—".to_owned(), |k| shortcuts::display(&k))
                    };
                    let button = egui::Button::new(RichText::new(text).monospace())
                        .selected(recording)
                        .min_size(vec2(170.0, 0.0));
                    if ui.add(button).clicked() {
                        toggle_recording = true;
                    }
                    if !c.shortcut.is_empty()
                        && ui
                            .small_button("✖")
                            .on_hover_text(tr("No shortcut"))
                            .clicked()
                    {
                        c.shortcut.clear();
                    }
                });
                // The same clash the Shortcuts page shows.
                let clash = shortcut_of(c).and_then(|k| keymap.conflict(Binding::Custom(i), &k));
                match clash {
                    Some(Binding::Builtin(b)) => {
                        ui.colored_label(
                            WARN,
                            trf!(
                                "⚠ Already the shortcut of “{other}”, which goes first. Change it in Settings → Shortcuts or pick other keys.",
                                other = tr(shortcuts::command_info(b).label)
                            ),
                        );
                    }
                    Some(Binding::Custom(j)) => {
                        let name = others.get(j).cloned().unwrap_or_default();
                        ui.colored_label(
                            WARN,
                            trf!("⚠ Also the shortcut of “{other}”", other = name),
                        );
                    }
                    None => {}
                }
            });
            ui.end_row();
        });
    if toggle_recording {
        app.commands.recording = !recording;
    }
    let c = &app.settings.commands.0[i];
    ui.add_space(6.0);
    let (runs, made_up) = preview_runs(c, preview);
    ui.label(RichText::new(tr("Preview")).strong());
    ui.label(
        RichText::new(if made_up {
            tr("For example items (select items in a folder tab to see them here):")
        } else {
            tr("For the selection of the folder tab:")
        })
        .small()
        .weak(),
    );
    if c.command.trim().is_empty() {
        ui.label(RichText::new(tr("No command line yet")).weak());
        return;
    }
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .inner_margin(egui::Margin::same(6))
        .corner_radius(4.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for r in runs.iter().take(5) {
                ui.add(egui::Label::new(RichText::new(&r.line).monospace()).wrap());
            }
            if runs.len() > 5 {
                ui.label(trf!("… and {n} more runs", n = runs.len() - 5));
            }
        });
    if let Some(first) = runs.first() {
        let how = match launch_of(first) {
            Ok(Launch::Shell) => RichText::new(tr("Runs through cmd /C.")).weak(),
            Ok(Launch::Direct(exe)) => {
                RichText::new(trf!("Starts {exe} directly.", exe = exe.display())).weak()
            }
            Err(p) => RichText::new(trf!("Program not found: {p}", p = p)).color(WARN),
        };
        ui.label(how.small());
        ui.label(
            RichText::new(trf!("In {dir}", dir = first.dir.display()))
                .small()
                .weak(),
        );
    }
}

/// While recording, the next key press (with its modifiers) becomes the
/// shortcut of command `i`; Esc cancels.
fn record_shortcut(ui: &mut Ui, app: &mut OxApp, i: usize) {
    if !app.commands.recording {
        return;
    }
    let Some(keys) = ui.input(shortcuts::recorded) else {
        return;
    };
    app.commands.recording = false;
    if let Some(k) = keys {
        app.settings.commands.0[i].shortcut = shortcuts::to_text(&k);
    }
}

fn placeholder_help(p: &str) -> &'static str {
    match p {
        "{path}" => "The first selected item, its full path",
        "{paths}" | "{selection}" => "All selected items, each in quotes if needed",
        "{name}" => "The name of the first item",
        "{stem}" => "The name without its extension",
        "{ext}" => "The extension, without the dot",
        "{parent}" => "The folder the first item is in",
        "{folder}" => "The current folder",
        _ => "How many items",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_translated() {
        let labels = Many::ALL
            .map(Many::label)
            .into_iter()
            .chain(Window::ALL.map(Window::label))
            .chain(PLACEHOLDERS.iter().map(|p| placeholder_help(p)));
        for l in labels {
            assert!(ox_core::i18n::german(l).is_some(), "no German for {l:?}");
        }
    }
}
