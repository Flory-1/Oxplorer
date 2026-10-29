//! The minimize, maximize and close buttons when the tabs sit in the title bar.

use ox_core::i18n::t as tr;

use egui::{vec2, Color32, Rect, Sense, Stroke};

use super::CAPTION_W;
use crate::app::Action;

pub fn caption_buttons_id() -> egui::Id {
    egui::Id::unique("caption_buttons")
}

/// Minimize, maximize/restore and close at the top right, drawn like
/// Windows 11's. Returns the rectangles of minimize and close (clicked here)
/// and of maximize (Windows handles that one, for its snap layouts).
pub(super) fn caption_buttons(
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
