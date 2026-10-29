//! All drawing code. Each submodule draws one area of the window.

pub mod cloud;
mod compare;
mod dialogs;
mod files;
pub mod header;
pub(crate) mod help_page;
mod home;
mod menus;
pub(crate) mod palette;
pub(crate) mod preview;
mod settings_page;
mod sidebar;
mod transfers;

pub use menus::*;
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

    if app.ui.show_transfers
        && (!app.engine.jobs().is_empty() || !app.engine.resumable().is_empty())
    {
        egui::Panel::bottom("transfers")
            .resizable(true)
            .default_size(150.0)
            .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::same(8)))
            .show(ui, |ui| transfers::show(app, ui));
    }

    // The side panels have fixed widths that line up with the header. egui
    // draws a panel's edge line half a point inside the panel.
    let area = ui.available_rect_before_wrap();
    if app.settings.sidebar.visible {
        let width = app
            .ui
            .sidebar_edge
            .map_or(SIDEBAR_WIDTH, |x| x + 0.5 - area.left());
        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(width)
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

    let preview_shown =
        app.settings.browsing.show_preview && app.tabs.active().location().shows_files();
    let as_window = app.settings.preview.as_window;
    // The pane only opens for a selection (the window stays open).
    let pane_shown =
        preview_shown && !as_window && !app.tabs.active().view.selected_entries().is_empty();
    if !pane_shown && !(preview_shown && as_window) {
        // Nothing plays where it can't be seen (or stopped).
        app.preview.stop_playback();
        // It opens on the Info tab again.
        app.preview.reset_tab();
    }
    if pane_shown {
        let width = app
            .ui
            .preview_edge
            .map_or(PREVIEW_WIDTH, |x| area.right() - x + 0.5);
        egui::Panel::right("preview")
            .resizable(false)
            .exact_size(width)
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

    if preview_shown && app.settings.preview.as_window {
        preview_window(app, ui.ctx());
    }
    dialogs::show(app, ui.ctx());
    palette::show(app, ui.ctx());
    toasts(app, ui.ctx());
    transfers::countdown(app, ui.ctx());
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
    crate::icons::paint_glyph_centered(
        ui.painter(),
        rect.center(),
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
    compare::bar(app, ui);
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
    compare::split_button(app, ui, egui::pos2(rect.left() + half, rect.top()));
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
        // As high with nothing in it (Settings) as with text, so it doesn't jump.
        let text = ui.text_style_height(&egui::TextStyle::Body);
        ui.set_min_height(text.max(ui.spacing().interact_size.y));
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

/// Widths of the sidebar and the preview pane until the header has been
/// measured (in points).
const SIDEBAR_WIDTH: f32 = 180.0;
const PREVIEW_WIDTH: f32 = 280.0;

/// The preview as a separate window (Settings → Preview pane → Open as a window).
fn preview_window(app: &mut OxApp, ctx: &egui::Context) {
    let mut open = true;
    let screen = ctx.content_rect();
    let width = 320.0;
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
