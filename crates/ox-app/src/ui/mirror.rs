//! Mirror / sync: the dialog (source, target, options), the preview grouped
//! by what happens to each item, and the "Mirror to" context menu.

use super::*;
use egui::{Context, Key};
use ox_core::i18n::count;
use ox_core::mirror::{CompareBy, Group, Mode, Plan, Row, Totals};
use ox_core::trf;

use crate::app::mirror::{MirrorAction, MirrorDialog};

const WARN: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

fn open(source: &Path, target: Option<&Path>) -> Action {
    Action::Mirror(MirrorAction::Open {
        source: source.to_path_buf(),
        target: target.map(Path::to_path_buf),
    })
}

/// "🔁 Mirror to" in a folder's context menu: favorites and open tabs, or
/// any other folder.
pub fn menu(
    ui: &mut Ui,
    dir: &Path,
    destinations: &[(String, PathBuf)],
    actions: &mut Vec<Action>,
) {
    ui.menu_button(tr("🔁 Mirror to"), |ui| {
        for (label, dest) in destinations.iter().filter(|(_, d)| d != dir) {
            if ui.button(label).clicked() {
                actions.push(open(dir, Some(dest)));
            }
        }
        if !destinations.is_empty() {
            ui.separator();
        }
        if ui
            .button(tr("Mirror / sync…"))
            .on_hover_text(tr("Choose the target folder and options, then preview"))
            .clicked()
        {
            actions.push(open(dir, None));
        }
    });
}

/// Buttons for the split view: mirror the left pane to the right or back.
pub fn split_buttons(ui: &mut Ui, left: &Path, right: &Path, actions: &mut Vec<Action>) {
    if ui
        .button(tr("🔁 Mirror ➡"))
        .on_hover_text(tr(
            "Make the right folder like the left one (preview first)",
        ))
        .clicked()
    {
        actions.push(open(left, Some(right)));
    }
    if ui
        .button(tr("🔁 Mirror ⬅"))
        .on_hover_text(tr(
            "Make the left folder like the right one (preview first)",
        ))
        .clicked()
    {
        actions.push(open(right, Some(left)));
    }
}

/// The dialog, while it is open.
pub fn show(app: &mut OxApp, ctx: &Context) {
    let destinations = quick_destinations(app);
    let Some(d) = &mut app.mirror.dialog else {
        return;
    };
    let recent: Vec<PathBuf> = app
        .mirror
        .recent
        .pairs
        .iter()
        .filter(|p| p.source == d.source)
        .map(|p| p.target.clone())
        .collect();
    let mut actions = Vec::new();
    let mut open = true;
    egui::Window::new(tr("🔁 Mirror / sync folders"))
        .id(Id::unique("mirror_dialog"))
        .open(&mut open)
        .default_size([640.0, 560.0])
        .resizable(true)
        .collapsible(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            form(ui, d, &destinations, &recent, &mut actions);
            ui.separator();
            preview(ui, d);
            ui.separator();
            buttons(ui, d, &mut actions);
        });
    if !open {
        actions.push(Action::Mirror(MirrorAction::Close));
    }
    app.actions.extend(actions);
}

fn form(
    ui: &mut Ui,
    d: &mut MirrorDialog,
    destinations: &[(String, PathBuf)],
    recent: &[PathBuf],
    actions: &mut Vec<Action>,
) {
    let preview_again = || Action::Mirror(MirrorAction::Preview);
    egui::Grid::new("mirror_form")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("From"));
            ui.label(RichText::new(d.source.display().to_string()).strong());
            ui.end_row();

            ui.label(tr("To"));
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut d.target)
                        .hint_text(tr("Folder to mirror to"))
                        .desired_width(ui.available_width() - 70.0),
                );
                if std::mem::take(&mut d.focus) {
                    field.request_focus();
                }
                if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    actions.push(preview_again());
                }
                ui.menu_button("⏷", |ui| {
                    for t in recent {
                        if ui.button(format!("🕘 {}", t.display())).clicked() {
                            d.target = t.display().to_string();
                            actions.push(preview_again());
                        }
                    }
                    if !recent.is_empty() && !destinations.is_empty() {
                        ui.separator();
                    }
                    for (label, dest) in destinations.iter().filter(|(_, p)| *p != d.source) {
                        if ui.button(label).clicked() {
                            d.target = dest.display().to_string();
                            actions.push(preview_again());
                        }
                    }
                })
                .response
                .on_hover_text(tr("Recent targets, favorites and open tabs"));
                let target = d.target_path();
                if ui
                    .add_enabled(target.is_dir(), egui::Button::new("⇄"))
                    .on_hover_text(tr("Swap: mirror the other way"))
                    .clicked()
                {
                    d.target = d.source.display().to_string();
                    d.source = target;
                    actions.push(preview_again());
                }
            });
            ui.end_row();

            ui.label(tr("Mode"));
            ui.vertical(|ui| {
                ui.radio_value(
                    &mut d.options.mode,
                    Mode::Mirror,
                    tr("Mirror: make the target exactly like the source"),
                )
                .on_hover_text(tr(
                    "Copies what is new or changed and moves what only the target has to the Recycle Bin.",
                ));
                ui.radio_value(
                    &mut d.options.mode,
                    Mode::Update,
                    tr("Update: copy new and changed files, remove nothing"),
                );
            });
            ui.end_row();

            ui.label(tr("Compare by"));
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.options.by, CompareBy::SizeAndDate, tr("Size and date"))
                    .on_hover_text(tr(
                        "Dates within two seconds count as the same (FAT and network drives).",
                    ));
                ui.radio_value(&mut d.options.by, CompareBy::Contents, tr("Contents (slower)"));
            });
            ui.end_row();

            ui.label(tr("Leave out"));
            ui.add(
                egui::TextEdit::singleline(&mut d.options.exclude)
                    .hint_text("*.tmp; node_modules; .git")
                    .desired_width(f32::INFINITY),
            )
            .on_hover_text(tr(
                "Names or paths separated by ;  — * and ? are wildcards. Left-out items are \
                 neither copied nor removed.",
            ));
            ui.end_row();
        });
}

fn preview(ui: &mut Ui, d: &mut MirrorDialog) {
    if let Some(e) = &d.error {
        ui.colored_label(WARN, format!("⛔ {e}"));
        return;
    }
    if let Some(job) = &d.job {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(trf!(
                "Comparing… {v1} scanned",
                v1 = count(job.scanned(), "item")
            ));
        });
        return;
    }
    let current = d.is_current();
    let Some(plan) = &mut d.plan else {
        ui.label(RichText::new(tr("Choose the target, then preview what will change.")).weak());
        return;
    };
    if !current {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            tr("The target or options changed: preview again."),
        );
    }
    notes(ui, plan);
    egui::ScrollArea::vertical()
        .id_salt("mirror_groups")
        .max_height((ui.available_height() - 80.0).max(160.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.add_enabled_ui(current, |ui| {
                for group in Group::ALL {
                    group_section(ui, plan, group);
                }
            });
        });
    if plan.removes_most() {
        let frame = egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, WARN))
            .corner_radius(4)
            .inner_margin(8);
        frame.show(ui, |ui| {
            ui.colored_label(
                WARN,
                trf!(
                    "⚠ This removes {v1} of the {v2} files in the target — more than half of them.",
                    v1 = plan.removed_files(),
                    v2 = plan.target_files
                ),
            );
            ui.checkbox(&mut d.confirm_many, tr("Yes, move them to the Recycle Bin"));
        });
    }
}

/// What the preview should say besides the groups.
fn notes(ui: &mut Ui, plan: &Plan) {
    let weak = |ui: &mut Ui, text: String| {
        ui.label(RichText::new(text).weak());
    };
    if plan.new_target {
        weak(
            ui,
            tr("The target folder doesn't exist yet: it will be created.").into(),
        );
    }
    let removals = plan.totals(Group::Remove, false).items;
    if !plan.can_remove && removals > 0 {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            tr("Network drives have no Recycle Bin: nothing is removed from this target."),
        );
    }
    if plan.kept_extras > 0 {
        weak(
            ui,
            trf!(
                "{v1} only the target has are kept (update removes nothing).",
                v1 = count(plan.kept_extras as u64, "item")
            ),
        );
    }
    if plan.conflicts > 0 {
        weak(
            ui,
            trf!(
                "{v1} are a file on one side and a folder on the other: left alone.",
                v1 = count(plan.conflicts as u64, "item")
            ),
        );
    }
    if plan.errors > 0 {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            trf!(
                "⚠ {v1} couldn't be read and are left alone.",
                v1 = count(plan.errors as u64, "item")
            ),
        );
    }
}

fn group_title(group: Group) -> &'static str {
    tr(match group {
        Group::New => "Will be copied (new)",
        Group::Update => "Will be updated (newer or different)",
        Group::Remove => "Will be removed from the target (to the Recycle Bin)",
        Group::Same => "Unchanged",
    })
}

/// "12 of 14 · 3 files · 4.2 MB".
fn totals_text(ticked: Totals, all: Totals, group: Group) -> String {
    let items = if group == Group::Same || ticked.items == all.items {
        count(all.items as u64, "item")
    } else {
        trf!(
            "{v1} of {v2}",
            v1 = ticked.items,
            v2 = count(all.items as u64, "item")
        )
    };
    let shown = if group == Group::Same { all } else { ticked };
    format!(
        "{items} · {} · {}",
        count(shown.files, "file"),
        format::size(shown.bytes)
    )
}

fn group_section(ui: &mut Ui, plan: &mut Plan, group: Group) {
    let all = plan.totals(group, false);
    if all.items == 0 {
        return;
    }
    let ticked = plan.totals(group, true);
    let locked = group == Group::Same || (group == Group::Remove && !plan.can_remove);
    let title = format!(
        "{}  —  {}",
        group_title(group),
        totals_text(ticked, all, group)
    );
    let id = ui.make_persistent_id(("mirror_group", group as u8));
    egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        group != Group::Same && all.items <= 200,
    )
    .show_header(ui, |ui| {
        if !locked {
            let mut on = ticked.items > 0;
            let partly = ticked.items > 0 && ticked.items < all.items;
            if ui
                .add(egui::Checkbox::without_text(&mut on).indeterminate(partly))
                .clicked()
            {
                plan.tick_group(group, partly || on);
            }
        }
        let text = RichText::new(title).strong();
        let text = if group == Group::Remove && plan.can_remove {
            text.color(WARN)
        } else {
            text
        };
        ui.label(text);
    })
    .body(|ui| {
        let row_height = ui.spacing().interact_size.y;
        let indices: Vec<usize> = (0..plan.rows.len())
            .filter(|&i| plan.rows[i].group == group)
            .collect();
        egui::ScrollArea::vertical()
            .id_salt(("mirror_rows", group as u8))
            .max_height(240.0)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, indices.len(), |ui, range| {
                for &i in &indices[range] {
                    row(ui, &mut plan.rows[i], locked);
                }
            });
    });
}

fn row(ui: &mut Ui, r: &mut Row, locked: bool) {
    ui.horizontal(|ui| {
        if !locked {
            ui.checkbox(&mut r.include, "");
        }
        let icon = if r.is_dir { "📁" } else { "📄" };
        let path = r.rel.display().to_string();
        let label = ui.label(format!("{icon} {path}"));
        let mut hover = r.target.display().to_string();
        if let Some(src) = &r.source {
            hover = format!("{} → {hover}", src.display());
        }
        label.on_hover_text(hover);
        ui.label(RichText::new(r.why.label()).weak().small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let size = if r.is_dir {
                format!("{} · {}", count(r.files, "file"), format::size(r.bytes))
            } else {
                format::size(r.bytes)
            };
            ui.label(RichText::new(size).weak());
        });
    });
}

fn buttons(ui: &mut Ui, d: &MirrorDialog, actions: &mut Vec<Action>) {
    let busy = d.job.is_some();
    let ready = d.is_current()
        && d.plan
            .as_ref()
            .is_some_and(|p| !p.is_empty() && (!p.removes_most() || d.confirm_many));
    ui.horizontal(|ui| {
        let start = ui
            .add_enabled(
                ready && !busy,
                egui::Button::new(RichText::new(tr("🔁 Start")).strong()),
            )
            .on_hover_text(tr(
                "The copies run in the transfer panel. Removed items go to the Recycle Bin \
                 and can be undone with Ctrl+Z.",
            ));
        if start.clicked() {
            actions.push(Action::Mirror(MirrorAction::Start));
        }
        let stale = !d.is_current() && !busy;
        let label = if d.plan.is_some() || d.error.is_some() {
            tr("↻ Preview again")
        } else {
            tr("Preview")
        };
        let text = if stale {
            RichText::new(label).strong()
        } else {
            RichText::new(label)
        };
        if ui.add_enabled(!busy, egui::Button::new(text)).clicked() {
            actions.push(Action::Mirror(MirrorAction::Preview));
        }
        if ui.button(tr("Cancel")).clicked() {
            actions.push(Action::Mirror(MirrorAction::Close));
        }
        if d.is_current() && d.plan.as_ref().is_some_and(Plan::is_empty) {
            ui.label(RichText::new(tr("✔ The target is up to date.")).weak());
        }
    });
}
