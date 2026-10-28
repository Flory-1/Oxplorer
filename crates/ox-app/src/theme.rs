//! Applies appearance settings to egui's style.

use egui::{vec2, Color32, Context, CornerRadius, Stroke, Theme, ThemePreference};
use ox_core::settings::{self, Settings};

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

pub fn apply(ctx: &Context, settings: &Settings) {
    ctx.set_theme(match settings.appearance.theme {
        settings::Theme::System => ThemePreference::System,
        settings::Theme::Light => ThemePreference::Light,
        settings::Theme::Dark => ThemePreference::Dark,
    });
    ctx.set_zoom_factor(settings.appearance.ui_scale.clamp(0.5, 3.0));

    let accent = accent(settings);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            let dark = theme == Theme::Dark;
            let v = &mut style.visuals;
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
            if dark {
                v.panel_fill = Color32::from_rgb(0x20, 0x21, 0x24);
                v.window_fill = Color32::from_rgb(0x2a, 0x2b, 0x2f);
                v.extreme_bg_color = Color32::from_rgb(0x17, 0x18, 0x1a);
                v.faint_bg_color = Color32::from_rgb(0x26, 0x27, 0x2b);
            } else {
                v.panel_fill = Color32::from_rgb(0xf3, 0xf4, 0xf6);
                v.window_fill = Color32::from_rgb(0xff, 0xff, 0xff);
                v.extreme_bg_color = Color32::from_rgb(0xff, 0xff, 0xff);
                v.faint_bg_color = Color32::from_rgb(0xec, 0xee, 0xf1);
            }
            style.spacing.item_spacing = vec2(6.0, 4.0);
            style.spacing.button_padding = vec2(8.0, 4.0);
            style.spacing.menu_margin = egui::Margin::same(6);
            style.interaction.selectable_labels = false;
        });
    }
}
