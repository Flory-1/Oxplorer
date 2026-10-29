//! The question about a failed file, asked while the job waits.

use egui::{Color32, RichText, Ui};
use ox_core::entry::display_name;
use ox_core::i18n::t as tr;
use ox_core::transfer::{ErrorChoice, Job, TransferKind};

/// Title of a job that waits for an answer about a failed file.
pub(super) fn title(job: &Job) -> Option<String> {
    job.is_asking().then(|| {
        ox_core::trf!(
            "{v1} to {dest} — a file failed",
            v1 = job.kind.verb(),
            dest = display_name(&job.destination)
        )
    })
}

/// Draw the open question of `job`, if it has one.
pub(super) fn question(ui: &mut Ui, job: &Job) {
    let Some(q) = job.error_question() else {
        return;
    };
    let name = display_name(&q.path);
    let text = match job.kind {
        TransferKind::Copy => ox_core::trf!("\"{name}\" couldn't be copied:", name = name),
        TransferKind::Move => ox_core::trf!("\"{name}\" couldn't be moved:", name = name),
    };
    ui.label(text);
    ui.label(RichText::new(format!("⚠ {}", q.message)).color(Color32::from_rgb(0xe5, 0x48, 0x4d)));
    ui.label(RichText::new(q.path.display().to_string()).small().weak());
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(tr("↻ Retry"))
            .on_hover_text(tr(
                "Try this file again, e.g. after closing the program that uses it.",
            ))
            .clicked()
        {
            job.answer_error(ErrorChoice::Retry);
        }
        if ui
            .button(tr("Skip"))
            .on_hover_text(tr(
                "Leave this file out. It is listed with the errors at the end.",
            ))
            .clicked()
        {
            job.answer_error(ErrorChoice::Skip);
        }
        if ui
            .button(tr("Skip all errors"))
            .on_hover_text(tr(
                "Leave out this and every other file of this job that fails, without asking again.",
            ))
            .clicked()
        {
            job.answer_error(ErrorChoice::SkipAll);
        }
        if ui.button(tr("Cancel")).clicked() {
            job.cancel();
        }
    });
}
