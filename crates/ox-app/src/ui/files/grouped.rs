//! Group by: the collapsible group headers in the details list and the
//! grid, the grid laid out group by group, and the *Group by* menu.

use super::*;
use crate::app::ViewAction;
use ox_core::grid_zoom::Tile;
use ox_core::grouping::{GroupBy, GroupRow};

pub(super) const HEADER_H: f32 = 30.0;

/// The *Group by* choices (in the Sort menu).
pub(crate) fn group_by_menu(ui: &mut Ui, current: GroupBy, actions: &mut Vec<Action>) {
    for by in GroupBy::ALL {
        if ui
            .add(egui::Button::selectable(current == by, tr(by.label())))
            .clicked()
        {
            actions.push(Action::View(ViewAction::GroupBy(by)));
            ui.close();
        }
    }
    if current != GroupBy::None {
        ui.separator();
        if ui.button(tr("Collapse all groups")).clicked() {
            actions.push(Action::View(ViewAction::CollapseAll(true)));
        }
        if ui.button(tr("Expand all groups")).clicked() {
            actions.push(Action::View(ViewAction::CollapseAll(false)));
        }
    }
}

/// A group's header: ⏷/⏵, its name and how many items it has. Clicking
/// it collapses or expands the group.
pub(super) fn header_row(
    ui: &mut Ui,
    rect: Rect,
    group: &GroupRow,
    tab_id: u64,
    cx: &Ctx<'_>,
    actions: &mut Vec<Action>,
) {
    let resp = ui.interact(
        rect,
        Id::unique(("group_header", tab_id, &group.id)),
        Sense::click(),
    );
    let v = ui.visuals();
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), v.widgets.hovered.weak_bg_fill);
    }
    let y = rect.center().y;
    let chevron = if group.collapsed { "⏵" } else { "⏷" };
    let p = ui.painter().with_clip_rect(rect);
    p.text(
        egui::pos2(rect.left() + 10.0, y),
        Align2::LEFT_CENTER,
        chevron,
        FontId::proportional(12.0),
        v.weak_text_color(),
    );
    let label = p.text(
        egui::pos2(rect.left() + 28.0, y),
        Align2::LEFT_CENTER,
        &group.label,
        FontId::proportional(13.5),
        cx.accent,
    );
    let count = p.text(
        egui::pos2(label.right() + 8.0, y),
        Align2::LEFT_CENTER,
        format!("({})", group.count),
        FontId::proportional(12.5),
        v.weak_text_color(),
    );
    let line_x = count.right() + 10.0;
    if line_x < rect.right() - 8.0 {
        p.line_segment(
            [egui::pos2(line_x, y), egui::pos2(rect.right() - 8.0, y)],
            v.widgets.noninteractive.bg_stroke,
        );
    }
    if resp.clicked() {
        actions.push(Action::View(ViewAction::ToggleGroup(group.id.clone())));
    }
    resp.context_menu(|ui| {
        let text = if group.collapsed {
            tr("Expand group")
        } else {
            tr("Collapse group")
        };
        if ui.button(text).clicked() {
            actions.push(Action::View(ViewAction::ToggleGroup(group.id.clone())));
        }
        ui.separator();
        if ui.button(tr("Collapse all groups")).clicked() {
            actions.push(Action::View(ViewAction::CollapseAll(true)));
        }
        if ui.button(tr("Expand all groups")).clicked() {
            actions.push(Action::View(ViewAction::CollapseAll(false)));
        }
    });
}

/// The grid with a header above each group; each group starts a new row.
/// The tiles are the plain grid's (`grid::paint_tile`).
pub(super) fn grid(
    ui: &mut Ui,
    view: &mut FolderView,
    tab_id: u64,
    cx: &Ctx<'_>,
    actions: &mut Vec<Action>,
    tile: &Tile,
    gap: f32,
) {
    let size = vec2(tile.width, tile.height);
    let width = ui.available_width();
    let cols = (((width - 12.0) / (tile.width + gap)).floor() as usize).max(1);
    ui.data_mut(|d| d.insert_temp(Id::unique(("grid_columns", tab_id)), cols));
    let row_h = tile.height + gap;
    let groups = view.groups.clone();
    // Where each group's header is, from the top of the content.
    let mut tops = Vec::with_capacity(groups.len());
    let mut y = 0.0;
    for g in &groups {
        tops.push(y);
        y += HEADER_H + g.len.div_ceil(cols) as f32 * row_h;
    }
    let total = y;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("grid", tab_id))
        .auto_shrink([false, false]);
    if let Some(target) = view.scroll_to.take() {
        if let Some(g) = ox_core::grouping::group_of(&groups, target) {
            let row = (target - groups[g].start) / cols;
            let y = tops[g] + HEADER_H + row as f32 * row_h;
            // Show the group's header too when going to its first row.
            let (y, h) = if row == 0 {
                (tops[g], HEADER_H + row_h)
            } else {
                (y, row_h)
            };
            area = scroll_into_view(ui, area, ("grid", tab_id), y, h);
        }
    }
    let out = area.show_viewport(ui, |ui, viewport| {
        let origin = ui.min_rect().min;
        ui.set_min_size(vec2(width, total));
        for (g, group) in groups.iter().enumerate() {
            let top = tops[g];
            let rows = group.len.div_ceil(cols);
            let bottom = top + HEADER_H + rows as f32 * row_h;
            if bottom < viewport.min.y || top > viewport.max.y {
                continue;
            }
            let head =
                Rect::from_min_size(origin + vec2(4.0, top), vec2(width - 8.0, HEADER_H - 4.0));
            header_row(ui, head, group, tab_id, cx, actions);
            let first = ((viewport.min.y - top - HEADER_H) / row_h).floor().max(0.0) as usize;
            let last =
                (((viewport.max.y - top - HEADER_H) / row_h).ceil().max(0.0) as usize).min(rows);
            for row in first..last {
                for col in 0..cols {
                    let local = row * cols + col;
                    if local >= group.len {
                        break;
                    }
                    let i = group.start + local;
                    let min = origin
                        + vec2(
                            6.0 + col as f32 * (tile.width + gap),
                            top + HEADER_H + row as f32 * row_h,
                        );
                    let rect = Rect::from_min_size(min, size);
                    let resp = ui.interact(
                        Rect::from_min_size(min, vec2(tile.width, row_h)),
                        Id::unique(("tile", tab_id, i)),
                        Sense::click_and_drag(),
                    );
                    let Some(entry) = view.visible_entry(i).cloned() else {
                        continue;
                    };
                    let selected = view.selection.contains(&entry.path);
                    let drop_hover = item_interact(view, i, &entry, &resp, cx, actions);
                    let state = super::grid::TileState {
                        selected,
                        drop_hover,
                    };
                    super::grid::paint_tile(ui, rect, tile, &entry, state, &resp, cx);
                    item_context_menu(&resp, view, &entry, cx, actions);
                    // There's no Folder column here.
                    item_tooltip(ui, resp, &entry, cx, cx.is_search || cx.flat);
                }
            }
        }
    });
    remember_scroll(
        ui,
        ("grid", tab_id),
        out.state.clamped_offset().y,
        out.inner_rect.height(),
    );
}
