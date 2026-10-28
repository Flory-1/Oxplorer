//! All drawing code. Each submodule draws one area of the window.

mod dialogs;
mod files;
mod header;
pub(crate) mod help_page;
mod home;
pub(crate) mod palette;
pub(crate) mod preview;
mod settings_page;
mod sidebar;
mod transfers;

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, FontId, Id, LayerId, Order, Rect, Response, RichText, Ui};
use ox_core::entry::display_name;
use ox_core::format;
use ox_core::tags::{TagColor, TagStore};
use ox_core::transfer::TransferKind;

use crate::app::{Action, OxApp, TabAction};
use crate::shortcuts::{self, Command};
use crate::tabs::{Location, TabId};

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
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(230.0)
            .frame(
                egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(6, 8)),
            )
            .show(ui, |ui| sidebar::show(app, ui));
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

    if app.settings.browsing.show_preview && app.tabs.active().location().shows_files() {
        egui::Panel::right("preview")
            .resizable(true)
            .default_size(320.0)
            .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::same(10)))
            .show(ui, |ui| preview::show(app, ui));
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

    dialogs::show(app, ui.ctx());
    palette::show(app, ui.ctx());
    toasts(app, ui.ctx());
    drag_preview(ui.ctx());
}

/// Title row with the shell's buttons, then the terminal itself.
fn terminal_panel(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Terminal").strong());
        if let Some(t) = &app.terminal {
            if !t.title.is_empty() {
                ui.label(RichText::new(&t.title).weak());
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("✖")
                .on_hover_text(shortcuts::hint("Close", Command::ToggleTerminal))
                .clicked()
            {
                actions.push(Action::ToggleTerminal);
            }
            let exited = app.terminal.as_ref().is_none_or(|t| t.exited);
            if exited && ui.small_button("⟲ Restart").clicked() {
                actions.push(Action::RestartTerminal);
            }
            if let Some(folder) = app.current_folder().filter(|_| !exited) {
                if ui
                    .small_button("📂 cd here")
                    .on_hover_text(folder.display().to_string())
                    .clicked()
                {
                    actions.push(Action::TerminalCd(folder));
                }
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
                    "The shell has ended.",
                    FontId::proportional(13.0),
                    ui.visuals().weak_text_color(),
                );
            }
        }
        None => {
            app.ui.terminal_focused = false;
            ui.label(RichText::new("The shell couldn't be started.").weak());
        }
    }
    app.actions.extend(actions);
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
        Location::Tagged(c) => format!("Tagged \"{}\"", app.tags.label(*c)),
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
            ui.label(format::count(total as u64, "item"));
            let selected = tab.view.selected_entries();
            if !selected.is_empty() {
                ui.separator();
                // Folders count once their size is known (see `poll_folder_sizes`).
                let bytes: u64 = selected.iter().map(|e| e.size).sum();
                if bytes > 0 {
                    ui.label(format!(
                        "{} selected · {}",
                        selected.len(),
                        format::size(bytes)
                    ));
                } else {
                    ui.label(format!("{} selected", selected.len()));
                }
            }
            if let Some(s) = &tab.search {
                ui.separator();
                match (&s.done, &s.handle) {
                    (Some(done), _) => ui.label(done),
                    (None, Some(h)) => {
                        ui.spinner();
                        ui.label(format!("Searching… {} items scanned", h.scanned()))
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
                        format!(
                            "Git: {branch} · {}",
                            format::count(changed as u64, "change")
                        )
                    } else {
                        format!("Git: {branch}")
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
                ui.label(RichText::new(format!("Indexing… {} items", app.index.progress())).weak());
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
        many => format::count(many.len() as u64, "item"),
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
        if ui.button("✖ Remove all tags").clicked() {
            actions.push(Action::ClearTags(sel.to_vec()));
        }
    }
}

/// Menu for items inside an archive: open, extract, copy path.
fn archive_item_menu(ui: &mut Ui, t: &MenuTarget<'_>, actions: &mut Vec<Action>) {
    let sel = t.selection.to_vec();
    if ui.button("📂 Open").clicked() {
        for p in &sel {
            actions.push(Action::Open(p.clone()));
        }
    }
    if ui
        .add(egui::Button::new("📋 Copy").shortcut_text(shortcuts::label(Command::Copy)))
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
        if ui.button("📦 Extract next to the archive").clicked() {
            actions.push(Action::Transfer(TransferKind::Copy, sel.clone(), dir));
        }
    }
    if !t.destinations.is_empty() {
        ui.menu_button("📦 Extract to", |ui| {
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
        .add(egui::Button::new("🔗 Copy path").shortcut_text(shortcuts::label(Command::CopyPath)))
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
        if ui.button("📂 Open").clicked() {
            actions.push(Action::Open(dir.to_path_buf()));
        }
        if ui.button("📑 Open in new tab").clicked() {
            actions.push(Action::OpenInNewTab(
                Location::Folder(dir.to_path_buf()),
                false,
            ));
        }
    } else if ui.button("📂 Open").clicked() {
        for p in &sel {
            actions.push(Action::Open(p.clone()));
        }
    }
    if t.shell && t.single_dir.is_none() && sel.len() == 1 && ui.button("🗗 Open with…").clicked()
    {
        actions.push(Action::OpenWith(sel[0].clone()));
    }
    ui.separator();
    if ui
        .add(egui::Button::new("✂ Cut").shortcut_text(shortcuts::label(Command::Cut)))
        .clicked()
    {
        actions.push(Action::CutItems(sel.clone()));
    }
    if ui
        .add(egui::Button::new("📋 Copy").shortcut_text(shortcuts::label(Command::Copy)))
        .clicked()
    {
        actions.push(Action::CopyItems(sel.clone()));
    }
    if let Some(dir) = t.single_dir {
        if ui
            .add_enabled(t.can_paste, egui::Button::new("📥 Paste into folder"))
            .clicked()
        {
            actions.push(Action::Paste(dir.to_path_buf()));
        }
    }
    if !t.destinations.is_empty() {
        ui.menu_button("➡ Copy to", |ui| {
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
        ui.menu_button("➡ Move to", |ui| {
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
            .add(egui::Button::new("✏ Rename").shortcut_text(shortcuts::label(Command::Rename)))
            .clicked()
    {
        actions.push(Action::Rename(sel[0].clone()));
    }
    if sel.len() > 1
        && ui
            .add(
                egui::Button::new(format!("✏ Rename {} items…", sel.len()))
                    .shortcut_text(shortcuts::label(Command::Rename)),
            )
            .clicked()
    {
        actions.push(Action::BulkRename(sel.clone()));
    }
    if ui
        .add(egui::Button::new("🗑 Delete").shortcut_text(shortcuts::label(Command::Delete)))
        .clicked()
    {
        actions.push(Action::Delete(sel.clone(), false));
    }
    if ui
        .add(
            egui::Button::new("⛔ Delete permanently")
                .shortcut_text(shortcuts::label(Command::DeletePermanently)),
        )
        .clicked()
    {
        actions.push(Action::Delete(sel.clone(), true));
    }
    ui.separator();
    ui.menu_button("🏷 Tags", |ui| tag_menu(ui, t.tags, &sel, actions));
    if sel.len() == 1 && ox_core::archive::is_archive(&sel[0]) {
        let name = ox_core::archive::base_name(&sel[0]);
        if ui.button(format!("📦 Extract to \"{name}\"")).clicked() {
            actions.push(Action::ExtractArchive {
                archive: sel[0].clone(),
                own_folder: true,
            });
        }
        if ui.button("📦 Extract here").clicked() {
            actions.push(Action::ExtractArchive {
                archive: sel[0].clone(),
                own_folder: false,
            });
        }
    }
    if ui.button("🗜 Compress to ZIP").clicked() {
        actions.push(Action::Compress(sel.clone()));
    }
    let theme_file = sel.len() == 1
        && sel[0]
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
        && ox_core::themes::read(&sel[0]).is_ok();
    if theme_file && ui.button("🎨 Use as theme").clicked() {
        actions.push(Action::InstallTheme(sel[0].clone()));
    }
    if let Some(dir) = t.single_dir {
        if t.pinned {
            if ui.button("☆ Unpin from Favorites").clicked() {
                actions.push(Action::Unpin(dir.to_path_buf()));
            }
        } else if ui.button("⭐ Pin to Favorites").clicked() {
            actions.push(Action::Pin(dir.to_path_buf()));
        }
        if ui.button("🖳 Open in Terminal").clicked() {
            actions.push(Action::OpenTerminal(dir.to_path_buf()));
        }
    }
    if ui
        .add(egui::Button::new("🔗 Copy path").shortcut_text(shortcuts::label(Command::CopyPath)))
        .clicked()
    {
        actions.push(Action::CopyPathText(sel.clone()));
    }
    if sel.len() == 1 && ui.button("🗔 Show in Windows Explorer").clicked() {
        actions.push(Action::Reveal(sel[0].clone()));
    }
    ui.separator();
    if ui
        .add(egui::Button::new("ℹ Properties").shortcut_text(shortcuts::label(Command::Properties)))
        .clicked()
    {
        actions.push(Action::Properties(sel.clone()));
    }
    if t.shell {
        ui.separator();
        if ui
            .add(
                egui::Button::new("⋯ Show more options")
                    .shortcut_text(shortcuts::label(Command::ShellMenu)),
            )
            .on_hover_text("The Windows Explorer menu, with entries from other apps")
            .clicked()
        {
            actions.push(Action::ShellMenu(sel));
        }
    }
}

/// Context menu for the empty area of a folder.
pub fn background_menu(
    ui: &mut Ui,
    folder: &Path,
    can_paste: bool,
    pinned: bool,
    actions: &mut Vec<Action>,
) {
    ui.menu_button("➕ New", |ui| {
        if ui.button("📁 Folder").clicked() {
            actions.push(Action::NewFolder(folder.to_path_buf()));
        }
        if ui.button("📄 Text document").clicked() {
            actions.push(Action::NewFile(folder.to_path_buf()));
        }
    });
    if ui
        .add_enabled(
            can_paste,
            egui::Button::new("📥 Paste").shortcut_text(shortcuts::label(Command::Paste)),
        )
        .clicked()
    {
        actions.push(Action::Paste(folder.to_path_buf()));
    }
    if ui
        .add(egui::Button::new("🔄 Refresh").shortcut_text(shortcuts::label(Command::Refresh)))
        .clicked()
    {
        actions.push(Action::Refresh);
    }
    if ui
        .add(egui::Button::new("☑ Select all").shortcut_text(shortcuts::label(Command::SelectAll)))
        .clicked()
    {
        actions.push(Action::SelectAll);
    }
    ui.separator();
    if pinned {
        if ui.button("☆ Unpin from Favorites").clicked() {
            actions.push(Action::Unpin(folder.to_path_buf()));
        }
    } else if ui.button("⭐ Pin to Favorites").clicked() {
        actions.push(Action::Pin(folder.to_path_buf()));
    }
    if ui.button("🖳 Open in Terminal").clicked() {
        actions.push(Action::OpenTerminal(folder.to_path_buf()));
    }
    if ui.button("🗔 Open in Windows Explorer").clicked() {
        actions.push(Action::Reveal(folder.to_path_buf()));
    }
    if ui.button("ℹ Properties").clicked() {
        actions.push(Action::Properties(vec![folder.to_path_buf()]));
    }
}

/// Context menu for a folder link (sidebar, home page cards).
pub fn folder_link_menu(ui: &mut Ui, path: &Path, pinned: bool, actions: &mut Vec<Action>) {
    if ui.button("📂 Open").clicked() {
        actions.push(Action::Navigate(Location::Folder(path.to_path_buf())));
    }
    if ui.button("📑 Open in new tab").clicked() {
        actions.push(Action::OpenInNewTab(
            Location::Folder(path.to_path_buf()),
            false,
        ));
    }
    ui.separator();
    if pinned {
        if ui.button("☆ Unpin from Favorites").clicked() {
            actions.push(Action::Unpin(path.to_path_buf()));
        }
        if ui.button("⬆ Move up").clicked() {
            actions.push(Action::MoveFavorite(path.to_path_buf(), -1));
        }
        if ui.button("⬇ Move down").clicked() {
            actions.push(Action::MoveFavorite(path.to_path_buf(), 1));
        }
    } else if ui.button("⭐ Pin to Favorites").clicked() {
        actions.push(Action::Pin(path.to_path_buf()));
    }
    ui.separator();
    if ui.button("📥 Paste into folder").clicked() {
        actions.push(Action::Paste(path.to_path_buf()));
    }
    if ui.button("🖳 Open in Terminal").clicked() {
        actions.push(Action::OpenTerminal(path.to_path_buf()));
    }
    if ui.button("🔗 Copy path").clicked() {
        actions.push(Action::CopyPathText(vec![path.to_path_buf()]));
    }
    if ui.button("ℹ Properties").clicked() {
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
