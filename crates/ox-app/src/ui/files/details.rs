//! Details list: column rows with name, date, type, size and git state.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn details(
    ui: &mut Ui,
    view: &mut FolderView,
    tab_id: u64,
    cx: &Ctx<'_>,
    actions: &mut Vec<Action>,
    sort_clicked: &mut Option<SortKey>,
    sort_key: SortKey,
    sort_order: ox_core::entry::SortOrder,
) {
    let width = ui.available_width();
    let with_folder = cx.is_search || cx.flat || cx.is_bin;
    let columns = Columns::fit(width, with_folder, cx.git.is_some());

    // Column header
    let (header, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
    let v = ui.visuals().clone();
    ui.painter().line_segment(
        [header.left_bottom(), header.right_bottom()],
        v.widgets.noninteractive.bg_stroke,
    );
    let mut x = header.left() + 8.0;
    let mut cols: Vec<(&str, f32, Option<SortKey>)> =
        vec![(tr("Name"), columns.name, Some(SortKey::Name))];
    if columns.folder > 0.0 {
        let label = if cx.is_bin {
            tr("Original location")
        } else {
            tr("Folder")
        };
        cols.push((label, columns.folder, None));
    }
    let date_label = if cx.is_bin {
        tr("Date deleted")
    } else {
        tr("Date modified")
    };
    cols.extend([
        (tr("Git"), columns.git, None),
        (date_label, columns.date, Some(SortKey::Modified)),
        (tr("Type"), columns.kind, Some(SortKey::Type)),
        (tr("Size"), columns.size, Some(SortKey::Size)),
    ]);
    for (label, w, key) in cols.into_iter().filter(|c| c.1 > 0.0) {
        let rect = Rect::from_min_size(egui::pos2(x, header.top()), vec2(w, header.height()));
        let resp = ui.interact(rect, Id::unique(("col", tab_id, label)), Sense::click());
        if resp.hovered() && key.is_some() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), v.widgets.hovered.weak_bg_fill);
        }
        let arrow = match (key == Some(sort_key), sort_order) {
            (true, ox_core::entry::SortOrder::Ascending) => " ⏶",
            (true, ox_core::entry::SortOrder::Descending) => " ⏷",
            _ => "",
        };
        ui.painter().text(
            rect.left_center() + vec2(6.0, 0.0),
            Align2::LEFT_CENTER,
            format!("{label}{arrow}"),
            FontId::proportional(12.5),
            v.weak_text_color(),
        );
        if resp.clicked() {
            *sort_clicked = key;
        }
        x += w;
    }

    let n = view.visible.len();
    let row_h = cx.row_h;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("details", tab_id))
        .auto_shrink([false, false]);
    if let Some(target) = view.scroll_to.take() {
        area = scroll_into_view(ui, area, ("details", tab_id), target as f32 * row_h, row_h);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = area.show_rows(ui, row_h, n, |ui, range| {
        for i in range {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, row_h), Sense::click_and_drag());
            let Some(entry) = view.visible_entry(i).cloned() else {
                continue;
            };
            let selected = view.selection.contains(&entry.path);
            let drop_hover = item_interact(view, i, &entry, &resp, cx, actions);
            paint_row(ui, rect, &entry, selected, drop_hover, &resp, cx, columns);
            item_context_menu(&resp, view, &entry, cx, actions);
            item_tooltip(ui, resp, &entry, cx, false);
        }
    });
    remember_scroll(
        ui,
        ("details", tab_id),
        out.state.clamped_offset().y,
        out.inner_rect.height(),
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &Ui,
    rect: Rect,
    e: &FileEntry,
    selected: bool,
    drop_hover: bool,
    resp: &Response,
    cx: &Ctx<'_>,
    columns: Columns,
) {
    let name_w = columns.name;
    let v = ui.visuals();
    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), cx.selection_fill);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), v.widgets.hovered.weak_bg_fill);
    }
    if drop_hover {
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

    let p = ui.painter().with_clip_rect(Rect::from_min_max(
        rect.min,
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
    x += name_w;

    let col = |x: f32, w: f32, s: String, align_right: bool| {
        let p = ui.painter().with_clip_rect(Rect::from_min_max(
            egui::pos2(x, rect.top()),
            egui::pos2(x + w - 6.0, rect.bottom()),
        ));
        if align_right {
            p.text(
                egui::pos2(x + w - 14.0, y),
                Align2::RIGHT_CENTER,
                s,
                FontId::proportional(12.5),
                weak,
            );
        } else {
            p.text(
                egui::pos2(x + 6.0, y),
                Align2::LEFT_CENTER,
                s,
                FontId::proportional(12.5),
                weak,
            );
        }
    };
    if cx.is_bin && columns.folder > 0.0 {
        let original = cx
            .bin
            .get(&e.path)
            .map(|i| i.original_parent.display().to_string())
            .unwrap_or_default();
        col(x, columns.folder, original, false);
        x += columns.folder;
    } else if (cx.is_search || cx.flat) && columns.folder > 0.0 {
        col(x, columns.folder, folder_text(e, cx), false);
        x += columns.folder;
    }
    if let (true, Some(git)) = (columns.git > 0.0, cx.git) {
        if let Some(state) = git.state(&e.path) {
            ui.painter().text(
                egui::pos2(x + 14.0, y),
                Align2::LEFT_CENTER,
                state.letter(),
                FontId::monospace(12.5),
                fade(git_color(state, v)),
            );
        }
        x += columns.git;
    }
    if columns.date > 0.0 {
        col(
            x,
            columns.date,
            e.modified
                .map(|t| format::time(t, cx.date_format))
                .unwrap_or_default(),
            false,
        );
        x += columns.date;
    }
    if columns.kind > 0.0 {
        col(x, columns.kind, e.type_label(), false);
        x += columns.kind;
    }
    let size = match (e.is_dir, cx.sizes) {
        (false, _) => format::size(e.size),
        (true, Some(sizes)) if !e.is_symlink => match sizes.get(&e.path) {
            Some(s) => format::size(s.bytes),
            None => "…".into(),
        },
        (true, _) => String::new(),
    };
    col(x, columns.size, size, true);
}

/// The folder an item is in, relative to the search or flat view's root
/// (e.g. `Projects\app\src`).
pub(super) fn folder_text(e: &FileEntry, cx: &Ctx<'_>) -> String {
    let parent = e.path.parent().unwrap_or(&e.path);
    match &cx.search_root {
        Some(root) => {
            let name = ox_core::entry::display_name(root);
            match parent.strip_prefix(root) {
                Ok(rel) if rel.as_os_str().is_empty() => name,
                Ok(rel) => format!("{name}{}{}", std::path::MAIN_SEPARATOR, rel.display()),
                Err(_) => parent.display().to_string(),
            }
        }
        None => parent.display().to_string(),
    }
}
