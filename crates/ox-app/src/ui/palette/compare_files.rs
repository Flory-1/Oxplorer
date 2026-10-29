//! Palette entry for comparing two text files: two selected in this tab,
//! or one selected in each pane of the split view.

use super::*;

pub(super) fn entries(app: &OxApp, v: &mut Vec<Entry>) {
    let Some((old, new)) = crate::ui::text_compare::pair(app) else {
        return;
    };
    let detail = format!("{} ↔ {}", display_name(&old), display_name(&new));
    v.push(Entry {
        icon: "⇄",
        label: tr("Compare these two files").to_owned(),
        detail,
        run: act(move || Action::CompareTextFiles(old, new)),
    });
}
