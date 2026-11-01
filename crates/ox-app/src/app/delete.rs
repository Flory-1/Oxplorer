//! Deleting items with the delete engine (`ox_core::delete`): jobs show up
//! in the transfer panel when they take a moment; what can't be deleted opens
//! the "Couldn't delete" dialog with the reason and the programs holding it.

use super::*;
use crate::ui::delete_failed::{DeleteFailed, Failed};
use ox_core::delete::{DeleteEngine, DeleteJob, DeleteMode, DeleteRequest, DeleteState};
use ox_core::locks;

/// Failed items whose locking programs are looked up (each lookup asks the
/// Restart Manager, which takes a moment).
const MAX_LOCKER_LOOKUPS: usize = 50;

/// The delete engine and which of its jobs were shown.
#[derive(Default)]
pub struct Deletes {
    pub engine: DeleteEngine,
    /// Jobs the transfer panel was opened for.
    shown: HashSet<u64>,
}

impl OxApp {
    /// Delete `paths` (to the Recycle Bin or for good), after ending the
    /// programs with the process ids in `end`.
    pub(super) fn start_delete(&mut self, paths: Vec<PathBuf>, to_trash: bool, end: Vec<u32>) {
        let mode = if to_trash {
            DeleteMode::Trash
        } else {
            DeleteMode::Permanent
        };
        self.deletes
            .engine
            .submit(DeleteRequest { paths, mode, end });
    }

    pub(super) fn poll_deletes(&mut self) {
        for job in self.deletes.engine.jobs() {
            if job.is_slow() && self.deletes.shown.insert(job.id) {
                self.ui.show_transfers = true;
            }
            if job.state().is_finished() {
                self.deletes.engine.remove(job.id);
                self.deletes.shown.remove(&job.id);
                self.delete_finished(&job);
            }
        }
    }

    fn delete_finished(&mut self, job: &DeleteJob) {
        let to_trash = job.mode == DeleteMode::Trash;
        if to_trash {
            self.undo.recorder().trashed(&job.paths, job.since);
        }
        let mut dirs: Vec<PathBuf> = job
            .paths
            .iter()
            .filter_map(|p| p.parent())
            .map(Path::to_path_buf)
            .collect();
        dirs.dedup();
        for d in &dirs {
            self.mark_changed(Some(d));
        }
        if job.state() == DeleteState::Cancelled {
            self.toast(ox_core::trf!(
                "Deleting cancelled · {v1} of {v2} deleted",
                v1 = job.removed().len(),
                v2 = ox_core::i18n::count(job.paths.len() as u64, "item")
            ));
        }
        let notes = job.notes();
        let errors = job.errors();
        if errors.is_empty() {
            if !notes.is_empty() {
                // Deleted after all; only ending a program failed.
                self.toast(notes.join("\n"));
            }
            return;
        }
        // Trying again deletes the selected items the failures are in.
        let mut retry: Vec<PathBuf> = Vec::new();
        for e in &errors {
            if !retry.contains(&e.item) {
                retry.push(e.item.clone());
            }
        }
        let dir = dirs.into_iter().next();
        self.in_background(move || {
            let items = errors
                .into_iter()
                .enumerate()
                .map(|(i, e)| Failed {
                    lockers: if i < MAX_LOCKER_LOOKUPS {
                        locks::lockers(std::slice::from_ref(&e.path))
                    } else {
                        Vec::new()
                    },
                    path: e.path,
                    error: e.message,
                })
                .collect();
            let failed = DeleteFailed {
                items,
                retry,
                to_trash,
                notes,
            };
            Notice::DeleteFailed(Box::new(failed), dir)
        });
    }

    /// Delete `paths` permanently with administrator rights.
    pub(super) fn delete_as_admin(&mut self, paths: Vec<PathBuf>) {
        let dir = paths
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        self.run_background(tr("Delete as administrator"), dir, move || {
            locks::delete_as_admin(&paths)
        });
    }
}
