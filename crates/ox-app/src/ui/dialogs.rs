//! Modal dialogs: rename, delete confirmation, properties, rename tab group.

use egui::{Context, Id, Key, RichText};
use ox_core::entry::display_name;
use ox_core::{format, fsops};

use crate::app::{Action, Dialog, OxApp};

pub fn show(app: &mut OxApp, ctx: &Context) {
    let Some(mut dialog) = app.dialog.take() else {
        return;
    };
    let mut keep = true;
    let mut actions = Vec::new();
    let date_fmt = app.settings.browsing.date_format.clone();

    let modal = egui::Modal::new(Id::unique("dialog")).show(ctx, |ui| {
        ui.set_width(420.0);
        match &mut dialog {
            Dialog::Rename {
                path,
                text,
                error,
                focus,
            } => {
                ui.heading("Rename");
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
                    let ok = ui.button("Rename").clicked() || enter;
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                    if ok {
                        if *text == display_name(path) {
                            keep = false;
                        } else {
                            match fsops::rename(path, text.trim()) {
                                Ok(new_path) => {
                                    keep = false;
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
                    "Move to Recycle Bin?"
                } else {
                    "Delete permanently?"
                });
                ui.add_space(6.0);
                let what = match paths.as_slice() {
                    [one] => format!("\"{}\"", display_name(one)),
                    many => format!("these {} items", many.len()),
                };
                ui.label(if to_trash {
                    format!("Do you want to move {what} to the Recycle Bin?")
                } else {
                    format!("Do you want to permanently delete {what}? This can't be undone.")
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let enter = ui.input(|i| i.key_pressed(Key::Enter));
                    let label = if to_trash {
                        "Move to Recycle Bin"
                    } else {
                        "Delete"
                    };
                    if ui.button(RichText::new(label).strong()).clicked() || enter {
                        actions.push(Action::DeleteConfirmed(paths.clone(), *permanent));
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
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
                    many => format::count(many.len() as u64, "item"),
                };
                ui.heading(format!("ℹ {title}"));
                ui.add_space(6.0);
                egui::Grid::new("props")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        if let [e] = p.entries.as_slice() {
                            ui.label("Type");
                            ui.label(e.type_label());
                            ui.end_row();
                            ui.label("Location");
                            ui.label(
                                e.path
                                    .parent()
                                    .map(|x| x.display().to_string())
                                    .unwrap_or_default(),
                            );
                            ui.end_row();
                        }
                        ui.label("Size");
                        match p.totals {
                            Some((bytes, files, dirs)) => {
                                ui.label(format!("{} ({bytes} bytes)", format::size(bytes)));
                                ui.end_row();
                                ui.label("Contains");
                                ui.label(format!("{files} files, {dirs} folders"));
                            }
                            None => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("Calculating…");
                                });
                            }
                        }
                        ui.end_row();
                        if let [e] = p.entries.as_slice() {
                            for (label, t) in [("Created", e.created), ("Modified", e.modified)] {
                                if let Some(t) = t {
                                    ui.label(label);
                                    ui.label(format::time(t, &date_fmt));
                                    ui.end_row();
                                }
                            }
                            let mut attrs = Vec::new();
                            if e.readonly {
                                attrs.push("Read-only");
                            }
                            if e.hidden {
                                attrs.push("Hidden");
                            }
                            if e.is_symlink {
                                attrs.push("Link");
                            }
                            if !attrs.is_empty() {
                                ui.label("Attributes");
                                ui.label(attrs.join(", "));
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(8.0);
                if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                    p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    keep = false;
                }
            }
            Dialog::RenameGroup { id, text, focus } => {
                ui.heading("Name this group");
                ui.add_space(6.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text("e.g. Work, Photos, Project X")
                        .desired_width(f32::INFINITY),
                );
                if *focus {
                    *focus = false;
                    resp.request_focus();
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() || enter {
                        if let Some(g) = app.tabs.group_mut(*id) {
                            g.name = text.trim().to_owned();
                        }
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
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
