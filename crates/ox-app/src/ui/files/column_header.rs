//! The header of the details view: click to sort, drag to reorder, drag an
//! edge to resize, right-click to choose the columns.

use super::*;
use crate::app::{ColumnEdit, ViewAction};
use crate::tabs::Tab;
use ox_core::columns::{ColumnContext, ColumnId, ColumnLayout, MIN_NAME_WIDTH};

pub(super) const HEADER_H: f32 = 28.0;

/// Which columns apply to what `tab` shows.
pub(crate) fn column_context(tab: &Tab) -> ColumnContext {
    let loc = tab.location();
    ColumnContext {
        folder: matches!(loc, Location::Search { .. } | Location::Tagged(_))
            || loc.is_bin()
            || loc.is_collection()
            || tab.is_flat(),
        git: tab.view.git.is_some(),
    }
}

fn edit(actions: &mut Vec<Action>, e: ColumnEdit) {
    actions.push(Action::View(ViewAction::Columns(e)));
}

/// A checkbox per column that applies here, and *Reset columns*. Used by
/// the header's context menu and the View menu.
pub(crate) fn column_menu(
    ui: &mut Ui,
    layout: &ColumnLayout,
    cx: ColumnContext,
    is_bin: bool,
    actions: &mut Vec<Action>,
) {
    for id in ColumnId::ALL.into_iter().filter(|c| c.available(cx)) {
        let mut on = layout.is_shown(id);
        let label = super::cells::header_label(id, is_bin);
        if ui
            .add_enabled(id != ColumnId::Name, egui::Checkbox::new(&mut on, label))
            .changed()
        {
            edit(actions, ColumnEdit::Toggle(id));
        }
    }
    ui.separator();
    if ui.button(tr("Reset columns")).clicked() {
        edit(actions, ColumnEdit::Reset);
        ui.close();
    }
}

/// *Reset view of this folder* and *Use this view for all folders* (View menu).
pub(crate) fn folder_view_menu(ui: &mut Ui, app: &OxApp, actions: &mut Vec<Action>) {
    let folder = app.remembered_folder();
    let own = folder.is_some_and(|f| app.folder_views.get(&f).is_some());
    if ui
        .add_enabled(own, egui::Button::new(tr("Reset view of this folder")))
        .on_hover_text(tr("Show the folder with the default view again"))
        .clicked()
    {
        actions.push(Action::View(ViewAction::ResetFolder));
    }
    if ui
        .button(tr("Use this view for all folders"))
        .on_hover_text(tr(
            "Make this view mode, sort order and these columns the default for folders without a view of their own",
        ))
        .clicked()
    {
        actions.push(Action::View(ViewAction::MakeDefault));
    }
}

/// Where a resize started: the column's width, Name's width and the pointer.
#[derive(Clone, Copy)]
struct Resize {
    width: f32,
    name: f32,
    x: f32,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn header(
    ui: &mut Ui,
    tab_id: u64,
    cx: &Ctx<'_>,
    layout: &ColumnLayout,
    col_cx: ColumnContext,
    fitted: &[(ColumnId, f32)],
    sort: (SortKey, ox_core::entry::SortOrder),
    sort_clicked: &mut Option<SortKey>,
    actions: &mut Vec<Action>,
) {
    let width = ui.available_width();
    let (header, bg) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::click());
    let v = ui.visuals().clone();
    ui.painter().line_segment(
        [header.left_bottom(), header.right_bottom()],
        v.widgets.noninteractive.bg_stroke,
    );
    bg.context_menu(|ui| column_menu(ui, layout, col_cx, cx.is_bin, actions));

    let mut x = header.left() + 8.0;
    let cells: Vec<(ColumnId, Rect)> = fitted
        .iter()
        .map(|&(id, w)| {
            let r = Rect::from_min_size(egui::pos2(x, header.top()), vec2(w, header.height()));
            x += w;
            (id, r)
        })
        .collect();
    let name_w = fitted
        .iter()
        .find(|c| c.0 == ColumnId::Name)
        .map_or(0.0, |c| c.1);
    let pointer = ui.ctx().pointer_latest_pos();

    for &(id, rect) in &cells {
        let resp = ui.interact(
            rect,
            Id::unique(("col", tab_id, id.key())),
            Sense::click_and_drag(),
        );
        let key = id.info().sort;
        let dragging = resp.dragged() && id != ColumnId::Name;
        if (resp.hovered() && key.is_some()) || dragging {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), v.widgets.hovered.weak_bg_fill);
        }
        let arrow = match (key == Some(sort.0), sort.1) {
            (true, ox_core::entry::SortOrder::Ascending) => " ⏶",
            (true, ox_core::entry::SortOrder::Descending) => " ⏷",
            _ => "",
        };
        let label = super::cells::header_label(id, cx.is_bin);
        let text = format!("{label}{arrow}");
        let align_right = id.info().align_right;
        ui.painter()
            .with_clip_rect(rect.shrink2(vec2(2.0, 0.0)))
            .text(
                if align_right {
                    rect.right_center() - vec2(14.0, 0.0)
                } else {
                    rect.left_center() + vec2(6.0, 0.0)
                },
                if align_right {
                    Align2::RIGHT_CENTER
                } else {
                    Align2::LEFT_CENTER
                },
                &text,
                FontId::proportional(12.5),
                v.weak_text_color(),
            );
        if resp.clicked() {
            *sort_clicked = key;
        }
        resp.context_menu(|ui| column_menu(ui, layout, col_cx, cx.is_bin, actions));

        // Reordering: the header follows the pointer; a line shows where it goes.
        if id == ColumnId::Name {
            continue;
        }
        let Some(p) = pointer else { continue };
        let before = || {
            cells
                .iter()
                .filter(|c| c.0 != ColumnId::Name && c.0 != id)
                .find(|c| c.1.center().x > p.x)
                .map(|c| c.0)
        };
        if dragging {
            let ghost = Rect::from_center_size(egui::pos2(p.x, rect.center().y), rect.size());
            let layer = egui::LayerId::new(egui::Order::Tooltip, Id::unique(("col_drag", tab_id)));
            let painter = ui.ctx().layer_painter(layer);
            painter.rect_filled(ghost, CornerRadius::same(4), v.widgets.active.weak_bg_fill);
            painter.text(
                ghost.left_center() + vec2(6.0, 0.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(12.5),
                v.text_color(),
            );
            let line_x = match before() {
                Some(b) => cells.iter().find(|c| c.0 == b).map(|c| c.1.left()),
                None => cells.last().map(|c| c.1.right()),
            };
            if let Some(lx) = line_x {
                painter.line_segment(
                    [
                        egui::pos2(lx, header.top() + 3.0),
                        egui::pos2(lx, header.bottom() - 3.0),
                    ],
                    Stroke::new(2.0, cx.accent),
                );
            }
        }
        if resp.drag_stopped() {
            edit(actions, ColumnEdit::MoveBefore(id, before()));
        }
    }

    // Resizing: the edge left of a column (Name takes what is left over, so
    // the columns are anchored on the right). Added last so it wins over
    // the headers below it.
    let header_hovered = ui.rect_contains_pointer(header);
    for &(id, rect) in cells.iter().filter(|c| c.0 != ColumnId::Name) {
        let edge = Rect::from_center_size(
            egui::pos2(rect.left(), header.center().y),
            vec2(8.0, header.height()),
        );
        let key = Id::unique(("col_edge", tab_id, id.key()));
        let resp = ui.interact(edge, key, Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if header_hovered || resp.dragged() {
            let stroke = if resp.dragged() {
                Stroke::new(2.0, cx.accent)
            } else {
                v.widgets.noninteractive.bg_stroke
            };
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left(), header.top() + 6.0),
                    egui::pos2(rect.left(), header.bottom() - 6.0),
                ],
                stroke,
            );
        }
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                let start = Resize {
                    width: layout.width(id),
                    name: name_w,
                    x: p.x,
                };
                ui.data_mut(|d| d.insert_temp(key, start));
            }
        }
        if resp.dragged() {
            let start = ui.data(|d| d.get_temp::<Resize>(key));
            if let (Some(s), Some(p)) = (start, resp.interact_pointer_pos()) {
                // Dragging left widens the column; Name keeps its minimum.
                let grow = (s.name - MIN_NAME_WIDTH).max(0.0);
                let w = (s.width - (p.x - s.x)).min(s.width + grow);
                edit(actions, ColumnEdit::Width(id, w));
            }
        }
        if resp.drag_stopped() {
            ui.data_mut(|d| d.remove::<Resize>(key));
            edit(actions, ColumnEdit::Save);
        }
    }
}
