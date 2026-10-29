//! The Git tab: the repository's changes (stage, unstage, discard, commit),
//! its branches (see `git_branches`) and its history (see `git_graph`).

use super::git_panel::{GitPanel, GitView, Snapshot};
use super::*;
use egui::{Align, Layout};
use ox_core::git::{Change, Group, Head};
use ox_core::trf;

/// More files than this per list aren't shown.
const MAX_ROWS: usize = 500;

pub(super) const ERROR: egui::Color32 = egui::Color32::from_rgb(0xe5, 0x48, 0x4d);

/// The Git tab of a folder in the repository `info` is about.
pub(super) fn git_tab(
    ui: &mut Ui,
    panel: &mut GitPanel,
    info: &Arc<GitInfo>,
    date_fmt: &str,
    actions: &mut Vec<Action>,
) {
    panel.update(ui.ctx(), &info.root, actions);
    let snap = panel.snapshot();
    let history = panel.history.clone().unwrap_or_else(|| info.clone());
    let branch = snap
        .as_ref()
        .map(|s| branch_label(&s.changes.head))
        .or_else(|| info.branch.clone());
    git_header(ui, &history, branch.as_deref(), actions);

    let changed = snap
        .as_ref()
        .map_or(info.changed, |s| s.changes.files.len());
    ui.horizontal_wrapped(|ui| {
        let views = [
            (GitView::Changes, format!("{} ({changed})", tr("Changes"))),
            (GitView::Branches, tr("Branches").to_owned()),
            (GitView::History, tr("History").to_owned()),
        ];
        for (view, label) in views {
            if ui.selectable_label(panel.view == view, label).clicked() {
                panel.view = view;
            }
        }
        ui.separator();
        if let Some(s) = &snap {
            git_branches::remote_buttons(ui, panel, s);
        }
        let refresh = ui
            .add_enabled(!panel.reading(), egui::Button::new("🔄").small())
            .on_hover_text(tr("Read the repository again"));
        if refresh.clicked() {
            panel.refresh();
        }
    });
    status_line(ui, panel);
    ui.add_space(2.0);

    let error = panel
        .snapshot
        .as_ref()
        .and_then(|s| s.as_ref().err().cloned());
    match (panel.view, snap, error) {
        (GitView::History, ..) => git_graph(ui, &history, date_fmt, actions),
        (_, _, Some(e)) => {
            ui.colored_label(ERROR, e);
        }
        (_, None, None) => {
            ui.spinner();
        }
        (GitView::Changes, Some(s), None) => changes_view(ui, panel, &s, &info.root, actions),
        (GitView::Branches, Some(s), None) => git_branches::branches_view(ui, panel, &s, date_fmt),
    }
    confirm_discard(ui, panel, &info.root, actions);
}

/// "main ↑2 ↓1".
fn branch_label(head: &Head) -> String {
    let mut s = head
        .branch
        .clone()
        .unwrap_or_else(|| tr("Detached HEAD").to_owned());
    if head.ahead > 0 {
        s.push_str(&format!(" ↑{}", head.ahead));
    }
    if head.behind > 0 {
        s.push_str(&format!(" ↓{}", head.behind));
    }
    s
}

/// What is running, or what the last command printed (errors in red).
fn status_line(ui: &mut Ui, panel: &mut GitPanel) {
    if let Some(what) = panel.busy() {
        let live = panel.live();
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(what);
            if let Some(live) = &live {
                let button = ui
                    .add_enabled(!live.cancelling, egui::Button::new(tr("Cancel")).small())
                    .on_hover_text(tr("Stop git (what it already did stays done)."));
                cancel = button.clicked();
            }
        });
        // Fetch, pull and push: git's latest progress line, with a bar
        // when it says how far it is.
        if let Some(p) = live.and_then(|l| l.progress) {
            if let Some(f) = p.fraction {
                ui.add(egui::ProgressBar::new(f).desired_height(6.0));
            }
            ui.add(egui::Label::new(RichText::new(&p.line).monospace().small().weak()).truncate());
        }
        if cancel {
            panel.cancel();
        }
        return;
    }
    let Some(outcome) = &panel.outcome else {
        return;
    };
    let mut close = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            let title = match outcome.ok {
                true => RichText::new(tr("✔ Done")).weak(),
                false => RichText::new(tr("⚠ Git reported an error")).color(ERROR),
            };
            ui.label(title.small());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = ui.small_button("✖").on_hover_text(tr("Close")).clicked();
            });
        });
        egui::ScrollArea::vertical()
            .id_salt("git_outcome")
            .max_height(110.0)
            .show(ui, |ui| {
                let text = RichText::new(&outcome.text).monospace().size(11.5);
                let text = match outcome.ok {
                    true => text,
                    false => text.color(ERROR),
                };
                ui.add(egui::Label::new(text).selectable(true).wrap());
            });
    });
    if close {
        panel.outcome = None;
    }
}

/// The commit message, the lists of changed files and the selected file's
/// diff.
fn changes_view(
    ui: &mut Ui,
    panel: &mut GitPanel,
    snap: &Arc<Snapshot>,
    root: &Path,
    actions: &mut Vec<Action>,
) {
    panel.update_diff(ui.ctx(), snap);
    egui::ScrollArea::vertical()
        .id_salt("git_changes")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            commit_box(ui, panel, snap);
            ui.add_space(6.0);
            if snap.changes.files.is_empty() {
                ui.label(RichText::new(tr("No changes")).weak());
            }
            for g in Group::ALL {
                file_list(ui, panel, snap, g);
            }
            if let Some((path, _)) = panel.selected.clone() {
                ui.add_space(6.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(&path).strong()).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("✖").on_hover_text(tr("Close")).clicked() {
                            panel.selected = None;
                        }
                        let open = ui
                            .small_button("📂")
                            .on_hover_text(tr("📂 Open with default app"));
                        if open.clicked() {
                            let file = root.join(path.split('/').collect::<PathBuf>());
                            actions.push(Action::Open(file));
                        }
                    });
                });
                git_diff::diff_view(
                    ui,
                    &mut panel.side_by_side,
                    &panel.diff,
                    "git_changes_diff",
                    420.0,
                );
            }
        });
}

/// Summary and description, *Amend* and *Commit*.
fn commit_box(ui: &mut Ui, panel: &mut GitPanel, snap: &Snapshot) {
    let busy = panel.busy().is_some();
    let staged = snap.changes.count(Group::Staged);
    let has_commit = snap.changes.head.commit.is_some();
    ui.add(
        egui::TextEdit::singleline(&mut panel.summary)
            .hint_text(tr("Commit message"))
            .desired_width(f32::INFINITY),
    );
    ui.add(
        egui::TextEdit::multiline(&mut panel.description)
            .hint_text(tr("Description (optional)"))
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    ui.horizontal(|ui| {
        let amend = ui
            .add_enabled(
                has_commit && !busy,
                egui::Checkbox::new(&mut panel.amend, tr("Amend last commit")),
            )
            .on_hover_text(tr(
                "Replace the last commit: its changes and the staged ones, with this message.",
            ));
        // Start from the last commit's message.
        if amend.changed()
            && panel.amend
            && panel.summary.trim().is_empty()
            && panel.description.trim().is_empty()
        {
            if let Some(m) = &snap.last_message {
                let (summary, description) = m.split_once('\n').unwrap_or((m, ""));
                panel.summary = summary.trim().to_owned();
                panel.description = description.trim().to_owned();
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let ready = !busy
                && (staged > 0 || panel.amend)
                && (panel.amend || !panel.summary.trim().is_empty());
            let label = match panel.amend {
                true => tr("✔ Amend").to_owned(),
                false => trf!("✔ Commit ({n})", n = staged),
            };
            let resp = ui
                .add_enabled(ready, egui::Button::new(RichText::new(label).strong()))
                .on_disabled_hover_text(tr("Stage changes and write a message to commit them."));
            if resp.clicked() {
                panel.commit(ui.ctx());
            }
        });
    });
}

/// One list of changed files with its *Stage all* / *Unstage all*.
fn file_list(ui: &mut Ui, panel: &mut GitPanel, snap: &Snapshot, g: Group) {
    let files: Vec<&Change> = snap.changes.group(g).collect();
    if files.is_empty() {
        return;
    }
    let busy = panel.busy().is_some();
    let ctx = ui.ctx().clone();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} ({})", tr(g.label()), files.len())).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = match g {
                Group::Staged => tr("Unstage all"),
                _ => tr("Stage all"),
            };
            if ui
                .add_enabled(!busy, egui::Button::new(label).small())
                .clicked()
            {
                match g {
                    Group::Staged => panel.unstage_all(&ctx),
                    _ => panel.stage(
                        &ctx,
                        files
                            .iter()
                            .flat_map(|c| c.paths())
                            .map(str::to_owned)
                            .collect(),
                    ),
                }
            }
        });
    });
    for c in files.iter().take(MAX_ROWS) {
        file_row(ui, panel, c, g, busy);
    }
    if files.len() > MAX_ROWS {
        ui.label(
            RichText::new(trf!("… and {n} more", n = files.len() - MAX_ROWS))
                .weak()
                .small(),
        );
    }
}

/// A changed file: ticked when staged, its state letter, its name (click:
/// show its diff) and *Discard* for changes that aren't staged.
fn file_row(ui: &mut Ui, panel: &mut GitPanel, c: &Change, g: Group, busy: bool) {
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        let mut staged = g == Group::Staged;
        let hover = match g {
            Group::Staged => tr("Unstage"),
            Group::Conflicts => tr("Mark as resolved (stage)"),
            _ => tr("Stage"),
        };
        let tick = ui
            .add_enabled(!busy, egui::Checkbox::without_text(&mut staged))
            .on_hover_text(hover);
        if tick.changed() {
            let paths = c.paths().into_iter().map(str::to_owned).collect();
            match staged {
                true => panel.stage(&ctx, paths),
                false => panel.unstage(&ctx, paths),
            }
        }
        let state = c.state(g);
        let color = crate::ui::files::git_color(state, ui.visuals());
        ui.label(
            RichText::new(c.letter(g).to_string())
                .monospace()
                .strong()
                .color(color),
        )
        .on_hover_text(tr(state.label()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let discard = match g {
                Group::Unstaged => Some(("↺", tr("Discard changes…"))),
                Group::Untracked => Some(("🗑", tr("Move to Recycle Bin…"))),
                _ => None,
            };
            if let Some((icon, hover)) = discard {
                let b = ui.add_enabled(!busy, egui::Button::new(icon).small());
                if b.on_hover_text(hover).clicked() {
                    panel.confirm = Some((c.clone(), g));
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let (dir, name) = c.path.rsplit_once('/').unwrap_or(("", &c.path));
                let selected = panel
                    .selected
                    .as_ref()
                    .is_some_and(|(p, sg)| *p == c.path && *sg == g);
                let mut text = format!("{name}  ");
                if let Some(orig) = &c.orig {
                    text = format!("{orig} → {name}  ");
                }
                let row = egui::Button::selectable(
                    selected,
                    (RichText::new(text), RichText::new(dir).weak().small()),
                )
                .truncate();
                let resp = ui.add(row).on_hover_text(&c.path);
                if resp.clicked() {
                    panel.selected = match selected {
                        true => None,
                        false => Some((c.path.clone(), g)),
                    };
                }
            });
        });
    });
}

/// The question before a file's changes are thrown away (or an untracked
/// file goes to the Recycle Bin).
fn confirm_discard(ui: &mut Ui, panel: &mut GitPanel, root: &Path, actions: &mut Vec<Action>) {
    let Some((change, group)) = panel.confirm.clone() else {
        return;
    };
    let name = change.path.rsplit('/').next().unwrap_or(&change.path);
    let untracked = group == Group::Untracked;
    let (mut keep, mut go) = (true, false);
    let modal = egui::Modal::new(egui::Id::unique("git_discard")).show(ui.ctx(), |ui| {
        ui.set_width(420.0);
        ui.heading(match untracked {
            true => tr("Move to Recycle Bin?"),
            false => tr("Discard changes?"),
        });
        ui.add_space(6.0);
        ui.label(match untracked {
            true => trf!(
                "Git doesn't track “{name}” yet. Do you want to move it to the Recycle Bin?",
                name = name
            ),
            false => trf!(
                "The changes to “{name}” that aren't staged will be lost. This can't be undone.",
                name = name
            ),
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let label = match untracked {
                true => tr("Move to Recycle Bin"),
                false => tr("Discard changes"),
            };
            if ui.button(RichText::new(label).strong()).clicked() {
                go = true;
                keep = false;
            }
            if ui.button(tr("Cancel")).clicked() {
                keep = false;
            }
        });
    });
    if go {
        if untracked {
            actions.push(Action::MoveToBin(vec![change.abs(root)]));
            panel.refresh_soon();
        } else {
            panel.discard(ui.ctx(), &change);
        }
    }
    if !keep || modal.should_close() {
        panel.confirm = None;
    }
}
