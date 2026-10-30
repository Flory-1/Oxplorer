//! Modal dialogs: rename, delete confirmation, properties, rename tab group.

use egui::{Color32, Context, Id, Key, RichText, Ui};
use ox_core::entry::{display_name, FileEntry};
use ox_core::i18n::t as tr;
use ox_core::rename::{self, Case};
use ox_core::undo::{Kind, Op};
use ox_core::{format, fsops};

use super::preview;
use crate::app::{Action, Dialog, OxApp, Properties};

pub fn show(app: &mut OxApp, ctx: &Context) {
    let Some(mut dialog) = app.dialog.take() else {
        return;
    };
    let mut keep = true;
    let mut actions = Vec::new();
    let date_fmt = app.settings.browsing.date_format.clone();

    let width = match dialog {
        Dialog::BulkRename { .. } => 680.0,
        Dialog::Properties(_) | Dialog::DeleteFailed(_) => 460.0,
        _ => 420.0,
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
                                let pairs: Vec<_> = plan
                                    .iter()
                                    .filter(|p| p.changes())
                                    .map(|p| (p.from.clone(), p.to.clone()))
                                    .collect();
                                app.undo.record(Kind::Rename, Op::Renamed(pairs.clone()));
                                actions.push(Action::PathsMoved(pairs));
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
                                    app.undo.record(
                                        Kind::Rename,
                                        Op::Renamed(vec![(path.clone(), new_path.clone())]),
                                    );
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
                if !properties(ui, p, &date_fmt, &mut actions) {
                    p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    keep = false;
                }
            }
            Dialog::NewLink(d) => keep = super::link_dialog::show(ui, d, app),
            Dialog::Pictures(d) => keep = super::picture_tools::dialog(ui, d, &mut actions),
            Dialog::DeleteFailed(d) => keep = super::delete_failed::show(ui, d, &mut actions),
            Dialog::SessionName { old, text, focus } => {
                keep =
                    super::sessions::name_dialog(ui, &app.sessions, old, text, focus, &mut actions);
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
            Dialog::SelectPattern {
                text,
                select,
                focus,
            } => {
                let view = &app.tabs.active().view;
                keep = super::select_pattern::show(ui, text, select, focus, view, &mut actions);
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

/// The Properties dialog: what the items are, their size and dates, the
/// Read-only and Hidden attributes to change, and the details of the type.
/// Returns whether it stays open.
fn properties(ui: &mut Ui, p: &mut Properties, date_fmt: &str, actions: &mut Vec<Action>) -> bool {
    if let Some(t) = p.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
        p.totals = Some(t);
        p.rx = None;
    }
    if let Some(more) = p.more_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
        p.more = Some(more);
        p.more_rx = None;
    }
    let title = match p.paths.as_slice() {
        [one] => display_name(one),
        many => ox_core::i18n::count(many.len() as u64, "item"),
    };
    ui.heading(ox_core::trf!("ℹ {title}", title = title));
    ui.add_space(6.0);

    let details = p.more.as_ref().map(|m| &m.details);
    let mut rows = match p.entries.as_slice() {
        [e] => preview::info_rows(e, details, date_fmt),
        many => {
            let first = many.first().map(FileEntry::type_label).unwrap_or_default();
            let same = many.iter().all(|e| e.type_label() == first);
            let mut rows = vec![(
                tr("Type"),
                if same {
                    first
                } else {
                    tr("Various types").to_owned()
                },
            )];
            let parent = many.first().and_then(|e| e.path.parent());
            if let Some(parent) = parent.filter(|d| many.iter().all(|e| e.path.parent() == Some(d)))
            {
                rows.push((tr("Location"), parent.display().to_string()));
            }
            rows
        }
    };
    // Folders and several items: the size once it's counted, after the type.
    let single_file = matches!(p.entries.as_slice(), [e] if !e.is_dir);
    if let (false, Some((bytes, files, dirs))) = (single_file, p.totals) {
        let at = rows
            .iter()
            .position(|(k, _)| *k == tr("Location"))
            .unwrap_or(rows.len());
        rows.insert(
            at,
            (
                tr("Size"),
                ox_core::trf!(
                    "{v1} ({bytes} bytes)",
                    v1 = format::size(bytes),
                    bytes = bytes
                ),
            ),
        );
        rows.insert(
            at + 1,
            (
                tr("Contains"),
                ox_core::trf!("{files} files, {dirs} folders", files = files, dirs = dirs),
            ),
        );
    }
    grid(ui, "props", &rows);
    if !single_file && p.totals.is_none() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(tr("Calculating…")).weak());
        });
    }

    ui.separator();
    // Checked when all items are, empty when none is, mixed otherwise.
    let initial = |on: fn(&FileEntry) -> bool| {
        let n = p.entries.iter().filter(|e| on(e)).count();
        (n == 0 || n == p.entries.len()).then_some(n > 0)
    };
    let (readonly0, hidden0) = (initial(|e| e.readonly), initial(|e| e.hidden));
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Attributes")).weak());
        ui.add_space(8.0);
        tri_checkbox(ui, &mut p.readonly, tr("Read-only"));
        tri_checkbox(ui, &mut p.hidden, tr("Hidden"));
    });
    let others: Vec<String> = details
        .into_iter()
        .flat_map(preview::other_attributes)
        .collect();
    if !others.is_empty() {
        ui.label(RichText::new(others.join(", ")).weak());
    }

    // The type's details and Git.
    let mut more: Vec<(&str, String)> = Vec::new();
    if let Some(m) = &p.more {
        more.extend(m.facts.iter().cloned());
        more.extend(m.details.facts.iter().cloned());
    }
    if let Some(state) = p.git {
        more.push((tr("Git"), state.to_owned()));
    }
    if let Some(c) = p.more.as_ref().and_then(|m| m.last_commit.as_ref()) {
        let when = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(c.time.max(0) as u64);
        more.push((
            tr("Last commit"),
            format!(
                "{} {}\n{}, {}",
                c.short(),
                c.subject,
                c.author,
                format::time(when, date_fmt)
            ),
        ));
    }
    if !more.is_empty() {
        ui.separator();
        grid(ui, "props_more", &more);
    } else if p.more_rx.is_some() {
        ui.spinner();
    }

    ui.add_space(8.0);
    let readonly = p.readonly.filter(|_| p.readonly != readonly0);
    let hidden = p.hidden.filter(|_| p.hidden != hidden0);
    let changed = readonly.is_some() || hidden.is_some();
    let mut apply = false;
    let mut keep = true;
    ui.horizontal(|ui| {
        if ui.button(tr("OK")).clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
            apply = true;
            keep = false;
        }
        if ui.button(tr("Cancel")).clicked() {
            keep = false;
        }
        if ui
            .add_enabled(changed, egui::Button::new(tr("Apply")))
            .clicked()
        {
            apply = true;
        }
    });
    if apply && changed {
        actions.push(Action::SetAttributes(p.paths.clone(), readonly, hidden));
        // What the boxes compare against from now on.
        for e in &mut p.entries {
            e.readonly = readonly.unwrap_or(e.readonly);
            e.hidden = hidden.unwrap_or(e.hidden);
        }
    }
    keep
}

/// A check box that can also show "some are, some aren't" (`None`).
fn tri_checkbox(ui: &mut Ui, state: &mut Option<bool>, text: &str) {
    let mut on = state.unwrap_or(false);
    let box_ = egui::Checkbox::new(&mut on, text).indeterminate(state.is_none());
    if ui.add(box_).changed() {
        *state = Some(on);
    }
}

/// Two columns: a weak label and its value.
fn grid(ui: &mut Ui, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (name, value) in rows {
                ui.label(RichText::new(*name).weak());
                ui.add(egui::Label::new(value.as_str()).wrap());
                ui.end_row();
            }
        });
}
