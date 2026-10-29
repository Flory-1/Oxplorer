//! All drawing code. Each submodule draws one area of the window.

mod dialogs;
mod files;
pub mod header;
pub(crate) mod help_page;
mod home;
pub(crate) mod palette;
pub(crate) mod preview;
mod settings_page;
mod sidebar;
mod transfers;

use ox_core::i18n::t as tr;
use std::path::{Path, PathBuf};

use egui::{Align2, Color32, FontId, Id, LayerId, Order, Rect, Response, RichText, Ui};
use ox_core::entry::display_name;
use ox_core::format;
use ox_core::tags::{TagColor, TagStore};
use ox_core::transfer::TransferKind;

use crate::app::{Action, OxApp, TabAction};
use crate::shortcuts::{self, Command};
use crate::tabs::{Location, TabId};
use ox_core::settings::NewItem;

/// Drag payload: files/folders being dragged inside the app.
#[derive(Clone)]
pub struct DragItems(pub Vec<PathBuf>);

/// Drag payload: a tab being reordered.
#[derive(Clone, Copy)]
pub struct DragTab(pub TabId);

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    egui::Panel::top("header")
        .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 6)))
        .show(ui, |ui| header::show(app, ui));

    if app.settings.browsing.show_status_bar {
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::side_top_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| status_bar(app, ui));
    }

    if app.ui.show_transfers && !app.engine.jobs().is_empty() {
        egui::Panel::bottom("transfers")
            .resizable(true)
            .default_size(150.0)
            .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::same(8)))
            .show(ui, |ui| transfers::show(app, ui));
    }

    if app.settings.sidebar.visible {
        let salt = app.ui.sidebar_width.salt(ui, app.settings.sidebar.width);
        let resp = egui::Panel::left(("sidebar", salt))
            .resizable(true)
            .default_size(app.settings.sidebar.width)
            .size_range(SIDEBAR_WIDTH)
            .frame(
                egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(6, 8)),
            )
            .show(ui, |ui| sidebar::show(app, ui));
        if let Some(w) =
            app.ui
                .sidebar_width
                .dragged(ui, &resp.response, app.settings.sidebar.width)
        {
            app.settings.sidebar.width = w;
            app.push(Action::SettingsChanged);
        }
    }

    if app.ui.show_terminal {
        egui::Panel::bottom("terminal")
            .resizable(true)
            .default_size(240.0)
            .min_size(90.0)
            .frame(
                egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| terminal_panel(app, ui));
    } else {
        app.ui.terminal_focused = false;
    }

    let preview_shown =
        app.settings.browsing.show_preview && app.tabs.active().location().shows_files();
    if !preview_shown {
        // Nothing plays where it can't be seen (or stopped).
        app.preview.stop_playback();
    }
    if preview_shown && !app.settings.preview.as_window {
        let salt = app.ui.preview_width.salt(ui, app.settings.preview.width);
        let resp = egui::Panel::right(("preview", salt))
            .resizable(true)
            .default_size(app.settings.preview.width)
            .size_range(PREVIEW_WIDTH)
            .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::same(10)))
            .show(ui, |ui| preview::show(app, ui));
        if let Some(w) =
            app.ui
                .preview_width
                .dragged(ui, &resp.response, app.settings.preview.width)
        {
            app.settings.preview.width = w;
            app.push(Action::SettingsChanged);
        }
    }

    let central = egui::Frame::central_panel(ui.style())
        .inner_margin(egui::Margin::same(0))
        .fill(ui.visuals().extreme_bg_color);
    egui::CentralPanel::default()
        .frame(central)
        .show(ui, |ui| match app.tabs.panes {
            Some(panes) => split_view(app, ui, panes),
            None => content(app, ui),
        });

    if preview_shown && app.settings.preview.as_window {
        preview_window(app, ui.ctx());
    }
    dialogs::show(app, ui.ctx());
    palette::show(app, ui.ctx());
    toasts(app, ui.ctx());
    drag_preview(ui.ctx());
}

/// Title row with the shell's buttons, then the terminal itself.
fn terminal_panel(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Terminal")).strong());
        if let Some(t) = &app.terminal {
            if !t.title.is_empty() {
                ui.label(RichText::new(&t.title).weak());
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if close_button(ui)
                .on_hover_text(shortcuts::hint(tr("Close"), Command::ToggleTerminal))
                .clicked()
            {
                actions.push(Action::ToggleTerminal);
            }
            let exited = app.terminal.as_ref().is_none_or(|t| t.exited);
            if exited && ui.small_button(tr("⟲ Restart")).clicked() {
                actions.push(Action::RestartTerminal);
            }
        });
    });
    match &mut app.terminal {
        Some(t) => {
            app.ui.terminal_focused = t.ui(ui);
            if t.exited {
                ui.painter().text(
                    ui.min_rect().center_bottom() - egui::vec2(0.0, 16.0),
                    Align2::CENTER_CENTER,
                    tr("The shell has ended."),
                    FontId::proportional(13.0),
                    ui.visuals().weak_text_color(),
                );
            }
        }
        None => {
            app.ui.terminal_focused = false;
            ui.label(RichText::new(tr("The shell couldn't be started.")).weak());
        }
    }
    app.actions.extend(actions);
}

/// An entry of the section list on the Settings and Help pages. `name` is
/// "<icon> <label>"; the icon is drawn centred in its own slot so it lines
/// up whatever font it comes from.
pub fn nav_item(ui: &mut Ui, name: &str, selected: bool) -> Response {
    let (icon, label) = name.split_once(' ').unwrap_or(("", name));
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(180.0, 30.0), egui::Sense::click());
    let v = ui.visuals();
    if selected {
        ui.painter()
            .rect_filled(rect, v.widgets.active.corner_radius, v.selection.bg_fill);
    } else if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            v.widgets.hovered.corner_radius,
            v.widgets.hovered.weak_bg_fill,
        );
    }
    let color = if selected {
        v.selection.stroke.color
    } else {
        v.text_color()
    };
    crate::icons::paint_glyph_centered(
        ui.painter(),
        egui::pos2(rect.left() + 18.0, rect.center().y),
        icon,
        FontId::proportional(15.0),
        color,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 34.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        color,
    );
    resp
}

/// A small "✖" without a background (highlighted on hover), like on tabs.
fn close_button(ui: &mut Ui) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "✖",
        FontId::proportional(11.0),
        ui.visuals().text_color(),
    );
    resp
}

/// The page of the active tab.
fn content(app: &mut OxApp, ui: &mut Ui) {
    match app.tabs.active().location().clone() {
        Location::Home => home::show(app, ui),
        Location::Settings => settings_page::show(app, ui),
        Location::Help => help_page::show(app, ui),
        Location::Folder(_)
        | Location::Search { .. }
        | Location::RecycleBin
        | Location::Tagged(_)
        | Location::Archive { .. } => files::show(app, ui),
    }
}

/// Two panes side by side. Everything acts on the active tab, so the other
/// pane is drawn with its tab made active for the moment, and pressing the
/// mouse in it moves the focus there before any of its actions run.
fn split_view(app: &mut OxApp, ui: &mut Ui, panes: [TabId; 2]) {
    let rect = ui.available_rect_before_wrap();
    let half = (rect.width() - 1.0) / 2.0;
    for (i, id) in panes.into_iter().enumerate() {
        let pane = Rect::from_min_size(
            egui::pos2(rect.left() + i as f32 * (half + 1.0), rect.top()),
            egui::vec2(half, rect.height()),
        );
        let pressed = ui.input(|inp| {
            inp.pointer.any_pressed()
                && inp.pointer.press_origin().is_some_and(|p| pane.contains(p))
        });
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(pane).id_salt(("pane", id)));
        child.set_clip_rect(pane.intersect(ui.clip_rect()));
        let focused = id == app.tabs.active;
        pane_bar(app, &mut child, id, focused);
        if focused {
            content(app, &mut child);
        } else {
            let start = app.actions.len();
            let active = app.tabs.active;
            app.tabs.active = id;
            content(app, &mut child);
            app.tabs.active = active;
            if pressed {
                app.actions
                    .insert(start, Action::Tab(TabAction::Activate(id)));
            }
        }
    }
    let x = rect.left() + half + 0.5;
    ui.painter().vline(
        x,
        rect.y_range(),
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
    ui.allocate_rect(rect, egui::Sense::hover());
}

/// Title strip above a pane: where it is, with the accent on the focused one.
fn pane_bar(app: &OxApp, ui: &mut Ui, id: TabId, focused: bool) {
    let Some(tab) = app.tabs.get(id) else { return };
    let title = match tab.location() {
        Location::Folder(p) => p.display().to_string(),
        Location::Tagged(c) => ox_core::trf!("Tagged \"{v1}\"", v1 = app.tags.label(*c)),
        Location::Archive { archive, inner } => archive.join(inner).display().to_string(),
        other => other.title(),
    };
    let (bar, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    let v = ui.visuals();
    ui.painter().rect_filled(bar, 0.0, v.panel_fill);
    let accent = crate::theme::accent(&app.settings);
    if focused {
        ui.painter().hline(
            bar.x_range(),
            bar.bottom() - 1.0,
            egui::Stroke::new(2.0, accent),
        );
    }
    let color = if focused {
        v.strong_text_color()
    } else {
        v.weak_text_color()
    };
    let icon = match tab.location() {
        Location::Folder(p) if p.parent().is_none() => crate::icons::Icon::Drive,
        Location::Folder(_) => crate::icons::Icon::Folder,
        Location::Tagged(c) => crate::icons::Icon::Dot(tag_color(*c)),
        other => crate::icons::Icon::Glyph(other.icon()),
    };
    let painter = ui.painter().with_clip_rect(bar.shrink(2.0));
    crate::icons::paint_icon(
        &painter,
        crate::icons::icon_rect(bar.left() + 10.0, bar.center().y, 14.0),
        icon,
        color,
        accent,
    );
    painter.text(
        bar.left_center() + egui::vec2(30.0, 0.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(12.5),
        color,
    );
}

fn status_bar(app: &mut OxApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let tab = app.tabs.active();
        if tab.location().shows_files() {
            let total = tab.view.visible.len();
            ui.label(ox_core::i18n::count(total as u64, "item"));
            let selected = tab.view.selected_entries();
            if !selected.is_empty() {
                ui.separator();
                // Folders count once their size is known (see `poll_folder_sizes`).
                let bytes: u64 = selected.iter().map(|e| e.size).sum();
                if bytes > 0 {
                    ui.label(ox_core::trf!(
                        "{v1} selected · {v2}",
                        v1 = selected.len(),
                        v2 = format::size(bytes)
                    ));
                } else {
                    ui.label(ox_core::trf!("{v1} selected", v1 = selected.len()));
                }
            }
            if let Some(s) = &tab.search {
                ui.separator();
                match (&s.done, &s.handle) {
                    (Some(done), _) => ui.label(done),
                    (None, Some(h)) => {
                        ui.spinner();
                        ui.label(ox_core::trf!(
                            "Searching… {v1} items scanned",
                            v1 = h.scanned()
                        ))
                    }
                    _ => ui.label(""),
                };
            }
            if tab.view.loading.is_some() {
                ui.spinner();
            }
            if let Some(git) = &tab.view.git {
                if let Some(branch) = git.branch_label() {
                    ui.separator();
                    let changed = git.changed_files();
                    let text = if changed > 0 {
                        ox_core::trf!(
                            "Git: {branch} · {v1}",
                            branch = branch,
                            v1 = ox_core::i18n::count(changed as u64, "change")
                        )
                    } else {
                        ox_core::trf!("Git: {branch}", branch = branch)
                    };
                    ui.label(RichText::new(text).weak())
                        .on_hover_text(git.root.display().to_string());
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(summary) = app.transfer_summary() {
                if ui.link(summary).clicked() {
                    app.ui.show_transfers = true;
                }
            }
            if app.settings.search.index_enabled && app.index.is_building() {
                ui.label(
                    RichText::new(ox_core::trf!(
                        "Indexing… {v1} items",
                        v1 = app.index.progress()
                    ))
                    .weak(),
                );
                ui.spinner();
            }
        });
    });
}

fn toasts(app: &OxApp, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    egui::Area::new(Id::unique("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, [-16.0, -40.0])
        .order(Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            for t in &app.toasts {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(&t.text);
                });
                ui.add_space(4.0);
            }
        });
}

/// Allowed widths of the sidebar and the preview pane (in points).
pub const SIDEBAR_WIDTH: std::ops::RangeInclusive<f32> = 160.0..=520.0;
pub const PREVIEW_WIDTH: std::ops::RangeInclusive<f32> = 200.0..=900.0;

/// Keeps a resizable side panel and its width setting in sync: dragging the
/// panel's edge changes the setting, and changing the setting (Settings page)
/// resizes the panel.
#[derive(Default)]
pub struct PanelWidth {
    /// The setting as last seen here.
    seen: Option<f32>,
    /// Part of the panel's id. egui remembers a panel's width by id, so a
    /// new id makes it start over at the configured width.
    generation: u32,
}

impl PanelWidth {
    /// The id salt for this frame's panel; `width` is the setting.
    pub fn salt(&mut self, ui: &Ui, width: f32) -> u32 {
        match self.seen {
            None => self.seen = Some(width),
            // Changed elsewhere (Settings page): resize the panel, but only
            // once the mouse is done. The slider sits in the area the panel
            // resizes, so resizing while it is pressed (or on the frame it is
            // released) would move it under the pointer and change the value.
            Some(seen) if seen != width => {
                let busy = ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
                if busy {
                    ui.ctx().request_repaint(); // try again next frame
                } else {
                    self.generation += 1;
                    self.seen = Some(width);
                }
            }
            Some(_) => {}
        }
        self.generation
    }

    /// The panel's new width once the user has finished dragging its edge.
    pub fn dragged(&mut self, ui: &Ui, panel: &Response, width: f32) -> Option<f32> {
        let actual = panel.rect.width().round();
        let released = !ui.input(|i| i.pointer.any_down());
        if released && (actual - width).abs() >= 1.0 {
            self.seen = Some(actual);
            Some(actual)
        } else {
            None
        }
    }
}

/// The preview as a separate window (Settings → Preview pane → Open as a window).
fn preview_window(app: &mut OxApp, ctx: &egui::Context) {
    let mut open = true;
    let screen = ctx.content_rect();
    let width = app.settings.preview.width;
    egui::Window::new(tr("Preview"))
        .id(Id::unique("preview_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(width)
        .default_height((screen.height() * 0.7).max(300.0))
        .default_pos(screen.right_top() + egui::vec2(-width - 40.0, 90.0))
        .show(ctx, |ui| preview::show(app, ui));
    if !open {
        app.settings.browsing.show_preview = false;
        app.push(Action::SettingsChanged);
    }
}

/// Small label following the pointer while dragging files.
fn drag_preview(ctx: &egui::Context) {
    let Some(items) = egui::DragAndDrop::payload::<DragItems>(ctx) else {
        return;
    };
    let Some(pos) = ctx.pointer_interact_pos() else {
        return;
    };
    let text = match items.0.as_slice() {
        [one] => display_name(one),
        many => ox_core::i18n::count(many.len() as u64, "item"),
    };
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("drag_preview")));
    let galley = painter.layout_no_wrap(
        format!("📄 {text}"),
        FontId::proportional(13.0),
        Color32::WHITE,
    );
    let rect = Align2::LEFT_TOP
        .anchor_size(pos + egui::vec2(14.0, 14.0), galley.size())
        .expand(6.0);
    painter.rect_filled(rect, 6.0, Color32::from_black_alpha(200));
    painter.galley(rect.min + egui::vec2(6.0, 6.0), galley, Color32::WHITE);
}

/// Make `response` accept dropped files, moving/copying them into `dest`.
/// Returns true while files are hovering over it.
pub fn drop_target(actions: &mut Vec<Action>, response: &Response, dest: &Path) -> bool {
    let hovering = response.dnd_hover_payload::<DragItems>().is_some();
    if let Some(items) = response.dnd_release_payload::<DragItems>() {
        actions.push(Action::Drop(items.0.clone(), dest.to_path_buf()));
    }
    hovering
}

/// Context information for the item context menu.
pub struct MenuTarget<'a> {
    pub selection: &'a [PathBuf],
    /// Items inside an archive: only open, copy (extract) and copy path.
    pub read_only: bool,
    pub tags: &'a TagStore,
    pub single_dir: Option<&'a Path>,
    pub pinned: bool,
    pub can_paste: bool,
    pub destinations: &'a [(String, PathBuf)],
    /// The Windows Shell is available ("Open with", "Show more options").
    pub shell: bool,
    /// Show the developer entries (Settings → Developer).
    pub dev: bool,
}

/// Folders offered in "Copy to" / "Move to": favorites and other open tabs.
pub fn quick_destinations(app: &OxApp) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = app
        .quick
        .favorites
        .iter()
        .map(|f| (format!("⭐ {}", f.label()), f.path.clone()))
        .collect();
    for t in &app.tabs.tabs {
        if let Some(f) = t.location().folder() {
            if !out.iter().any(|(_, p)| p == f) {
                out.push((format!("📑 {}", display_name(f)), f.to_path_buf()));
            }
        }
    }
    out
}

pub fn tag_color(c: TagColor) -> Color32 {
    let [r, g, b] = c.rgb();
    Color32::from_rgb(r, g, b)
}

/// Tick a colour to tag the selection, again to remove the tag.
pub fn tag_menu(ui: &mut Ui, tags: &TagStore, sel: &[PathBuf], actions: &mut Vec<Action>) {
    for color in TagColor::ALL {
        let all = sel.iter().all(|p| tags.has(p, color));
        let label = RichText::new(format!("●  {}", tags.label(color))).color(tag_color(color));
        if ui.add(egui::Button::selectable(all, label)).clicked() {
            actions.push(Action::ToggleTag(sel.to_vec(), color));
        }
    }
    if sel.iter().any(|p| !tags.tags(p).is_empty()) {
        ui.separator();
        if ui.button(tr("✖ Remove all tags")).clicked() {
            actions.push(Action::ClearTags(sel.to_vec()));
        }
    }
}

/// Menu for items inside an archive: open, extract, copy path.
fn archive_item_menu(ui: &mut Ui, t: &MenuTarget<'_>, actions: &mut Vec<Action>) {
    let sel = t.selection.to_vec();
    if ui.button(tr("📂 Open")).clicked() {
        for p in &sel {
            actions.push(Action::Open(p.clone()));
        }
    }
    if ui
        .add(egui::Button::new(tr("📋 Copy")).shortcut_text(shortcuts::label(Command::Copy)))
        .clicked()
    {
        actions.push(Action::CopyItems(sel.clone()));
    }
    ui.separator();
    let next_to = sel
        .first()
        .and_then(|p| ox_core::archive::split_virtual(p))
        .and_then(|(arc, _)| arc.parent().map(Path::to_path_buf));
    if let Some(dir) = next_to {
        if ui.button(tr("📦 Extract next to the archive")).clicked() {
            actions.push(Action::Transfer(TransferKind::Copy, sel.clone(), dir));
        }
    }
    if !t.destinations.is_empty() {
        ui.menu_button(tr("📦 Extract to"), |ui| {
            for (label, dest) in t.destinations {
                if ui.button(label).clicked() {
                    actions.push(Action::Transfer(
                        TransferKind::Copy,
                        sel.clone(),
                        dest.clone(),
                    ));
                }
            }
        });
    }
    ui.separator();
    if ui
        .add(
            egui::Button::new(tr("🔗 Copy path"))
                .shortcut_text(shortcuts::label(Command::CopyPath)),
        )
        .clicked()
    {
        actions.push(Action::CopyPathText(sel.clone()));
    }
}

pub fn item_menu(ui: &mut Ui, t: &MenuTarget<'_>, actions: &mut Vec<Action>) {
    if t.read_only {
        archive_item_menu(ui, t, actions);
        return;
    }
    let sel = t.selection.to_vec();
    if let Some(dir) = t.single_dir {
        if ui.button(tr("📂 Open")).clicked() {
            actions.push(Action::Open(dir.to_path_buf()));
        }
        if ui.button(tr("📑 Open in new tab")).clicked() {
            actions.push(Action::OpenInNewTab(
                Location::Folder(dir.to_path_buf()),
                false,
            ));
        }
    } else if ui.button(tr("📂 Open")).clicked() {
        for p in &sel {
            actions.push(Action::Open(p.clone()));
        }
    }
    if t.shell
        && t.single_dir.is_none()
        && sel.len() == 1
        && ui.button(tr("🗗 Open with…")).clicked()
    {
        actions.push(Action::OpenWith(sel[0].clone()));
    }
    ui.separator();
    if ui
        .add(egui::Button::new(tr("✂ Cut")).shortcut_text(shortcuts::label(Command::Cut)))
        .clicked()
    {
        actions.push(Action::CutItems(sel.clone()));
    }
    if ui
        .add(egui::Button::new(tr("📋 Copy")).shortcut_text(shortcuts::label(Command::Copy)))
        .clicked()
    {
        actions.push(Action::CopyItems(sel.clone()));
    }
    if let Some(dir) = t.single_dir {
        if ui
            .add_enabled(t.can_paste, egui::Button::new(tr("📥 Paste into folder")))
            .clicked()
        {
            actions.push(Action::Paste(dir.to_path_buf()));
        }
    }
    if !t.destinations.is_empty() {
        ui.menu_button(tr("➡ Copy to"), |ui| {
            for (label, dest) in t.destinations {
                if ui.button(label).clicked() {
                    actions.push(Action::Transfer(
                        TransferKind::Copy,
                        sel.clone(),
                        dest.clone(),
                    ));
                }
            }
        });
        ui.menu_button(tr("➡ Move to"), |ui| {
            for (label, dest) in t.destinations {
                if ui.button(label).clicked() {
                    actions.push(Action::Transfer(
                        TransferKind::Move,
                        sel.clone(),
                        dest.clone(),
                    ));
                }
            }
        });
    }
    ui.separator();
    if sel.len() == 1
        && ui
            .add(egui::Button::new(tr("✏ Rename")).shortcut_text(shortcuts::label(Command::Rename)))
            .clicked()
    {
        actions.push(Action::Rename(sel[0].clone()));
    }
    if sel.len() > 1
        && ui
            .add(
                egui::Button::new(ox_core::trf!("✏ Rename {v1} items…", v1 = sel.len()))
                    .shortcut_text(shortcuts::label(Command::Rename)),
            )
            .clicked()
    {
        actions.push(Action::BulkRename(sel.clone()));
    }
    if ui
        .add(egui::Button::new(tr("🗑 Delete")).shortcut_text(shortcuts::label(Command::Delete)))
        .clicked()
    {
        actions.push(Action::Delete(sel.clone(), false));
    }
    if ui
        .add(
            egui::Button::new(tr("⛔ Delete permanently"))
                .shortcut_text(shortcuts::label(Command::DeletePermanently)),
        )
        .clicked()
    {
        actions.push(Action::Delete(sel.clone(), true));
    }
    ui.separator();
    ui.menu_button(tr("🏷 Tags"), |ui| tag_menu(ui, t.tags, &sel, actions));
    if sel.len() == 1 && ox_core::archive::is_archive(&sel[0]) {
        let name = ox_core::archive::base_name(&sel[0]);
        if ui
            .button(ox_core::trf!("📦 Extract to \"{name}\"", name = name))
            .clicked()
        {
            actions.push(Action::ExtractArchive {
                archive: sel[0].clone(),
                own_folder: true,
            });
        }
        if ui.button(tr("📦 Extract here")).clicked() {
            actions.push(Action::ExtractArchive {
                archive: sel[0].clone(),
                own_folder: false,
            });
        }
    }
    if ui.button(tr("📦 Compress to ZIP")).clicked() {
        actions.push(Action::Compress(sel.clone()));
    }
    let theme_file = sel.len() == 1
        && sel[0]
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
        && ox_core::themes::read(&sel[0]).is_ok();
    if theme_file && ui.button(tr("🎨 Use as theme")).clicked() {
        actions.push(Action::InstallTheme(sel[0].clone()));
    }
    if let Some(dir) = t.single_dir {
        if t.pinned {
            if ui.button(tr("☆ Unpin from Favorites")).clicked() {
                actions.push(Action::Unpin(dir.to_path_buf()));
            }
        } else if ui.button(tr("⭐ Pin to Favorites")).clicked() {
            actions.push(Action::Pin(dir.to_path_buf()));
        }
        if ui.button(tr("🖳 Open in Terminal")).clicked() {
            actions.push(Action::OpenTerminal(dir.to_path_buf()));
        }
    }
    if ui
        .add(
            egui::Button::new(tr("🔗 Copy path"))
                .shortcut_text(shortcuts::label(Command::CopyPath)),
        )
        .clicked()
    {
        actions.push(Action::CopyPathText(sel.clone()));
    }
    if sel.len() == 1 && ui.button(tr("🖥 Show in Windows Explorer")).clicked() {
        actions.push(Action::Reveal(sel[0].clone()));
    }
    if t.dev {
        ui.separator();
        dev_menu(ui, &sel, actions);
    }
    ui.separator();
    if ui
        .add(
            egui::Button::new(tr("ℹ Properties"))
                .shortcut_text(shortcuts::label(Command::Properties)),
        )
        .clicked()
    {
        actions.push(Action::Properties(sel.clone()));
    }
    if t.shell {
        ui.separator();
        if ui
            .add(
                egui::Button::new(tr("⋯ Show more options"))
                    .shortcut_text(shortcuts::label(Command::ShellMenu)),
            )
            .on_hover_text(tr(
                "The Windows Explorer menu, with entries from other apps",
            ))
            .clicked()
        {
            actions.push(Action::ShellMenu(sel));
        }
    }
}

/// Developer entries (Settings → Developer): open in the editor, copy the
/// path in other forms and, in Git repositories, Git actions.
fn dev_menu(ui: &mut Ui, paths: &[PathBuf], actions: &mut Vec<Action>) {
    use ox_core::pathfmt::PathFormat;
    if ui
        .add(
            egui::Button::new(tr("📝 Open in editor"))
                .shortcut_text(shortcuts::label(Command::OpenInEditor)),
        )
        .clicked()
    {
        actions.push(Action::OpenInEditor(paths.to_vec()));
    }
    let repo = paths.first().and_then(|p| ox_core::git::repo_root(p));
    ui.menu_button(tr("🔗 Copy path as"), |ui| {
        for fmt in PathFormat::ALL {
            if fmt == PathFormat::GitRelative && repo.is_none() {
                continue;
            }
            if ui.button(fmt.label()).clicked() {
                actions.push(Action::CopyPathAs(paths.to_vec(), fmt));
            }
        }
    });
    if let (Some(root), [path]) = (repo, paths) {
        ui.menu_button(tr("🌿 Git"), |ui| {
            if ui.button(tr("🌐 Open on GitHub / GitLab")).clicked() {
                actions.push(Action::OpenGitRemote(path.clone()));
            }
            if ui.button(tr("📋 Copy branch name")).clicked() {
                actions.push(Action::CopyGitBranch(root.clone()));
            }
        });
    }
}

/// The entries of the "New" menus: Folder, Text document and the user's own
/// (Settings → New menu).
pub fn new_menu(ui: &mut Ui, folder: &Path, new_items: &[NewItem], actions: &mut Vec<Action>) {
    if ui
        .add(egui::Button::new(tr("📁 Folder")).shortcut_text(shortcuts::label(Command::NewFolder)))
        .clicked()
    {
        actions.push(Action::NewFolder(folder.to_path_buf()));
    }
    if ui.button(tr("📄 Text document")).clicked() {
        actions.push(Action::NewFile(folder.to_path_buf()));
    }
    if !new_items.is_empty() {
        ui.separator();
    }
    for (i, item) in new_items.iter().enumerate() {
        let text = format!("📄 {}", ox_core::i18n::t_any(&item.label));
        if ui.button(text).on_hover_text(&item.file_name).clicked() {
            actions.push(Action::NewFromItem(folder.to_path_buf(), i));
        }
    }
}

/// Context menu for the empty area of a folder.
pub fn background_menu(
    ui: &mut Ui,
    folder: &Path,
    can_paste: bool,
    pinned: bool,
    new_items: &[NewItem],
    dev: bool,
    actions: &mut Vec<Action>,
) {
    ui.menu_button(tr("➕ New"), |ui| new_menu(ui, folder, new_items, actions));
    if ui
        .add_enabled(
            can_paste,
            egui::Button::new(tr("📥 Paste")).shortcut_text(shortcuts::label(Command::Paste)),
        )
        .clicked()
    {
        actions.push(Action::Paste(folder.to_path_buf()));
    }
    if ui
        .add(egui::Button::new(tr("🔄 Refresh")).shortcut_text(shortcuts::label(Command::Refresh)))
        .clicked()
    {
        actions.push(Action::Refresh);
    }
    if ui
        .add(
            egui::Button::new(tr("☑ Select all"))
                .shortcut_text(shortcuts::label(Command::SelectAll)),
        )
        .clicked()
    {
        actions.push(Action::SelectAll);
    }
    ui.separator();
    if pinned {
        if ui.button(tr("☆ Unpin from Favorites")).clicked() {
            actions.push(Action::Unpin(folder.to_path_buf()));
        }
    } else if ui.button(tr("⭐ Pin to Favorites")).clicked() {
        actions.push(Action::Pin(folder.to_path_buf()));
    }
    if ui.button(tr("🖳 Open in Terminal")).clicked() {
        actions.push(Action::OpenTerminal(folder.to_path_buf()));
    }
    if ui.button(tr("🖥 Open in Windows Explorer")).clicked() {
        actions.push(Action::Reveal(folder.to_path_buf()));
    }
    if dev {
        ui.separator();
        dev_menu(ui, &[folder.to_path_buf()], actions);
    }
    ui.separator();
    if ui.button(tr("ℹ Properties")).clicked() {
        actions.push(Action::Properties(vec![folder.to_path_buf()]));
    }
}

/// Context menu for a folder link (sidebar, home page cards).
pub fn folder_link_menu(ui: &mut Ui, path: &Path, pinned: bool, actions: &mut Vec<Action>) {
    if ui.button(tr("📂 Open")).clicked() {
        actions.push(Action::Navigate(Location::Folder(path.to_path_buf())));
    }
    if ui.button(tr("📑 Open in new tab")).clicked() {
        actions.push(Action::OpenInNewTab(
            Location::Folder(path.to_path_buf()),
            false,
        ));
    }
    ui.separator();
    if pinned {
        if ui.button(tr("☆ Unpin from Favorites")).clicked() {
            actions.push(Action::Unpin(path.to_path_buf()));
        }
        if ui.button(tr("⬆ Move up")).clicked() {
            actions.push(Action::MoveFavorite(path.to_path_buf(), -1));
        }
        if ui.button(tr("⬇ Move down")).clicked() {
            actions.push(Action::MoveFavorite(path.to_path_buf(), 1));
        }
    } else if ui.button(tr("⭐ Pin to Favorites")).clicked() {
        actions.push(Action::Pin(path.to_path_buf()));
    }
    ui.separator();
    if ui.button(tr("📥 Paste into folder")).clicked() {
        actions.push(Action::Paste(path.to_path_buf()));
    }
    if ui.button(tr("🖳 Open in Terminal")).clicked() {
        actions.push(Action::OpenTerminal(path.to_path_buf()));
    }
    if ui.button(tr("🔗 Copy path")).clicked() {
        actions.push(Action::CopyPathText(vec![path.to_path_buf()]));
    }
    if ui.button(tr("ℹ Properties")).clicked() {
        actions.push(Action::Properties(vec![path.to_path_buf()]));
    }
}

/// Middle-click opens a folder link in a background tab.
pub fn open_link(actions: &mut Vec<Action>, response: &Response, path: &Path) {
    if response.clicked() {
        actions.push(Action::Navigate(Location::Folder(path.to_path_buf())));
    } else if response.middle_clicked() {
        actions.push(Action::OpenInNewTab(
            Location::Folder(path.to_path_buf()),
            false,
        ));
    }
}
