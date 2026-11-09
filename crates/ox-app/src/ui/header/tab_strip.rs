//! Header tab strip: tabs, tab groups and the drag gap.

use super::*;

pub(super) fn tab_strip(app: &mut OxApp, ui: &mut Ui) {
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
    // The Home tab and the pinned tabs show only their icon.
    let small = app.tabs.pinned_end();
    let visible_tabs = app
        .tabs
        .tabs
        .iter()
        .skip(small)
        .filter(|t| {
            t.group
                .and_then(|g| app.tabs.group(g))
                .is_none_or(|g| !g.collapsed)
        })
        .count()
        .max(1);
    let chips = app.tabs.groups.len() as f32 * 70.0;
    let small_w = small as f32 * (HOME_TAB_W + 2.0);
    let tab_w = ((ui.available_width() - 48.0 - chips - reserve - small_w) / visible_tabs as f32)
        .clamp(110.0, 230.0);

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
        let plus = plus.on_hover_text(format!(
            "{}\n{}",
            shortcuts::hint(tr("New tab"), Command::NewTab),
            tr("Right-click for sessions")
        ));
        if plus.clicked() {
            actions.push(Action::Tab(TabAction::New));
        }
        plus.context_menu(|ui| {
            new_window_button(ui, &mut actions);
            ui.separator();
            super::super::sessions::sessions_menu(ui, &mut app.sessions, &mut actions);
        });
    });
    let strip = row.response.rect;
    ui.ctx()
        .data_mut(|d| d.insert_temp(super::tab_strip_rect_id(), strip));
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
    // The fixed Home tab: only its icon, no close button, stays first.
    let home = app.tabs.is_home(id);
    // Pinned tabs look like it, but can be moved (among themselves) and closed.
    let pinned = tab.pinned;
    let width = if home || pinned { HOME_TAB_W } else { width };
    let is_active = id == active;
    // In split view the other pane's tab looks open too, but without the accent.
    let is_partner = app.tabs.split_partner() == Some(id);
    let sense = if home {
        Sense::click()
    } else {
        Sense::click_and_drag()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, TAB_HEIGHT), sense);
    let hit = resp.interact_rect;
    let visuals = ui.visuals().clone();

    // Reordering and dropping files onto tabs.
    if !home {
        crate::ui::start_drag(&resp, DragTab(id));
    }
    // Pinned and other tabs don't mix, so only show where it can go.
    let tab_hover = resp
        .dnd_hover_payload::<DragTab>()
        .is_some_and(|d| d.0 != id && app.tabs.is_pinned(d.0) == pinned && !home);
    if let Some(dragged) = crate::ui::dropped::<DragTab>(&resp) {
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
        super::super::drop_target(actions, &resp, folder);
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
        Location::Tagged(c) => Icon::Dot(super::super::tag_color(*c)),
        other => Icon::Glyph(other.icon()),
    };
    let title = match tab.location() {
        Location::Tagged(c) => app.tags.label(*c).to_owned(),
        _ => tab.title(),
    };
    let accent = crate::theme::accent(&app.settings);
    if home || pinned {
        crate::icons::paint_icon(
            ui.painter(),
            crate::icons::icon_rect(body.center().x - 7.5, body.center().y, 15.0),
            icon,
            text_color,
            accent,
        );
    }
    if home {
        let resp = resp.on_hover_text(tr("Home"));
        if resp.clicked() {
            actions.push(Action::Tab(TabAction::Activate(id)));
        }
        resp.context_menu(|ui| {
            if ui
                .add(
                    egui::Button::new(tr("➕ New tab"))
                        .shortcut_text(shortcuts::label(Command::NewTab)),
                )
                .clicked()
            {
                actions.push(Action::Tab(TabAction::New));
            }
            new_window_button(ui, actions);
        });
        return hit;
    }
    if pinned {
        let resp = resp.on_hover_text(match tab.location() {
            Location::Folder(p) => format!("{title}\n{}", p.display()),
            _ => title,
        });
        if resp.clicked() {
            actions.push(Action::Tab(TabAction::Activate(id)));
        }
        // No close button, and middle-click doesn't close it either (easy to
        // do by accident on a small tab); its menu and Ctrl+W still do.
        tab_menu(app, &resp, id, actions);
        return hit;
    }
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
        // Centred by its visible pixels: the icon font's line metrics would
        // put it off centre.
        crate::icons::paint_glyph_centered(
            ui.painter(),
            close_rect.center(),
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
    tab_menu(app, &resp, id, actions);
    hit
}

/// The context menu of a tab (other than the Home tab).
fn tab_menu(app: &OxApp, resp: &egui::Response, id: crate::tabs::TabId, actions: &mut Vec<Action>) {
    let Some(tab) = app.tabs.get(id) else {
        return;
    };
    resp.context_menu(|ui| {
        if ui.button(tr("📑 Duplicate")).clicked() {
            actions.push(Action::Tab(TabAction::Duplicate(id)));
        }
        if ui.button(tr("🗔 Move to new window")).clicked() {
            actions.push(Action::Tab(TabAction::ToNewWindow(id, None)));
        }
        if tab.pinned {
            if ui.button(tr("📍 Unpin tab")).clicked() {
                actions.push(Action::Tab(TabAction::Pin(id, false)));
            }
        } else if ui.button(tr("📌 Pin tab")).clicked() {
            actions.push(Action::Tab(TabAction::Pin(id, true)));
        }
        ui.separator();
        // Pinned tabs aren't in groups (joining one unpins them).
        if !tab.pinned {
            group_items(app, ui, id, tab.group, actions);
            ui.separator();
        }
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
}

fn new_window_button(ui: &mut Ui, actions: &mut Vec<Action>) {
    if ui
        .add(
            egui::Button::new(tr("🗔 New window"))
                .shortcut_text(shortcuts::label(Command::NewWindow)),
        )
        .clicked()
    {
        actions.push(Action::Tab(TabAction::NewWindow));
    }
}

/// "Add tab to new group", "Add tab to group" and "Remove from group".
fn group_items(
    app: &OxApp,
    ui: &mut Ui,
    id: crate::tabs::TabId,
    group: Option<crate::tabs::GroupId>,
    actions: &mut Vec<Action>,
) {
    if ui.button(tr("🔖 Add tab to new group")).clicked() {
        actions.push(Action::Tab(TabAction::NewGroup(id)));
    }
    let others: Vec<&TabGroup> = app
        .tabs
        .groups
        .iter()
        .filter(|g| Some(g.id) != group)
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
    if group.is_some() && ui.button(tr("⛓ Remove from group")).clicked() {
        actions.push(Action::Tab(TabAction::RemoveFromGroup(id)));
    }
}
