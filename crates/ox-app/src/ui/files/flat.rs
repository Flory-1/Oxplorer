//! The note above a flat view: that it's on, how the listing goes, and a
//! button to turn it off.

use super::*;
use crate::tabs::FlatState;

pub(super) fn bar(ui: &mut Ui, state: &FlatState, files: usize, actions: &mut Vec<Action>) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let chip = egui::Button::new(tr("🗂 Flat view ✖"))
                    .selected(true)
                    .corner_radius(CornerRadius::same(12));
                if ui
                    .add(chip)
                    .on_hover_text(shortcuts::hint(tr("Turn off flat view"), Command::FlatView))
                    .clicked()
                {
                    actions.push(Action::ToggleFlatView);
                }
                let weak = |ui: &mut Ui, text: String| {
                    ui.label(egui::RichText::new(text).weak());
                };
                weak(
                    ui,
                    tr("All files in this folder and its sub-folders").into(),
                );
                ui.separator();
                if state.walking() {
                    ui.spinner();
                    weak(
                        ui,
                        ox_core::trf!(
                            "Listing… {v1}",
                            v1 = ox_core::i18n::count(files as u64, "file")
                        ),
                    );
                } else {
                    weak(ui, ox_core::i18n::count(files as u64, "file"));
                }
                if state.truncated {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(ox_core::trf!(
                            "⚠ Limit reached: only the first {v1} files are listed",
                            v1 = ox_core::flat::MAX_FILES
                        ))
                        .color(ui.visuals().warn_fg_color),
                    )
                    .on_hover_text(tr(
                        "Open a sub-folder to see all of its files, or search for what you need.",
                    ));
                }
                if state.unreadable > 0 {
                    ui.separator();
                    weak(
                        ui,
                        ox_core::trf!(
                            "{v1} couldn't be read",
                            v1 = ox_core::i18n::count(state.unreadable, "folder")
                        ),
                    );
                }
            });
        });
}
