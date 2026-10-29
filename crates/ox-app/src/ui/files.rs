//! The file view: details list and grid, with selection, drag & drop and context menus.

mod cells;
mod column_header;
mod details;
mod grid;
mod item;
pub(crate) use cells::header_label;
pub(crate) use column_header::{column_context, column_menu};
use details::details;
use grid::grid;
use item::{item_context_menu, item_interact};
use ox_core::i18n::t as tr;
use std::collections::HashSet;
use std::path::PathBuf;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    vec2, Align2, Color32, CornerRadius, FontId, Id, Modifiers, Rect, Response, Sense, Stroke, Ui,
};
use ox_core::entry::{FileEntry, SortKey};
use ox_core::format;
use ox_core::recycle::BinListing;
use ox_core::settings::ViewMode;

use super::{DragItems, MenuTarget};
use crate::app::{Action, OxApp};
use crate::icons;
use crate::shortcuts::{self, Command};
use crate::tabs::{FolderView, Location};

const TILE: egui::Vec2 = egui::Vec2::new(108.0, 104.0);

fn git_color(state: ox_core::git::GitState, v: &egui::Visuals) -> Color32 {
    use ox_core::git::GitState::*;
    match state {
        Conflicted | Deleted => Color32::from_rgb(0xe5, 0x48, 0x4d),
        Modified => Color32::from_rgb(0xf0, 0x8a, 0x24),
        Added => Color32::from_rgb(0x3b, 0xb2, 0x73),
        Renamed => Color32::from_rgb(0x4c, 0x8d, 0xf6),
        Untracked => Color32::from_rgb(0x9b, 0x6b, 0xf2),
        Ignored => v.weak_text_color().gamma_multiply(0.7),
    }
}

/// Everything the rows need besides the tab itself.
struct Ctx<'a> {
    show_ext: bool,
    date_format: &'a str,
    single_click: bool,
    row_h: f32,
    accent: Color32,
    selection_fill: Color32,
    is_search: bool,
    is_bin: bool,
    /// Inside an archive: items can be opened and copied (extracted) only.
    read_only: bool,

    /// Shell icons (Windows), or `None` to paint the built-in icons.
    icons: Option<&'a crate::icons::IconCache>,
    /// Picture and video previews in the grid, or `None` when they are off.
    thumbs: Option<&'a crate::thumbs::Thumbnails>,
    /// Values of the extra columns, or `None` where files can't be read.
    props: Option<&'a crate::column_values::ColumnValues>,
    /// Trash items behind the entries when showing the Recycle Bin.
    bin: &'a BinListing,
    tags: &'a ox_core::tags::TagStore,
    /// Git status when the folder is in a repository.
    git: Option<&'a ox_core::git::RepoStatus>,
    /// Compare folders: how the items compare with the other pane's.
    compare: Option<&'a std::collections::HashMap<PathBuf, ox_core::compare::Mark>>,
    /// Folder sizes for the Size column (`None` when they are off).
    sizes: Option<&'a ox_core::dirsize::FolderSizes>,
    /// Root of the current search; result folders are shown relative to it.
    search_root: Option<PathBuf>,
    folder: Option<PathBuf>,
    cut: HashSet<PathBuf>,
    favorites: HashSet<PathBuf>,
    destinations: Vec<(String, PathBuf)>,
    can_paste: bool,
    /// Transfers are running, so "Paste after current transfers" makes sense.
    transfers_running: bool,
    /// The user's entries of the "New" menu.
    new_items: Vec<ox_core::settings::NewItem>,
    /// Developer entries in the menus.
    dev: bool,
    modifiers: Modifiers,
}

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let dark = ui.visuals().dark_mode;
    let folder = app.current_folder();
    // Borrowed by the rows while the tab is mutably borrowed; put back at the end.
    let bin = std::mem::take(&mut app.tabs.active_mut().view.bin);
    let git = app.tabs.active().view.git.clone();
    let tab_view_paths: Vec<PathBuf> = app
        .tabs
        .active()
        .view
        .entries
        .iter()
        .map(|e| e.path.clone())
        .collect();
    let cx = Ctx {
        show_ext: app.settings.browsing.show_extensions,
        date_format: &app.settings.browsing.date_format.clone(),
        single_click: app.settings.browsing.single_click_open,
        row_h: crate::theme::row_height(&app.settings),
        accent: crate::theme::accent(&app.settings),
        selection_fill: crate::theme::selection_fill(&app.settings, dark),
        is_search: matches!(
            app.tabs.active().location(),
            Location::Search { .. } | Location::Tagged(_)
        ),
        tags: &app.tags,
        git: git.as_deref(),
        compare: (app.compare.as_ref()).and_then(|c| c.marks_for(app.tabs.active)),
        is_bin: app.tabs.active().location().is_bin(),
        read_only: app.tabs.active().location().is_read_only(),
        icons: app.settings.appearance.system_icons.then_some(&app.icons),
        // Bin and archive entries aren't real files the Shell can read.
        thumbs: (app.settings.appearance.thumbnails
            && !app.tabs.active().location().is_bin()
            && !app.tabs.active().location().is_read_only())
        .then_some(&app.thumbs),
        props: (!app.tabs.active().location().is_bin()
            && !app.tabs.active().location().is_read_only())
        .then_some(&app.column_values),
        bin: &bin,
        sizes: (app.settings.browsing.folder_sizes
            && matches!(app.tabs.active().location(), Location::Folder(_)))
        .then_some(&app.folder_sizes),
        search_root: match app.tabs.active().location() {
            Location::Search { root, .. } => root.clone(),
            _ => None,
        },
        cut: tab_view_paths
            .into_iter()
            .filter(|p| app.clipboard.is_cut(p))
            .collect(),
        favorites: app.quick.favorites.iter().map(|f| f.path.clone()).collect(),
        destinations: super::quick_destinations(app),
        can_paste: folder.is_some(),
        transfers_running: app.engine.has_active(),
        new_items: app.settings.new_items.0.clone(),
        dev: app.settings.developer.context_menus,
        folder,
        modifiers: ui.input(|i| i.modifiers),
    };
    let folders_first = app.settings.browsing.folders_first;
    let case = app.settings.search.case_sensitive;
    let layout = &app.settings.browsing.columns;
    let col_cx = column_context(app.tabs.active());
    let mut actions = Vec::new();
    let tab = app.tabs.active_mut();

    if let Some(err) = &tab.view.error {
        centered_message(
            ui,
            "⚠",
            &ox_core::trf!("Can't open this folder.\n{err}", err = err),
        );
        tab.view.bin = bin;
        app.actions.extend(actions);
        return;
    }

    if let Location::Search { root, query } = tab.location().clone() {
        filter_chips(ui, &root, &query, &mut actions);
    }

    // Background (empty space) interactions. Rows are added later and sit on top.
    let bg_rect = ui.available_rect_before_wrap();
    let bg = ui.interact(bg_rect, Id::unique(("files_bg", tab.id)), Sense::click());

    let mut sort_clicked = None;
    match tab.view_mode {
        ViewMode::Details => details(
            ui,
            &mut tab.view,
            tab.id,
            &cx,
            layout,
            col_cx,
            &mut actions,
            &mut sort_clicked,
            (tab.sort_key, tab.sort_order),
        ),
        ViewMode::Grid => grid(ui, &mut tab.view, tab.id, &cx, &mut actions),
    }
    if let Some(key) = sort_clicked {
        if tab.sort_key == key {
            tab.sort_order = tab.sort_order.toggled();
        } else {
            tab.sort_key = key;
        }
        tab.refresh_visible(folders_first, case);
    }

    if tab.view.visible.is_empty() {
        let searching = tab.search.as_ref().is_some_and(|s| s.done.is_none());
        if tab.view.loading.is_some() || searching {
            ui.put(
                Rect::from_center_size(bg_rect.center(), vec2(40.0, 40.0)),
                egui::Spinner::new().size(28.0),
            );
        } else if let Location::Tagged(c) = tab.location() {
            centered_message(
                ui,
                "🏷",
                &ox_core::trf!(
                    "Nothing is tagged \"{v1}\" yet.\nRight-click files or folders → Tags.",
                    v1 = cx.tags.label(*c)
                ),
            );
        } else if cx.is_search {
            centered_message(ui, "🔍", tr("No results."));
        } else if cx.is_bin && tab.filter.is_empty() {
            centered_message(ui, "🗑", tr("The Recycle Bin is empty."));
        } else if !tab.filter.is_empty() {
            centered_message(
                ui,
                "🔍",
                tr("No items match your filter.\nPress Enter to search sub-folders too."),
            );
        } else {
            centered_message(ui, "📂", tr("This folder is empty."));
        }
    }

    if bg.clicked() && !cx.modifiers.command && !cx.modifiers.shift {
        tab.view.clear_selection();
    }
    if cx.is_bin {
        let all: Vec<PathBuf> = tab.view.entries.iter().map(|e| e.path.clone()).collect();
        bg.context_menu(|ui| {
            if ui
                .add_enabled(
                    !all.is_empty(),
                    egui::Button::new(tr("♻ Restore all items")),
                )
                .clicked()
            {
                actions.push(Action::RestoreFromBin(all.clone()));
            }
            if ui
                .add_enabled(
                    !all.is_empty(),
                    egui::Button::new(tr("🗑 Empty Recycle Bin")),
                )
                .clicked()
            {
                actions.push(Action::EmptyBin);
            }
            if ui
                .add(
                    egui::Button::new(tr("🔄 Refresh"))
                        .shortcut_text(shortcuts::label(Command::Refresh)),
                )
                .clicked()
            {
                actions.push(Action::Refresh);
            }
        });
    }
    if let Location::Archive { archive, .. } = tab.location() {
        let archive = archive.clone();
        bg.context_menu(|ui| {
            if ui.button(tr("📦 Extract all to a new folder")).clicked() {
                actions.push(Action::ExtractArchive {
                    archive: archive.clone(),
                    own_folder: true,
                });
            }
            if ui.button(tr("📂 Open the archive's folder")).clicked() {
                if let Some(parent) = archive.parent() {
                    actions.push(Action::Navigate(Location::Folder(parent.to_path_buf())));
                }
            }
        });
    }
    if let Some(folder) = &cx.folder {
        let pinned = cx.favorites.contains(folder);
        bg.context_menu(|ui| {
            super::background_menu(
                ui,
                folder,
                cx.can_paste,
                cx.transfers_running,
                pinned,
                &cx.new_items,
                cx.dev,
                &mut actions,
            )
        });
        if super::drop_target(&mut actions, &bg, folder) {
            ui.painter().rect_stroke(
                bg_rect.shrink(2.0),
                CornerRadius::same(6),
                Stroke::new(2.0, cx.accent),
                egui::StrokeKind::Inside,
            );
        }
    }
    tab.view.bin = bin;
    app.actions.extend(actions);
}

/// Quick filters above search results; each chip adds or removes a query token.
fn filter_chips(ui: &mut Ui, root: &Option<PathBuf>, query: &str, actions: &mut Vec<Action>) {
    const KIND: &[&str] = &["kind:", "type:"];
    const DATE: &[&str] = &["modified:", "date:"];
    const SIZE: &[&str] = &["size:"];
    let chips: [(&str, &str, u8, &[&str]); 12] = [
        (tr("📁 Folders"), "type:folder", 0, KIND),
        (tr("🖼 Images"), "kind:image", 0, KIND),
        (tr("🎬 Videos"), "kind:video", 0, KIND),
        (tr("🎵 Audio"), "kind:audio", 0, KIND),
        (tr("📄 Documents"), "kind:document", 0, KIND),
        (tr("📦 Archives"), "kind:archive", 0, KIND),
        (tr("Today"), "modified:today", 1, DATE),
        (tr("This week"), "modified:week", 1, DATE),
        (tr("This month"), "modified:month", 1, DATE),
        ("> 10 MB", "size:>10mb", 2, SIZE),
        ("> 100 MB", "size:>100mb", 2, SIZE),
        ("> 1 GB", "size:>1gb", 2, SIZE),
    ];
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let mut last_group = 0;
                for (label, token, group_id, group) in chips {
                    if group_id != last_group {
                        ui.separator();
                        last_group = group_id;
                    }
                    let on = ox_core::search::has_filter(query, token);
                    let chip = egui::Button::new(label)
                        .selected(on)
                        .corner_radius(CornerRadius::same(12));
                    if ui.add(chip).clicked() {
                        let q = ox_core::search::toggle_filter(query, token, group);
                        actions.push(Action::SearchLive(root.clone(), q));
                    }
                }
                ui.separator();
                if ui
                    .button(tr("☆ Save search"))
                    .on_hover_text(tr("Keep this search in the sidebar"))
                    .clicked()
                {
                    actions.push(Action::SaveSearch(root.clone(), query.to_owned()));
                }
            });
        });
}

fn centered_message(ui: &mut Ui, icon: &str, text: &str) {
    let rect = ui.available_rect_before_wrap();
    let rect = if rect.height() < 100.0 {
        ui.max_rect()
    } else {
        rect
    };
    let c = rect.center();
    let v = ui.visuals();
    ui.painter().text(
        c - vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(36.0),
        v.weak_text_color(),
    );
    ui.painter().text(
        c + vec2(0.0, 14.0),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(14.0),
        v.weak_text_color(),
    );
}
/// Scroll so that the item at `y` is visible, using last frame's offset.
fn scroll_into_view(
    ui: &Ui,
    area: egui::ScrollArea,
    key: (&str, u64),
    y: f32,
    h: f32,
) -> egui::ScrollArea {
    let (offset, view_h) = ui
        .data(|d| d.get_temp::<(f32, f32)>(Id::unique(key)))
        .unwrap_or((0.0, 400.0));
    if y < offset {
        area.vertical_scroll_offset(y)
    } else if y + h > offset + view_h {
        area.vertical_scroll_offset(y + h - view_h)
    } else {
        area
    }
}

fn remember_scroll(ui: &Ui, key: (&str, u64), offset: f32, height: f32) {
    ui.data_mut(|d| d.insert_temp(Id::unique(key), (offset, height)));
}
