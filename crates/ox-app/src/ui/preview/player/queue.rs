//! The Player tab's queue list and the subtitles drawn over the video.

use super::*;
use egui::text::{LayoutJob, TextFormat};
use ox_core::player::subtitles::Subtitles;

/// "1×", "0.75×" ("0,75×" in German).
pub fn rate_label(rate: f64) -> String {
    let text = format!("{rate}×");
    match ox_core::i18n::is_german() {
        true => text.replace('.', ","),
        false => text,
    }
}

/// The files of the queue, the current one highlighted; click to play one.
pub(super) fn queue_list(ui: &mut Ui, state: &mut PlayerState) {
    let title = ox_core::trf!("☰ Queue ({n})", n = state.queue.len());
    egui::CollapsingHeader::new(title)
        .id_salt("player_queue")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(
                &mut state.auto_advance,
                tr("Play the next file when one ends"),
            );
            let scroll = std::mem::take(&mut state.scroll_queue);
            let current = state.queue.current_index();
            let mut jump = None;
            egui::ScrollArea::vertical()
                .id_salt("player_queue_list")
                .max_height(220.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, item) in state.queue.items().iter().enumerate() {
                        let here = current == Some(i);
                        let text = format!("{}. {}", i + 1, item.label());
                        let resp = ui
                            .add(egui::Button::selectable(here, text).truncate())
                            .on_hover_text(item.path.display().to_string());
                        if here && scroll {
                            resp.scroll_to_me(Some(egui::Align::Center));
                        }
                        if resp.clicked() && !here {
                            jump = Some(i);
                        }
                    }
                });
            if let Some(i) = jump {
                let ctx = ui.ctx().clone();
                state.jump(&ctx, i);
            }
        });
}

/// The cues at `position`, at the bottom of the video's `rect`: white text
/// on a dark box, italics kept.
pub(super) fn subtitle_overlay(ui: &Ui, rect: egui::Rect, subs: &Subtitles, position: Duration) {
    let size = (rect.height() * 0.06).clamp(11.0, 30.0);
    let mut job = LayoutJob::default();
    for (n, cue) in subs.at(position).enumerate() {
        for (l, line) in cue.lines.iter().enumerate() {
            if n + l > 0 {
                let font_id = egui::FontId::proportional(size);
                job.append("\n", 0.0, TextFormat::simple(font_id, egui::Color32::WHITE));
            }
            for span in line {
                let color = match span.bold {
                    true => egui::Color32::WHITE,
                    false => egui::Color32::from_gray(235),
                };
                job.append(
                    &span.text,
                    0.0,
                    TextFormat {
                        font_id: egui::FontId::proportional(size),
                        color,
                        italics: span.italic,
                        ..Default::default()
                    },
                );
            }
        }
    }
    if job.is_empty() {
        return;
    }
    job.wrap.max_width = rect.width() * 0.9;
    job.halign = egui::Align::Center;
    let painter = ui.painter_at(rect);
    let galley = painter.layout_job(job);
    let pos = egui::pos2(
        rect.center().x,
        rect.bottom() - size * 0.8 - galley.rect.height(),
    );
    let bg = galley.rect.translate(pos.to_vec2()).expand(size * 0.25);
    painter.rect_filled(bg, 4.0, egui::Color32::from_black_alpha(170));
    painter.galley(pos, galley, egui::Color32::WHITE);
}
