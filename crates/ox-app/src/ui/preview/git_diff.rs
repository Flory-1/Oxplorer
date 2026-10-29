//! Diffs drawn: added and removed lines in the theme's diff colours with
//! the old and new line numbers, and what changed within a line stronger;
//! in one column or side by side.

use super::git_changes::ERROR;
use super::git_panel::{DiffPane, GitPanel};
use super::*;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Rect, Sense};
use ox_core::git::diff::{self as gitdiff, Diff, DiffLine, FileDiff, LineKind};
use ox_core::trf;

/// The Diff tab of a changed file selected in the file list.
pub(super) fn file_tab(ui: &mut Ui, panel: &mut GitPanel, e: &FileEntry) {
    if !panel.update_file_diff(ui.ctx(), e) {
        ui.label(RichText::new(tr("No changes")).weak());
        return;
    }
    diff_view(
        ui,
        &mut panel.side_by_side,
        &panel.file_diff,
        "preview_diff",
        f32::INFINITY,
    );
}

/// One row of the drawn diff.
enum Row {
    /// A file's name (when the diff has several files).
    File(usize),
    /// A hunk's "@@ -1,3 +1,4 @@" line.
    Hunk(usize, usize),
    /// A line, or (side by side) the old and the new line next to each other.
    Line(usize, usize, Option<usize>, Option<usize>),
}

/// Sizes of the drawn rows.
struct Lay {
    font: FontId,
    char_w: f32,
    row_h: f32,
    /// Width of a line number.
    num_w: f32,
    /// Width of the text column(s).
    text_w: f32,
}

/// The diff `pane` shows, at most `max_h` high.
pub(super) fn diff_view(
    ui: &mut Ui,
    side_by_side: &mut bool,
    pane: &DiffPane,
    id: &str,
    max_h: f32,
) {
    let diff: Arc<Diff> = match &pane.shown {
        None => {
            ui.spinner();
            return;
        }
        Some(Err(e)) => {
            ui.colored_label(ERROR, e);
            return;
        }
        Some(Ok(d)) => d.clone(),
    };
    draw(ui, side_by_side, &diff, id, max_h, |ui| {
        if pane.loading() {
            ui.spinner();
        }
    });
}

/// `diff` drawn, at most `max_h` high; `toolbar` adds to the row above it
/// (after the counts and the side-by-side switch). Also used for comparing
/// two files (`ui::text_compare`).
pub(crate) fn draw(
    ui: &mut Ui,
    side_by_side: &mut bool,
    diff: &Diff,
    id: &str,
    max_h: f32,
    toolbar: impl FnOnce(&mut Ui),
) {
    if diff.too_large {
        ui.label(RichText::new(tr("This diff is too large to show.")).weak());
        return;
    }
    let colors = crate::theme::diff_colors(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        let added: usize = diff.files.iter().map(FileDiff::added).sum();
        let removed: usize = diff.files.iter().map(FileDiff::removed).sum();
        ui.label(
            RichText::new(format!("+{added}"))
                .monospace()
                .color(colors.added_text),
        );
        ui.label(
            RichText::new(format!("-{removed}"))
                .monospace()
                .color(colors.removed_text),
        );
        ui.checkbox(side_by_side, tr("Side by side"));
        toolbar(ui);
    });
    for f in &diff.files {
        about(ui, f);
    }
    if diff.is_empty() {
        ui.label(RichText::new(tr("No differences.")).weak());
        return;
    }

    let rows = rows(diff, *side_by_side);
    let partners: Vec<Vec<Vec<Option<usize>>>> = diff
        .files
        .iter()
        .map(|f| {
            f.hunks
                .iter()
                .map(|h| gitdiff::partners(&h.lines))
                .collect()
        })
        .collect();
    let font = FontId::monospace(12.0);
    let (char_w, row_h) = ui.fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
    let last = diff
        .files
        .iter()
        .flat_map(|f| &f.hunks)
        .map(|h| (h.old_start + h.old_count).max(h.new_start + h.new_count))
        .max()
        .unwrap_or(1);
    let longest = diff
        .files
        .iter()
        .flat_map(|f| &f.hunks)
        .flat_map(|h| &h.lines)
        .map(|l| l.text.len() + l.text.matches('\t').count() * 3)
        .max()
        .unwrap_or(0);
    let lay = Lay {
        num_w: (last.to_string().len() as f32 + 1.0) * char_w,
        text_w: (longest as f32 + 2.0) * char_w,
        font,
        char_w,
        row_h: row_h + 2.0,
    };
    let half = 2.0 * lay.char_w + lay.num_w + lay.text_w;
    let width = match side_by_side {
        true => (2.0 * half).max(ui.available_width()),
        false => (lay.num_w * 2.0 + 2.0 * lay.char_w + lay.text_w).max(ui.available_width()),
    };
    let v = ui.visuals().clone();
    egui::ScrollArea::both()
        .id_salt(id)
        .max_height(max_h)
        .auto_shrink([false, true])
        .show_rows(ui, lay.row_h, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &rows[range] {
                let (rect, _) = ui.allocate_exact_size(vec2(width, lay.row_h), Sense::hover());
                let painter = ui.painter_at(rect);
                match *row {
                    Row::File(fi) => {
                        let f = &diff.files[fi];
                        let name = f.new_path.as_ref().or(f.old_path.as_ref());
                        painter.rect_filled(rect, 0.0, v.faint_bg_color);
                        painter.text(
                            rect.left_center() + vec2(4.0, 0.0),
                            Align2::LEFT_CENTER,
                            name.map_or("", String::as_str),
                            lay.font.clone(),
                            v.strong_text_color(),
                        );
                    }
                    Row::Hunk(fi, hi) => {
                        painter.rect_filled(rect, 0.0, v.faint_bg_color);
                        painter.text(
                            rect.left_center() + vec2(4.0, 0.0),
                            Align2::LEFT_CENTER,
                            diff.files[fi].hunks[hi].header(),
                            lay.font.clone(),
                            v.weak_text_color(),
                        );
                    }
                    Row::Line(fi, hi, a, b) => {
                        let lines = &diff.files[fi].hunks[hi].lines;
                        let partner = |i: usize| partners[fi][hi][i].map(|p| &lines[p]);
                        if *side_by_side {
                            let (left, right) = rect.split_left_right_at_x(rect.left() + half);
                            let old = a.map(|i| (&lines[i], partner(i)));
                            let new = b.map(|i| (&lines[i], partner(i)));
                            side(&painter, left, old, true, &lay, &colors, &v);
                            side(&painter, right, new, false, &lay, &colors, &v);
                        } else if let Some(i) = a {
                            unified(&painter, rect, &lines[i], partner(i), &lay, &colors, &v);
                        }
                    }
                }
            }
        });
}

/// What the diff says about the file besides its lines: renamed, mode
/// changed, binary.
fn about(ui: &mut Ui, f: &FileDiff) {
    let weak = |ui: &mut Ui, text: String| {
        ui.label(RichText::new(text).weak().small());
    };
    if f.renamed {
        let (old, new) = (
            f.old_path.as_deref().unwrap_or_default(),
            f.new_path.as_deref().unwrap_or_default(),
        );
        let mut text = trf!("Renamed: {old} → {new}", old = old, new = new);
        if let Some(s) = f.similarity {
            text.push_str(&format!(" ({s} %)"));
        }
        weak(ui, text);
    }
    if let (Some(old), Some(new)) = (&f.old_mode, &f.new_mode) {
        weak(ui, trf!("File mode: {old} → {new}", old = old, new = new));
    }
    if f.binary {
        weak(
            ui,
            tr("A binary file: there are no lines to compare.").to_owned(),
        );
    }
}

/// The rows of `diff`: one per line, or (side by side) old and new next to
/// each other.
fn rows(diff: &Diff, side_by_side: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    for (fi, f) in diff.files.iter().enumerate() {
        if diff.files.len() > 1 {
            rows.push(Row::File(fi));
        }
        for (hi, h) in f.hunks.iter().enumerate() {
            rows.push(Row::Hunk(fi, hi));
            if side_by_side {
                rows.extend(
                    gitdiff::side_by_side(&h.lines)
                        .into_iter()
                        .map(|(a, b)| Row::Line(fi, hi, a, b)),
                );
            } else {
                rows.extend((0..h.lines.len()).map(|i| Row::Line(fi, hi, Some(i), None)));
            }
        }
    }
    rows
}

/// A line's background and the colour of its sign and numbers.
fn line_colors(
    kind: LineKind,
    c: &crate::theme::DiffColors,
    v: &egui::Visuals,
) -> (Color32, Color32) {
    match kind {
        LineKind::Added => (c.added, c.added_text),
        LineKind::Removed => (c.removed, c.removed_text),
        LineKind::Context => (Color32::TRANSPARENT, v.weak_text_color()),
    }
}

/// One line in the single column: old and new number, sign, text.
fn unified(
    painter: &Painter,
    rect: Rect,
    line: &DiffLine,
    partner: Option<&DiffLine>,
    lay: &Lay,
    c: &crate::theme::DiffColors,
    v: &egui::Visuals,
) {
    let (bg, fg) = line_colors(line.kind, c, v);
    painter.rect_filled(rect, 0.0, bg);
    let y = rect.center().y;
    for (column, n) in [line.old, line.new].into_iter().enumerate() {
        if let Some(n) = n {
            let x = rect.left() + lay.num_w * (column + 1) as f32;
            painter.text(
                pos2(x, y),
                Align2::RIGHT_CENTER,
                n.to_string(),
                lay.font.clone(),
                fg,
            );
        }
    }
    let x = rect.left() + 2.0 * lay.num_w + lay.char_w;
    text(painter, pos2(x, y), line, partner, lay, c, v);
}

/// One side of a side-by-side row: the old (`old`) or the new line, or
/// nothing.
fn side(
    painter: &Painter,
    rect: Rect,
    line: Option<(&DiffLine, Option<&DiffLine>)>,
    old: bool,
    lay: &Lay,
    c: &crate::theme::DiffColors,
    v: &egui::Visuals,
) {
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let Some((line, partner)) = line else {
        painter.rect_filled(rect, 0.0, v.faint_bg_color);
        return;
    };
    let (bg, fg) = line_colors(line.kind, c, v);
    painter.rect_filled(rect, 0.0, bg);
    let y = rect.center().y;
    let n = if old { line.old } else { line.new };
    if let Some(n) = n {
        painter.text(
            pos2(rect.left() + lay.num_w, y),
            Align2::RIGHT_CENTER,
            n.to_string(),
            lay.font.clone(),
            fg,
        );
    }
    text(
        &painter,
        pos2(rect.left() + lay.num_w + lay.char_w, y),
        line,
        partner,
        lay,
        c,
        v,
    );
}

/// The sign and text of a line from `at` (left centre), what changed
/// against `partner` highlighted.
fn text(
    painter: &Painter,
    at: egui::Pos2,
    line: &DiffLine,
    partner: Option<&DiffLine>,
    lay: &Lay,
    c: &crate::theme::DiffColors,
    v: &egui::Visuals,
) {
    let (sign, word) = match line.kind {
        LineKind::Added => ("+", c.added_word),
        LineKind::Removed => ("-", c.removed_word),
        LineKind::Context => (" ", Color32::TRANSPARENT),
    };
    let fmt = |background: Color32, color: Color32| egui::TextFormat {
        font_id: lay.font.clone(),
        color,
        background,
        ..Default::default()
    };
    let plain = fmt(Color32::TRANSPARENT, v.text_color());
    let mut job = egui::text::LayoutJob::default();
    job.append(
        sign,
        0.0,
        fmt(Color32::TRANSPARENT, line_colors(line.kind, c, v).1),
    );
    let own = line.text.trim_end_matches('\r');
    // What changed within a replaced line.
    let changed = partner.and_then(|p| {
        let other = p.text.trim_end_matches('\r');
        let (old, new) = match line.kind {
            LineKind::Removed => (own, other),
            _ => (other, own),
        };
        let (o, n) = gitdiff::changed_ranges(old, new)?;
        Some(if line.kind == LineKind::Removed { o } else { n })
    });
    let tabs = |s: &str| s.replace('\t', "    ");
    match changed {
        Some(r) => {
            job.append(&tabs(&own[..r.start]), 0.0, plain.clone());
            job.append(&tabs(&own[r.clone()]), 0.0, fmt(word, v.text_color()));
            job.append(&tabs(&own[r.end..]), 0.0, plain);
        }
        None => job.append(&tabs(own), 0.0, plain),
    }
    if line.no_newline {
        job.append(
            &format!("  {}", tr("(no newline at the end)")),
            0.0,
            fmt(Color32::TRANSPARENT, v.weak_text_color()),
        );
    }
    let galley = painter.layout_job(job);
    let top = at.y - galley.size().y / 2.0;
    painter.galley(pos2(at.x, top), galley, v.text_color());
}
