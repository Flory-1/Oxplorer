//! The Hex tab: offset, 16 bytes in hex and as text per row. Only the rows
//! on screen are read from the file, so any size scrolls smoothly.

use super::*;
use egui::text::{LayoutJob, TextFormat};
use egui::{pos2, vec2, FontId, Rect, Sense};
use ox_core::hexview::{self, HexFile, ROW};

/// What the tab remembers for the file it shows.
#[derive(Default)]
pub(super) struct HexView {
    key: Option<(PathBuf, Option<SystemTime>)>,
    /// The file can be shown (a local file that isn't online-only).
    available: bool,
    file: Option<Result<HexFile, String>>,
    /// First row shown, and how far the wheel has scrolled past it (rows).
    top: u64,
    scrolled: f32,
    /// Selected bytes: where the selection started and where it ends.
    anchor: Option<u64>,
    cursor: Option<u64>,
    goto: String,
    goto_error: bool,
}

impl HexView {
    /// Adds the Hex tab (last) for files it can show.
    pub(super) fn offer(&mut self, e: &FileEntry, tabs: &mut Vec<PreviewTab>) {
        let key = (e.path.clone(), e.modified);
        if self.key.as_ref() != Some(&key) {
            // Inside archives the bytes aren't at hand; online-only files
            // would be downloaded.
            let available = !e.is_dir
                && ox_core::archive::split_virtual(&e.path).is_none()
                && std::fs::metadata(&e.path)
                    .is_ok_and(|m| m.is_file() && m.len() > 0 && !ox_core::cloud::online(&m));
            *self = Self {
                key: Some(key),
                available,
                ..Default::default()
            };
        }
        if self.available {
            tabs.push(PreviewTab::Hex);
        }
    }

    fn selection(&self) -> Option<std::ops::RangeInclusive<u64>> {
        Some(hexview::selection(self.anchor?, self.cursor?))
    }
}

/// Column positions of a row, in points.
struct Layout {
    font: FontId,
    char_w: f32,
    row_h: f32,
    digits: usize,
    hex_x: f32,
    ascii_x: f32,
    width: f32,
}

impl Layout {
    fn new(ui: &Ui, len: u64) -> Self {
        let font = FontId::monospace(12.0);
        let (char_w, row_h) = ui.fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
        let digits = hexview::offset_digits(len);
        let hex_x = (digits + 2) as f32 * char_w;
        let ascii_x = hex_x + (hexview::HEX_WIDTH + 2) as f32 * char_w;
        Self {
            font,
            char_w,
            row_h: row_h + 3.0,
            digits,
            hex_x,
            ascii_x,
            width: ascii_x + (ROW + 1) as f32 * char_w,
        }
    }

    /// The byte (of its row) under `x` (from the row's left edge).
    fn byte_at(&self, x: f32) -> Option<usize> {
        if x >= self.ascii_x - self.char_w {
            let i = ((x - self.ascii_x) / self.char_w).floor().max(0.0) as usize;
            return Some(i.min(ROW - 1));
        }
        if x >= self.hex_x - self.char_w {
            let col = ((x - self.hex_x) / self.char_w).floor().max(0.0) as usize;
            return Some(hexview::byte_at_hex_column(col).unwrap_or(ROW - 1));
        }
        None
    }
}

pub(super) fn show(ui: &mut Ui, hv: &mut HexView, e: &FileEntry, actions: &mut Vec<Action>) {
    if hv.file.is_none() {
        hv.file = Some(HexFile::open(&e.path).map_err(|e| e.to_string()));
    }
    let file = match hv.file.take() {
        Some(Ok(f)) => f,
        Some(Err(err)) => {
            ui.colored_label(ui.visuals().error_fg_color, &err);
            hv.file = Some(Err(err));
            return;
        }
        None => return,
    };
    let mut file = file;
    toolbar(ui, hv, &mut file, actions);
    ui.add_space(2.0);
    rows(ui, hv, &mut file, actions);
    hv.file = Some(Ok(file));
}

/// Go to an offset, what's selected, copy.
fn toolbar(ui: &mut Ui, hv: &mut HexView, file: &mut HexFile, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let edit = ui.add(
            egui::TextEdit::singleline(&mut hv.goto)
                .desired_width(110.0)
                .hint_text(tr("Go to offset")),
        );
        let edit = edit.on_hover_text(tr("Decimal, or hex with 0x (e.g. 0x1F00)"));
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter || ui.button("➡").on_hover_text(tr("Go to offset")).clicked() {
            match hexview::parse_offset(&hv.goto).filter(|&o| o < file.len()) {
                Some(offset) => {
                    let row = offset / ROW as u64;
                    hv.top = row.saturating_sub(2);
                    hv.scrolled = 0.0;
                    hv.anchor = Some(offset);
                    hv.cursor = Some(offset);
                    hv.goto_error = false;
                }
                None => hv.goto_error = true,
            }
        }
        if hv.goto_error {
            ui.colored_label(ui.visuals().error_fg_color, tr("Not in this file"));
        }
        if let Some(sel) = hv.selection() {
            let n = sel.end() - sel.start() + 1;
            ui.label(
                RichText::new(format!(
                    "0x{:X} · {}",
                    sel.start(),
                    ox_core::i18n::count(n, "byte")
                ))
                .weak(),
            );
            if ui.button(tr("📋 Copy as hex")).clicked() {
                copy(ui, hv, file, false, actions);
            }
        }
    });
}

/// The rows on screen, with a scroll bar for the whole file.
fn rows(ui: &mut Ui, hv: &mut HexView, file: &mut HexFile, actions: &mut Vec<Action>) {
    let lay = Layout::new(ui, file.len());
    let full = ui.available_rect_before_wrap();
    let full = full.with_max_y(full.max.y.max(full.min.y + 60.0));
    let bar_w = 10.0;
    let bar = full.with_min_x(full.max.x - bar_w);
    let body = full.with_max_x(bar.min.x - 2.0);
    let visible = ((body.height() / lay.row_h).floor() as u64).max(1);
    let total = file.rows();
    let last_top = total.saturating_sub(visible);

    // The wheel scrolls rows (taken before the sideways scroll area sees it).
    if ui.rect_contains_pointer(full) {
        let dy = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta.y));
        hv.scrolled -= dy / lay.row_h;
        let whole = hv.scrolled.trunc();
        hv.scrolled -= whole;
        hv.top = (hv.top as i64 + whole as i64).clamp(0, last_top as i64) as u64;
    }
    hv.top = hv.top.min(last_top);

    scroll_bar(ui, hv, bar, total, visible);
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
    egui::ScrollArea::horizontal()
        .id_salt("preview_hex")
        .auto_shrink([false, false])
        .show(&mut body_ui, |ui| {
            let size = vec2(lay.width.max(ui.available_width()), body.height());
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
            select(ui, hv, &lay, rect, &resp, file.len(), visible);
            paint(ui, hv, file, &lay, rect, visible);
            resp.context_menu(|ui| {
                if hv.selection().is_none() {
                    ui.close();
                    return;
                }
                if ui.button(tr("📋 Copy as hex")).clicked() {
                    copy(ui, hv, file, false, actions);
                    ui.close();
                }
                if ui.button(tr("📋 Copy as text")).clicked() {
                    copy(ui, hv, file, true, actions);
                    ui.close();
                }
                if ui.button(tr("📋 Copy offset")).clicked() {
                    let at = hv.selection().map_or(0, |s| *s.start());
                    table::copy(ui, format!("0x{at:X}"), actions);
                    ui.close();
                }
            });
        });
    ui.allocate_rect(full, Sense::hover());
}

/// Clicking and dragging selects bytes (in either column); Shift-click
/// extends the selection.
fn select(
    ui: &Ui,
    hv: &mut HexView,
    lay: &Layout,
    rect: Rect,
    resp: &egui::Response,
    len: u64,
    visible: u64,
) {
    let total = len.div_ceil(ROW as u64);
    let Some(pos) = resp.interact_pointer_pos() else {
        return;
    };
    if !(resp.drag_started() || resp.dragged() || resp.clicked() || resp.secondary_clicked()) {
        return;
    }
    // Dragging past the top or bottom scrolls.
    if resp.dragged() && total > visible {
        if pos.y < rect.top() {
            hv.top = hv.top.saturating_sub(1);
        } else if pos.y > rect.bottom() {
            hv.top = (hv.top + 1).min(total - visible);
        }
    }
    let y = (pos.y - rect.top()).clamp(0.0, rect.height() - 1.0);
    let row = hv.top + (y / lay.row_h) as u64;
    let Some(i) = lay.byte_at(pos.x - rect.left()) else {
        return;
    };
    let at = (row * ROW as u64 + i as u64).min(len.saturating_sub(1));
    let shift = ui.input(|i| i.modifiers.shift);
    let inside = hv.selection().is_some_and(|s| s.contains(&at));
    if resp.secondary_clicked() && inside {
        return; // Keep the selection for the menu.
    }
    let fresh = (resp.drag_started() || resp.clicked() || resp.secondary_clicked()) && !shift;
    if fresh || hv.anchor.is_none() {
        hv.anchor = Some(at);
    }
    hv.cursor = Some(at);
}

fn paint(ui: &Ui, hv: &HexView, file: &mut HexFile, lay: &Layout, rect: Rect, visible: u64) {
    let v = ui.visuals();
    let painter = ui.painter_at(rect);
    let sel = hv.selection();
    let len = file.len();
    for r in 0..visible + 1 {
        let row = hv.top + r;
        let offset = row * ROW as u64;
        if offset >= len {
            break;
        }
        let Ok(bytes) = file.row(row) else {
            break;
        };
        let y = rect.top() + r as f32 * lay.row_h;
        if let Some(sel) = &sel {
            let first = (*sel.start()).max(offset);
            let last = (*sel.end()).min(offset + bytes.len() as u64 - 1);
            if first <= last {
                let (a, b) = ((first - offset) as usize, (last - offset) as usize);
                let x = |col: usize| rect.left() + col as f32 * lay.char_w;
                let fill = v.selection.bg_fill;
                let hex = Rect::from_min_max(
                    pos2(x(hexview::hex_column(a)) + lay.hex_x - 2.0, y),
                    pos2(
                        x(hexview::hex_column(b) + 2) + lay.hex_x + 2.0,
                        y + lay.row_h,
                    ),
                );
                let ascii = Rect::from_min_max(
                    pos2(x(a) + lay.ascii_x, y),
                    pos2(x(b + 1) + lay.ascii_x, y + lay.row_h),
                );
                painter.rect_filled(hex, 2.0, fill);
                painter.rect_filled(ascii, 2.0, fill);
            }
        }
        let text_y = y + 1.5;
        painter.text(
            pos2(rect.left(), text_y),
            egui::Align2::LEFT_TOP,
            hexview::offset_label(offset, lay.digits),
            lay.font.clone(),
            v.weak_text_color(),
        );
        // Zero bytes are drawn faint.
        let mut job = LayoutJob::default();
        let text = hexview::hex_row(&bytes);
        let mut at = 0;
        for (i, b) in bytes.iter().enumerate() {
            let start = hexview::hex_column(i);
            let color = if *b == 0 {
                v.weak_text_color()
            } else {
                v.text_color()
            };
            job.append(
                &text[at..start + 2],
                0.0,
                TextFormat::simple(lay.font.clone(), color),
            );
            at = start + 2;
        }
        let galley = painter.layout_job(job);
        painter.galley(
            pos2(rect.left() + lay.hex_x, text_y),
            galley,
            v.text_color(),
        );
        painter.text(
            pos2(rect.left() + lay.ascii_x, text_y),
            egui::Align2::LEFT_TOP,
            hexview::ascii_row(&bytes),
            lay.font.clone(),
            v.text_color(),
        );
    }
}

/// A scroll bar over all rows (egui's would lose precision on big files).
fn scroll_bar(ui: &mut Ui, hv: &mut HexView, bar: Rect, total: u64, visible: u64) {
    if total <= visible {
        return;
    }
    let resp = ui.interact(bar, ui.scope_id().with("hex_bar"), Sense::click_and_drag());
    let range = total - visible;
    let thumb_h = (bar.height() * visible as f32 / total as f32)
        .max(24.0)
        .min(bar.height());
    let free = (bar.height() - thumb_h).max(1.0);
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let f = ((pos.y - bar.top() - thumb_h / 2.0) / free).clamp(0.0, 1.0);
            hv.top = (f as f64 * range as f64).round() as u64;
            hv.scrolled = 0.0;
        }
    }
    let f = hv.top as f64 / range as f64;
    let top = bar.top() + (f * free as f64) as f32;
    let thumb = Rect::from_min_size(
        pos2(bar.left() + 2.0, top),
        vec2(bar.width() - 4.0, thumb_h),
    );
    let v = ui.visuals();
    let color = if resp.hovered() || resp.dragged() {
        v.widgets.hovered.bg_fill
    } else {
        v.widgets.inactive.bg_fill
    };
    ui.painter().rect_filled(bar, 3.0, v.faint_bg_color);
    ui.painter().rect_filled(thumb, 3.0, color);
}

/// Copy the selection as hex (or as text, with what isn't printable as dots).
fn copy(ui: &Ui, hv: &HexView, file: &mut HexFile, text: bool, actions: &mut Vec<Action>) {
    let Some(sel) = hv.selection() else {
        return;
    };
    let len = (sel.end() - sel.start() + 1).min(hexview::MAX_COPY);
    let Ok(bytes) = file.read(*sel.start(), len) else {
        return;
    };
    let out = if text {
        hexview::ascii_row(&bytes)
    } else {
        hexview::to_hex(&bytes)
    };
    table::copy(ui, out, actions);
    if len < sel.end() - sel.start() + 1 {
        actions.push(Action::Toast(ox_core::trf!(
            "Only the first {v1} were copied.",
            v1 = format::size(hexview::MAX_COPY)
        )));
    }
}
