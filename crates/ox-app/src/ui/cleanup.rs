//! Find duplicates and empty folders: the page with the scan's progress and
//! what it found, ticks and helpers to tick, and the question before the
//! ticked items go to the Recycle Bin.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::{Align, Color32, Id, Layout, RichText, Sense, Ui};
use ox_core::cleanup::duplicates::{Keep, Report as DupReport};
use ox_core::cleanup::{CleanupMode, Outcome, Progress};
use ox_core::i18n::{count, t as tr};
use ox_core::{format, trf};

use crate::app::{Action, CleanupAction, CleanupView, ConfirmCleanup, OxApp};
use crate::tabs::{Location, TabId};

const WARN: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
const ROW_HEIGHT: f32 = 24.0;

/// The "Clean up" submenu of a folder's context menu.
pub fn menu(ui: &mut Ui, folder: &Path, actions: &mut Vec<Action>) {
    ui.menu_button(tr("✨ Clean up"), |ui| {
        for (mode, label) in [
            (CleanupMode::Duplicates, tr("📋 Find duplicates")),
            (CleanupMode::EmptyFolders, tr("📂 Find empty folders")),
        ] {
            if ui.button(label).clicked() {
                let root = folder.to_path_buf();
                actions.push(Action::OpenInNewTab(Location::Cleanup { root, mode }, true));
            }
        }
    });
}

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let tab = app.tabs.active().id;
    let date_fmt = app.settings.browsing.date_format.clone();
    let mut actions = Vec::new();
    let mut byte_compare = app.cleanups.byte_compare;
    let Some(view) = app.cleanups.get_mut(tab) else {
        // The scan starts with the next frame.
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(20, 14))
        .show(ui, |ui| {
            top(ui, view, &mut actions);
            if view.mode == CleanupMode::Duplicates {
                byte_compare_option(ui, &mut byte_compare, &mut actions);
            }
            ui.add_space(6.0);
            ui.separator();
            match &view.outcome {
                Some(Outcome::Duplicates(_)) => duplicates(ui, view, tab, &date_fmt, &mut actions),
                Some(Outcome::EmptyFolders(_)) => empty_folders(ui, view, tab, &mut actions),
                None => {}
            }
        });
    if view.confirm.is_some() {
        confirm(ui, view, tab, &mut actions);
    }
    app.cleanups.byte_compare = byte_compare;
    app.actions.extend(actions);
}

/// "Compare contents byte by byte"; changing it scans again.
fn byte_compare_option(ui: &mut Ui, on: &mut bool, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    let changed = ui
        .checkbox(on, tr("Compare contents byte by byte"))
        .on_hover_text(tr(
            "After the hashes match, read the files again and compare every byte. Slower; \
             the hash alone is practically certain.",
        ))
        .changed();
    if changed {
        actions.push(Action::Refresh);
    }
}

/// Title, progress or summary, and the buttons to scan again or stop.
fn top(ui: &mut Ui, view: &CleanupView, actions: &mut Vec<Action>) {
    let icon = Location::cleanup_icon(view.mode);
    let title = match view.mode {
        CleanupMode::Duplicates => tr("Duplicates"),
        CleanupMode::EmptyFolders => tr("Empty folders"),
    };
    ui.horizontal(|ui| {
        ui.heading(RichText::new(format!("{icon} {title}")).strong());
        ui.label(RichText::new(view.root.display().to_string()).weak());
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if let Some(scan) = &view.scan {
            ui.spinner();
            ui.label(progress_text(view.mode, &scan.progress));
            if ui.button(tr("Cancel")).clicked() {
                scan.cancel();
            }
            return;
        }
        let Some(outcome) = &view.outcome else { return };
        if outcome.cancelled() {
            ui.label(tr("The scan was stopped."));
        } else {
            match outcome {
                Outcome::Duplicates(r) if r.groups.is_empty() => {
                    ui.label(tr("No duplicates found."));
                }
                Outcome::Duplicates(r) => {
                    ui.label(
                        RichText::new(trf!("Duplicate groups: {n}", n = r.groups.len())).strong(),
                    );
                    ui.label("·");
                    let wasted = format::size(r.wasted());
                    ui.label(RichText::new(trf!("Wasted space: {size}", size = wasted)).strong());
                }
                Outcome::EmptyFolders(r) if r.folders.is_empty() => {
                    ui.label(tr("No empty folders found."));
                }
                Outcome::EmptyFolders(r) => {
                    let n = r.folders.len();
                    ui.label(RichText::new(trf!("Empty folders: {n}", n = n)).strong());
                }
            }
            let took = format::duration(view.elapsed);
            ui.label(RichText::new(trf!("(took {time})", time = took)).weak());
        }
        if ui.button(tr("🔄 Scan again")).clicked() {
            actions.push(Action::Refresh);
        }
        let unreadable = outcome.unreadable();
        if unreadable > 0 {
            ui.label(
                RichText::new(trf!(
                    "⚠ {items} couldn't be read",
                    items = count(unreadable, "item")
                ))
                .weak(),
            );
        }
        if let Outcome::Duplicates(r) = outcome {
            if r.online_only > 0 {
                ui.label(
                    RichText::new(trf!(
                    "☁ {files} only in the cloud were left out (reading them would download them)",
                    files = count(r.online_only, "file")
                ))
                    .weak(),
                );
            }
        }
    });
}

fn progress_text(mode: CleanupMode, p: &Progress) -> String {
    let files = Progress::get(&p.files);
    let folders = Progress::get(&p.folders);
    let candidates = Progress::get(&p.candidates);
    match mode {
        CleanupMode::Duplicates if candidates > 0 => trf!(
            "Comparing {files} of the same size… {bytes} read",
            files = count(candidates, "file"),
            bytes = format::size(Progress::get(&p.hashed))
        ),
        CleanupMode::Duplicates => trf!(
            "Looking through {files} in {folders}…",
            files = count(files, "file"),
            folders = count(folders, "folder")
        ),
        CleanupMode::EmptyFolders => trf!(
            "Looking through {folders}…",
            folders = count(folders, "folder")
        ),
    }
}

/// A line of the duplicates list.
enum Row {
    Group(usize),
    File(usize, usize),
}

fn duplicates(
    ui: &mut Ui,
    view: &mut CleanupView,
    tab: TabId,
    date_fmt: &str,
    actions: &mut Vec<Action>,
) {
    let Some(Outcome::Duplicates(report)) = &view.outcome else {
        return;
    };
    if report.groups.is_empty() {
        return;
    }
    let (n, bytes) = report.selected(&view.selected);
    ui.horizontal(|ui| {
        if ui.button(tr("Select all but the newest")).clicked() {
            view.selected = report.all_but(Keep::Newest).into_iter().collect();
        }
        if ui.button(tr("Select all but the oldest")).clicked() {
            view.selected = report.all_but(Keep::Oldest).into_iter().collect();
        }
        if ui.button(tr("Select none")).clicked() {
            view.selected.clear();
        }
        let what = format!("{}, {}", count(n as u64, "file"), format::size(bytes));
        delete_button(ui, &mut view.confirm, n, &what);
    });
    ui.add_space(4.0);
    let mut rows = Vec::new();
    for (g, group) in report.groups.iter().enumerate() {
        rows.push(Row::Group(g));
        rows.extend((0..group.files.len()).map(|f| Row::File(g, f)));
    }
    egui::ScrollArea::vertical()
        .id_salt(("cleanup", tab))
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
            for row in &rows[range] {
                match *row {
                    Row::Group(g) => group_row(ui, report, g, &view.selected),
                    Row::File(g, f) => {
                        let file = &report.groups[g].files[f];
                        let date = file
                            .modified
                            .map(|t| format::time(t, date_fmt))
                            .unwrap_or_default();
                        let row = (file.path.as_path(), false, date.as_str());
                        item_row(ui, &view.root, &mut view.selected, row, actions);
                    }
                }
            }
        });
}

/// A group's heading: size × copies, and a warning when every copy is ticked.
fn group_row(ui: &mut Ui, report: &DupReport, g: usize, selected: &HashSet<PathBuf>) {
    let group = &report.groups[g];
    ui.allocate_ui(egui::vec2(ui.available_width(), ROW_HEIGHT), |ui| {
        ui.horizontal_centered(|ui| {
            let n = group.files.len();
            ui.label(RichText::new(format!("{} × {n}", format::size(group.size))).strong());
            let wasted = format::size(group.wasted());
            ui.label(RichText::new(trf!("{size} wasted", size = wasted)).weak());
            if group.files.iter().all(|f| selected.contains(&f.path)) {
                ui.label(RichText::new(tr("⚠ Every copy is selected")).color(WARN));
            }
        });
    });
}

/// A file or folder with its tick, the date (files) and *Show in folder*.
/// `row`: the path, whether it's a folder, and the date.
fn item_row(
    ui: &mut Ui,
    root: &Path,
    selected: &mut HashSet<PathBuf>,
    (path, is_dir, date): (&Path, bool, &str),
    actions: &mut Vec<Action>,
) {
    let shown = path.strip_prefix(root).unwrap_or(path);
    ui.allocate_ui(egui::vec2(ui.available_width(), ROW_HEIGHT), |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button("📁")
                .on_hover_text(tr("Open file location"))
                .clicked()
            {
                actions.push(Action::Cleanup(CleanupAction::ShowInFolder(
                    path.to_path_buf(),
                )));
            }
            if !date.is_empty() {
                ui.label(RichText::new(date).weak());
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(if is_dir { 0.0 } else { 14.0 });
                let mut on = selected.contains(path);
                let tick = ui.checkbox(&mut on, "");
                let name = ui
                    .add(
                        egui::Label::new(shown.display().to_string())
                            .truncate()
                            .sense(Sense::click()),
                    )
                    .on_hover_text(path.display().to_string());
                if name.clicked() {
                    on = !on;
                }
                if tick.changed() || name.clicked() {
                    if on {
                        selected.insert(path.to_path_buf());
                    } else {
                        selected.remove(path);
                    }
                }
                let open = || {
                    if is_dir {
                        Action::OpenInNewTab(Location::Folder(path.to_path_buf()), true)
                    } else {
                        Action::Open(path.to_path_buf())
                    }
                };
                if name.double_clicked() {
                    actions.push(open());
                }
                name.context_menu(|ui| {
                    if ui.button(tr("📂 Open")).clicked() {
                        actions.push(open());
                    }
                    if ui.button(tr("📁 Open file location")).clicked() {
                        actions.push(Action::Cleanup(CleanupAction::ShowInFolder(
                            path.to_path_buf(),
                        )));
                    }
                });
            });
        });
    });
}

fn empty_folders(ui: &mut Ui, view: &mut CleanupView, tab: TabId, actions: &mut Vec<Action>) {
    let Some(Outcome::EmptyFolders(report)) = &view.outcome else {
        return;
    };
    if report.folders.is_empty() {
        return;
    }
    let folders = &report.folders;
    let n = folders
        .iter()
        .filter(|p| view.selected.contains(*p))
        .count();
    ui.horizontal(|ui| {
        if ui.button(tr("Select all")).clicked() {
            view.selected = folders.iter().cloned().collect();
        }
        if ui.button(tr("Select none")).clicked() {
            view.selected.clear();
        }
        delete_button(ui, &mut view.confirm, n, &count(n as u64, "folder"));
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt(("cleanup", tab))
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, folders.len(), |ui, range| {
            for path in &folders[range] {
                item_row(
                    ui,
                    &view.root,
                    &mut view.selected,
                    (path, true, ""),
                    actions,
                );
            }
        });
}

/// *Delete selected*, at the right; it asks first.
fn delete_button(ui: &mut Ui, confirm: &mut Option<ConfirmCleanup>, n: usize, what: &str) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let label = trf!("🗑 Delete selected ({what})", what = what);
        if ui.add_enabled(n > 0, egui::Button::new(label)).clicked() {
            *confirm = Some(ConfirmCleanup::default());
        }
    });
}

/// The question before the ticked items go to the Recycle Bin. When every
/// copy of some file is ticked, it says so and has to be confirmed apart.
fn confirm(ui: &mut Ui, view: &mut CleanupView, tab: TabId, actions: &mut Vec<Action>) {
    let selection = &view.selected;
    let (what, all_copies) = match &view.outcome {
        Some(Outcome::Duplicates(r)) => {
            let (n, bytes) = r.selected(selection);
            let what = format!("{} ({})", count(n as u64, "file"), format::size(bytes));
            (what, r.fully_selected(selection))
        }
        _ => (count(view.selected_paths().len() as u64, "folder"), 0),
    };
    let Some(state) = &mut view.confirm else {
        return;
    };
    let mut keep = true;
    let modal = egui::Modal::new(Id::unique("cleanup_confirm")).show(ui.ctx(), |ui| {
        ui.set_width(420.0);
        ui.heading(tr("Move to Recycle Bin?"));
        ui.add_space(6.0);
        ui.label(trf!(
            "Do you want to move {what} to the Recycle Bin?",
            what = what
        ));
        if all_copies > 0 {
            ui.add_space(6.0);
            ui.label(
                RichText::new(trf!(
                    "⚠ For {files} every copy is selected: none of them would be left.",
                    files = count(all_copies as u64, "file")
                ))
                .color(WARN),
            );
            ui.checkbox(
                &mut state.all_copies_ok,
                tr("Delete every copy of these anyway"),
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let allowed = all_copies == 0 || state.all_copies_ok;
            let button = egui::Button::new(RichText::new(tr("Move to Recycle Bin")).strong());
            if ui.add_enabled(allowed, button).clicked() {
                actions.push(Action::Cleanup(CleanupAction::DeleteConfirmed(tab)));
                keep = false;
            }
            if ui.button(tr("Cancel")).clicked() {
                keep = false;
            }
        });
    });
    if !keep || modal.should_close() {
        view.confirm = None;
    }
}
