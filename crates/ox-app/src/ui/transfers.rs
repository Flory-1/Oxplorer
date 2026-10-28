//! The transfer panel: replaces the Windows copy dialog.

use std::sync::atomic::Ordering;

use egui::{Color32, RichText, Ui};
use ox_core::entry::display_name;
use ox_core::format;
use ox_core::transfer::{ConflictPolicy, Job, JobState};

use crate::app::OxApp;

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let accent = crate::theme::accent(&app.settings);
    let jobs = app.engine.jobs();
    let mut remove = Vec::new();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Transfers").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✖").on_hover_text("Hide").clicked() {
                app.ui.show_transfers = false;
            }
            if jobs.iter().any(|j| j.state().is_finished())
                && ui.small_button("Clear finished").clicked()
            {
                app.engine.clear_finished();
            }
        });
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for job in jobs.iter().rev() {
                job_row(ui, job, accent, &mut remove);
                ui.add_space(6.0);
            }
        });
    for id in remove {
        app.engine.remove(id);
    }
}

fn job_row(ui: &mut Ui, job: &Job, accent: Color32, remove: &mut Vec<u64>) {
    let state = job.state();
    let files = job.total_files.load(Ordering::Relaxed);
    let done_files = job.done_files.load(Ordering::Relaxed);
    let dest = display_name(&job.destination);
    let title = match state {
        JobState::Queued => format!(
            "Waiting · {} {} to {dest}",
            job.kind.verb(),
            format::count(job.sources.len() as u64, "item")
        ),
        JobState::Scanning => format!(
            "Preparing · {} to {dest} · {} found",
            format::count(job.sources.len() as u64, "item"),
            format::count(files, "file")
        ),
        JobState::AwaitingDecision => {
            format!("{} to {dest} — some files already exist", job.kind.verb())
        }
        JobState::Running if job.is_paused() => {
            format!(
                "Paused · {} {} to {dest}",
                job.kind.verb(),
                format::count(files, "file")
            )
        }
        JobState::Running => format!(
            "{} {} to {dest}",
            job.kind.verb(),
            format::count(files, "file")
        ),
        JobState::Completed => format!(
            "✔ Done · {done_files} of {} to {dest} in {}",
            format::count(files, "file"),
            format::duration(job.elapsed())
        ),
        JobState::Cancelled => format!(
            "✖ Cancelled · {done_files} of {} to {dest}",
            format::count(files, "file")
        ),
    };

    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if state.is_finished() {
                        if ui.small_button("✖").on_hover_text("Dismiss").clicked() {
                            remove.push(job.id);
                        }
                    } else {
                        if ui.small_button("⏹").on_hover_text("Cancel").clicked() {
                            job.cancel();
                        }
                        if state == JobState::Running {
                            let (icon, tip) = if job.is_paused() {
                                ("▶", "Resume")
                            } else {
                                ("⏸", "Pause")
                            };
                            if ui.small_button(icon).on_hover_text(tip).clicked() {
                                if job.is_paused() {
                                    job.resume();
                                } else {
                                    job.pause();
                                }
                            }
                        }
                    }
                });
            });

            if state == JobState::AwaitingDecision {
                let conflicts = job.conflicts();
                ui.label(format!(
                    "{} already exist in \"{}\". What should happen with all of them?",
                    format::count(conflicts.len() as u64, "file"),
                    job.destination.display()
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
                    if ui.button("Cancel").clicked() {
                        job.cancel();
                    }
                });
                egui::CollapsingHeader::new("Show files")
                    .id_salt(("conflicts", job.id))
                    .show(ui, |ui| {
                        for p in conflicts.iter().take(200) {
                            ui.label(RichText::new(p.display().to_string()).small());
                        }
                        if conflicts.len() > 200 {
                            ui.label(format!("… and {} more", conflicts.len() - 200));
                        }
                    });
                return;
            }

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
                        text.push_str(&format!(" · {} left", format::duration(eta)));
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

            let errors = job.errors();
            if !errors.is_empty() {
                egui::CollapsingHeader::new(
                    RichText::new(format!("⚠ {}", format::count(errors.len() as u64, "error")))
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
                    RichText::new(format!("{} skipped", format::count(skipped, "file")))
                        .small()
                        .weak(),
                );
            }
        });
}
