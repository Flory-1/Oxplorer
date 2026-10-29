//! The Git tab's stashes (in the Branches view): put the local changes
//! aside, and apply, pop or drop what was put aside.

use super::git_changes::ERROR;
use super::git_panel::{GitPanel, Snapshot, Then};
use super::*;
use egui::{Align, Layout};
use ox_core::git::Stash;
use ox_core::trf;

impl GitPanel {
    pub(super) fn stash_push(&mut self, ctx: &egui::Context) {
        let message = self.stash_message.trim().to_owned();
        self.start(ctx, tr("Stashing…").into(), Then::Stashed, move |root| {
            ox_core::git::stash_push(root, &message)
        });
    }

    fn stash_apply(&mut self, ctx: &egui::Context, stash: Stash, pop: bool) {
        let what = trf!("Applying {stash}…", stash = stash.name());
        self.start(ctx, what, Then::Nothing, move |root| match pop {
            true => ox_core::git::stash_pop(root, &stash),
            false => ox_core::git::stash_apply(root, &stash),
        });
    }

    fn stash_drop(&mut self, ctx: &egui::Context, stash: Stash) {
        let what = trf!("Dropping {stash}…", stash = stash.name());
        self.start(ctx, what, Then::Nothing, move |root| {
            ox_core::git::stash_drop(root, &stash)
        });
    }
}

/// *Stash changes* with an optional message, and the list of stashes.
pub(super) fn stashes(ui: &mut Ui, panel: &mut GitPanel, snap: &Snapshot, date_fmt: &str) {
    let ctx = ui.ctx().clone();
    let busy = panel.busy().is_some();
    ui.add_space(6.0);
    ui.label(RichText::new(format!("{} ({})", tr("Stashes"), snap.stashes.len())).strong());
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 130.0).max(80.0);
        let message = ui.add(
            egui::TextEdit::singleline(&mut panel.stash_message)
                .hint_text(tr("Message (optional)"))
                .desired_width(w),
        );
        let ok = !busy && !snap.changes.files.is_empty();
        let stash = ui
            .add_enabled(ok, egui::Button::new(tr("📦 Stash changes")))
            .on_hover_text(tr(
                "Put all local changes aside, untracked files too (git stash). Apply or pop them later.",
            ))
            .on_disabled_hover_text(tr("There are no local changes to stash."));
        let enter = message.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ok && (stash.clicked() || enter) {
            panel.stash_push(&ctx);
        }
    });
    let mut apply: Option<(Stash, bool)> = None;
    for s in &snap.stashes {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let drop = ui
                    .add_enabled(!busy, egui::Button::new("🗑").small())
                    .on_hover_text(tr("Drop: throw the stash away"));
                if drop.clicked() {
                    panel.confirm_drop = Some(s.clone());
                }
                let pop = ui
                    .add_enabled(!busy, egui::Button::new(tr("Pop")).small())
                    .on_hover_text(tr(
                        "Bring the changes back and remove the stash (git stash pop).",
                    ));
                if pop.clicked() {
                    apply = Some((s.clone(), true));
                }
                let keep = ui
                    .add_enabled(!busy, egui::Button::new(tr("Apply")).small())
                    .on_hover_text(tr(
                        "Bring the changes back and keep the stash (git stash apply).",
                    ));
                if keep.clicked() {
                    apply = Some((s.clone(), false));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.label(RichText::new(s.name()).weak().small().monospace());
                    let when = format::time(
                        SystemTime::UNIX_EPOCH + Duration::from_secs(s.time.max(0) as u64),
                        date_fmt,
                    );
                    ui.add(egui::Label::new(&s.message).truncate())
                        .on_hover_ui(|ui| {
                            ui.set_max_width(360.0);
                            ui.label(RichText::new(&s.message).strong());
                            if let Some(b) = &s.branch {
                                ui.label(RichText::new(trf!("On {branch}", branch = b)).weak());
                            }
                            ui.label(RichText::new(when).weak());
                        });
                });
            });
        });
    }
    if let Some((stash, pop)) = apply {
        panel.stash_apply(&ctx, stash, pop);
    }
    confirm_drop(ui, panel, busy);
}

/// Dropping a stash can't be undone: asked first, under the list.
fn confirm_drop(ui: &mut Ui, panel: &mut GitPanel, busy: bool) {
    let Some(stash) = panel.confirm_drop.clone() else {
        return;
    };
    let ctx = ui.ctx().clone();
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(
            RichText::new(trf!(
                "Drop {stash} “{message}”? Its changes are gone then; this can't be undone.",
                stash = stash.name(),
                message = stash.message
            ))
            .color(ERROR),
        );
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new(tr("🗑 Drop")))
                .clicked()
            {
                panel.confirm_drop = None;
                panel.stash_drop(&ctx, stash.clone());
            }
            if ui.button(tr("Cancel")).clicked() {
                panel.confirm_drop = None;
            }
        });
    });
}
