//! Interrupted transfers (crash, sleep, pulled USB stick) in the transfer
//! panel, with "Resume" and "Discard".

use egui::{RichText, Ui};
use ox_core::entry::display_name;
use ox_core::format;
use ox_core::i18n::t as tr;

use crate::app::OxApp;

pub(super) fn interrupted(app: &mut OxApp, ui: &mut Ui) {
    let date_fmt = app.settings.browsing.date_format.clone();
    for pending in app.engine.resumable() {
        let req = &pending.request;
        let items = req.sources.len() + req.pairs.len();
        let mut resume = false;
        let mut discard = false;
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(ox_core::trf!(
                            "⚠ Interrupted · {v1} {v2} to {dest}",
                            v1 = req.kind.verb(),
                            v2 = ox_core::i18n::count(items as u64, "item"),
                            dest = display_name(&req.destination)
                        ))
                        .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        discard = ui
                            .button(tr("Discard"))
                            .on_hover_text(tr(
                                "Forget this transfer. Files that were already copied stay where they are.",
                            ))
                            .clicked();
                        resume = ui
                            .button(tr("Resume"))
                            .on_hover_text(tr(
                                "Continue where it stopped. Files that were copied completely are skipped, a file that was cut off is copied again.",
                            ))
                            .clicked();
                    });
                });
                let mut names: Vec<String> = req
                    .sources
                    .iter()
                    .chain(req.pairs.iter().map(|(src, _)| src))
                    .take(3)
                    .map(|p| display_name(p))
                    .collect();
                if items > 3 {
                    names.push(ox_core::trf!("… and {v1} more", v1 = items - 3));
                }
                ui.label(
                    RichText::new(format!(
                        "{} → {}",
                        names.join(", "),
                        req.destination.display()
                    ))
                    .small(),
                );
                let mut status = ox_core::trf!(
                    "{v1} ({v2}) already done",
                    v1 = ox_core::i18n::count(pending.done_files, "file"),
                    v2 = format::size(pending.done_bytes)
                );
                if let Some(when) = pending.saved {
                    status.push_str(&ox_core::trf!(
                        " · stopped {v1}",
                        v1 = format::time(when, &date_fmt)
                    ));
                }
                ui.label(RichText::new(status).small().weak());
            });
        ui.add_space(6.0);
        if resume {
            if !pending.servers().is_empty() {
                // Connects to the server(s) first.
                app.resume_remote_transfer(pending.clone());
            } else if req.destination.exists() {
                app.engine.resume(&pending);
            } else {
                app.toast(ox_core::trf!(
                    "\"{v1}\" isn't available. Connect the drive and try again.",
                    v1 = req.destination.display()
                ));
            }
        } else if discard {
            app.engine.discard(&pending);
        }
    }
}
