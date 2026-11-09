//! Settings → Browsing: remembering the view of each folder, and the list of
//! folders with a view of their own.

use std::path::PathBuf;

use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};
use ox_core::entry::{display_name, SortOrder};
use ox_core::folder_views::SavedView;
use ox_core::grouping::GroupBy;
use ox_core::i18n::t as tr;
use ox_core::settings::ViewMode;

use super::{row, toggle};
use crate::app::{Action, OxApp, ViewAction};
use crate::tabs::Location;

const ROW_H: f32 = 22.0;
/// The list shows this many rows before it scrolls.
const MAX_ROWS: usize = 12;

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
    if n > 0 {
        let actions = folder_list(ui, app);
        app.actions.extend(actions);
        ui.add_space(10.0);
    }
}

/// One line of the list, taken out of the store so the table can be drawn
/// while `app` is borrowed elsewhere.
struct Line {
    path: PathBuf,
    location: Location,
    view: SavedView,
}

/// Every folder with a view of its own and how it is shown. Clicking a
/// folder opens it; ✖ forgets its view.
fn folder_list(ui: &mut Ui, app: &OxApp) -> Vec<Action> {
    let lines: Vec<Line> = app
        .folder_views
        .list()
        .into_iter()
        .map(|(path, view)| Line {
            path: path.to_path_buf(),
            location: Location::search_root(path.to_path_buf()),
            view: view.clone(),
        })
        .collect();
    let mut actions = Vec::new();
    let height = (lines.len().min(MAX_ROWS) as f32 + 1.0) * ROW_H + 8.0;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        TableBuilder::new(ui)
            .id_salt("folder_views")
            .striped(true)
            .min_scrolled_height(0.0)
            .max_scroll_height(height)
            .column(Column::remainder().at_least(160.0).clip(true))
            .column(Column::auto().at_least(70.0))
            .column(Column::auto().at_least(110.0))
            .column(Column::auto().at_least(70.0))
            .column(Column::auto().at_least(60.0))
            .column(Column::exact(24.0))
            .header(ROW_H, |mut header| {
                let titles = [
                    tr("Folder"),
                    tr("View"),
                    tr("Sort by"),
                    tr("Group by"),
                    tr("Columns"),
                ];
                for title in titles {
                    header.col(|ui| {
                        ui.label(RichText::new(title).strong());
                    });
                }
                header.col(|_| {});
            })
            .body(|body| {
                body.rows(ROW_H, lines.len(), |mut row| {
                    let line = &lines[row.index()];
                    let v = &line.view;
                    row.col(|ui| {
                        let name = format!("{}  {}", line.location.icon(), location_name(line));
                        let link = ui.add(egui::Button::new(name).frame(false).truncate());
                        if link
                            .on_hover_text(line.path.display().to_string())
                            .clicked()
                        {
                            actions.push(Action::OpenInNewTab(line.location.clone(), true));
                        }
                    });
                    row.col(|ui| {
                        ui.label(match v.mode {
                            ViewMode::Details => tr("☰ Details"),
                            ViewMode::Grid => tr("▦ Grid"),
                        });
                    });
                    row.col(|ui| {
                        let arrow = match v.order {
                            SortOrder::Ascending => "⏶",
                            SortOrder::Descending => "⏷",
                        };
                        ui.label(format!("{} {arrow}", tr(v.sort.label())));
                    });
                    row.col(|ui| {
                        ui.label(match v.group {
                            GroupBy::None => "–",
                            g => tr(g.label()),
                        });
                    });
                    row.col(|ui| {
                        let own = v.columns.as_ref();
                        let label = ui.label(if own.is_some() {
                            tr("Own")
                        } else {
                            tr("Default")
                        });
                        if let Some(c) = own {
                            let names: Vec<&str> =
                                c.shown.iter().map(|id| tr(id.label())).collect();
                            label.on_hover_text(names.join(", "));
                        }
                    });
                    row.col(|ui| {
                        if ui
                            .small_button("✖")
                            .on_hover_text(tr("Forget the view of this folder"))
                            .clicked()
                        {
                            actions.push(Action::View(ViewAction::Forget(line.path.clone())));
                        }
                    });
                });
            });
    });
    actions
}

/// A folder's name, or a library's.
fn location_name(line: &Line) -> String {
    match &line.location {
        Location::Library { name, .. } => name.clone(),
        _ => display_name(&line.path),
    }
}
