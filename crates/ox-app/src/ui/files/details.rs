//! Details list: a row per item with the columns the user chose.

use super::cells::{cell, Cell};
use super::column_header::header;
use super::*;
use ox_core::columns::{ColumnContext, ColumnId, ColumnLayout};
use ox_core::grouping::{self, ListRow};

#[allow(clippy::too_many_arguments)]
pub(super) fn details(
    ui: &mut Ui,
    view: &mut FolderView,
    tab_id: u64,
    cx: &Ctx<'_>,
    layout: &ColumnLayout,
    col_cx: ColumnContext,
    actions: &mut Vec<Action>,
    sort_clicked: &mut Option<SortKey>,
    sort: (SortKey, ox_core::entry::SortOrder),
) {
    let width = ui.available_width();
    let fitted = layout.fit(col_cx, width);
    header(
        ui,
        tab_id,
        cx,
        layout,
        col_cx,
        &fitted,
        sort,
        sort_clicked,
        actions,
    );
    // Only read files when a shown column needs it.
    let props = cx.props.filter(|_| fitted.iter().any(|c| c.0.info().props));

    // Group headers are rows too, as high as the items.
    let groups = view.groups.clone();
    let rows = grouping::row_count(&groups, view.visible.len());
    let row_h = cx.row_h;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("details", tab_id))
        .auto_shrink([false, false]);
    if let Some(target) = view.scroll_to.take() {
        let row = grouping::row_of(&groups, target);
        // The first item of a group brings its header along.
        let first = grouping::group_of(&groups, target).is_some_and(|g| groups[g].start == target);
        let (y, h) = if first {
            ((row - 1) as f32 * row_h, 2.0 * row_h)
        } else {
            (row as f32 * row_h, row_h)
        };
        area = scroll_into_view(ui, area, ("details", tab_id), y, h);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = area.show_rows(ui, row_h, rows, |ui, range| {
        for r in range {
            let i = match grouping::row_at(&groups, r) {
                ListRow::Item(i) => i,
                ListRow::Header(g) => {
                    let (rect, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
                    super::grouped::header_row(ui, rect, &groups[g], tab_id, cx, actions);
                    continue;
                }
            };
            let (rect, resp) = ui.allocate_exact_size(vec2(width, row_h), Sense::click_and_drag());
            let Some(entry) = view.visible_entry(i).cloned() else {
                continue;
            };
            let selected = view.selection.contains(&entry.path);
            let drop_hover = item_interact(view, i, &entry, &resp, cx, actions);
            let values = props.and_then(|p| p.get(&entry));
            let row = Row {
                rect,
                selected,
                drop_hover,
                props: values.as_deref(),
            };
            paint_row(ui, &row, &entry, &resp, cx, &fitted);
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

struct Row<'a> {
    rect: Rect,
    selected: bool,
    drop_hover: bool,
    props: Option<&'a ox_core::props::Props>,
}

fn paint_row(
    ui: &Ui,
    row: &Row<'_>,
    e: &FileEntry,
    resp: &Response,
    cx: &Ctx<'_>,
    columns: &[(ColumnId, f32)],
) {
    let rect = row.rect;
    let v = ui.visuals();
    if row.selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), cx.selection_fill);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), v.widgets.hovered.weak_bg_fill);
    }
    if row.drop_hover {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.5, cx.accent),
            egui::StrokeKind::Inside,
        );
    }
    super::super::compare::paint_mark(ui, rect, cx.compare, &e.path);
    let dim = cx.cut.contains(&e.path) || e.hidden;
    let fade = |c: Color32| if dim { c.gamma_multiply(0.5) } else { c };
    let text = fade(v.text_color());
    let weak = fade(v.weak_text_color());
    let y = rect.center().y;
    let mut x = rect.left() + 8.0;

    for &(id, w) in columns {
        if id == ColumnId::Name {
            paint_name(ui, rect, x, w, e, cx, dim, text, &fade);
            x += w;
            continue;
        }
        let p = ui.painter().with_clip_rect(Rect::from_min_max(
            egui::pos2(x, rect.top()),
            egui::pos2(x + w - 6.0, rect.bottom()),
        ));
        match cell(id, e, cx, row.props, v) {
            Cell::Text(s) if id.info().align_right => {
                p.text(
                    egui::pos2(x + w - 14.0, y),
                    Align2::RIGHT_CENTER,
                    s,
                    FontId::proportional(12.5),
                    weak,
                );
            }
            Cell::Text(s) => {
                p.text(
                    egui::pos2(x + 6.0, y),
                    Align2::LEFT_CENTER,
                    s,
                    FontId::proportional(12.5),
                    weak,
                );
            }
            Cell::Symbol(s, color) => {
                p.text(
                    egui::pos2(x + 14.0, y),
                    Align2::LEFT_CENTER,
                    s,
                    FontId::monospace(12.5),
                    fade(color),
                );
            }
            Cell::Empty => {}
        }
        x += w;
    }
}

/// Icon, name and tag dots.
#[allow(clippy::too_many_arguments)]
fn paint_name(
    ui: &Ui,
    rect: Rect,
    x: f32,
    name_w: f32,
    e: &FileEntry,
    cx: &Ctx<'_>,
    dim: bool,
    text: Color32,
    fade: &dyn Fn(Color32) -> Color32,
) {
    let y = rect.center().y;
    let p = ui.painter().with_clip_rect(Rect::from_min_max(
        egui::pos2(rect.left(), rect.top()),
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
        p.circle_filled(c, 4.0, fade(super::super::tag_color(tag)));
    }
}
