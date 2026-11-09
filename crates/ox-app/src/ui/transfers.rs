//! The transfer panel: replaces the Windows copy dialog.

mod ask;
pub mod deletes;
mod queue;
mod when_done;
pub use when_done::countdown;

use ox_core::i18n::t as tr;
use std::sync::atomic::Ordering;

use egui::{Color32, RichText, Ui};
use ox_core::entry::display_name;
use ox_core::format;
use ox_core::transfer::{history, ConflictPolicy, Job, JobState, TransferRequest};

use crate::app::OxApp;

mod resume;

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let accent = crate::theme::accent(&app.settings);
    let jobs = app.engine.jobs();
    let rows = queue::order(&jobs, &app.engine.queue());
    let mut remove = Vec::new();
    let mut retries = Vec::new();
    let mut moves = Vec::new();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Transfers")).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✖").on_hover_text(tr("Hide")).clicked() {
                app.ui.show_transfers = false;
                app.ui.transfers_opened = false;
            }
            if ui
                .small_button(tr("📜 History"))
                .on_hover_text(tr("Open the log of finished transfers"))
                .clicked()
            {
                let log = history::log_path();
                if !log.exists() {
                    app.toast(tr("No transfers have finished yet."));
                } else if let Err(e) = ox_core::fsops::open_default(&log) {
                    app.toast(ox_core::trf!("Couldn't open the transfer log: {e}", e = e));
                }
            }
            if jobs.iter().any(|j| j.state().is_finished())
                && ui.small_button(tr("Clear finished")).clicked()
            {
                app.engine.clear_finished();
            }
        });
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if rows.is_empty() && app.engine.resumable().is_empty() && !deletes::any(app) {
                ui.add_space(8.0);
                ui.label(RichText::new(tr("No transfers right now.")).weak());
            }
            resume::interrupted(app, ui);
            deletes::rows(app, ui, accent);
            for (job, place) in &rows {
                job_row(
                    ui,
                    job,
                    accent,
                    *place,
                    &mut moves,
                    &mut remove,
                    &mut retries,
                );
                ui.add_space(6.0);
            }
        });
    for (id, places) in moves {
        app.engine.move_in_queue(id, places);
    }
    for id in remove {
        app.engine.remove(id);
    }
    for req in retries {
        app.engine.submit(req);
    }
}

fn job_row(
    ui: &mut Ui,
    job: &Job,
    accent: Color32,
    place: queue::Place,
    moves: &mut Vec<(u64, isize)>,
    remove: &mut Vec<u64>,
    retries: &mut Vec<TransferRequest>,
) {
    let state = job.state();
    let files = job.total_files.load(Ordering::Relaxed);
    let done_files = job.done_files.load(Ordering::Relaxed);
    let dest = display_name(&job.destination);
    let title = match state {
        JobState::Queued => ox_core::trf!(
            "Waiting for the drive · {v1} {v2} to {dest}",
            v1 = job.kind.verb(),
            v2 = ox_core::i18n::count(job.item_count() as u64, "item"),
            dest = dest
        ),
        JobState::Scanning => ox_core::trf!(
            "Preparing · {v1} to {dest} · {v2} found",
            v1 = ox_core::i18n::count(job.item_count() as u64, "item"),
            dest = dest,
            v2 = ox_core::i18n::count(files, "file")
        ),
        JobState::AwaitingDecision => {
            ox_core::trf!(
                "{v1} to {dest} — some files already exist",
                v1 = job.kind.verb(),
                dest = dest
            )
        }
        JobState::Running if job.is_paused() => {
            ox_core::trf!(
                "Paused · {v1} {v2} to {dest}",
                v1 = job.kind.verb(),
                v2 = ox_core::i18n::count(files, "file"),
                dest = dest
            )
        }
        JobState::Running => {
            let mut t = ox_core::trf!(
                "{v1} {v2} to {dest}",
                v1 = job.kind.verb(),
                v2 = ox_core::i18n::count(files, "file"),
                dest = dest
            );
            if job.hdd.load(Ordering::Relaxed) {
                t.push_str(tr(" · one at a time (HDD)"));
            }
            if job.is_remote() {
                t.push_str(tr(" · server transfer"));
            }
            t
        }
        JobState::Completed => ox_core::trf!(
            "✔ Done · {done_files} of {v1} to {dest} in {v2}",
            done_files = done_files,
            v1 = ox_core::i18n::count(files, "file"),
            dest = dest,
            v2 = format::duration(job.elapsed())
        ),
        JobState::Cancelled => ox_core::trf!(
            "✖ Cancelled · {done_files} of {v1} to {dest}",
            done_files = done_files,
            v1 = ox_core::i18n::count(files, "file"),
            dest = dest
        ),
    };
    let title = ask::title(job)
        .or_else(|| queue::title(job))
        .unwrap_or(title);

    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if state.is_finished() {
                        if ui.small_button("✖").on_hover_text(tr("Dismiss")).clicked() {
                            remove.push(job.id);
                        }
                    } else {
                        if ui.small_button("⏹").on_hover_text(tr("Cancel")).clicked() {
                            job.cancel();
                        }
                        if matches!(state, JobState::Running | JobState::Queued) {
                            let (icon, tip) = if job.is_paused() {
                                ("▶", tr("Resume"))
                            } else {
                                ("⏸", tr("Pause"))
                            };
                            if ui.small_button(icon).on_hover_text(tip).clicked() {
                                if job.is_paused() {
                                    job.resume();
                                } else {
                                    job.pause();
                                }
                                moves.push((job.id, 0)); // the queue checks again
                            }
                        }
                        queue::controls(ui, job, place, moves);
                    }
                });
            });

            if state == JobState::AwaitingDecision {
                let conflicts = job.conflicts();
                ui.label(ox_core::trf!(
                    "{v1} already exist in \"{v2}\". What should happen with all of them?",
                    v1 = ox_core::i18n::count(conflicts.len() as u64, "file"),
                    v2 = job.destination.display()
                ));
                ui.horizontal_wrapped(|ui| {
                    for p in [
                        ConflictPolicy::Overwrite,
                        ConflictPolicy::Skip,
                        ConflictPolicy::KeepBoth,
                        ConflictPolicy::OverwriteIfNewer,
                    ] {
                        if ui.button(p.label()).clicked() {
                            job.resolve_conflicts(p);
                        }
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        job.cancel();
                    }
                });
                egui::CollapsingHeader::new(tr("Show files"))
                    .id_salt(("conflicts", job.id))
                    .show(ui, |ui| {
                        for p in conflicts.iter().take(200) {
                            ui.label(RichText::new(p.display().to_string()).small());
                        }
                        if conflicts.len() > 200 {
                            ui.label(ox_core::trf!("… and {v1} more", v1 = conflicts.len() - 200));
                        }
                    });
                return;
            }

            ask::question(ui, job);

            if !state.is_finished() && state != JobState::Queued {
                let total = job.total_bytes.load(Ordering::Relaxed);
                let done = job.done_bytes.load(Ordering::Relaxed);
                let mut text = format!(
                    "{:.0}% · {} of {}",
                    job.progress() * 100.0,
                    format::size(done),
                    format::size(total)
                );
                if state == JobState::Running && !job.is_paused() {
                    text.push_str(&format!(" · {}", format::rate(job.speed())));
                    if let Some(eta) = job.eta() {
                        text.push_str(&ox_core::trf!(" · {v1} left", v1 = format::duration(eta)));
                    }
                }
                ui.add(
                    egui::ProgressBar::new(job.progress())
                        .text(text)
                        .fill(if job.is_paused() {
                            Color32::GRAY
                        } else {
                            accent
                        })
                        .animate(state == JobState::Scanning),
                );
                let current = job.current_item();
                if !current.as_os_str().is_empty() {
                    ui.label(RichText::new(current.display().to_string()).small().weak());
                }
            }

            let verified = job.verified_files.load(Ordering::Relaxed);
            if verified > 0 {
                ui.label(
                    RichText::new(ox_core::trf!(
                        "✔ {v1} verified",
                        v1 = ox_core::i18n::count(verified, "file")
                    ))
                    .small()
                    .weak(),
                );
            }
            let sized = job.size_checked_files.load(Ordering::Relaxed);
            if sized > 0 {
                // The server offers no checksum: only the sizes were compared.
                ui.label(
                    RichText::new(ox_core::trf!(
                        "✔ {v1}: size checked only",
                        v1 = ox_core::i18n::count(sized, "file")
                    ))
                    .small()
                    .weak(),
                );
            }
            let errors = job.errors();
            if state.is_finished() && !errors.is_empty() {
                if let Some(req) = job.retry_request() {
                    let n = req.pairs.len() as u64;
                    if ui
                        .button(ox_core::trf!(
                            "↻ Retry {v1}",
                            v1 = ox_core::i18n::count(n, "failed file")
                        ))
                        .clicked()
                    {
                        retries.push(req);
                        remove.push(job.id);
                    }
                }
            }
            if !errors.is_empty() {
                egui::CollapsingHeader::new(
                    RichText::new(format!(
                        "⚠ {}",
                        ox_core::i18n::count(errors.len() as u64, "error")
                    ))
                    .color(Color32::from_rgb(0xe5, 0x48, 0x4d)),
                )
                .id_salt(("errors", job.id))
                .show(ui, |ui| {
                    for e in errors.iter().take(200) {
                        ui.label(
                            RichText::new(format!("{} — {}", e.path.display(), e.message)).small(),
                        );
                    }
                });
            }
            let skipped = job.skipped_files.load(Ordering::Relaxed);
            if skipped > 0 {
                ui.label(
                    RichText::new(ox_core::trf!(
                        "{v1} skipped",
                        v1 = ox_core::i18n::count(skipped, "file")
                    ))
                    .small()
                    .weak(),
                );
            }
        });
}
