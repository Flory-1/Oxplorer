//! What the Git tab asks of the app (the tab runs git itself, see
//! `ui/preview/git_panel.rs`).

use super::*;

pub enum GitAction {
    /// A Git command changed the repository: read the status of the shown
    /// folders again (the Git column and the status bar).
    StatusChanged,
}

impl OxApp {
    pub(super) fn git_action(&mut self, action: GitAction) {
        match action {
            GitAction::StatusChanged => self.git_key.clear(),
        }
    }
}
