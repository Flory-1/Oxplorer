//! The Appearance page's theme editor and colour swatches.

use egui::{vec2, Color32, RichText, Ui};
use ox_core::i18n::t as tr;

use super::row;
use crate::app::{Action, OxApp};

/// Little colour squares: panel, file list, text and accent.
pub(super) fn swatch(ui: &mut Ui, t: &ox_core::themes::ColorTheme) {
    let c = |rgb: ox_core::themes::Rgb| Color32::from_rgb(rgb.0[0], rgb.0[1], rgb.0[2]);
    let text = t.text.map(c).unwrap_or(if t.dark {
        Color32::from_gray(210)
    } else {
        Color32::from_gray(40)
    });
    let mut colors = vec![c(t.panel), c(t.background), text];
    colors.extend(t.accent.map(c));
    let (rect, _) = ui.allocate_exact_size(vec2(14.0 * 4.0, 14.0), egui::Sense::hover());
    for (i, col) in colors.into_iter().enumerate() {
        let r = egui::Rect::from_min_size(rect.min + vec2(i as f32 * 14.0, 0.0), vec2(12.0, 12.0));
        ui.painter().rect_filled(r, 2.0, col);
        ui.painter().rect_stroke(
            r,
            2.0,
            egui::Stroke::new(1.0, Color32::from_gray(128).gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
}

/// Edit the colours of the theme in use; built-in themes are copied first.
pub(super) fn theme_editor(ui: &mut Ui, app: &mut OxApp) {
    use ox_core::themes;
    ui.add_space(14.0);
    ui.label(RichText::new(tr("Theme colours")).size(16.0).strong());
    let dark = ui.visuals().dark_mode;
    let a = &app.settings.appearance;
    let name = if dark { &a.dark_theme } else { &a.light_theme };
    let mut t = crate::theme::scheme(&app.themes, name, dark).clone();
    ui.label(
        RichText::new(ox_core::trf!(
            "Editing \"{v1}\" ({v2} mode). Built-in themes are copied when you change them.",
            v1 = t.name,
            v2 = if dark { tr("dark") } else { tr("light") }
        ))
        .weak(),
    );
    let mut changed = false;
    for (label, color) in [
        (tr("Panels"), &mut t.panel),
        (tr("Menus and dialogs"), &mut t.window),
        (tr("File list and fields"), &mut t.background),
        (tr("Stripes"), &mut t.faint),
    ] {
        row(ui, label, "", |ui| {
            changed |= ui.color_edit_button_srgb(&mut color.0).changed();
        });
    }
    row(
        ui,
        tr("Text"),
        tr("Buttons and lines are mixed from it."),
        |ui| {
            let mut own = t.text.is_some();
            if ui.checkbox(&mut own, tr("Own colour")).changed() {
                let default = if dark {
                    [0xd8, 0xdb, 0xe0]
                } else {
                    [0x20, 0x22, 0x26]
                };
                t.text = own.then_some(themes::Rgb(default));
                changed = true;
            }
            if let Some(text) = &mut t.text {
                changed |= ui.color_edit_button_srgb(&mut text.0).changed();
            }
        },
    );
    if changed {
        if t.builtin {
            t.builtin = false;
            t.name = themes::free_name(
                &ox_core::trf!("{name} (custom)", name = t.name),
                &app.themes,
            );
            let a = &mut app.settings.appearance;
            if dark {
                a.dark_theme = t.name.clone();
            } else {
                a.light_theme = t.name.clone();
            }
        }
        match themes::save(&t) {
            Ok(_) => {
                match app.themes.iter_mut().find(|x| x.name == t.name) {
                    Some(x) => *x = t.clone(),
                    None => app.themes.push(t.clone()),
                }
                app.themes_changed = true;
            }
            Err(e) => app.toast(ox_core::trf!("Could not save the theme: {e}", e = e)),
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .button(tr("📤 Export"))
            .on_hover_text(tr("Save this theme as a file you can share"))
            .clicked()
        {
            let mut export = t.clone();
            export.accent = Some(themes::Rgb(app.settings.appearance.accent));
            match themes::save(&export) {
                Ok(path) => {
                    app.toast(ox_core::trf!("Saved as {v1}", v1 = path.display()));
                    app.push(Action::OpenInNewTab(
                        crate::tabs::Location::Folder(themes::dir()),
                        true,
                    ));
                    app.themes = themes::load_all();
                }
                Err(e) => app.toast(ox_core::trf!("Could not save the theme: {e}", e = e)),
            }
        }
        if ui.button(tr("📂 Themes folder")).clicked() {
            let _ = std::fs::create_dir_all(themes::dir());
            app.push(Action::OpenInNewTab(
                crate::tabs::Location::Folder(themes::dir()),
                true,
            ));
        }
        if !t.builtin && ui.button(tr("🗑 Delete theme")).clicked() {
            if let Err(e) = themes::remove(&t.name) {
                app.toast(ox_core::trf!("Could not delete the theme: {e}", e = e));
            }
            app.themes = themes::load_all();
            let a = &mut app.settings.appearance;
            if dark {
                a.dark_theme = themes::DEFAULT_DARK.to_owned();
            } else {
                a.light_theme = themes::DEFAULT_LIGHT.to_owned();
            }
            app.themes_changed = true;
        }
    });
    ui.label(
        RichText::new(tr(
            "To use a theme someone sent you, right-click its .toml file → Use as theme.",
        ))
        .weak()
        .small(),
    );
}
