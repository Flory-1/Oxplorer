#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod icons;
mod tabs;
mod theme;
mod ui;

use std::path::PathBuf;

fn main() -> eframe::Result {
    // `oxplorer C:\some\folder` opens that folder in a new tab.
    let open = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(ox_core::APP_NAME)
            .with_app_id("oxplorer")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 420.0])
            .with_drag_and_drop(true),
        // We handle Ctrl+C/X/V ourselves so that pasting *files* works too.
        clipboard_shortcuts: false,
        ..Default::default()
    };
    eframe::run_native(
        ox_core::APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::OxApp::new(cc, open)))),
    )
}
