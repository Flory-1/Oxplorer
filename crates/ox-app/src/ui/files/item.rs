//! Per-item selection, opening, drag and drop and the context menu.

use super::*;

/// Selection, opening and drag & drop for one item. Returns true while files hover over it.
pub(super) fn item_interact(
    view: &mut FolderView,
    i: usize,
    e: &FileEntry,
    resp: &Response,
    cx: &Ctx<'_>,
    actions: &mut Vec<Action>,
) -> bool {
    let m = cx.modifiers;
    if cx.is_bin {
        // Deleted items can only be selected, restored or purged.
        if resp.clicked() {
            if m.command {
                view.toggle(i);
            } else if m.shift {
                view.select_range(i, false);
            } else {
                view.select_only(i);
            }
        }
        if resp.secondary_clicked() && !view.selection.contains(&e.path) {
            view.select_only(i);
        }
        return false;
    }
    if resp.clicked() {
        if m.command {
            view.toggle(i);
        } else if m.shift {
            view.select_range(i, false);
        } else {
            view.select_only(i);
            if cx.single_click {
                actions.push(Action::Open(e.path.clone()));
            }
        }
    }
    if resp.double_clicked() && !cx.single_click {
        actions.push(Action::Open(e.path.clone()));
    }
    if resp.secondary_clicked() && !view.selection.contains(&e.path) {
        view.select_only(i);
    }
    if resp.middle_clicked() && e.is_dir {
        actions.push(Action::OpenInNewTab(
            Location::Folder(e.path.clone()),
            false,
        ));
    }
    if resp.drag_started() {
        if !view.selection.contains(&e.path) {
            view.select_only(i);
        }
        resp.dnd_set_drag_payload(DragItems(view.selected_paths()));
    }
    if e.is_dir {
        super::super::drop_target(actions, resp, &e.path)
    } else {
        false
    }
}

pub(super) fn item_context_menu(
    resp: &Response,
    view: &FolderView,
    e: &FileEntry,
    cx: &Ctx<'_>,
    actions: &mut Vec<Action>,
) {
    resp.context_menu(|ui| {
        let selection = view.selected_paths();
        if cx.is_bin {
            if ui.button(tr("♻ Restore")).clicked() {
                actions.push(Action::RestoreFromBin(selection.clone()));
            }
            if ui
                .add(
                    egui::Button::new(tr("⛔ Delete permanently"))
                        .shortcut_text(shortcuts::label(Command::Delete)),
                )
                .clicked()
            {
                actions.push(Action::PurgeFromBin(selection.clone()));
            }
            ui.separator();
            if ui.button(tr("🔗 Copy original location")).clicked() {
                let originals = selection
                    .iter()
                    .filter_map(|p| cx.bin.get(p))
                    .map(|i| i.original_path())
                    .collect();
                actions.push(Action::CopyPathText(originals));
            }
            if let Some(parent) = cx.bin.get(&e.path).map(|i| i.original_parent.clone()) {
                if ui.button(tr("📁 Open original folder")).clicked() {
                    actions.push(Action::Navigate(Location::Folder(parent)));
                }
            }
            return;
        }
        let single_dir = (selection.len() == 1 && e.is_dir).then_some(e.path.as_path());
        if (cx.is_search || cx.flat) && ui.button(tr("📁 Open file location")).clicked() {
            actions.push(Action::ShowInFolder(e.path.clone()));
        }
        let target = MenuTarget {
            selection: &selection,
            read_only: cx.read_only,
            tags: cx.tags,
            single_dir,
            pinned: cx.favorites.contains(&e.path),
            can_paste: cx.can_paste || e.is_dir,
            destinations: &cx.destinations,
            shell: ox_core::shell::supported(),
            dev: cx.dev,
        };
        super::super::item_menu(ui, &target, actions);
    });
}
