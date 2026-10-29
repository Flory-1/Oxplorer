//! Header command bar: file actions, sort and view buttons.

use super::*;

pub(super) fn command_bar(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    let folder = app.current_folder();
    let tab = app.tabs.active();
    let selection = tab.view.selected_paths();
    let shows_files = tab.location().shows_files();
    let (key, order, mode) = (tab.sort_key, tab.sort_order, tab.view_mode);
    let (flat, can_flat) = (tab.is_flat(), tab.can_flat());
    let has_sel = !selection.is_empty() && shows_files;
    // The system clipboard is only read on paste, so with it enabled Paste is always offered.
    let can_paste = folder.is_some()
        && (app.settings.transfer.use_system_clipboard || app.clipboard.get(false).is_some());
    let pinned = folder.as_deref().is_some_and(|f| app.quick.is_favorite(f));
    let in_bin = tab.location().is_bin();
    let bin_empty = tab.view.entries.is_empty();
    // Where the separator after the edit buttons is: the sidebar's edge.
    let mut sidebar_edge = None;

    ui.horizontal(|ui| {
        let btn = |ui: &mut Ui, enabled: bool, text: &str, tip: &str| {
            ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(text).size(13.5)).frame(false),
            )
            .on_hover_text(tip)
            .on_disabled_hover_text(tip)
            .clicked()
        };
        ui.add_enabled_ui(folder.is_some(), |ui| {
            ui.menu_button(RichText::new(tr("➕ New")).size(13.5), |ui| {
                if let Some(f) = &folder {
                    super::super::new_menu(ui, f, &app.settings.new_items.0, &mut actions);
                }
            });
        });
        ui.separator();
        if in_bin {
            // Wider buttons here, so lay out the edit buttons unseen to keep
            // the sidebar where it is in folders.
            let mut ghost = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("edit_buttons_ghost")
                    .max_rect(ui.available_rect_before_wrap())
                    .layout(Layout::left_to_right(Align::Center))
                    .invisible()
                    .disabled(),
            );
            for icon in ["✂", "📋", "📥", "✏", "🗑"] {
                ghost.add(egui::Button::new(RichText::new(icon).size(13.5)).frame(false));
            }
            sidebar_edge = Some(ghost.separator().rect.center().x);
            if btn(
                ui,
                has_sel,
                tr("♻ Restore"),
                tr("Put the selected items back where they were"),
            ) {
                actions.push(Action::RestoreFromBin(selection.clone()));
            }
            if btn(
                ui,
                has_sel,
                tr("⛔ Delete"),
                tr("Delete the selected items permanently (Del)"),
            ) {
                actions.push(Action::PurgeFromBin(selection.clone()));
            }
            if btn(
                ui,
                !bin_empty,
                tr("🗑 Empty Recycle Bin"),
                tr("Delete everything permanently"),
            ) {
                actions.push(Action::EmptyBin);
            }
        } else {
            if btn(ui, has_sel, "✂", &shortcuts::hint(tr("Cut"), Command::Cut)) {
                actions.push(Action::CutItems(selection.clone()));
            }
            if btn(
                ui,
                has_sel,
                "📋",
                &shortcuts::hint(tr("Copy"), Command::Copy),
            ) {
                actions.push(Action::CopyItems(selection.clone()));
            }
            if btn(
                ui,
                can_paste,
                "📥",
                &shortcuts::hint(tr("Paste"), Command::Paste),
            ) {
                if let Some(f) = &folder {
                    actions.push(Action::Paste(f.clone()));
                }
            }
            if btn(
                ui,
                has_sel,
                "✏",
                &ox_core::trf!(
                    "{v1} — several items: bulk rename",
                    v1 = shortcuts::hint(tr("Rename"), Command::Rename)
                ),
            ) {
                if selection.len() == 1 {
                    actions.push(Action::Rename(selection[0].clone()));
                } else {
                    actions.push(Action::BulkRename(selection.clone()));
                }
            }
            if btn(ui, has_sel, "🗑", tr("Delete (Del)")) {
                actions.push(Action::Delete(selection.clone(), false));
            }
        }
        let separator = ui.separator();
        if !in_bin {
            sidebar_edge = Some(separator.rect.center().x);
        }

        ui.add_enabled_ui(shows_files, |ui| {
            ui.menu_button(RichText::new(tr("⇅ Sort")).size(13.5), |ui| {
                let mut new_key = key;
                let mut new_order = order;
                for k in SortKey::ALL {
                    ui.radio_value(&mut new_key, k, tr(k.label()));
                }
                ui.separator();
                ui.radio_value(&mut new_order, SortOrder::Ascending, tr("Ascending"));
                ui.radio_value(&mut new_order, SortOrder::Descending, tr("Descending"));
                ui.separator();
                let mut ff = app.settings.browsing.folders_first;
                if ui.checkbox(&mut ff, tr("Folders first")).changed() {
                    app.settings.browsing.folders_first = ff;
                    actions.push(Action::SettingsChanged);
                }
                ui.separator();
                let group = app.tabs.active().group_by;
                ui.menu_button(tr("Group by"), |ui| {
                    crate::ui::files::group_by_menu(ui, group, &mut actions);
                });
                if new_key != key || new_order != order {
                    let folders_first = app.settings.browsing.folders_first;
                    let case = app.settings.search.case_sensitive;
                    let t = app.tabs.active_mut();
                    t.sort_key = new_key;
                    t.sort_order = new_order;
                    t.refresh_visible(folders_first, case);
                }
            });
            ui.menu_button(RichText::new(tr("👁 View")).size(13.5), |ui| {
                let mut m = mode;
                ui.radio_value(&mut m, ViewMode::Details, tr("☰ Details"));
                ui.radio_value(&mut m, ViewMode::Grid, tr("▦ Grid"));
                if m != mode {
                    app.tabs.active_mut().view_mode = m;
                }
                let mut on = flat;
                let flat_box = ui
                    .add_enabled(
                        can_flat,
                        egui::Checkbox::new(
                            &mut on,
                            shortcuts::hint(tr("Flat view"), Command::FlatView),
                        ),
                    )
                    .on_hover_text(tr(
                        "Every file in this folder and its sub-folders in one list",
                    ));
                if flat_box.changed() {
                    actions.push(Action::ToggleFlatView);
                }
                ui.add_enabled_ui(m == ViewMode::Details, |ui| {
                    ui.menu_button(tr("Columns"), |ui| {
                        crate::ui::files::column_menu(
                            ui,
                            app.columns_of(app.tabs.active()),
                            crate::ui::files::column_context(app.tabs.active()),
                            in_bin,
                            &mut actions,
                        );
                    });
                });
                super::super::files::folder_view_menu(ui, app, &mut actions);
                ui.separator();
                let b = &mut app.settings.browsing;
                let mut changed = ui
                    .checkbox(&mut b.show_hidden, tr("Hidden items"))
                    .changed();
                changed |= ui
                    .checkbox(&mut b.show_extensions, tr("File name extensions"))
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut app.settings.appearance.compact_rows,
                        tr("Compact rows"),
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut app.settings.browsing.show_status_bar, tr("Status bar"))
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut app.settings.browsing.show_preview,
                        shortcuts::hint(tr("Preview pane"), Command::TogglePreview),
                    )
                    .changed();
                if changed {
                    actions.push(Action::SettingsChanged);
                }
                let mut terminal = app.ui.show_terminal;
                if ui
                    .checkbox(
                        &mut terminal,
                        shortcuts::hint(tr("Terminal panel"), Command::ToggleTerminal),
                    )
                    .changed()
                {
                    actions.push(Action::ToggleTerminal);
                }
                let mut split = app.tabs.panes.is_some();
                if ui
                    .checkbox(
                        &mut split,
                        shortcuts::hint(tr("Split view"), Command::ToggleSplit),
                    )
                    .changed()
                {
                    actions.push(Action::Tab(TabAction::ToggleSplit));
                }
            });
            let tip = shortcuts::hint(tr("Flat view"), Command::FlatView);
            let flat_button = egui::Button::new(RichText::new("🗃").size(13.5))
                .frame(flat)
                .selected(flat);
            if ui
                .add_enabled(can_flat, flat_button)
                .on_hover_text(&tip)
                .on_disabled_hover_text(&tip)
                .clicked()
            {
                actions.push(Action::ToggleFlatView);
            }
        });
        ui.separator();
        if let Some(f) = &folder {
            let (text, tip) = if pinned {
                ("★", tr("Unpin from Favorites"))
            } else {
                ("☆", tr("Pin to Favorites"))
            };
            if btn(ui, true, text, tip) {
                actions.push(if pinned {
                    Action::Unpin(f.clone())
                } else {
                    Action::Pin(f.clone())
                });
            }
            if btn(ui, true, "🖳", tr("Open in Terminal")) {
                actions.push(Action::OpenTerminal(f.clone()));
            }
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if btn(
                ui,
                true,
                "⚙",
                &shortcuts::hint(tr("Settings"), Command::Settings),
            ) {
                actions.push(Action::OpenInNewTab(Location::Settings, true));
            }
            if keyboard_button(ui)
                .on_hover_text(shortcuts::hint(tr("Command palette"), Command::Palette))
                .clicked()
            {
                app.ui.palette = Some(Default::default());
            }
            if btn(ui, true, "◧", tr("Toggle sidebar")) {
                app.settings.sidebar.visible = !app.settings.sidebar.visible;
                actions.push(Action::SettingsChanged);
            }
            if btn(
                ui,
                true,
                "◫",
                &shortcuts::hint(tr("Split view"), Command::ToggleSplit),
            ) {
                actions.push(Action::Tab(TabAction::ToggleSplit));
            }
            let jobs = app.engine.jobs().len();
            if jobs > 0 {
                let active = app.engine.has_active();
                let label = if active {
                    ox_core::trf!("⇄ Transfers ({jobs})", jobs = jobs)
                } else {
                    ox_core::trf!("✔ Transfers ({jobs})", jobs = jobs)
                };
                if ui
                    .add(egui::Button::new(label).selected(app.ui.show_transfers))
                    .clicked()
                {
                    app.ui.show_transfers = !app.ui.show_transfers;
                }
            }
        });
    });
    app.ui.sidebar_edge = sidebar_edge;
    app.actions.extend(actions);
}

/// The command palette button: a small drawn keyboard, as wide as the
/// symbols next to it (the ⌨ glyph of Windows' emoji font is much wider).
fn keyboard_button(ui: &mut Ui) -> egui::Response {
    // The symbols next to it are drawn wider than the space their font
    // reserves, so reserve the same narrow slot to get the same gaps, and
    // click on the whole icon.
    let (slot, _) =
        ui.allocate_exact_size(vec2(11.0, ui.spacing().interact_size.y), Sense::hover());
    let hit = Rect::from_center_size(slot.center(), vec2(15.0, slot.height()));
    let resp = ui.interact(
        hit,
        egui::Id::unique("command_palette_button"),
        Sense::click(),
    );
    let color = ui.style().interact(&resp).text_color();
    let p = ui.painter();
    // Same size as the ◫ / ◧ symbols next to it, on whole pixels so the
    // keys stay crisp.
    let ppp = ui.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let min = egui::pos2(snap(slot.center().x - 5.5), snap(slot.center().y - 5.5));
    let body = Rect::from_min_size(min, vec2(11.0, 11.0));
    p.rect_stroke(
        body,
        CornerRadius::same(2),
        Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
    // Two rows of three keys, and the space bar: a plain mesh, because egui
    // softens the edges of shapes by half a pixel and 1-pixel keys would fade.
    let mut mesh = egui::Mesh::default();
    let mut px = |x: f32, y: f32, w: f32| {
        mesh.add_colored_rect(Rect::from_min_size(min + vec2(x, y), vec2(w, 1.0)), color);
    };
    for y in [3.0, 5.0] {
        for x in [3.0, 5.0, 7.0] {
            px(x, y, 1.0);
        }
    }
    px(3.0, 7.0, 5.0);
    p.add(mesh);
    resp
}
