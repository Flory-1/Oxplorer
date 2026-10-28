//! Modal dialogs: rename, delete confirmation, properties, rename tab group.

use egui::{Color32, Context, Id, Key, RichText};
use ox_core::entry::display_name;
use ox_core::i18n::t as tr;
use ox_core::rename::{self, Case};
use ox_core::{format, fsops};

use crate::app::{Action, Dialog, OxApp};

pub fn show(app: &mut OxApp, ctx: &Context) {
    let Some(mut dialog) = app.dialog.take() else {
        return;
    };
    let mut keep = true;
    let mut actions = Vec::new();
    let date_fmt = app.settings.browsing.date_format.clone();

    let width = if matches!(dialog, Dialog::BulkRename { .. }) {
        680.0
    } else {
        420.0
    };
    let modal = egui::Modal::new(Id::unique("dialog")).show(ctx, |ui| {
        ui.set_width(width);
        match &mut dialog {
            Dialog::BulkRename { items, rule, focus } => {
                ui.heading(ox_core::trf!("Rename {v1} items", v1 = items.len()));
                ui.add_space(6.0);
                egui::Grid::new("bulk_rename")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(tr("New name"));
                        ui.vertical(|ui| {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut rule.pattern)
                                    .desired_width(460.0)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if *focus {
                                *focus = false;
                                resp.request_focus();
                            }
                            ui.horizontal(|ui| {
                                for (token, tip) in [
                                    ("{name}", tr("Original name")),
                                    ("{n:3}", tr("Counter: 001, 002, …")),
                                    ("{date}", tr("Date modified (YYYY-MM-DD)")),
                                    ("{ext}", tr("Extension")),
                                    ("{parent}", tr("Folder name")),
                                ] {
                                    if ui.small_button(token).on_hover_text(tip).clicked() {
                                        rule.pattern.push_str(token);
                                    }
                                }
                            });
                        });
                        ui.end_row();
                        ui.label(tr("Find"));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut rule.find).desired_width(170.0));
                            ui.label(tr("Replace with"));
                            ui.add(
                                egui::TextEdit::singleline(&mut rule.replace).desired_width(170.0),
                            );
                            ui.checkbox(&mut rule.regex, tr("Regex"));
                        });
                        ui.end_row();
                        ui.label(tr("Case"));
                        ui.horizontal(|ui| {
                            for (case, label) in [
                                (Case::Keep, tr("As is")),
                                (Case::Lower, tr("lower case")),
                                (Case::Upper, tr("UPPER CASE")),
                                (Case::Title, tr("Title Case")),
                            ] {
                                ui.selectable_value(&mut rule.case, case, label);
                            }
                        });
                        ui.end_row();
                        ui.label(tr("Counter"));
                        ui.horizontal(|ui| {
                            ui.label(tr("starts at"));
                            ui.add(egui::DragValue::new(&mut rule.start));
                            ui.label(tr("step"));
                            ui.add(egui::DragValue::new(&mut rule.step).range(1..=1000));
                            ui.separator();
                            ui.checkbox(&mut rule.keep_extension, tr("Keep extensions"));
                        });
                        ui.end_row();
                    });

                let plan = rename::plan(items, rule);
                let changes = plan.iter().filter(|p| p.changes()).count();
                let errors = plan.iter().filter(|p| p.error.is_some()).count();
                ui.add_space(8.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            egui::Grid::new("bulk_preview")
                                .num_columns(3)
                                .striped(true)
                                .min_col_width(40.0)
                                .show(ui, |ui| {
                                    ui.label(RichText::new(tr("Current name")).strong());
                                    ui.label("");
                                    ui.label(RichText::new(tr("New name")).strong());
                                    ui.end_row();
                                    for p in plan.iter().take(500) {
                                        ui.label(display_name(&p.from));
                                        ui.label("→");
                                        let new = display_name(&p.to);
                                        match &p.error {
                                            Some(e) => {
                                                ui.colored_label(
                                                    Color32::from_rgb(0xe5, 0x48, 0x4d),
                                                    format!("{new}  ⚠"),
                                                )
                                                .on_hover_text(e);
                                            }
                                            None if p.changes() => {
                                                ui.label(RichText::new(new).strong());
                                            }
                                            None => {
                                                ui.label(RichText::new(new).weak());
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                            if plan.len() > 500 {
                                ui.label(ox_core::trf!("… and {v1} more", v1 = plan.len() - 500));
                            }
                        });
                });
                ui.add_space(6.0);
                if errors > 0 {
                    ui.colored_label(
                        Color32::from_rgb(0xe5, 0x48, 0x4d),
                        ox_core::trf!(
                            "{v1} can't be renamed — hover the ⚠ for details.",
                            v1 = ox_core::i18n::count(errors as u64, "item")
                        ),
                    );
                } else {
                    ui.label(
                        RichText::new(ox_core::trf!(
                            "{v1} will be renamed.",
                            v1 = ox_core::i18n::count(changes as u64, "item")
                        ))
                        .weak(),
                    );
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ok = errors == 0 && changes > 0;
                    if ui
                        .add_enabled(ok, egui::Button::new(RichText::new(tr("Rename")).strong()))
                        .clicked()
                    {
                        match rename::apply(&plan) {
                            Ok(n) => {
                                actions.push(Action::Toast(ox_core::trf!(
                                    "Renamed {v1}",
                                    v1 = ox_core::i18n::count(n as u64, "item")
                                )));
                                actions.push(Action::PathsMoved(
                                    plan.iter()
                                        .filter(|p| p.changes())
                                        .map(|p| (p.from.clone(), p.to.clone()))
                                        .collect(),
                                ));
                                app.tabs.mark_stale(None);
                                keep = false;
                            }
                            Err(e) => actions
                                .push(Action::Toast(ox_core::trf!("Rename failed: {e}", e = e))),
                        }
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::Rename {
                path,
                text,
                error,
                focus,
            } => {
                ui.heading(tr("Rename"));
                ui.add_space(6.0);
                let resp = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                if *focus {
                    *focus = false;
                    resp.request_focus();
                    // Select the name without the extension, like Explorer.
                    let stem_len = if path.is_dir() {
                        text.chars().count()
                    } else {
                        fsops::split_name(text).0.chars().count()
                    };
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(stem_len),
                            )));
                        state.store(ui.ctx(), resp.id);
                    }
                }
                if let Some(e) = error {
                    ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), e.as_str());
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ok = ui.button(tr("Rename")).clicked() || enter;
                    if ui.button(tr("Cancel")).clicked() {
                        keep = false;
                    }
                    if ok {
                        if *text == display_name(path) {
                            keep = false;
                        } else {
                            match fsops::rename(path, text.trim()) {
                                Ok(new_path) => {
                                    keep = false;
                                    actions.push(Action::PathsMoved(vec![(
                                        path.clone(),
                                        new_path.clone(),
                                    )]));
                                    let dir = new_path.parent().map(|p| p.to_path_buf());
                                    let tab = app.tabs.active_mut();
                                    tab.view.select_after_load = Some(new_path);
                                    app.tabs.mark_stale(dir.as_deref());
                                }
                                Err(e) => *error = Some(e.to_string()),
                            }
                        }
                    }
                });
            }
            Dialog::ConfirmDelete { paths, permanent } => {
                let to_trash = !*permanent && app.settings.browsing.delete_to_trash;
                ui.heading(if to_trash {
                    tr("Move to Recycle Bin?")
                } else {
                    tr("Delete permanently?")
                });
                ui.add_space(6.0);
                let what = match paths.as_slice() {
                    [one] => format!("\"{}\"", display_name(one)),
                    many => ox_core::trf!("these {v1} items", v1 = many.len()),
                };
                ui.label(if to_trash {
                    ox_core::trf!(
                        "Do you want to move {what} to the Recycle Bin?",
                        what = what
                    )
                } else {
                    ox_core::trf!(
                        "Do you want to permanently delete {what}? This can't be undone.",
                        what = what
                    )
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let enter = ui.input(|i| i.key_pressed(Key::Enter));
                    let label = if to_trash {
                        tr("Move to Recycle Bin")
                    } else {
                        tr("Delete")
                    };
                    if ui.button(RichText::new(label).strong()).clicked() || enter {
                        actions.push(Action::DeleteConfirmed(paths.clone(), *permanent));
                        keep = false;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::ConfirmPurge { items, everything } => {
                ui.heading(tr("Delete permanently?"));
                ui.add_space(6.0);
                let what = match (items.as_slice(), *everything) {
                    (_, true) => tr("everything in the Recycle Bin").to_owned(),
                    ([one], _) => ox_core::trf!("\"{name}\"", name = one.name.to_string_lossy()),
                    (many, _) => ox_core::trf!("these {v1} items", v1 = many.len()),
                };
                ui.label(ox_core::trf!(
                    "Do you want to permanently delete {what}? This can't be undone.",
                    what = what
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let enter = ui.input(|i| i.key_pressed(Key::Enter));
                    let label = if *everything {
                        tr("Empty Recycle Bin")
                    } else {
                        tr("Delete")
                    };
                    if ui.button(RichText::new(label).strong()).clicked() || enter {
                        actions.push(Action::PurgeConfirmed(std::mem::take(items), *everything));
                        keep = false;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::Properties(p) => {
                if let Some(rx) = &p.rx {
                    if let Ok(t) = rx.try_recv() {
                        p.totals = Some(t);
                        p.rx = None;
                    }
                }
                let title = match p.paths.as_slice() {
                    [one] => display_name(one),
                    many => ox_core::i18n::count(many.len() as u64, "item"),
                };
                ui.heading(ox_core::trf!("ℹ {title}", title = title));
                ui.add_space(6.0);
                egui::Grid::new("props")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        if let [e] = p.entries.as_slice() {
                            ui.label(tr("Type"));
                            ui.label(e.type_label());
                            ui.end_row();
                            ui.label(tr("Location"));
                            ui.label(
                                e.path
                                    .parent()
                                    .map(|x| x.display().to_string())
                                    .unwrap_or_default(),
                            );
                            ui.end_row();
                        }
                        ui.label(tr("Size"));
                        match p.totals {
                            Some((bytes, files, dirs)) => {
                                ui.label(ox_core::trf!(
                                    "{v1} ({bytes} bytes)",
                                    v1 = format::size(bytes),
                                    bytes = bytes
                                ));
                                ui.end_row();
                                ui.label(tr("Contains"));
                                ui.label(ox_core::trf!(
                                    "{files} files, {dirs} folders",
                                    files = files,
                                    dirs = dirs
                                ));
                            }
                            None => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(tr("Calculating…"));
                                });
                            }
                        }
                        ui.end_row();
                        if let [e] = p.entries.as_slice() {
                            for (label, t) in
                                [(tr("Created"), e.created), (tr("Modified"), e.modified)]
                            {
                                if let Some(t) = t {
                                    ui.label(label);
                                    ui.label(format::time(t, &date_fmt));
                                    ui.end_row();
                                }
                            }
                            let mut attrs = Vec::new();
                            if e.readonly {
                                attrs.push(tr("Read-only"));
                            }
                            if e.hidden {
                                attrs.push(tr("Hidden"));
                            }
                            if e.is_symlink {
                                attrs.push(tr("Link"));
                            }
                            if !attrs.is_empty() {
                                ui.label(tr("Attributes"));
                                ui.label(attrs.join(", "));
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(8.0);
                if ui.button(tr("Close")).clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                    p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    keep = false;
                }
            }
            Dialog::RenameGroup { id, text, focus } => {
                ui.heading(tr("Name this group"));
                ui.add_space(6.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(tr("e.g. Work, Photos, Project X"))
                        .desired_width(f32::INFINITY),
                );
                if *focus {
                    *focus = false;
                    resp.request_focus();
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Save")).clicked() || enter {
                        if let Some(g) = app.tabs.group_mut(*id) {
                            g.name = text.trim().to_owned();
                        }
                        keep = false;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        keep = false;
                    }
                });
            }
        }
    });
    if modal.should_close() {
        if let Dialog::Properties(p) = &dialog {
            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        keep = false;
    }
    if keep {
        app.dialog = Some(dialog);
    }
    app.actions.extend(actions);
}
