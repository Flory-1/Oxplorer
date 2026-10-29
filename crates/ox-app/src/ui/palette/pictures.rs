//! Palette entries for the picture tools, when every selected item is a picture.

use super::*;
use crate::app::PictureAction;
use crate::ui::picture_tools::{all_pictures, icon};
use ox_core::picture_tools::Transform;

pub(super) fn entries(app: &OxApp, v: &mut Vec<Entry>) {
    let tab = app.tabs.active();
    if tab.location().is_bin() || tab.location().is_read_only() {
        return;
    }
    let sel = tab.view.selected_paths();
    if !all_pictures(&sel) {
        return;
    }
    let detail = ox_core::i18n::count(sel.len() as u64, "picture");
    for tf in Transform::ALL {
        let paths = sel.clone();
        v.push(Entry {
            icon: icon(tf),
            label: tf.label().to_owned(),
            detail: detail.clone(),
            run: act(move || Action::Pictures(PictureAction::Transform(paths, tf))),
        });
    }
    for (icon, label, resize) in [
        ("🔁", tr("Convert pictures to…"), false),
        ("🖼", tr("Resize pictures…"), true),
    ] {
        let paths = sel.clone();
        v.push(Entry {
            icon,
            label: label.to_owned(),
            detail: detail.clone(),
            run: act(move || Action::Pictures(PictureAction::Dialog(paths, resize))),
        });
    }
}
