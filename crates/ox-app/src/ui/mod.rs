//! All drawing code. Each submodule draws one area of the window.

mod dialogs;
mod files;
mod header;
mod home;
mod settings_page;
mod sidebar;
mod transfers;

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, FontId, Id, LayerId, Order, Response, Ui};
use ox_core::entry::display_name;
use ox_core::format;
use ox_core::transfer::TransferKind;

use crate::app::{Action, OxApp};
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

    let central = egui::Frame::central_panel(ui.style())
        .inner_margin(egui::Margin::same(0))
        .fill(ui.visuals().extreme_bg_color);
    egui::CentralPanel::default().frame(central).show(ui, |ui| {
        match app.tabs.active().location().clone() {
            Location::Home => home::show(app, ui),
            Location::Settings => settings_page::show(app, ui),
            Location::Folder(_) | Location::Search { .. } => files::show(app, ui),
        }
    });

    dialogs::show(app, ui.ctx());
    toasts(app, ui.ctx());
    drag_preview(ui.ctx());
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
                let bytes: u64 = selected.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
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
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(summary) = app.transfer_summary() {
                if ui.link(summary).clicked() {
                    app.ui.show_transfers = true;
                }
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
    pub single_dir: Option<&'a Path>,
    pub pinned: bool,
    pub can_paste: bool,
    pub destinations: &'a [(String, PathBuf)],
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

pub fn item_menu(ui: &mut Ui, t: &MenuTarget<'_>, actions: &mut Vec<Action>) {
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
    ui.separator();
    if ui
        .add(egui::Button::new("✂ Cut").shortcut_text("Ctrl+X"))
        .clicked()
    {
        actions.push(Action::CutItems(sel.clone()));
    }
    if ui
        .add(egui::Button::new("📋 Copy").shortcut_text("Ctrl+C"))
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
            .add(egui::Button::new("✏ Rename").shortcut_text("F2"))
            .clicked()
    {
        actions.push(Action::Rename(sel[0].clone()));
    }
    if ui
        .add(egui::Button::new("🗑 Delete").shortcut_text("Del"))
        .clicked()
    {
        actions.push(Action::Delete(sel.clone(), false));
    }
    if ui
        .add(egui::Button::new("⛔ Delete permanently").shortcut_text("Shift+Del"))
        .clicked()
    {
        actions.push(Action::Delete(sel.clone(), true));
    }
    ui.separator();
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
        .add(egui::Button::new("🔗 Copy path").shortcut_text("Ctrl+Shift+C"))
        .clicked()
    {
        actions.push(Action::CopyPathText(sel.clone()));
    }
    if sel.len() == 1 && ui.button("🗔 Show in Windows Explorer").clicked() {
        actions.push(Action::Reveal(sel[0].clone()));
    }
    ui.separator();
    if ui
        .add(egui::Button::new("ℹ Properties").shortcut_text("Alt+Enter"))
        .clicked()
    {
        actions.push(Action::Properties(sel));
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
            egui::Button::new("📥 Paste").shortcut_text("Ctrl+V"),
        )
        .clicked()
    {
        actions.push(Action::Paste(folder.to_path_buf()));
    }
    if ui
        .add(egui::Button::new("🔄 Refresh").shortcut_text("F5"))
        .clicked()
    {
        actions.push(Action::Refresh);
    }
    if ui
        .add(egui::Button::new("☑ Select all").shortcut_text("Ctrl+A"))
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
