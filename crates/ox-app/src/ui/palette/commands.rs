//! Palette entries for the custom commands that fit the selection (or the
//! current folder, with nothing selected).

use super::*;
use crate::app::CommandAction;
use ox_core::commands::Target;

pub(super) fn entries(app: &OxApp, v: &mut Vec<Entry>) {
    if app.settings.commands.0.is_empty() {
        return;
    }
    let Some(items) = app.command_selection() else {
        return;
    };
    let folder = app.current_folder();
    let target = if items.is_empty() {
        if folder.is_none() {
            return;
        }
        Target::Background
    } else {
        Target::Items(&items)
    };
    let paths: Vec<std::path::PathBuf> = items.iter().map(|(p, _)| p.clone()).collect();
    for (index, c) in app.settings.commands.0.iter().enumerate() {
        if !c.applies(target) {
            continue;
        }
        let items = paths.clone();
        let folder = folder.clone();
        v.push(Entry {
            icon: crate::ui::commands::icon(c),
            label: c.name.clone(),
            detail: c.command.clone(),
            run: act(move || {
                Action::Command(CommandAction::Run {
                    index,
                    items,
                    folder,
                })
            }),
        });
    }
}
