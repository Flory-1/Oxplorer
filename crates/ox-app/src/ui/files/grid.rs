//! Grid view: icon tiles.

use super::*;

pub(super) fn grid(
    ui: &mut Ui,
    view: &mut FolderView,
    tab_id: u64,
    cx: &Ctx<'_>,
    actions: &mut Vec<Action>,
) {
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
                    // There's no Folder column here.
                    item_tooltip(ui, resp, &entry, cx, cx.is_search || cx.flat);
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
    super::super::compare::paint_mark(ui, rect, cx.compare, &e.path);
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
        if e.is_symlink {
            icons::paint_link_badge(ui.painter(), r.right_bottom() + vec2(-8.0, -8.0), 7.0);
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
            fade(super::super::tag_color(tag)),
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
