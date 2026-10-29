//! The file view: details list and grid, with selection, drag & drop and context menus.

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

const DATE_W: f32 = 140.0;
const TYPE_W: f32 = 150.0;
const SIZE_W: f32 = 90.0;
const FOLDER_W: f32 = 260.0;
const GIT_W: f32 = 44.0;
const TILE: egui::Vec2 = egui::Vec2::new(108.0, 104.0);
const MIN_NAME_W: f32 = 180.0;

/// Widths of the details columns (0 = hidden). Narrow views (e.g. a split
/// pane) drop Type first, then shrink or drop Folder, then drop Date.
#[derive(Clone, Copy)]
struct Columns {
    name: f32,
    folder: f32,
    git: f32,
    date: f32,
    kind: f32,
    size: f32,
}

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

impl Columns {
    fn fit(width: f32, with_folder: bool, with_git: bool) -> Self {
        let avail = width - 16.0;
        let mut c = Columns {
            name: 0.0,
            folder: if with_folder { FOLDER_W } else { 0.0 },
            git: if with_git { GIT_W } else { 0.0 },
            date: DATE_W,
            kind: TYPE_W,
            size: SIZE_W,
        };
        let rest = |c: &Columns| avail - (c.folder + c.git + c.date + c.kind + c.size);
        if rest(&c) < MIN_NAME_W {
            c.kind = 0.0;
        }
        if rest(&c) < MIN_NAME_W && c.folder > 0.0 {
            let w = c.folder - (MIN_NAME_W - rest(&c));
            c.folder = if w >= 120.0 { w } else { 0.0 };
        }
        if rest(&c) < MIN_NAME_W {
            c.date = 0.0;
        }
        c.name = rest(&c).max(MIN_NAME_W.min(avail - c.size)).max(60.0);
        c
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
        new_items: app.settings.new_items.0.clone(),
        dev: app.settings.developer.context_menus,
        folder,
        modifiers: ui.input(|i| i.modifiers),
    };
    let folders_first = app.settings.browsing.folders_first;
    let case = app.settings.search.case_sensitive;
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
            &mut actions,
            &mut sort_clicked,
            tab.sort_key,
            tab.sort_order,
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

#[allow(clippy::too_many_arguments)]
fn details(
    ui: &mut Ui,
    view: &mut FolderView,
    tab_id: u64,
    cx: &Ctx<'_>,
    actions: &mut Vec<Action>,
    sort_clicked: &mut Option<SortKey>,
    sort_key: SortKey,
    sort_order: ox_core::entry::SortOrder,
) {
    let width = ui.available_width();
    let with_folder = cx.is_search || cx.is_bin;
    let columns = Columns::fit(width, with_folder, cx.git.is_some());

    // Column header
    let (header, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
    let v = ui.visuals().clone();
    ui.painter().line_segment(
        [header.left_bottom(), header.right_bottom()],
        v.widgets.noninteractive.bg_stroke,
    );
    let mut x = header.left() + 8.0;
    let mut cols: Vec<(&str, f32, Option<SortKey>)> =
        vec![(tr("Name"), columns.name, Some(SortKey::Name))];
    if columns.folder > 0.0 {
        let label = if cx.is_bin {
            tr("Original location")
        } else {
            tr("Folder")
        };
        cols.push((label, columns.folder, None));
    }
    let date_label = if cx.is_bin {
        tr("Date deleted")
    } else {
        tr("Date modified")
    };
    cols.extend([
        (tr("Git"), columns.git, None),
        (date_label, columns.date, Some(SortKey::Modified)),
        (tr("Type"), columns.kind, Some(SortKey::Type)),
        (tr("Size"), columns.size, Some(SortKey::Size)),
    ]);
    for (label, w, key) in cols.into_iter().filter(|c| c.1 > 0.0) {
        let rect = Rect::from_min_size(egui::pos2(x, header.top()), vec2(w, header.height()));
        let resp = ui.interact(rect, Id::unique(("col", tab_id, label)), Sense::click());
        if resp.hovered() && key.is_some() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), v.widgets.hovered.weak_bg_fill);
        }
        let arrow = match (key == Some(sort_key), sort_order) {
            (true, ox_core::entry::SortOrder::Ascending) => " ⏶",
            (true, ox_core::entry::SortOrder::Descending) => " ⏷",
            _ => "",
        };
        ui.painter().text(
            rect.left_center() + vec2(6.0, 0.0),
            Align2::LEFT_CENTER,
            format!("{label}{arrow}"),
            FontId::proportional(12.5),
            v.weak_text_color(),
        );
        if resp.clicked() {
            *sort_clicked = key;
        }
        x += w;
    }

    let n = view.visible.len();
    let row_h = cx.row_h;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("details", tab_id))
        .auto_shrink([false, false]);
    if let Some(target) = view.scroll_to.take() {
        area = scroll_into_view(ui, area, ("details", tab_id), target as f32 * row_h, row_h);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = area.show_rows(ui, row_h, n, |ui, range| {
        for i in range {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, row_h), Sense::click_and_drag());
            let Some(entry) = view.visible_entry(i).cloned() else {
                continue;
            };
            let selected = view.selection.contains(&entry.path);
            let drop_hover = item_interact(view, i, &entry, &resp, cx, actions);
            paint_row(ui, rect, &entry, selected, drop_hover, &resp, cx, columns);
            item_context_menu(&resp, view, &entry, cx, actions);
        }
    });
    remember_scroll(
        ui,
        ("details", tab_id),
        out.state.clamped_offset().y,
        out.inner_rect.height(),
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

#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &Ui,
    rect: Rect,
    e: &FileEntry,
    selected: bool,
    drop_hover: bool,
    resp: &Response,
    cx: &Ctx<'_>,
    columns: Columns,
) {
    let name_w = columns.name;
    let v = ui.visuals();
    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), cx.selection_fill);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), v.widgets.hovered.weak_bg_fill);
    }
    if drop_hover {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.5, cx.accent),
            egui::StrokeKind::Inside,
        );
    }
    super::compare::paint_mark(ui, rect, cx.compare, &e.path);
    let dim = cx.cut.contains(&e.path) || e.hidden;
    let fade = |c: Color32| if dim { c.gamma_multiply(0.5) } else { c };
    let text = fade(v.text_color());
    let weak = fade(v.weak_text_color());
    let y = rect.center().y;
    let mut x = rect.left() + 8.0;

    let p = ui.painter().with_clip_rect(Rect::from_min_max(
        rect.min,
        egui::pos2(x + name_w - 8.0, rect.max.y),
    ));
    let icon_size = (cx.row_h - 8.0).min(20.0);
    let texture = cx
        .icons
        .and_then(|c| c.texture(e, cx.is_bin || cx.read_only));
    icons::paint_entry(&p, icons::icon_rect(x + 2.0, y, icon_size), e, dim, texture);
    let name = if e.is_symlink {
        format!("{} ↗", e.display_name(cx.show_ext))
    } else {
        e.display_name(cx.show_ext).to_owned()
    };
    let name_rect = p.text(
        egui::pos2(x + 28.0, y),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.5),
        text,
    );
    for (i, &tag) in cx.tags.tags(&e.path).iter().enumerate() {
        let c = egui::pos2(name_rect.right() + 10.0 + i as f32 * 10.0, y);
        p.circle_filled(c, 4.0, fade(super::tag_color(tag)));
    }
    x += name_w;

    let col = |x: f32, w: f32, s: String, align_right: bool| {
        let p = ui.painter().with_clip_rect(Rect::from_min_max(
            egui::pos2(x, rect.top()),
            egui::pos2(x + w - 6.0, rect.bottom()),
        ));
        if align_right {
            p.text(
                egui::pos2(x + w - 14.0, y),
                Align2::RIGHT_CENTER,
                s,
                FontId::proportional(12.5),
                weak,
            );
        } else {
            p.text(
                egui::pos2(x + 6.0, y),
                Align2::LEFT_CENTER,
                s,
                FontId::proportional(12.5),
                weak,
            );
        }
    };
    if cx.is_bin && columns.folder > 0.0 {
        let original = cx
            .bin
            .get(&e.path)
            .map(|i| i.original_parent.display().to_string())
            .unwrap_or_default();
        col(x, columns.folder, original, false);
        x += columns.folder;
    } else if cx.is_search && columns.folder > 0.0 {
        let parent = e.path.parent().unwrap_or(&e.path);
        let parent = match &cx.search_root {
            Some(root) => {
                let name = ox_core::entry::display_name(root);
                match parent.strip_prefix(root) {
                    Ok(rel) if rel.as_os_str().is_empty() => name,
                    Ok(rel) => format!("{name}{}{}", std::path::MAIN_SEPARATOR, rel.display()),
                    Err(_) => parent.display().to_string(),
                }
            }
            None => parent.display().to_string(),
        };
        col(x, columns.folder, parent, false);
        x += columns.folder;
    }
    if let (true, Some(git)) = (columns.git > 0.0, cx.git) {
        if let Some(state) = git.state(&e.path) {
            ui.painter().text(
                egui::pos2(x + 14.0, y),
                Align2::LEFT_CENTER,
                state.letter(),
                FontId::monospace(12.5),
                fade(git_color(state, v)),
            );
        }
        x += columns.git;
    }
    if columns.date > 0.0 {
        col(
            x,
            columns.date,
            e.modified
                .map(|t| format::time(t, cx.date_format))
                .unwrap_or_default(),
            false,
        );
        x += columns.date;
    }
    if columns.kind > 0.0 {
        col(x, columns.kind, e.type_label(), false);
        x += columns.kind;
    }
    let size = match (e.is_dir, cx.sizes) {
        (false, _) => format::size(e.size),
        (true, Some(sizes)) if !e.is_symlink => match sizes.get(&e.path) {
            Some(s) => format::size(s.bytes),
            None => "…".into(),
        },
        (true, _) => String::new(),
    };
    col(x, columns.size, size, true);
}

fn grid(ui: &mut Ui, view: &mut FolderView, tab_id: u64, cx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let width = ui.available_width();
    let gap = 6.0;
    let cols = (((width - 12.0) / (TILE.x + gap)).floor() as usize).max(1);
    ui.data_mut(|d| d.insert_temp(Id::unique(("grid_columns", tab_id)), cols));
    let n = view.visible.len();
    let rows = n.div_ceil(cols);
    let row_h = TILE.y + gap;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("grid", tab_id))
        .auto_shrink([false, false]);
    if let Some(target) = view.scroll_to.take() {
        area = scroll_into_view(
            ui,
            area,
            ("grid", tab_id),
            (target / cols) as f32 * row_h,
            row_h,
        );
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = area.show_rows(ui, row_h, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.add_space(6.0);
                for col in 0..cols {
                    let i = row * cols + col;
                    if i >= n {
                        break;
                    }
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(TILE.x, row_h), Sense::click_and_drag());
                    let rect = Rect::from_min_size(rect.min, TILE);
                    let Some(entry) = view.visible_entry(i).cloned() else {
                        continue;
                    };
                    let selected = view.selection.contains(&entry.path);
                    let drop_hover = item_interact(view, i, &entry, &resp, cx, actions);
                    paint_tile(ui, rect, &entry, selected, drop_hover, &resp, cx);
                    item_context_menu(&resp, view, &entry, cx, actions);
                }
            });
        }
    });
    remember_scroll(
        ui,
        ("grid", tab_id),
        out.state.clamped_offset().y,
        out.inner_rect.height(),
    );
}

fn paint_tile(
    ui: &Ui,
    rect: Rect,
    e: &FileEntry,
    selected: bool,
    drop_hover: bool,
    resp: &Response,
    cx: &Ctx<'_>,
) {
    let v = ui.visuals();
    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), cx.selection_fill);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), v.widgets.hovered.weak_bg_fill);
    }
    if drop_hover {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.5, cx.accent),
            egui::StrokeKind::Inside,
        );
    }
    super::compare::paint_mark(ui, rect, cx.compare, &e.path);
    let dim = cx.cut.contains(&e.path) || e.hidden;
    let fade = |c: Color32| if dim { c.gamma_multiply(0.5) } else { c };
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 32.0),
        vec2(48.0, 48.0),
    );
    let thumb = cx
        .thumbs
        .filter(|_| crate::thumbs::wanted(e))
        .and_then(|t| t.get(e));
    if let Some((tex, size)) = thumb {
        let area = Rect::from_center_size(
            egui::pos2(rect.center().x, rect.top() + 31.0),
            vec2(rect.width() - 16.0, 52.0),
        );
        let r = crate::thumbs::fit(area, size);
        let tint = if dim {
            Color32::from_white_alpha(110)
        } else {
            Color32::WHITE
        };
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(tex, r, uv, tint);
        ui.painter().rect_stroke(
            r,
            CornerRadius::ZERO,
            Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
            egui::StrokeKind::Outside,
        );
        if e.kind() == ox_core::entry::FileKind::Video {
            // Play badge so videos stand out from pictures.
            let c = r.left_bottom() + vec2(9.0, -9.0);
            ui.painter()
                .circle_filled(c, 7.0, Color32::from_black_alpha(160));
            ui.painter().text(
                c + vec2(0.5, 0.0),
                egui::Align2::CENTER_CENTER,
                "▶",
                FontId::proportional(8.0),
                Color32::WHITE,
            );
        }
    } else {
        let texture = cx
            .icons
            .and_then(|c| c.texture(e, cx.is_bin || cx.read_only));
        icons::paint_entry(ui.painter(), icon_rect, e, dim, texture);
    }
    // Tag dots in the top-right corner of the icon.
    for (i, &tag) in cx.tags.tags(&e.path).iter().enumerate() {
        let c = egui::pos2(
            icon_rect.right() - 2.0 - i as f32 * 9.0,
            icon_rect.top() + 6.0,
        );
        ui.painter().circle(
            c,
            4.5,
            fade(super::tag_color(tag)),
            Stroke::new(1.5, v.panel_fill),
        );
    }
    let mut job = LayoutJob::single_section(
        e.display_name(cx.show_ext).to_owned(),
        TextFormat::simple(FontId::proportional(12.5), fade(v.text_color())),
    );
    job.wrap.max_width = rect.width() - 10.0;
    job.wrap.max_rows = 2;
    job.wrap.overflow_character = Some('…');
    job.halign = egui::Align::Center;
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(rect.center().x, rect.top() + 62.0),
        galley,
        v.text_color(),
    );
}

/// Selection, opening and drag & drop for one item. Returns true while files hover over it.
fn item_interact(
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
        super::drop_target(actions, resp, &e.path)
    } else {
        false
    }
}

fn item_context_menu(
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
        if cx.is_search {
            if let Some(parent) = e.path.parent() {
                if ui.button(tr("📁 Open file location")).clicked() {
                    actions.push(Action::Navigate(Location::Folder(parent.to_path_buf())));
                }
            }
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
        super::item_menu(ui, &target, actions);
    });
}
