#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod column_values;
#[cfg(test)]
mod i18n_check;
#[cfg(test)]
mod icon_check;
mod icons;
mod shortcuts;
mod tabs;
mod terminal;
mod theme;
mod thumbs;
mod ui;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let crashed = ox_core::logfile::init().is_some();
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
    let launch = ox_core::instance::parse(std::env::args().skip(1));
    let open = launch
        .folder
        .as_deref()
        .map(|a| PathBuf::from(ox_core::default_app::folder_argument(a)))
        .filter(|p| ox_core::network::is_dir(p));
    ox_core::default_app::refresh();
    // Every window is a process of its own. The first one started normally
    // owns the saved session; windows opened from another one don't.
    let session_lock = if launch.new_window {
        None
    } else {
        ox_core::instance::own_session(&ox_core::config_dir())
    };
    let handed = launch.tabs.as_deref().and_then(|p| {
        let session = ox_core::settings::load_toml::<tabs::Session>(p);
        let _ = std::fs::remove_file(p);
        session
    });
    // Only the session's window keeps its place and size in window.ron, so
    // other windows don't open on top of it or move it next time.
    let main_window = session_lock.is_some();

    // Mica/Acrylic need a see-through window, which can only be chosen now.
    let backdrop = ox_core::shell::supported()
        && ox_core::settings::Settings::load().appearance.backdrop
            != ox_core::settings::Backdrop::Off;

    let mut viewport = egui::ViewportBuilder::default();
    if let Some(pos) = launch.pos {
        viewport = viewport.with_position(pos);
    }
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut options = eframe::NativeOptions {
        viewport: viewport
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
        // Window position, size and maximized state are kept in this file
        // (a file, despite the name); eframe keeps the window on screen if
        // its monitor is gone.
        persist_window: main_window,
        persistence_path: Some(ox_core::config_dir().join("window.ron")),
        ..Default::default()
    };
    #[cfg(windows)]
    if backdrop {
        options.renderer = eframe::Renderer::Glow;
    }
    eframe::run_native(
        ox_core::APP_NAME,
        options,
        Box::new(move |cc| {
            let start = app::Start {
                open,
                tabs: handed,
                session_lock,
            };
            let mut app = app::OxApp::new(cc, start, backdrop);
            if crashed {
                ui::help_page::crash_note(&mut app);
            }
            if app.settings.startup.check_updates {
                let ctx = cc.egui_ctx.clone();
                ox_core::update::start(move || ctx.request_repaint());
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    /// The version in the exe's resources (Properties → Details) and its
    /// manifest must match Cargo.toml; the release workflow checks the tag.
    #[test]
    fn windows_resources_carry_the_crate_version() {
        let version = env!("CARGO_PKG_VERSION");
        let commas = format!("{},0", version.replace('.', ","));
        let rc = include_str!("../assets/oxplorer.rc");
        assert!(
            rc.contains(&format!("FILEVERSION {commas}")),
            "oxplorer.rc FILEVERSION"
        );
        assert!(
            rc.contains(&format!("PRODUCTVERSION {commas}")),
            "oxplorer.rc PRODUCTVERSION"
        );
        assert!(
            rc.contains(&format!("\"FileVersion\", \"{version}\"")),
            "oxplorer.rc FileVersion"
        );
        assert!(
            rc.contains(&format!("\"ProductVersion\", \"{version}\"")),
            "oxplorer.rc ProductVersion"
        );
        let manifest = include_str!("../assets/oxplorer.manifest");
        assert!(
            manifest.contains(&format!("name=\"Oxplorer\" version=\"{version}.0\"")),
            "oxplorer.manifest"
        );
    }
}
