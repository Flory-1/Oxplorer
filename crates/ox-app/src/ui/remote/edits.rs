//! The bar that offers to upload a saved copy of a remote file (and asks
//! before replacing a file that changed on the server meanwhile).

use egui::{Align2, Context, Id, Order, RichText};
use ox_core::i18n::t as tr;

use crate::app::{EditAnswer, OxApp};

pub(super) fn show(app: &mut OxApp, ctx: &Context) {
    let edits = &app.remote.edits;
    let (local, conflict) = match (&edits.conflict, &edits.offer) {
        (Some((local, _)), _) => (local.clone(), true),
        (None, Some(local)) => (local.clone(), false),
        (None, None) => return,
    };
    let Some(file) = edits.files.get(&local) else {
        app.remote.edits.offer = None;
        app.remote.edits.conflict = None;
        return;
    };
    let name = file.addr.name();
    let server = file.addr.server_url();
    let mut answer = None;
    egui::Area::new(Id::unique("remote_edit_bar"))
        .anchor(Align2::CENTER_BOTTOM, [0.0, -48.0])
        .order(Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(460.0);
                if conflict {
                    ui.label(
                        RichText::new(ox_core::trf!(
                            "\"{name}\" changed on the server after you opened it.",
                            name = name
                        ))
                        .strong(),
                    );
                    ui.label(tr("Replace it with your version?"));
                    ui.horizontal(|ui| {
                        if ui.button(tr("Replace")).clicked() {
                            answer = Some(EditAnswer::Replace);
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            answer = Some(EditAnswer::Keep);
                        }
                    });
                } else {
                    ui.label(
                        RichText::new(ox_core::trf!(
                            "Upload the changes to {server}?",
                            server = server
                        ))
                        .strong(),
                    );
                    ui.label(ox_core::trf!("\"{name}\" was saved.", name = name));
                    ui.horizontal(|ui| {
                        if ui.button(tr("Upload")).clicked() {
                            answer = Some(EditAnswer::Upload);
                        }
                        if ui.button(tr("Ignore")).clicked() {
                            answer = Some(EditAnswer::Ignore);
                        }
                        if ui
                            .button(tr("Always for this file"))
                            .on_hover_text(tr(
                                "Upload this file's changes without asking until Oxplorer is closed.",
                            ))
                            .clicked()
                        {
                            answer = Some(EditAnswer::Always);
                        }
                    });
                }
            });
        });
    if let Some(answer) = answer {
        app.remote_edit_answer(&local, answer);
    }
}
