#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod icons;
mod shortcuts;
mod tabs;
mod terminal;
mod theme;
mod thumbs;
mod ui;

use std::path::PathBuf;

fn main() -> eframe::Result {
    // Used by the installer and uninstaller: set or undo "open folders in
    // Oxplorer" without opening a window.
    match std::env::args().nth(1).as_deref() {
        Some("--register-default") => {
            let _ = ox_core::default_app::set_enabled(true);
            return Ok(());
        }
        Some("--unregister-default") => {
            let _ = ox_core::default_app::set_enabled(false);
            return Ok(());
        }
        _ => {}
    }
    // `oxplorer C:\some\folder` opens that folder in a new tab.
    // Opened as the default folder handler, `"C:\"` arrives as `C:"`.
    let open = std::env::args()
        .nth(1)
        .map(|a| PathBuf::from(ox_core::default_app::folder_argument(&a)))
        .filter(|p| ox_core::network::is_dir(p));
    ox_core::default_app::refresh();

    // Mica/Acrylic need a see-through window, which can only be chosen now.
    let backdrop = ox_core::shell::supported()
        && ox_core::settings::Settings::load().appearance.backdrop
            != ox_core::settings::Backdrop::Off;

    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_transparent(backdrop)
            .with_title(ox_core::APP_NAME)
            .with_app_id("oxplorer")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 420.0])
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/oxplorer.png"))
                    .expect("app icon is a valid PNG"),
            ),
        // We handle Ctrl+C/X/V ourselves so that pasting *files* works too.
        clipboard_shortcuts: false,
        ..Default::default()
    };
    #[cfg(windows)]
    if backdrop {
        options.renderer = eframe::Renderer::Glow;
    }
    eframe::run_native(
        ox_core::APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::OxApp::new(cc, open, backdrop)))),
    )
}
