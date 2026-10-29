//! Palette entries for the look of the file view: columns, the folder's
//! view and grouping.

use super::*;
use crate::app::{ColumnEdit, ViewAction};
use ox_core::columns::ColumnId;
use ox_core::grouping::GroupBy;

pub(super) fn entries(app: &OxApp, v: &mut Vec<Entry>) {
    let tab = app.tabs.active();
    if !tab.location().shows_files() {
        return;
    }
    let mut cmd = |icon, label: String, run: Run| {
        v.push(Entry {
            icon,
            label,
            detail: String::new(),
            run,
        })
    };
    let layout = app.columns_of(tab);
    let col_cx = crate::ui::files::column_context(tab);
    let is_bin = tab.location().is_bin();
    for id in ColumnId::ALL {
        if id == ColumnId::Name || !id.available(col_cx) {
            continue;
        }
        let name = crate::ui::files::header_label(id, is_bin);
        let label = if layout.is_shown(id) {
            ox_core::trf!("Hide column: {name}", name = name)
        } else {
            ox_core::trf!("Show column: {name}", name = name)
        };
        cmd(
            "☰",
            label,
            act(move || Action::View(ViewAction::Columns(ColumnEdit::Toggle(id)))),
        );
    }
    cmd(
        "☰",
        tr("Reset columns").to_owned(),
        act(|| Action::View(ViewAction::Columns(ColumnEdit::Reset))),
    );
    if let Some(folder) = app.remembered_folder() {
        if app.folder_views.get(&folder).is_some() {
            cmd(
                "👁",
                tr("Reset view of this folder").to_owned(),
                act(|| Action::View(ViewAction::ResetFolder)),
            );
        }
    }
    cmd(
        "👁",
        tr("Use this view for all folders").to_owned(),
        act(|| Action::View(ViewAction::MakeDefault)),
    );
    for by in GroupBy::ALL {
        if by != tab.group_by {
            let label = ox_core::trf!("Group by: {name}", name = tr(by.label()));
            cmd(
                "☰",
                label,
                act(move || Action::View(ViewAction::GroupBy(by))),
            );
        }
    }
    if tab.group_by != GroupBy::None {
        cmd(
            "⏵",
            tr("Collapse all groups").to_owned(),
            act(|| Action::View(ViewAction::CollapseAll(true))),
        );
        cmd(
            "⏷",
            tr("Expand all groups").to_owned(),
            act(|| Action::View(ViewAction::CollapseAll(false))),
        );
    }
}
