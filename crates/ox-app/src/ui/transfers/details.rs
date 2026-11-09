//! The details of a transfer: where it copies from and to, what, and how it
//! is going.

use std::sync::atomic::Ordering;

use egui::{RichText, Ui};
use ox_core::entry::display_name;
use ox_core::format;
use ox_core::i18n::count;
use ox_core::transfer::{Job, JobState, TransferKind};

/// Names listed before "and N more".
const NAMES: usize = 5;

/// "Copy from <folder> to <folder>" and the names of the items.
pub(super) fn route(ui: &mut Ui, job: &Job) {
    let dest = job.destination.display().to_string();
    let text = match (job.kind, job.source_folder()) {
        (TransferKind::Copy, Some(src)) => ox_core::trf!(
            "Copy from {src} to {dest}",
            src = src.display(),
            dest = dest
        ),
        (TransferKind::Move, Some(src)) => ox_core::trf!(
            "Move from {src} to {dest}",
            src = src.display(),
            dest = dest
        ),
        (TransferKind::Copy, None) => {
            ox_core::trf!("Copy from several folders to {dest}", dest = dest)
        }
        (TransferKind::Move, None) => {
            ox_core::trf!("Move from several folders to {dest}", dest = dest)
        }
    };
    ui.label(RichText::new(text).small());

    let total = job.item_count();
    let names: Vec<String> = job.source_paths().take(NAMES).map(display_name).collect();
    if names.is_empty() {
        return;
    }
    let mut text = names.join(", ");
    if total > NAMES {
        text = ox_core::trf!("{names} and {n} more", names = text, n = total - NAMES);
    }
    ui.label(RichText::new(text).small().weak()).on_hover_text(
        job.source_paths()
            .take(50)
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// Files done, how many are copied at a time and for how long it has run.
pub(super) fn progress(ui: &mut Ui, job: &Job, state: JobState) {
    let files = job.total_files.load(Ordering::Relaxed);
    let done = job.done_files.load(Ordering::Relaxed);
    let mut parts = vec![ox_core::trf!(
        "{done} of {files}",
        done = done,
        files = count(files, "file")
    )];
    let workers = job.workers.load(Ordering::Relaxed);
    if state == JobState::Running && workers > 1 {
        parts.push(ox_core::trf!("{n} files at a time", n = workers));
    }
    if state == JobState::Running {
        parts.push(ox_core::trf!(
            "running for {time}",
            time = format::duration(job.elapsed())
        ));
    }
    ui.label(RichText::new(parts.join(" · ")).small().weak());
}
