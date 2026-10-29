//! How the file view looks: its columns.

use super::*;
use ox_core::columns::{ColumnId, ColumnLayout};

pub enum ViewAction {
    /// Change the columns of the details view.
    Columns(ColumnEdit),
}

pub enum ColumnEdit {
    /// Show a hidden column or hide a shown one.
    Toggle(ColumnId),
    /// Move a column in front of another one (to the end if `None`).
    MoveBefore(ColumnId, Option<ColumnId>),
    /// New width while an edge is dragged (saved with [`ColumnEdit::Save`]).
    Width(ColumnId, f32),
    /// The drag is over: keep the widths.
    Save,
    /// Back to the default columns.
    Reset,
}

impl ColumnEdit {
    fn apply(self, layout: &mut ColumnLayout) {
        match self {
            Self::Toggle(id) => layout.toggle(id),
            Self::MoveBefore(id, before) => layout.move_before(id, before),
            Self::Width(id, w) => layout.set_width(id, w),
            Self::Save => {}
            Self::Reset => *layout = ColumnLayout::default(),
        }
    }
}

impl OxApp {
    pub(super) fn process_view(&mut self, action: ViewAction) {
        match action {
            ViewAction::Columns(edit) => {
                let save = !matches!(edit, ColumnEdit::Width(..));
                edit.apply(&mut self.settings.browsing.columns);
                // The columns don't change the look; don't apply the settings again.
                if let Some(applied) = &mut self.applied_settings {
                    applied.browsing.columns = self.settings.browsing.columns.clone();
                }
                if save {
                    if let Err(e) = self.settings.save() {
                        self.toast(ox_core::trf!("Couldn't save settings: {e}", e = e));
                    }
                }
            }
        }
    }
}
