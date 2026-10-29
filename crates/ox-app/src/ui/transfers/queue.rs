//! The queue part of the transfer panel: waiting jobs in start order, held
//! jobs, moving them, and running one after the current ones.

use std::sync::Arc;

use egui::Ui;
use ox_core::entry::display_name;
use ox_core::i18n::t as tr;
use ox_core::transfer::{Job, JobState};

/// A waiting job's place in the queue: (index, queue length).
pub(super) type Place = Option<(usize, usize)>;

/// The panel's order: running jobs (newest first), then the waiting ones in
/// the order they will start, then finished jobs (newest first).
pub(super) fn order(jobs: &[Arc<Job>], queue: &[Arc<Job>]) -> Vec<(Arc<Job>, Place)> {
    let queued = |j: &Arc<Job>| queue.iter().any(|q| q.id == j.id);
    let mut rows: Vec<(Arc<Job>, Place)> = jobs
        .iter()
        .rev()
        .filter(|j| !j.state().is_finished() && !queued(j))
        .map(|j| (j.clone(), None))
        .collect();
    rows.extend(
        queue
            .iter()
            .enumerate()
            .map(|(i, j)| (j.clone(), Some((i, queue.len())))),
    );
    rows.extend(
        jobs.iter()
            .rev()
            .filter(|j| j.state().is_finished() && !queued(j))
            .map(|j| (j.clone(), None)),
    );
    rows
}

/// Title of a waiting job that is on hold or runs after the current ones.
pub(super) fn title(job: &Job) -> Option<String> {
    if job.state() != JobState::Queued {
        return None;
    }
    let v1 = job.kind.verb();
    let v2 = ox_core::i18n::count(job.item_count() as u64, "item");
    let dest = display_name(&job.destination);
    if job.is_on_hold() {
        Some(ox_core::trf!(
            "On hold · {v1} {v2} to {dest}",
            v1 = v1,
            v2 = v2,
            dest = dest
        ))
    } else if job.runs_after_current() {
        Some(ox_core::trf!(
            "Waiting for the other transfers · {v1} {v2} to {dest}",
            v1 = v1,
            v2 = v2,
            dest = dest
        ))
    } else {
        None
    }
}

/// Buttons of a waiting job (in a right-to-left layout): its menu and
/// moving it earlier or later. Moves are collected as (job id, places).
pub(super) fn controls(ui: &mut Ui, job: &Job, place: Place, moves: &mut Vec<(u64, isize)>) {
    let Some((index, len)) = place else {
        return;
    };
    ui.menu_button("⋯", |ui| {
        let mut after = job.runs_after_current();
        if ui
            .checkbox(&mut after, tr("Run after the current transfers"))
            .on_hover_text(tr(
                "Wait until everything ahead of it is done, then run alone.",
            ))
            .changed()
        {
            job.set_run_after_current(after);
            moves.push((job.id, 0)); // lets the queue check again
        }
        ui.separator();
        if ui
            .add_enabled(index > 0, egui::Button::new(tr("Start next")))
            .clicked()
        {
            moves.push((job.id, -(index as isize)));
        }
        if ui
            .add_enabled(index + 1 < len, egui::Button::new(tr("Start last")))
            .clicked()
        {
            moves.push((job.id, len as isize));
        }
    });
    if ui
        .add_enabled(index + 1 < len, egui::Button::new("⏷").small())
        .on_hover_text(tr("Start later"))
        .clicked()
    {
        moves.push((job.id, 1));
    }
    if ui
        .add_enabled(index > 0, egui::Button::new("⏶").small())
        .on_hover_text(tr("Start earlier"))
        .clicked()
    {
        moves.push((job.id, -1));
    }
}
