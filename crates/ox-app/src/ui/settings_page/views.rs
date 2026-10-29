//! Settings → Browsing: remembering the view of each folder.

use egui::Ui;
use ox_core::i18n::t as tr;

use super::{row, toggle};
use crate::app::{Action, OxApp, ViewAction};

pub(super) fn folder_views(ui: &mut Ui, app: &mut OxApp) {
    toggle(
        ui,
        &mut app.settings.browsing.remember_folder_views,
        tr("Remember the view of each folder"),
        tr("A folder opens with the view, sort order and columns it was last shown with. Other folders use the default view and sort."),
    );
    let n = app.folder_views.len();
    row(ui, tr("Folders with a view of their own"), "", |ui| {
        ui.label(n.to_string());
        if ui
            .add_enabled(n > 0, egui::Button::new(tr("Forget all")))
            .clicked()
        {
            app.push(Action::View(ViewAction::ForgetAll));
        }
    });
}
