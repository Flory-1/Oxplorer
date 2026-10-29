//! The font tabs: a sample at several sizes and every glyph.

use super::*;

pub(super) const SAMPLE: &str = "The quick brown fox jumps over the lazy dog";

/// Editable sample text at several sizes, plus the alphabet.
pub(super) fn font_sample(ui: &mut Ui, family: &str, sample: &mut String) {
    use egui::{FontFamily, FontId};
    if sample.is_empty() {
        *sample = tr(SAMPLE).to_owned();
    }
    ui.add(
        egui::TextEdit::singleline(sample)
            .hint_text(tr("Type to try the font"))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(6.0);
    let font = |size: f32| FontId::new(size, FontFamily::Name(family.into()));
    ui.label(
        RichText::new(
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ\nabcdefghijklmnopqrstuvwxyz\n0123456789 .,;:!?&@#%*()[]{}",
        )
        .font(font(18.0)),
    );
    for size in [12.0, 16.0, 20.0, 28.0, 36.0, 48.0, 64.0] {
        ui.add_space(6.0);
        ui.label(RichText::new(format!("{size:.0} px")).weak().small());
        ui.add(egui::Label::new(RichText::new(sample.as_str()).font(font(size))).wrap());
    }
}

/// Every character of the font in a grid; only visible rows are drawn.
/// Clicking a glyph copies it.
pub(super) fn glyph_grid(ui: &mut Ui, family: &str, chars: &[char], actions: &mut Vec<Action>) {
    use egui::{Align2, FontFamily, FontId, Sense};
    const CELL: egui::Vec2 = egui::vec2(44.0, 48.0);
    ui.label(
        RichText::new(ox_core::trf!(
            "{v1} · click one to copy it",
            v1 = ox_core::i18n::count(chars.len() as u64, "character")
        ))
        .weak()
        .small(),
    );
    let cols = ((ui.available_width() / CELL.x).floor() as usize).max(1);
    let rows = chars.len().div_ceil(cols);
    let font = FontId::new(24.0, FontFamily::Name(family.into()));
    let v = ui.visuals().clone();
    egui::ScrollArea::vertical()
        .id_salt("preview_glyphs")
        .auto_shrink([false, false])
        .show_rows(ui, CELL.y, rows, |ui, range| {
            for row in range {
                let (line, _) = ui
                    .allocate_exact_size(egui::vec2(cols as f32 * CELL.x, CELL.y), Sense::hover());
                for col in 0..cols {
                    let Some(&c) = chars.get(row * cols + col) else {
                        break;
                    };
                    let rect = egui::Rect::from_min_size(
                        line.min + egui::vec2(col as f32 * CELL.x, 0.0),
                        CELL,
                    )
                    .shrink(2.0);
                    let id = egui::Id::unique(("glyph", row, col));
                    let resp = ui.interact(rect, id, Sense::click());
                    let fill = if resp.hovered() {
                        v.widgets.hovered.weak_bg_fill
                    } else {
                        v.faint_bg_color
                    };
                    ui.painter().rect_filled(rect, 4.0, fill);
                    ui.painter().text(
                        rect.center() - egui::vec2(0.0, 5.0),
                        Align2::CENTER_CENTER,
                        c,
                        font.clone(),
                        v.text_color(),
                    );
                    ui.painter().text(
                        rect.center_bottom() - egui::vec2(0.0, 6.0),
                        Align2::CENTER_CENTER,
                        format!("{:04X}", c as u32),
                        FontId::monospace(8.0),
                        v.weak_text_color(),
                    );
                    let resp = resp.on_hover_text(format!("U+{:04X}", c as u32));
                    if resp.clicked() {
                        ui.ctx().copy_text(c.to_string());
                        actions.push(Action::Toast(ox_core::trf!(
                            "Copied “{c}” ({code})",
                            c = c,
                            code = format!("U+{:04X}", c as u32)
                        )));
                    }
                }
            }
        });
}
