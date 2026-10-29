//! The countdown before the computer goes to sleep after all transfers.

use std::time::{Duration, Instant};

use egui::{Align2, Context, RichText};
use ox_core::i18n::t as tr;

use crate::app::OxApp;

/// A small window in the corner, while the sleep countdown runs.
pub fn countdown(app: &mut OxApp, ctx: &Context) {
    let Some(seconds) = app.transfer_watch.seconds_to_sleep(Instant::now()) else {
        return;
    };
    let mut cancel = false;
    egui::Window::new(tr("All transfers are done"))
        .id(egui::Id::unique("sleep-countdown"))
        .anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0])
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(ox_core::trf!(
                    "🌙 The computer goes to sleep in {v1} s.",
                    v1 = seconds
                ))
                .strong(),
            );
            ui.add_space(4.0);
            cancel = ui.button(tr("Stay awake")).clicked();
        });
    if cancel {
        app.transfer_watch.cancel_sleep();
    }
    ctx.request_repaint_after(Duration::from_millis(250));
}
