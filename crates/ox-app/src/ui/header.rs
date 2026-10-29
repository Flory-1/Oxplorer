//! Header bar: tab strip with tab groups, navigation/address/search row and command bar.

use ox_core::i18n::t as tr;
use std::path::{Path, PathBuf};

use egui::{
    vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke, Ui,
};
use ox_core::entry::{display_name, SortKey, SortOrder};
use ox_core::settings::ViewMode;

use super::{DragItems, DragTab};
use crate::app::{Action, OxApp, TabAction};
use crate::icons::Icon;
use crate::shortcuts::{self, Command};
use crate::tabs::{Location, TabGroup, GROUP_COLORS};

const TAB_HEIGHT: f32 = 32.0;
/// Caption buttons (minimize, maximize, close), as wide as Windows 11's.
const CAPTION_W: f32 = 46.0;
/// Free space after the tabs to grab the window by.
const DRAG_GAP: f32 = 40.0;

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    tab_strip(app, ui);
    ui.add_space(4.0);
    nav_row(app, ui);
    ui.add_space(2.0);
    command_bar(app, ui);
}

fn group_color(g: &TabGroup) -> Color32 {
    let [r, gg, b] = GROUP_COLORS[g.color % GROUP_COLORS.len()].1;
    Color32::from_rgb(r, gg, b)
}

// ---- tab strip -------------------------------------------------------------

fn tab_strip(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    // Things to click in the strip, for the title bar's hit testing.
    let mut hits = Vec::new();
    let title_bar = app.ui.title_bar;
    let reserve = if title_bar {
        3.0 * CAPTION_W + DRAG_GAP
    } else {
        0.0
    };
    let active = app.tabs.active;
    let visible_tabs = app
        .tabs
        .tabs
        .iter()
        .filter(|t| {
            t.group
                .and_then(|g| app.tabs.group(g))
                .is_none_or(|g| !g.collapsed)
        })
        .count()
        .max(1);
    let chips = app.tabs.groups.len() as f32 * 70.0;
    let tab_w =
        ((ui.available_width() - 48.0 - chips - reserve) / visible_tabs as f32).clamp(110.0, 230.0);

    let row = ui.horizontal(|ui| {
        egui::ScrollArea::horizontal()
            .id_salt("tab_strip")
            .max_width(ui.available_width() - 40.0 - reserve)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let mut prev_group = None;
                    for idx in 0..app.tabs.tabs.len() {
                        let tab = &app.tabs.tabs[idx];
                        let group = tab.group.and_then(|g| app.tabs.group(g)).cloned();
                        if tab.group != prev_group {
                            prev_group = tab.group;
                            if let Some(g) = &group {
                                ui.add_space(4.0);
                                hits.push(group_chip(ui, g, &app.tabs.groups, &mut actions));
                            }
                        }
                        if group.as_ref().is_some_and(|g| g.collapsed) {
                            continue;
                        }
                        hits.push(tab_widget(
                            app,
                            ui,
                            idx,
                            tab_w,
                            active,
                            group.as_ref(),
                            &mut actions,
                        ));
                    }
                });
            });
        let plus = ui.add(
            egui::Button::new(RichText::new("➕").size(14.0))
                .frame(false)
                .min_size(vec2(28.0, TAB_HEIGHT)),
        );
        hits.push(plus.interact_rect);
        if plus
            .on_hover_text(shortcuts::hint(tr("New tab"), Command::NewTab))
            .clicked()
        {
            actions.push(Action::Tab(TabAction::New));
        }
    });
    if title_bar {
        let height = row.response.rect.bottom();
        let (buttons, max) = caption_buttons(ui.ctx(), height, &mut actions);
        hits.extend(buttons);
        app.ui.title_height = height;
        app.ui.max_button = Some(max);
    }
    app.ui.title_hits = hits;
    app.actions.extend(actions);
}

pub fn caption_buttons_id() -> egui::Id {
    egui::Id::unique("caption_buttons")
}

/// Minimize, maximize/restore and close at the top right, drawn like
/// Windows 11's. Returns the rectangles of minimize and close (clicked here)
/// and of maximize (Windows handles that one, for its snap layouts).
fn caption_buttons(
    ctx: &egui::Context,
    height: f32,
    actions: &mut Vec<Action>,
) -> ([Rect; 2], Rect) {
    use egui::ViewportCommand;
    let screen = ctx.content_rect();
    let size = vec2(CAPTION_W, height - screen.top());
    let origin = egui::pos2(screen.right() - 3.0 * CAPTION_W, screen.top());
    let (maximized, focused) = ctx.input(|i| {
        (
            i.viewport().maximized.unwrap_or(false),
            i.viewport().focused.unwrap_or(true),
        )
    });
    let native_max = ox_core::titlebar::max_button();
    let mut rects = [Rect::NOTHING; 3];
    egui::Area::new(caption_buttons_id())
        .fixed_pos(origin)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let dark = ui.visuals().dark_mode;
            let text = ui.visuals().strong_text_color();
            let glyph_color = if focused {
                text
            } else {
                text.gamma_multiply(0.45)
            };
            let ppp = ui.pixels_per_point();
            for (i, glyph) in [Glyph::Minimize, Glyph::Maximize(maximized), Glyph::Close]
                .into_iter()
                .enumerate()
            {
                let rect = Rect::from_min_size(origin + vec2(i as f32 * CAPTION_W, 0.0), size);
                rects[i] = rect;
                let resp = ui.interact(rect, egui::Id::unique(("caption", i)), Sense::click());
                // Windows tells the window about the mouse on the maximize
                // button (it shows the snap layouts), not egui.
                let (hovered, pressed) = if i == 1 && native_max != Default::default() {
                    (native_max.hovered, native_max.pressed)
                } else {
                    (resp.hovered(), resp.is_pointer_button_down_on())
                };
                let close = glyph == Glyph::Close;
                let fill = match (close, hovered, pressed) {
                    (_, false, _) => Color32::TRANSPARENT,
                    (true, true, false) => Color32::from_rgb(0xc4, 0x2b, 0x1c),
                    (true, true, true) => Color32::from_rgb(0xc8, 0x40, 0x31),
                    (false, true, pressed) => {
                        let a = if pressed { 0x0c } else { 0x16 };
                        if dark {
                            Color32::from_white_alpha(a)
                        } else {
                            Color32::from_black_alpha(a)
                        }
                    }
                };
                ui.painter().rect_filled(rect, 0.0, fill);
                let color = if close && hovered {
                    Color32::WHITE
                } else {
                    glyph_color
                };
                paint_glyph(ui.painter(), rect.center(), glyph, color, ppp);
                let tip = match glyph {
                    Glyph::Minimize => tr("Minimize"),
                    Glyph::Maximize(true) => tr("Restore"),
                    Glyph::Maximize(false) => tr("Maximize"),
                    Glyph::Close => tr("Close"),
                };
                if resp.on_hover_text(tip).clicked() {
                    let cmd = match glyph {
                        Glyph::Minimize => ViewportCommand::Minimized(true),
                        Glyph::Maximize(m) => ViewportCommand::Maximized(!m),
                        Glyph::Close => ViewportCommand::Close,
                    };
                    ctx.send_viewport_cmd(cmd);
                }
            }
        });
    let _ = actions;
    ([rects[0], rects[2]], rects[1])
}

#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    Minimize,
    /// Whether the window is maximized (then it shows "restore").
    Maximize(bool),
    Close,
}

/// The caption button symbols, 10 points wide with 1 pixel lines, on pixel
/// centres so they stay sharp.
fn paint_glyph(
    painter: &egui::Painter,
    center: egui::Pos2,
    glyph: Glyph,
    color: Color32,
    ppp: f32,
) {
    use egui::emath::GuiRounding;
    let stroke = Stroke::new(1.0 / ppp, color);
    let half = 5.0;
    let p = |x: f32, y: f32| egui::pos2(center.x + x, center.y + y).round_to_pixel_center(ppp);
    match glyph {
        Glyph::Minimize => {
            painter.line_segment([p(-half, 0.0), p(half, 0.0)], stroke);
        }
        Glyph::Maximize(false) => {
            let r = Rect::from_min_max(p(-half, -half), p(half, half));
            painter.rect_stroke(r, 1.0, stroke, egui::StrokeKind::Middle);
        }
        Glyph::Maximize(true) => {
            // Two windows: the front one lower left, the back one peeking out.
            let front = Rect::from_min_max(p(-half, -half + 2.0), p(half - 2.0, half));
            painter.rect_stroke(front, 1.0, stroke, egui::StrokeKind::Middle);
            painter.line_segment([p(-half + 2.0, -half), p(half, -half)], stroke);
            painter.line_segment([p(half, -half), p(half, half - 2.0)], stroke);
        }
        Glyph::Close => {
            let stroke = Stroke::new(1.1, color);
            let c = |x: f32, y: f32| egui::pos2(center.x + x, center.y + y);
            painter.line_segment([c(-half, -half), c(half, half)], stroke);
            painter.line_segment([c(-half, half), c(half, -half)], stroke);
        }
    }
}

fn group_chip(ui: &mut Ui, g: &TabGroup, all: &[TabGroup], actions: &mut Vec<Action>) -> Rect {
    let color = group_color(g);
    let label = if g.name.is_empty() {
        " ".to_owned()
    } else {
        g.name.clone()
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label, FontId::proportional(12.5), Color32::WHITE);
    let size = vec2((galley.size().x + 18.0).max(22.0), 22.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(size.x, TAB_HEIGHT), Sense::click());
    let chip = Rect::from_center_size(rect.center(), size);
    let fill = if g.collapsed {
        color.gamma_multiply(0.7)
    } else {
        color
    };
    ui.painter().rect_filled(chip, CornerRadius::same(11), fill);
    ui.painter()
        .galley(chip.center() - galley.size() / 2.0, galley, Color32::WHITE);
    let resp = resp.on_hover_text(if g.collapsed {
        tr("Expand group")
    } else {
        tr("Collapse group")
    });
    if resp.clicked() {
        actions.push(Action::Tab(TabAction::ToggleGroup(g.id)));
    }
    let _ = all;
    resp.context_menu(|ui| {
        if ui.button(tr("✏ Rename group…")).clicked() {
            actions.push(Action::Tab(TabAction::RenameGroup(g.id)));
        }
        ui.horizontal(|ui| {
            for (i, (name, [r, gg, b])) in GROUP_COLORS.iter().enumerate() {
                let c = Color32::from_rgb(*r, *gg, *b);
                let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                ui.painter().circle_filled(rect.center(), 8.0, c);
                if i == g.color {
                    ui.painter().circle_stroke(
                        rect.center(),
                        8.0,
                        Stroke::new(2.0, ui.visuals().strong_text_color()),
                    );
                }
                if resp.on_hover_text(tr(name)).clicked() {
                    actions.push(Action::Tab(TabAction::SetGroupColor(g.id, i)));
                    ui.close();
                }
            }
        });
        ui.separator();
        if ui.button(tr("➕ New tab in group")).clicked() {
            actions.push(Action::Tab(TabAction::NewTabInGroup(g.id)));
        }
        if ui
            .button(if g.collapsed {
                tr("⏵ Expand")
            } else {
                tr("⏷ Collapse")
            })
            .clicked()
        {
            actions.push(Action::Tab(TabAction::ToggleGroup(g.id)));
        }
        if ui.button(tr("⛓ Ungroup")).clicked() {
            actions.push(Action::Tab(TabAction::Ungroup(g.id)));
        }
        if ui.button(tr("✖ Close group")).clicked() {
            actions.push(Action::Tab(TabAction::CloseGroup(g.id)));
        }
    });
    rect.intersect(ui.clip_rect())
}

fn tab_widget(
    app: &OxApp,
    ui: &mut Ui,
    idx: usize,
    width: f32,
    active: u64,
    group: Option<&TabGroup>,
    actions: &mut Vec<Action>,
) -> Rect {
    let tab = &app.tabs.tabs[idx];
    let id = tab.id;
    let is_active = id == active;
    // In split view the other pane's tab looks open too, but without the accent.
    let is_partner = app.tabs.split_partner() == Some(id);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, TAB_HEIGHT), Sense::click_and_drag());
    let hit = resp.interact_rect;
    let visuals = ui.visuals().clone();

    // Reordering and dropping files onto tabs.
    resp.dnd_set_drag_payload(DragTab(id));
    let tab_hover = resp
        .dnd_hover_payload::<DragTab>()
        .is_some_and(|d| d.0 != id);
    if let Some(dragged) = resp.dnd_release_payload::<DragTab>() {
        if dragged.0 != id {
            actions.push(Action::Tab(TabAction::Move(dragged.0, idx)));
        }
    }
    let files_hover = resp.dnd_hover_payload::<DragItems>().is_some();
    if files_hover && !is_active {
        // Hovering files over a tab switches to it so they can be dropped inside.
        actions.push(Action::Tab(TabAction::Activate(id)));
    }
    if let Some(folder) = tab.location().folder() {
        super::drop_target(actions, &resp, folder);
    }

    let fill = if is_active || is_partner {
        visuals.extreme_bg_color
    } else if resp.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let body = rect.shrink2(vec2(0.0, 1.0));
    ui.painter().rect_filled(body, CornerRadius::same(7), fill);
    if is_active || is_partner {
        ui.painter().rect_stroke(
            body,
            CornerRadius::same(7),
            visuals.widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    if is_active && app.tabs.panes.is_some() {
        ui.painter().line_segment(
            [
                egui::pos2(body.left() + 7.0, body.top() + 1.0),
                egui::pos2(body.right() - 7.0, body.top() + 1.0),
            ],
            Stroke::new(2.0, crate::theme::accent(&app.settings)),
        );
    }
    if tab_hover {
        ui.painter().line_segment(
            [rect.left_top(), rect.left_bottom()],
            Stroke::new(2.0, crate::theme::accent(&app.settings)),
        );
    }
    if let Some(g) = group {
        let y = body.bottom() - 2.0;
        ui.painter().line_segment(
            [
                egui::pos2(body.left() + 6.0, y),
                egui::pos2(body.right() - 6.0, y),
            ],
            Stroke::new(2.5, group_color(g)),
        );
    }

    // Icon + title, clipped so it never runs under the close button.
    let text_color = if is_active {
        visuals.strong_text_color()
    } else {
        visuals.text_color()
    };
    let close_rect = Rect::from_center_size(
        egui::pos2(body.right() - 15.0, body.center().y),
        vec2(18.0, 18.0),
    );
    let painter = ui.painter().with_clip_rect(Rect::from_min_max(
        body.min,
        egui::pos2(close_rect.left() - 2.0, body.max.y),
    ));
    let icon = match tab.location() {
        Location::Folder(p) if p.parent().is_none() => Icon::Drive,
        Location::Folder(_) => Icon::Folder,
        Location::Tagged(c) => Icon::Dot(super::tag_color(*c)),
        other => Icon::Glyph(other.icon()),
    };
    let title = match tab.location() {
        Location::Tagged(c) => app.tags.label(*c).to_owned(),
        _ => tab.title(),
    };
    let accent = crate::theme::accent(&app.settings);
    crate::icons::paint_icon(
        &painter,
        crate::icons::icon_rect(body.left() + 10.0, body.center().y, 15.0),
        icon,
        text_color,
        accent,
    );
    painter.text(
        egui::pos2(body.left() + 30.0, body.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        text_color,
    );

    // Close button. It always exists, also on inactive tabs: once the pointer
    // is over it, it covers the tab, so the tab itself no longer counts as
    // hovered (making it depend on that hid it again and the click went to
    // the tab instead).
    let close = ui.interact(
        close_rect,
        egui::Id::unique(("close_tab", id)),
        Sense::click(),
    );
    if is_active || resp.hovered() || close.hovered() {
        if close.hovered() {
            ui.painter().rect_filled(
                close_rect,
                CornerRadius::same(4),
                visuals.widgets.hovered.bg_fill,
            );
        }
        ui.painter().text(
            close_rect.center(),
            Align2::CENTER_CENTER,
            "✖",
            FontId::proportional(11.0),
            text_color,
        );
        if close.clicked() {
            actions.push(Action::Tab(TabAction::Close(id)));
            return hit;
        }
    }

    let resp = resp.on_hover_text(match tab.location() {
        Location::Folder(p) => p.display().to_string(),
        other => other.title(),
    });
    if resp.clicked() {
        actions.push(Action::Tab(TabAction::Activate(id)));
    }
    if resp.middle_clicked() {
        actions.push(Action::Tab(TabAction::Close(id)));
    }
    resp.context_menu(|ui| {
        if ui.button(tr("📑 Duplicate")).clicked() {
            actions.push(Action::Tab(TabAction::Duplicate(id)));
        }
        ui.separator();
        if ui.button(tr("🔖 Add tab to new group")).clicked() {
            actions.push(Action::Tab(TabAction::NewGroup(id)));
        }
        let others: Vec<&TabGroup> = app
            .tabs
            .groups
            .iter()
            .filter(|g| Some(g.id) != tab.group)
            .collect();
        if !others.is_empty() {
            ui.menu_button(tr("🔖 Add tab to group"), |ui| {
                for g in others {
                    let name = if g.name.is_empty() {
                        ox_core::trf!(
                            "{v1} group",
                            v1 = tr(GROUP_COLORS[g.color % GROUP_COLORS.len()].0)
                        )
                    } else {
                        g.name.clone()
                    };
                    if ui
                        .button(RichText::new(format!("● {name}")).color(group_color(g)))
                        .clicked()
                    {
                        actions.push(Action::Tab(TabAction::AddToGroup(id, g.id)));
                    }
                }
            });
        }
        if tab.group.is_some() && ui.button(tr("⛓ Remove from group")).clicked() {
            actions.push(Action::Tab(TabAction::RemoveFromGroup(id)));
        }
        ui.separator();
        if ui
            .add(
                egui::Button::new(tr("✖ Close")).shortcut_text(shortcuts::label(Command::CloseTab)),
            )
            .clicked()
        {
            actions.push(Action::Tab(TabAction::Close(id)));
        }
        if ui.button(tr("Close other tabs")).clicked() {
            actions.push(Action::Tab(TabAction::CloseOthers(id)));
        }
    });
    hit
}

// ---- navigation row ------------------------------------------------------------

fn nav_row(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        let tab = app.tabs.active();
        let nav = |ui: &mut Ui, icon: &str, enabled: bool, tip: &str| {
            ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(icon).size(15.0))
                    .frame(false)
                    .min_size(vec2(28.0, 28.0)),
            )
            .on_hover_text(tip)
            .clicked()
        };
        if nav(
            ui,
            "⬅",
            tab.can_go_back(),
            &shortcuts::hint(tr("Back"), Command::Back),
        ) {
            actions.push(Action::Back);
        }
        if nav(
            ui,
            "➡",
            tab.can_go_forward(),
            &shortcuts::hint(tr("Forward"), Command::Forward),
        ) {
            actions.push(Action::Forward);
        }
        let can_up = !matches!(tab.location(), Location::Home);
        if nav(ui, "⬆", can_up, &shortcuts::hint(tr("Up"), Command::Up)) {
            actions.push(Action::Up);
        }
        if nav(
            ui,
            "🔄",
            true,
            &shortcuts::hint(tr("Refresh"), Command::Refresh),
        ) {
            actions.push(Action::Refresh);
        }

        let search_w = 260.0_f32.min(ui.available_width() * 0.35);
        let addr_w = ui.available_width() - search_w - 8.0;
        ui.allocate_ui(vec2(addr_w, 30.0), |ui| address_bar(app, ui, &mut actions));
        search_box(app, ui, search_w, &mut actions);
    });
    app.actions.extend(actions);
}

fn field_frame(ui: &Ui, focused: bool, accent: Color32) -> egui::Frame {
    let v = ui.visuals();
    egui::Frame::new()
        .fill(v.extreme_bg_color)
        .stroke(if focused {
            Stroke::new(1.5, accent)
        } else {
            v.widgets.noninteractive.bg_stroke
        })
        .corner_radius(CornerRadius::same(7))
        .inner_margin(egui::Margin::symmetric(6, 2))
}

fn address_bar(app: &mut OxApp, ui: &mut Ui, actions: &mut Vec<Action>) {
    let accent = crate::theme::accent(&app.settings);
    let location = app.tabs.active().location().clone();
    let width = ui.available_width();
    field_frame(ui, app.ui.address_editing, accent).show(ui, |ui| {
        ui.set_width(width - 14.0);
        ui.set_height(24.0);
        if app.ui.address_editing {
            let edit = egui::TextEdit::singleline(&mut app.ui.address_text)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .align(Align2::LEFT_CENTER)
                .font(FontId::proportional(13.5));
            let resp = ui.add(edit);
            if app.ui.focus_address {
                resp.request_focus();
                app.ui.focus_address = false;
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                    let all = egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(app.ui.address_text.chars().count()),
                    );
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), resp.id);
                }
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let text = app.ui.address_text.trim().trim_matches('"').to_owned();
                    let path = PathBuf::from(expand_env(&text));
                    if text.is_empty() {
                    } else if ox_core::network::is_dir(&path) {
                        actions.push(Action::Navigate(Location::Folder(path)));
                    } else if path.is_file() {
                        actions.push(Action::Open(path));
                    } else {
                        actions.push(Action::Toast(ox_core::trf!(
                            "Can't find \"{text}\".",
                            text = text
                        )));
                    }
                }
                app.ui.address_editing = false;
            }
            return;
        }

        app.ui.address_text = match &location {
            Location::Folder(p) => p.display().to_string(),
            _ => String::new(),
        };
        let tag_label = match &location {
            Location::Tagged(c) => app.tags.label(*c).to_owned(),
            _ => String::new(),
        };
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            match &location {
                Location::Tagged(_) => {
                    ui.label(ox_core::trf!(
                        "🏷  Items tagged \"{tag_label}\"",
                        tag_label = tag_label
                    ));
                }
                Location::Folder(p) => breadcrumbs(ui, p, actions),
                Location::Home => {
                    ui.label(ox_core::trf!("{v1}  Home", v1 = crate::icons::HOME));
                }
                Location::Settings => {
                    ui.label(tr("⚙  Settings"));
                }
                Location::Help => {
                    ui.label(tr("❓  Help"));
                }
                Location::RecycleBin => {
                    ui.label(tr("🗑  Recycle Bin"));
                }
                Location::Archive { archive, inner } => {
                    if let Some(parent) = archive.parent() {
                        breadcrumbs(ui, parent, actions);
                    }
                    archive_crumbs(ui, archive, inner, actions);
                }
                Location::Search { root, query } => {
                    let place = root
                        .as_deref()
                        .map(display_name)
                        .unwrap_or_else(|| tr("This PC").into());
                    ui.label(ox_core::trf!(
                        "🔍  Search results for \"{query}\" in {place}",
                        query = query,
                        place = place
                    ));
                }
            }
            // Clicking the empty part of the bar switches to text editing.
            let rest = ui.available_size();
            if ui
                .allocate_response(rest, Sense::click())
                .on_hover_cursor(egui::CursorIcon::Text)
                .clicked()
            {
                app.ui.address_editing = true;
                app.ui.focus_address = true;
            }
        });
    });
}

/// `%USERPROFILE%\Desktop` → `C:\Users\me\Desktop`, and `~` → home on all platforms.
fn expand_env(text: &str) -> String {
    let mut out = text.to_owned();
    if let Some(rest) = out.strip_prefix('~') {
        out = format!("{}{rest}", ox_core::places::home_dir().display());
    }
    while let Some(start) = out.find('%') {
        let Some(len) = out[start + 1..].find('%') else {
            break;
        };
        let name = &out[start + 1..start + 1 + len];
        match std::env::var(name) {
            Ok(v) => out.replace_range(start..start + len + 2, &v),
            Err(_) => break,
        }
    }
    out
}

fn breadcrumbs(ui: &mut Ui, path: &Path, actions: &mut Vec<Action>) {
    let server = ox_core::network::server_of(path);
    let mut parts: Vec<&Path> = path.ancestors().collect();
    // `\\server\share` paths start at the server, which lists its shares.
    parts.extend(server.as_deref());
    parts.reverse();
    const MAX: usize = 5;
    let hidden = parts.len().saturating_sub(MAX);
    if hidden > 0 {
        crumb_menu(ui, RichText::new("«"), |ui| {
            for p in &parts[..hidden] {
                if ui.button(format!("📁 {}", crumb_name(p))).clicked() {
                    actions.push(Action::Navigate(Location::Folder(p.to_path_buf())));
                }
            }
        });
    }
    for (i, p) in parts.iter().enumerate().skip(hidden) {
        let last = i + 1 == parts.len();
        let text = RichText::new(crumb_name(p)).size(13.5);
        let resp = ui.add(egui::Button::new(if last { text.strong() } else { text }).frame(false));
        if resp.clicked() {
            actions.push(Action::Navigate(Location::Folder(p.to_path_buf())));
        }
        if resp.middle_clicked() {
            actions.push(Action::OpenInNewTab(
                Location::Folder(p.to_path_buf()),
                false,
            ));
        }
        super::drop_target(actions, &resp, p);
        // "›" opens a menu of the sub-folders, like Explorer's address bar.
        crumb_menu(ui, RichText::new("›").size(14.0), |ui| {
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    for sub in subfolders(p) {
                        if ui.button(format!("📁 {}", display_name(&sub))).clicked() {
                            actions.push(Action::Navigate(Location::Folder(sub)));
                        }
                    }
                });
        });
    }
}

/// A frameless menu button for the address bar's `«` and `›` arrows.
fn crumb_menu(ui: &mut Ui, text: RichText, content: impl FnOnce(&mut Ui)) {
    egui::containers::menu::MenuButton::from_button(
        egui::Button::new(text)
            .frame(false)
            .min_size(egui::vec2(18.0, 0.0)),
    )
    .ui(ui, content);
}

/// The archive and the folders inside it, after the folder crumbs.
fn archive_crumbs(ui: &mut Ui, archive: &Path, inner: &Path, actions: &mut Vec<Action>) {
    let mut parts = vec![PathBuf::new()];
    for c in inner.components() {
        let next = parts.last().cloned().unwrap_or_default().join(c);
        parts.push(next);
    }
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let name = if i == 0 {
            format!("📦 {}", display_name(archive))
        } else {
            display_name(part)
        };
        let text = RichText::new(name).size(13.5);
        let text = if last { text.strong() } else { text };
        if ui.add(egui::Button::new(text).frame(false)).clicked() {
            actions.push(Action::Navigate(Location::Archive {
                archive: archive.to_path_buf(),
                inner: part.clone(),
            }));
        }
        // "›" lists the folders inside, like for ordinary folders.
        crumb_menu(ui, RichText::new("›").size(14.0), |ui| {
            let mut dirs: Vec<_> = ox_core::archive::list(archive, part)
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.is_dir)
                .collect();
            dirs.sort_by(|a, b| ox_core::natural::natural_cmp(&a.name, &b.name));
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    for d in dirs {
                        if ui.button(format!("📁 {}", d.name)).clicked() {
                            actions.push(Action::Navigate(Location::Archive {
                                archive: archive.to_path_buf(),
                                inner: part.join(&d.name),
                            }));
                        }
                    }
                });
        });
    }
}

fn crumb_name(p: &Path) -> String {
    if let Some(server) = ox_core::network::server_name(p) {
        return server.to_owned();
    }
    if p.parent().is_none() && ox_core::network::server_of(p).is_some() {
        // A share's root: just the share, after the server crumb.
        let s = p.to_string_lossy();
        return s
            .trim_end_matches('\\')
            .rsplit('\\')
            .next()
            .unwrap_or("")
            .to_owned();
    }
    if p.parent().is_none() {
        let s = p.display().to_string();
        let s = s.trim_end_matches(['\\', '/']);
        if s.is_empty() {
            "/".into()
        } else {
            s.to_owned()
        }
    } else {
        display_name(p)
    }
}

fn subfolders(p: &Path) -> Vec<PathBuf> {
    if let Some(server) = ox_core::network::server_name(p) {
        let shares = ox_core::network::shares(server).unwrap_or_default();
        return shares.into_iter().map(|e| e.path).collect();
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .map(|e| e.path())
                .take(300)
                .collect()
        })
        .unwrap_or_default();
    dirs.sort_by(|a, b| ox_core::natural::natural_cmp(&display_name(a), &display_name(b)));
    dirs
}

fn search_box(app: &mut OxApp, ui: &mut Ui, width: f32, actions: &mut Vec<Action>) {
    let accent = crate::theme::accent(&app.settings);
    let folders_first = app.settings.browsing.folders_first;
    let case = app.settings.search.case_sensitive;
    let instant = app.settings.search.instant_filter;
    let focus = std::mem::take(&mut app.ui.focus_search);
    // With the index, Home and search tabs show results while typing.
    let live = app.settings.search.search_as_you_type && app.index_ready();
    let tab = app.tabs.active_mut();
    let live_here = live && matches!(tab.location(), Location::Home | Location::Search { .. });
    let (root, place) = match tab.location() {
        Location::Folder(p) => (Some(p.clone()), display_name(p)),
        Location::Search { root, .. } => (
            root.clone(),
            root.as_deref()
                .map(display_name)
                .unwrap_or(tr("This PC").into()),
        ),
        Location::RecycleBin => (None, tr("Recycle Bin").into()),
        _ => (None, tr("This PC").into()),
    };
    let in_bin = tab.location().is_bin();
    let id = ui.make_persistent_id("search_box");
    let focused = ui.memory(|m| m.has_focus(id));
    ui.allocate_ui(vec2(width, 30.0), |ui| {
        field_frame(ui, focused, accent).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new("🔍").weak());
                let edit = egui::TextEdit::singleline(&mut tab.filter)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .hint_text(ox_core::trf!("Search {place}", place = place))
                    .desired_width(ui.available_width() - 4.0)
                    .align(Align2::LEFT_CENTER);
                let resp = ui.add(edit);
                if focus {
                    resp.request_focus();
                }
                                if resp.changed() && instant {
                    tab.refresh_visible(folders_first, case);
                }
                if resp.changed() && live_here {
                    actions.push(Action::SearchLive(root.clone(), tab.filter.clone()));
                }
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if resp.lost_focus() && enter && !in_bin {
                    actions.push(Action::Search(root, tab.filter.clone()));
                } else if resp.lost_focus() && escape {
                    tab.filter.clear();
                    tab.refresh_visible(folders_first, case);
                }
                resp.on_hover_text(tr(
                    "Type to filter this folder, press Enter to search all sub-folders.\n\
                     On Home, results appear while you type (with the search index).\n\
                     Filters: *.ext  /regex/  ext:pdf,docx  kind:image  type:folder  size:>10mb  modified:7d  content:\"text\"  -exclude",
                ));
            });
        });
    });
}

// ---- command bar -------------------------------------------------------------------

fn command_bar(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    let folder = app.current_folder();
    let tab = app.tabs.active();
    let selection = tab.view.selected_paths();
    let shows_files = tab.location().shows_files();
    let (key, order, mode) = (tab.sort_key, tab.sort_order, tab.view_mode);
    let has_sel = !selection.is_empty() && shows_files;
    // The system clipboard is only read on paste, so with it enabled Paste is always offered.
    let can_paste = folder.is_some()
        && (app.settings.transfer.use_system_clipboard || app.clipboard.get(false).is_some());
    let pinned = folder.as_deref().is_some_and(|f| app.quick.is_favorite(f));
    let in_bin = tab.location().is_bin();
    let bin_empty = tab.view.entries.is_empty();

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
                    super::new_menu(ui, f, &app.settings.new_items.0, &mut actions);
                }
            });
        });
        ui.separator();
        if in_bin {
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
        ui.separator();

        ui.add_enabled_ui(shows_files, |ui| {
            ui.menu_button(RichText::new(tr("⇅ Sort")).size(13.5), |ui| {
                let mut new_key = key;
                let mut new_order = order;
                for (k, label) in [
                    (SortKey::Name, tr("Name")),
                    (SortKey::Modified, tr("Date modified")),
                    (SortKey::Type, tr("Type")),
                    (SortKey::Size, tr("Size")),
                ] {
                    ui.radio_value(&mut new_key, k, label);
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
