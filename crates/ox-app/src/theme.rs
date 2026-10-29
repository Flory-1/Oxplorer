//! Applies appearance settings to egui's style.

use egui::{vec2, Color32, Context, CornerRadius, Stroke, Theme, ThemePreference};
use ox_core::settings::{self, Settings};
use ox_core::themes::{self, ColorTheme, Rgb};

pub fn accent(settings: &Settings) -> Color32 {
    let [r, g, b] = settings.appearance.accent;
    Color32::from_rgb(r, g, b)
}

/// Translucent accent used for selected rows.
pub fn selection_fill(settings: &Settings, dark: bool) -> Color32 {
    accent(settings).gamma_multiply(if dark { 0.35 } else { 0.22 })
}

pub fn row_height(settings: &Settings) -> f32 {
    if settings.appearance.compact_rows {
        22.0
    } else {
        28.0
    }
}

/// Colours of diffs: the background of added and removed lines, and the
/// stronger one of what changed within a line.
pub struct DiffColors {
    pub added: Color32,
    pub removed: Color32,
    pub added_word: Color32,
    pub removed_word: Color32,
    /// The sign and line numbers of added and removed lines.
    pub added_text: Color32,
    pub removed_text: Color32,
}

pub fn diff_colors(dark: bool) -> DiffColors {
    // The green and red of the Git column.
    let (green, red) = ([0x3b, 0xb2, 0x73], [0xe5, 0x48, 0x4d]);
    let tint = |[r, g, b]: [u8; 3], alpha: u8| Color32::from_rgba_unmultiplied(r, g, b, alpha);
    let (line, word) = if dark { (40, 90) } else { (34, 80) };
    DiffColors {
        added: tint(green, line),
        removed: tint(red, line),
        added_word: tint(green, word),
        removed_word: tint(red, word),
        added_text: tint(green, 255),
        removed_text: tint(red, 255),
    }
}

fn color(c: Rgb) -> Color32 {
    let [r, g, b] = c.0;
    Color32::from_rgb(r, g, b)
}

/// The scheme called `name` (or the built-in default for the mode).
pub fn scheme<'a>(all: &'a [ColorTheme], name: &str, dark: bool) -> &'a ColorTheme {
    let default = if dark {
        themes::DEFAULT_DARK
    } else {
        themes::DEFAULT_LIGHT
    };
    all.iter()
        .find(|t| t.name == name && t.dark == dark)
        .or_else(|| all.iter().find(|t| t.name == default))
        .or_else(|| all.iter().find(|t| t.dark == dark))
        .expect("the built-in themes exist")
}

/// `see_through`: a Mica/Acrylic backdrop is on, so the panels are made
/// translucent (the file list, menus and dialogs stay solid).
pub fn apply(ctx: &Context, settings: &Settings, all: &[ColorTheme], see_through: bool) {
    ctx.set_theme(match settings.appearance.theme {
        settings::Theme::System => ThemePreference::System,
        settings::Theme::Light => ThemePreference::Light,
        settings::Theme::Dark => ThemePreference::Dark,
    });

    let accent = accent(settings);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            let dark = theme == Theme::Dark;
            let v = &mut style.visuals;
            let name = if dark {
                &settings.appearance.dark_theme
            } else {
                &settings.appearance.light_theme
            };
            let t = scheme(all, name, dark);
            // Start from egui's look for the mode, so switching schemes
            // doesn't keep colours of the previous one.
            let base = if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
            v.widgets = base.widgets;
            v.window_stroke = base.window_stroke;
            v.override_text_color = None;
            v.panel_fill = color(t.panel);
            v.window_fill = color(t.window);
            v.extreme_bg_color = color(t.background);
            v.text_edit_bg_color = None;
            v.faint_bg_color = color(t.faint);
            if see_through {
                let [r, g, b, _] = v.panel_fill.to_array();
                v.panel_fill = Color32::from_rgba_unmultiplied(r, g, b, 90);
            }
            if let Some(text) = t.text.map(color) {
                // Buttons and lines are mixed from the panel and text colours.
                let panel = color(t.panel);
                let mix = |amount: f32| panel.lerp_to_gamma(text, amount);
                v.widgets.noninteractive.bg_stroke.color = mix(0.18);
                v.widgets.noninteractive.fg_stroke.color = text;
                for (w, amount) in [
                    (&mut v.widgets.inactive, 0.10),
                    (&mut v.widgets.hovered, 0.18),
                    (&mut v.widgets.active, 0.26),
                    (&mut v.widgets.open, 0.14),
                ] {
                    w.bg_fill = mix(amount);
                    w.weak_bg_fill = mix(amount);
                    w.fg_stroke.color = text;
                }
                v.window_stroke.color = mix(0.2);
            }

            v.selection.bg_fill = accent.gamma_multiply(if dark { 0.6 } else { 0.35 });
            v.selection.stroke =
                Stroke::new(1.0, if dark { Color32::WHITE } else { Color32::BLACK });
            v.hyperlink_color = accent;
            v.window_corner_radius = CornerRadius::same(10);
            v.menu_corner_radius = CornerRadius::same(8);
            for w in [
                &mut v.widgets.noninteractive,
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
            ] {
                w.corner_radius = CornerRadius::same(6);
            }
            style.spacing.item_spacing = vec2(6.0, 4.0);
            style.spacing.button_padding = vec2(8.0, 4.0);
            style.spacing.menu_margin = egui::Margin::same(6);
            style.interaction.selectable_labels = false;
        });
    }
}
