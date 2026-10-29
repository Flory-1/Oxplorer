//! Grid zoom: Ctrl+mouse wheel over the file view, and the view buttons
//! with the size slider at the right end of the status bar.

use super::*;
use ox_core::grid_zoom::{self, WheelSteps, Zoom};

/// Ctrl+mouse wheel (or a pinch) over `rect`: bigger or smaller tiles;
/// zooming out of the smallest ones shows the details view, zooming in
/// from there the grid again. egui turns Ctrl+wheel into zoom instead of
/// scrolling and only zooms the whole interface from the keyboard, so the
/// wheel events are read here directly.
pub(super) fn wheel(app: &mut OxApp, ui: &Ui, rect: Rect) {
    if !ui.rect_contains_pointer(rect) {
        return;
    }
    let id = Id::unique("grid_zoom_wheel");
    let mut wheel = ui
        .data(|d| d.get_temp::<WheelSteps>(id))
        .unwrap_or_default();
    let steps: i32 = ui.input(|i| {
        i.events
            .iter()
            .map(|e| match e {
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } if modifiers.command => match unit {
                    egui::MouseWheelUnit::Point => wheel.points(delta.y),
                    _ => wheel.lines(delta.y),
                },
                egui::Event::Zoom(f) => wheel.lines(f.ln() / 1.25f32.ln()),
                _ => 0,
            })
            .sum()
    });
    ui.data_mut(|d| d.insert_temp(id, wheel));
    if steps != 0 {
        zoom(app, steps);
    }
}

/// Take `steps` zoom steps (positive: bigger) in the active tab, as the
/// wheel does (also from the command palette).
pub fn zoom(app: &mut OxApp, steps: i32) {
    let tab = app.tabs.active_mut();
    let mut size = app.settings.appearance.grid_size;
    let mut mode = tab.view_mode;
    for _ in 0..steps.unsigned_abs() {
        let next = if mode == ViewMode::Grid {
            grid_zoom::step(size, steps > 0)
        } else if steps > 0 {
            grid_zoom::from_details()
        } else {
            break;
        };
        match next {
            Zoom::Grid(s) => {
                mode = ViewMode::Grid;
                size = s;
            }
            Zoom::Details => {
                mode = ViewMode::Details;
                break;
            }
        }
    }
    tab.view_mode = mode;
    set_size(app, size);
}

fn set_size(app: &mut OxApp, size: u32) {
    if app.settings.appearance.grid_size != size {
        app.settings.appearance.grid_size = size;
        save(app);
    }
}

fn save(app: &OxApp) {
    if let Err(e) = app.settings.save() {
        log::warn!("could not save settings: {e}");
    }
}

/// View buttons and, in the grid view, the icon size slider. Goes into the
/// status bar's right-to-left part, so it ends up at the right edge.
pub fn status_controls(app: &mut OxApp, ui: &mut Ui) {
    if !app.tabs.active().location().shows_files() {
        return;
    }
    let mode = app.tabs.active().view_mode;
    for (m, icon, tip) in [
        (ViewMode::Grid, "▦", tr("▦ Grid")),
        (ViewMode::Details, "☰", tr("☰ Details")),
    ] {
        if ui
            .selectable_label(mode == m, icon)
            .on_hover_text(tip)
            .clicked()
        {
            app.tabs.active_mut().view_mode = m;
        }
    }
    if mode != ViewMode::Grid {
        return;
    }
    let mut size = app.settings.appearance.grid_size;
    ui.spacing_mut().slider_width = 100.0;
    let slider = ui
        .add(egui::Slider::new(&mut size, grid_zoom::MIN..=grid_zoom::MAX).show_value(false))
        .on_hover_text(tr("Icon size (Ctrl+mouse wheel)"));
    if slider.changed() {
        // Saved once the slider is let go.
        app.settings.appearance.grid_size = size;
        if !slider.dragged() {
            save(app);
        }
    }
    if slider.drag_stopped() {
        save(app);
    }
}
