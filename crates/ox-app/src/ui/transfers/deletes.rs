//! Delete jobs in the transfer panel: only those that take a moment (quick
//! deletes would only flash by), with progress, pause and cancel.

use ox_core::i18n::t as tr;
use std::sync::atomic::Ordering;

use egui::{Color32, RichText, Ui};
use ox_core::delete::{DeleteJob, DeleteMode, DeleteState};
use ox_core::format;
use ox_core::i18n::count;

use crate::app::OxApp;

/// Is there a delete job worth showing?
pub fn any(app: &OxApp) -> bool {
    app.deletes.engine.jobs().iter().any(|j| j.is_slow())
}

pub fn rows(app: &OxApp, ui: &mut Ui, accent: Color32) {
    for job in app.deletes.engine.jobs() {
        if job.is_slow() {
            row(ui, &job, accent);
            ui.add_space(6.0);
        }
    }
}

fn row(ui: &mut Ui, job: &DeleteJob, accent: Color32) {
    let state = job.state();
    let total = job.total_items.load(Ordering::Relaxed);
    let items = count(job.paths.len() as u64, "item");
    let title = match (state, job.mode) {
        (DeleteState::Scanning, _) => ox_core::trf!(
            "Preparing to delete {v1} · {v2} found",
            v1 = items,
            v2 = count(total, "item")
        ),
        (_, DeleteMode::Trash) => {
            ox_core::trf!("Moving {v1} to the Recycle Bin", v1 = items)
        }
        (_, DeleteMode::Permanent) => {
            ox_core::trf!("Deleting {v1} permanently", v1 = count(total, "item"))
        }
    };
    let title = if job.is_paused() {
        ox_core::trf!("Paused · {title}", title = title)
    } else {
        title
    };

    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("⏹").on_hover_text(tr("Cancel")).clicked() {
                        job.cancel();
                    }
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
                    }
                });
            });

            let done = job.done_items.load(Ordering::Relaxed);
            let mut text = format!(
                "{:.0}% · {}",
                job.progress() * 100.0,
                ox_core::trf!("{done} of {v1}", done = done, v1 = count(total, "item"))
            );
            let bytes = job.total_bytes.load(Ordering::Relaxed);
            if bytes > 0 {
                text.push_str(&ox_core::trf!(
                    " · {v1} of {v2} freed",
                    v1 = format::size(job.done_bytes.load(Ordering::Relaxed)),
                    v2 = format::size(bytes)
                ));
            }
            ui.add(
                egui::ProgressBar::new(job.progress())
                    .text(text)
                    .fill(if job.is_paused() {
                        Color32::GRAY
                    } else {
                        accent
                    })
                    .animate(state == DeleteState::Scanning),
            );
            let current = job.current_item();
            if !current.as_os_str().is_empty() {
                ui.label(RichText::new(current.display().to_string()).small().weak());
            }
            let errors = job.error_count();
            if errors > 0 {
                ui.label(
                    RichText::new(format!("⚠ {}", count(errors as u64, "error")))
                        .color(Color32::from_rgb(0xe5, 0x48, 0x4d)),
                );
            }
        });
}
