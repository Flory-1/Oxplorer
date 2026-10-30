//! Deleting items in the background; what can't be deleted opens the
//! "Couldn't delete" dialog with the reason and the programs holding it.

use super::*;
use crate::ui::delete_failed::{DeleteFailed, Failed};
use ox_core::locks;

impl OxApp {
    /// Delete `paths` (to the Recycle Bin or for good), after ending the
    /// programs with the process ids in `end`.
    pub(super) fn delete_in_background(
        &mut self,
        paths: Vec<PathBuf>,
        to_trash: bool,
        end: Vec<u32>,
    ) {
        let dir = paths
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        let undo = self.undo.recorder();
        self.in_background(move || {
            let notes: Vec<String> = end
                .iter()
                .filter_map(|&pid| {
                    locks::kill(pid)
                        .err()
                        .map(|e| ox_core::trf!("Couldn't end process {pid}: {e}", pid = pid, e = e))
                })
                .collect();
            let since = ox_core::undo::now();
            let failed = fsops::delete_each(&paths, to_trash);
            if to_trash {
                undo.trashed(&paths, since);
            }
            if failed.is_empty() && notes.is_empty() {
                return Notice::Changed(dir.into_iter().collect(), None);
            }
            let items = failed
                .into_iter()
                .map(|(path, error)| Failed {
                    lockers: locks::lockers(std::slice::from_ref(&path)),
                    path,
                    error,
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                // Deleted after all; only ending a program failed.
                return Notice::Changed(dir.into_iter().collect(), Some(notes.join("\n")));
            }
            let failed = DeleteFailed {
                items,
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
