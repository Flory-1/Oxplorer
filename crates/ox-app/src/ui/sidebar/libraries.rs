//! The sidebar's Libraries section: the Gallery and the Windows libraries.

use super::*;

pub(super) fn show(
    app: &OxApp,
    ui: &mut Ui,
    current: &Location,
    accent: Color32,
    actions: &mut Vec<Action>,
) {
    if !app.settings.sidebar.show_libraries || !cfg!(windows) {
        return;
    }
    section(ui, tr("Libraries"));
    let gallery = item(
        ui,
        Icon::Glyph("🎞"),
        tr("Gallery"),
        None,
        *current == Location::Gallery,
        accent,
    )
    .on_hover_text(tr(
        "All pictures and videos of the Pictures library, newest first",
    ));
    open_place(&gallery, Location::Gallery, actions);
    gallery.context_menu(|ui| {
        if ui.button(tr("📑 Open in new tab")).clicked() {
            actions.push(Action::OpenInNewTab(Location::Gallery, true));
        }
    });

    for lib in app.libraries.iter().filter(|l| l.pinned) {
        let loc = Location::Library {
            name: lib.name.clone(),
            file: lib.file.clone(),
        };
        let selected = matches!(current, Location::Library { file, .. } if *file == lib.file);
        let folders: Vec<String> = lib
            .folders
            .iter()
            .map(|f| f.display().to_string())
            .collect();
        let resp = item(ui, Icon::Glyph("📚"), &lib.name, None, selected, accent)
            .on_hover_text(folders.join("\n"));
        open_place(&resp, loc.clone(), actions);
        if let Some(save) = &lib.save_folder {
            highlight_drop(
                ui,
                &resp,
                super::super::drop_target(actions, &resp, save),
                accent,
            );
        }
        resp.context_menu(|ui| {
            if ui.button(tr("📑 Open in new tab")).clicked() {
                actions.push(Action::OpenInNewTab(loc.clone(), true));
            }
            if !lib.folders.is_empty() {
                ui.separator();
                ui.label(RichText::new(tr("Folders")).weak().small());
            }
            for folder in &lib.folders {
                let mut label = format!("📁 {}", folder.display());
                if lib.save_folder.as_ref() == Some(folder) {
                    label = ox_core::trf!("{folder} (save location)", folder = label);
                }
                if ui.button(label).clicked() {
                    actions.push(Action::Navigate(Location::Folder(folder.clone())));
                }
            }
        });
    }
}

/// Click opens, middle-click opens in a new tab.
fn open_place(resp: &egui::Response, loc: Location, actions: &mut Vec<Action>) {
    if resp.clicked() {
        actions.push(Action::Navigate(loc.clone()));
    }
    if resp.middle_clicked() {
        actions.push(Action::OpenInNewTab(loc, false));
    }
}
