//! Quick Look: a large preview of the selected item over the window
//! (Space). The arrow keys go through the items (see `app/quick_look.rs`);
//! Space, Esc or a click beside it closes it.

use egui::{Context, Id, RichText};
use ox_core::i18n::t as tr;

use super::preview::{self, PreviewState};
use crate::app::{Action, OxApp};
use crate::tabs::{FolderView, TabId};

pub struct QuickLook {
    /// Its own preview, so the pane keeps its state (and the two players
    /// don't get mixed up).
    pub preview: PreviewState,
    /// The tab it was opened in; it closes when another one is shown.
    pub tab: TabId,
}

impl QuickLook {
    pub fn new(tab: TabId) -> Self {
        Self {
            preview: PreviewState::for_quick_look(),
            tab,
        }
    }
}

/// The item Quick Look shows (position in the view): the one at the
/// keyboard cursor if it is selected, otherwise the first selected one.
pub fn current(view: &FolderView) -> Option<usize> {
    let selected = |i: usize| {
        view.visible_entry(i)
            .is_some_and(|e| view.selection.contains(&e.path))
    };
    view.cursor
        .filter(|&c| selected(c))
        .or_else(|| (0..view.visible.len()).find(|&i| selected(i)))
}

pub fn show(app: &mut OxApp, ctx: &Context) {
    let Some(mut ql) = app.quick_look.take() else {
        return;
    };
    let tab = app.tabs.active();
    let shown = (tab.id == ql.tab && tab.location().shows_files())
        .then(|| current(&tab.view))
        .flatten()
        .and_then(|i| Some((i, tab.view.visible_entry(i)?.clone())));
    let Some((index, entry)) = shown else {
        // Nothing selected any more, or another tab or place.
        ql.preview.release(ctx);
        return;
    };
    let count = tab.view.visible.len();
    let total = entry
        .is_dir
        .then(|| app.folder_sizes.get(&entry.path))
        .flatten();
    let date_fmt = app.settings.browsing.date_format.clone();
    let show_hidden = app.settings.browsing.show_hidden;
    let screen = ctx.content_rect();
    let size = egui::vec2(
        (screen.width() * 0.9).min(screen.width() - 40.0),
        (screen.height() * 0.88).min(screen.height() - 40.0),
    )
    .max(egui::vec2(200.0, 150.0));
    let mut actions = Vec::new();
    let mut close = false;
    let modal = egui::Modal::new(Id::unique("quick_look"))
        .frame(egui::Frame::popup(&ctx.global_style()).inner_margin(12))
        .show(ctx, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&entry.name).strong().size(16.0));
                ui.label(RichText::new(entry.type_label()).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("✖")
                        .on_hover_text(tr("Close (Space or Esc)"))
                        .clicked()
                    {
                        close = true;
                    }
                    let open = if entry.is_dir {
                        tr("📂 Open")
                    } else {
                        tr("📂 Open with default app")
                    };
                    if ui.button(open).clicked() {
                        actions.push(Action::Open(entry.path.clone()));
                        close = true;
                    }
                    ui.label(
                        RichText::new(format!("{} / {count}", index + 1))
                            .weak()
                            .monospace(),
                    )
                    .on_hover_text(tr("Arrow keys: the previous or next item"));
                });
            });
            ui.separator();
            preview::show_large(
                ui,
                &mut ql.preview,
                &entry,
                show_hidden,
                total,
                &date_fmt,
                &mut actions,
            );
        });
    if close || modal.should_close() {
        ql.preview.release(ctx);
    } else {
        app.quick_look = Some(ql);
    }
    app.actions.extend(actions);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ox_core::entry::FileEntry;

    #[test]
    fn shows_the_cursor_item_or_the_first_selected() {
        let dir = tempfile::tempdir().unwrap();
        let entries: Vec<FileEntry> = ["a", "b", "c"]
            .iter()
            .map(|n| {
                let p = dir.path().join(n);
                std::fs::write(&p, b"").unwrap();
                FileEntry::from_path(&p).unwrap()
            })
            .collect();
        let mut view = FolderView {
            visible: vec![0, 1, 2],
            entries,
            ..Default::default()
        };
        assert_eq!(current(&view), None);
        view.select_only(1);
        assert_eq!(current(&view), Some(1));
        view.toggle(2);
        assert_eq!(current(&view), Some(2));
        // The cursor on an item that isn't selected: the first selected one.
        view.toggle(2);
        assert_eq!(current(&view), Some(1));
    }
}
