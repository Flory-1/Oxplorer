//! The Git tab's Branches view: the current branch and its upstream,
//! switching to another branch (local or remote), creating one; fetch,
//! pull and push.

use super::git_changes::ERROR;
use super::git_panel::{GitPanel, Snapshot};
use super::*;
use egui::{Align, Layout};
use ox_core::git::Branch;
use ox_core::trf;

/// More branches than this per list aren't shown.
const MAX_ROWS: usize = 300;

const WARN: egui::Color32 = egui::Color32::from_rgb(0xf0, 0x8a, 0x24);

/// *Pull* and *Push* (when the repository has a remote).
pub(super) fn remote_buttons(ui: &mut Ui, panel: &mut GitPanel, snap: &Snapshot) {
    if !snap.has_remote() {
        return;
    }
    let ctx = ui.ctx().clone();
    let busy = panel.busy().is_some();
    let head = &snap.changes.head;
    let pull = ui
        .add_enabled(
            !busy && head.upstream.is_some(),
            egui::Button::new(tr("⬇ Pull")).small(),
        )
        .on_hover_text(tr("Get the new commits of the upstream branch (git pull)."))
        .on_disabled_hover_text(tr("The branch has no upstream to pull from."));
    if pull.clicked() {
        panel.pull(&ctx);
    }
    let hover = match head.upstream {
        Some(_) => tr("Send your commits to the upstream branch (git push)."),
        None => tr("Publish the branch on the remote and track it (git push --set-upstream)."),
    };
    let push = ui
        .add_enabled(
            !busy && head.branch.is_some(),
            egui::Button::new(tr("⬆ Push")).small(),
        )
        .on_hover_text(hover);
    if push.clicked() {
        panel.push(&ctx);
    }
}

pub(super) fn branches_view(ui: &mut Ui, panel: &mut GitPanel, snap: &Snapshot, date_fmt: &str) {
    let ctx = ui.ctx().clone();
    let busy = panel.busy().is_some();
    let mut switch_to: Option<(Branch, bool)> = None;
    egui::ScrollArea::vertical()
        .id_salt("git_branches")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            current(ui, snap);
            if snap.has_remote() {
                let fetch = ui
                    .add_enabled(!busy, egui::Button::new(tr("🔄 Fetch")).small())
                    .on_hover_text(tr(
                        "Get the remote's new branches and commits without changing yours (git fetch).",
                    ));
                if fetch.clicked() {
                    panel.fetch(&ctx);
                }
            }
            if let Some(b) = panel.blocked.clone() {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.label(
                        RichText::new(trf!(
                            "⚠ Switching to “{branch}” would overwrite local changes. Commit them first, or stash them and switch.",
                            branch = b.name
                        ))
                        .color(WARN),
                    );
                    ui.horizontal(|ui| {
                        let stash = ui
                            .add_enabled(!busy, egui::Button::new(tr("Stash and switch")))
                            .on_hover_text(tr(
                                "Put the local changes aside with git stash (git stash pop brings them back), then switch.",
                            ));
                        if stash.clicked() {
                            switch_to = Some((b.clone(), true));
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            panel.blocked = None;
                        }
                    });
                });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 100.0).max(80.0);
                let name = ui.add(
                    egui::TextEdit::singleline(&mut panel.new_branch)
                        .hint_text(tr("New branch name"))
                        .desired_width(w),
                );
                let ok = !busy && !panel.new_branch.trim().is_empty();
                let create = ui
                    .add_enabled(ok, egui::Button::new(tr("➕ Create")))
                    .on_hover_text(tr(
                        "Create the branch from the current commit and switch to it.",
                    ));
                let enter = name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ok && (create.clicked() || enter) {
                    panel.create_branch(&ctx);
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut panel.filter)
                    .hint_text(tr("Filter branches"))
                    .desired_width(f32::INFINITY),
            );
            let filter = panel.filter.to_lowercase();
            for remote in [false, true] {
                let list: Vec<&Branch> = snap
                    .branches
                    .iter()
                    .filter(|b| b.remote == remote)
                    .filter(|b| filter.is_empty() || b.name.to_lowercase().contains(&filter))
                    .collect();
                if list.is_empty() {
                    continue;
                }
                ui.add_space(6.0);
                let title = match remote {
                    true => tr("Remote branches"),
                    false => tr("Local branches"),
                };
                ui.label(RichText::new(format!("{title} ({})", list.len())).strong());
                for b in list.iter().take(MAX_ROWS) {
                    if branch_row(ui, b, &snap.branches, busy, date_fmt) {
                        switch_to = Some(((*b).clone(), false));
                    }
                }
                if list.len() > MAX_ROWS {
                    ui.label(
                        RichText::new(trf!("… and {n} more", n = list.len() - MAX_ROWS))
                            .weak()
                            .small(),
                    );
                }
            }
        });
    if let Some((branch, stash)) = switch_to {
        panel.switch(&ctx, branch, stash);
    }
}

/// The checked out branch, its upstream and how far apart they are.
fn current(ui: &mut Ui, snap: &Snapshot) {
    let head = &snap.changes.head;
    ui.horizontal_wrapped(|ui| {
        let name = head.branch.as_deref().unwrap_or(tr("Detached HEAD"));
        ui.label(
            RichText::new(format!("🌿 {name}"))
                .strong()
                .color(lane_color(0)),
        );
        match &head.upstream {
            Some(up) => {
                ui.label(RichText::new(format!("→ {up}")).weak());
                let text = match (head.ahead, head.behind) {
                    (0, 0) => tr("up to date").to_owned(),
                    (a, b) => format!("↑{a} ↓{b}"),
                };
                ui.label(RichText::new(text).weak())
                    .on_hover_text(tr("Commits to push (↑) and to pull (↓)"));
            }
            None if head.branch.is_some() => {
                ui.label(RichText::new(tr("not published yet")).weak());
            }
            None => {}
        }
    });
    let gone = snap.branches.iter().any(|b| b.head && b.gone);
    if gone {
        ui.label(
            RichText::new(tr("The upstream branch was deleted on the remote."))
                .color(ERROR)
                .small(),
        );
    }
}

/// One branch with *Switch*; `true` when that was clicked.
fn branch_row(ui: &mut Ui, b: &Branch, all: &[Branch], busy: bool, date_fmt: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if b.head {
                ui.label(RichText::new(tr("checked out")).weak().small());
            } else {
                let hover = match ox_core::git::switch_target(b, all) {
                    (local, Some(remote)) => trf!(
                        "Create the local branch “{local}” tracking “{remote}” and switch to it.",
                        local = local,
                        remote = remote
                    ),
                    (local, None) => trf!("Switch to “{branch}”.", branch = local),
                };
                clicked = ui
                    .add_enabled(!busy, egui::Button::new(tr("Switch")).small())
                    .on_hover_text(hover)
                    .clicked();
            }
            if b.ahead > 0 || b.behind > 0 {
                ui.label(
                    RichText::new(format!("↑{} ↓{}", b.ahead, b.behind))
                        .weak()
                        .small(),
                );
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let text = match b.head {
                    true => RichText::new(format!("● {}", b.name))
                        .strong()
                        .color(lane_color(0)),
                    false => RichText::new(&b.name),
                };
                let when = format::time(
                    SystemTime::UNIX_EPOCH + Duration::from_secs(b.time.max(0) as u64),
                    date_fmt,
                );
                ui.add(egui::Label::new(text).truncate()).on_hover_ui(|ui| {
                    ui.set_max_width(360.0);
                    ui.label(RichText::new(&b.name).strong());
                    if !b.subject.is_empty() {
                        ui.label(&b.subject);
                    }
                    ui.label(RichText::new(when).weak());
                    if let Some(up) = &b.upstream {
                        ui.label(RichText::new(format!("→ {up}")).weak());
                    }
                });
            });
        });
    });
    clicked
}
