//! The settings page (opens as a tab).

use egui::{vec2, Color32, RichText, Ui};
use ox_core::entry::{SortKey, SortOrder};
use ox_core::i18n::t as tr;
use ox_core::index::IndexState;
use ox_core::settings::{Backdrop, Settings, StartPage, Theme, ViewMode};
use ox_core::transfer::ConflictPolicy;

use crate::app::{Action, OxApp};

const SECTIONS: [&str; 8] = [
    "🎨 Appearance",
    "📂 Browsing",
    "🚀 Startup",
    "◧ Sidebar",
    "⇄ Copy engine",
    "🔍 Search",
    "⌨ Shortcuts",
    "ℹ About",
];

/// Query examples on the Search page: what to type, and what it does.
const QUERY_SYNTAX: [(&str, &str); 11] = [
    ("report", "name contains \"report\""),
    ("\"annual report\"", "exact phrase"),
    ("*.rs   img_??", "wildcards"),
    ("/^IMG_\\d+\\.jpe?g$/", "regular expression (also regex:…)"),
    ("-tmp", "exclude names containing \"tmp\""),
    ("ext:pdf,docx", "by extension"),
    ("type:folder / type:file", "only folders / files"),
    (
        "kind:image,video",
        "by kind: image, video, audio, document, archive, code, app",
    ),
    ("size:>10mb   size:<1kb", "by size"),
    ("modified:7d   modified:today", "recently modified"),
    ("content:\"TODO\"", "text inside files (≤ 32 MB)"),
];

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let before = app.settings.clone();
    // Section list on the left, content on the right.
    egui::Panel::left("settings_nav")
        .resizable(false)
        .exact_size(210.0)
        .frame(egui::Frame::new().inner_margin(egui::Margin::same(14)))
        .show(ui, |ui| {
            ui.heading(RichText::new(tr("Settings")).strong());
            ui.add_space(10.0);
            for (i, name) in SECTIONS.iter().enumerate() {
                let selected = app.ui.settings_section == i;
                let button = egui::Button::new(RichText::new(tr(name)).size(14.0))
                    .selected(selected)
                    .frame(selected)
                    .min_size(vec2(180.0, 30.0));
                if ui.add(button).clicked() {
                    app.ui.settings_section = i;
                }
            }
        });
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(24, 16))
                .show(ui, |ui| {
                    ui.set_max_width(680.0);
                    let section = app.ui.settings_section;
                    let title = tr(SECTIONS[section]);
                    let title = &title[title.find(' ').map_or(0, |i| i + 1)..];
                    ui.heading(RichText::new(title).size(20.0).strong());
                    ui.add_space(12.0);
                    match section {
                        0 => {
                            appearance(ui, app);
                            theme_editor(ui, app);
                        }
                        1 => browsing(ui, &mut app.settings),
                        2 => {
                            startup(ui, &mut app.settings);
                            default_folder_handler(ui, app);
                        }
                        3 => sidebar(ui, &mut app.settings),
                        4 => transfer(ui, &mut app.settings),
                        5 => {
                            search(ui, &mut app.settings);
                            index_section(ui, app);
                        }
                        6 => shortcuts(ui, app),
                        _ => about(ui),
                    }
                    ui.add_space(24.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        if section < 6 && ui.button(tr("Reset this page to defaults")).clicked() {
                            reset_section(&mut app.settings, section);
                        }
                        ui.label(
                            RichText::new(ox_core::trf!(
                                "Saved automatically to {v1}",
                                v1 = Settings::path().display()
                            ))
                            .small()
                            .weak(),
                        );
                    });
                });
        });
    if app.settings != before {
        app.push(Action::SettingsChanged);
    }
}

fn reset_section(s: &mut Settings, section: usize) {
    let d = Settings::default();
    match section {
        0 => s.appearance = d.appearance,
        1 => s.browsing = d.browsing,
        2 => s.startup = d.startup,
        3 => s.sidebar = d.sidebar,
        4 => s.transfer = d.transfer,
        5 => s.search = d.search,
        _ => {}
    }
    if section == 1 {
        s.terminal = d.terminal;
    }
}

/// Little colour squares: panel, file list, text and accent.
fn swatch(ui: &mut Ui, t: &ox_core::themes::ColorTheme) {
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
fn theme_editor(ui: &mut Ui, app: &mut OxApp) {
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

/// One labelled row with an optional description.
fn row(ui: &mut Ui, label: &str, help: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(300.0);
            ui.label(RichText::new(label).strong());
            if !help.is_empty() {
                ui.label(RichText::new(help).small().weak());
            }
        });
        add(ui);
    });
    ui.add_space(10.0);
}

fn toggle(ui: &mut Ui, value: &mut bool, label: &str, help: &str) {
    row(ui, label, help, |ui| {
        ui.checkbox(value, "");
    });
}

fn appearance(ui: &mut Ui, app: &mut OxApp) {
    let themes = &app.themes;
    let a = &mut app.settings.appearance;
    row(
        ui,
        tr("Language"),
        tr("System uses the language Windows is set to."),
        |ui| {
            use ox_core::settings::Language;
            ui.selectable_value(&mut a.language, Language::System, tr("System"));
            ui.selectable_value(&mut a.language, Language::English, "English");
            ui.selectable_value(&mut a.language, Language::German, "Deutsch");
        },
    );
    row(
        ui,
        tr("Mode"),
        tr("Follow Windows, or force light/dark."),
        |ui| {
            ui.selectable_value(&mut a.theme, Theme::System, tr("System"));
            ui.selectable_value(&mut a.theme, Theme::Light, tr("☀ Light"));
            ui.selectable_value(&mut a.theme, Theme::Dark, tr("🌙 Dark"));
        },
    );
    for (label, help, dark) in [
        (tr("Dark theme"), tr("Colours in dark mode."), true),
        (tr("Light theme"), tr("Colours in light mode."), false),
    ] {
        let mut picked = None;
        row(ui, label, help, |ui| {
            let current = if dark { &a.dark_theme } else { &a.light_theme };
            egui::ComboBox::from_id_salt(label)
                .selected_text(current.as_str())
                .width(200.0)
                .show_ui(ui, |ui| {
                    for t in themes.iter().filter(|t| t.dark == dark) {
                        ui.horizontal(|ui| {
                            swatch(ui, t);
                            if ui.selectable_label(*current == t.name, &t.name).clicked() {
                                picked = Some((t.name.clone(), t.accent));
                            }
                        });
                    }
                });
        });
        if let Some((name, accent)) = picked {
            if dark {
                a.dark_theme = name;
            } else {
                a.light_theme = name;
            }
            // A theme's own accent comes along; it can still be changed below.
            if let Some(accent) = accent {
                a.accent = accent.0;
            }
        }
    }
    row(
        ui,
        tr("Accent colour"),
        tr("Used for selection, focus and progress."),
        |ui| {
            ui.color_edit_button_srgb(&mut a.accent);
            for preset in [
                [0x4c, 0x8d, 0xf6],
                [0x3b, 0xb2, 0x73],
                [0xe5, 0x48, 0x4d],
                [0x9b, 0x6b, 0xf2],
                [0xf0, 0x7a, 0x3a],
                [0x2c, 0xb5, 0xc8],
            ] {
                let c = Color32::from_rgb(preset[0], preset[1], preset[2]);
                if ui
                    .add(egui::Button::new("").fill(c).min_size(vec2(18.0, 18.0)))
                    .clicked()
                {
                    a.accent = preset;
                }
            }
        },
    );
    row(ui, tr("UI scale"), tr("Zoom the whole interface."), |ui| {
        ui.add(
            egui::Slider::new(&mut a.ui_scale, 0.75..=2.0)
                .step_by(0.05)
                .custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
        );
    });
    toggle(
        ui,
        &mut a.compact_rows,
        tr("Compact rows"),
        tr("Smaller row height in the details view."),
    );
    toggle(
        ui,
        &mut a.thumbnails,
        tr("Thumbnails"),
        tr("Show previews of pictures and videos in the grid view."),
    );
    if ox_core::shell::supported() {
        row(
            ui,
            tr("Window backdrop"),
            tr("Windows 11 material behind the sidebar and header. Turning it on takes effect after restarting Oxplorer."),
            |ui| {
                ui.selectable_value(&mut a.backdrop, Backdrop::Off, tr("Off"));
                ui.selectable_value(&mut a.backdrop, Backdrop::Mica, tr("Mica"));
                ui.selectable_value(&mut a.backdrop, Backdrop::Acrylic, tr("Acrylic"));
            },
        );
        toggle(
            ui,
            &mut a.system_icons,
            tr("Windows file icons"),
            tr("Show the same icons as Windows Explorer instead of the built-in ones."),
        );
    }
}

fn browsing(ui: &mut Ui, s: &mut Settings) {
    let b = &mut s.browsing;
    toggle(ui, &mut b.show_hidden, tr("Show hidden items"), "");
    toggle(
        ui,
        &mut b.show_extensions,
        tr("Show file name extensions"),
        "",
    );
    toggle(
        ui,
        &mut b.folders_first,
        tr("Folders first"),
        tr("Sort folders before files."),
    );
    row(ui, tr("Default view"), "", |ui| {
        ui.selectable_value(&mut b.default_view, ViewMode::Details, tr("☰ Details"));
        ui.selectable_value(&mut b.default_view, ViewMode::Grid, tr("▦ Grid"));
    });
    row(ui, tr("Default sort"), "", |ui| {
        egui::ComboBox::from_id_salt("default_sort")
            .selected_text(format!("{:?}", b.default_sort))
            .show_ui(ui, |ui| {
                for k in [
                    SortKey::Name,
                    SortKey::Modified,
                    SortKey::Type,
                    SortKey::Size,
                ] {
                    ui.selectable_value(&mut b.default_sort, k, format!("{k:?}"));
                }
            });
        ui.selectable_value(&mut b.default_order, SortOrder::Ascending, "⏶");
        ui.selectable_value(&mut b.default_order, SortOrder::Descending, "⏷");
    });
    toggle(
        ui,
        &mut b.single_click_open,
        tr("Single-click to open"),
        tr("Open items with one click instead of a double click."),
    );
    toggle(ui, &mut b.confirm_delete, tr("Confirm before deleting"), "");
    toggle(
        ui,
        &mut b.delete_to_trash,
        tr("Delete to Recycle Bin"),
        tr("Shift+Delete always deletes permanently."),
    );
    toggle(
        ui,
        &mut b.auto_refresh,
        tr("Refresh automatically"),
        tr("Update the view when files change on disk."),
    );
    toggle(
        ui,
        &mut b.folder_sizes,
        tr("Show folder sizes"),
        tr("Calculate the size of folders in the background for the Size column."),
    );
    toggle(
        ui,
        &mut b.open_archives,
        tr("Open archives like folders"),
        tr("Browse zip, 7z and tar files in Oxplorer instead of opening them with another app."),
    );
    toggle(
        ui,
        &mut b.git_status,
        tr("Git status"),
        tr("In Git repositories, show a Git column and the branch (needs Git installed)."),
    );
    toggle(ui, &mut b.show_status_bar, tr("Show status bar"), "");
    row(
        ui,
        tr("Date format"),
        tr("chrono format, e.g. %d.%m.%Y %H:%M or %Y-%m-%d"),
        |ui| {
            ui.add(egui::TextEdit::singleline(&mut b.date_format).desired_width(160.0));
            ui.label(
                RichText::new(ox_core::format::time(
                    std::time::SystemTime::now(),
                    &b.date_format,
                ))
                .weak(),
            );
        },
    );

    ui.add_space(14.0);
    ui.label(RichText::new(tr("Terminal panel")).size(16.0).strong());
    let t = &mut s.terminal;
    let default_shell = if cfg!(windows) {
        "powershell.exe -NoLogo"
    } else {
        "$SHELL"
    };
    row(
        ui,
        tr("Shell"),
        tr("Used the next time the terminal starts. Empty uses the default."),
        |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut t.shell)
                    .hint_text(default_shell)
                    .desired_width(220.0),
            );
        },
    );
    toggle(
        ui,
        &mut t.follow_folder,
        tr("Follow the current folder"),
        tr("Change the terminal's folder whenever you open another folder."),
    );
}

fn startup(ui: &mut Ui, s: &mut Settings) {
    let st = &mut s.startup;
    row(ui, tr("When Oxplorer starts"), "", |ui| {
        ui.vertical(|ui| {
            ui.radio_value(
                &mut st.start_page,
                StartPage::Home,
                tr("Open the Home page"),
            );
            ui.radio_value(
                &mut st.start_page,
                StartPage::RestoreSession,
                tr("Restore tabs and groups from last time"),
            );
            ui.radio_value(
                &mut st.start_page,
                StartPage::Folder,
                tr("Open a specific folder"),
            );
        });
    });
    if st.start_page == StartPage::Folder {
        row(ui, tr("Start folder"), "", |ui| {
            let mut text = st
                .start_folder
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(280.0)
                        .hint_text(tr("C:\\Users\\…")),
                )
                .changed()
            {
                st.start_folder = (!text.is_empty()).then(|| text.into());
            }
        });
    }
}

/// "Open folders in Oxplorer" (Windows). Lives in the registry, not settings.toml.
fn default_folder_handler(ui: &mut Ui, app: &mut OxApp) {
    if !ox_core::default_app::supported() {
        return;
    }
    let mut on = *app
        .ui
        .default_folder_handler
        .get_or_insert_with(ox_core::default_app::is_enabled);
    let was = on;
    toggle(
        ui,
        &mut on,
        tr("Open folders in Oxplorer"),
        tr("Folders and drives opened anywhere else (desktop, other programs, Win+E) open in Oxplorer instead of Explorer. Turn off to go back to Explorer."),
    );
    if on != was {
        app.push(Action::SetDefaultFolderHandler(on));
    }
}

fn sidebar(ui: &mut Ui, s: &mut Settings) {
    let sb = &mut s.sidebar;
    toggle(ui, &mut sb.visible, tr("Show sidebar"), "");
    toggle(ui, &mut sb.show_favorites, tr("Favorites section"), "");
    toggle(ui, &mut sb.show_drives, tr("Drives section"), "");
    toggle(ui, &mut sb.show_recent, tr("Recent folders section"), "");
    row(ui, tr("Recent folders shown"), "", |ui| {
        ui.add(egui::Slider::new(&mut sb.recent_count, 1..=20));
    });
}

fn transfer(ui: &mut Ui, s: &mut Settings) {
    let t = &mut s.transfer;
    ui.label(
        RichText::new(tr(
            "Oxplorer copies files with its own multi-threaded engine instead of the Windows copy dialog. \
             On Windows each file is copied with CopyFileExW (kernel fast path), large files bypass the cache.",
        ))
        .weak(),
    );
    ui.add_space(10.0);
    row(
        ui,
        tr("Parallel file copies"),
        tr("Auto: one file at a time on HDDs, 8 on SSDs. More helps with many small files on SSDs."),
        |ui| {
            ui.add(
                egui::Slider::new(&mut t.workers, 0..=32).custom_formatter(|v, _| {
                    if v == 0.0 {
                        tr("Auto").into()
                    } else {
                        format!("{v:.0}")
                    }
                }),
            );
        },
    );
    row(
        ui,
        tr("Parallel jobs"),
        tr("How many copy jobs run at once; the rest wait in a queue."),
        |ui| {
            ui.add(egui::Slider::new(&mut t.max_concurrent_jobs, 1..=8));
        },
    );
    toggle(
        ui,
        &mut t.per_disk_queue,
        tr("One job per drive at a time"),
        tr("Copies on the same drive wait for each other; different drives still run in parallel."),
    );
    toggle(
        ui,
        &mut t.verify,
        tr("Verify copied files"),
        tr("Compare every copy with its original (xxh3). Slower, but moves only delete originals that were copied correctly."),
    );
    row(
        ui,
        tr("Speed limit"),
        tr("Per transfer. 0 = unlimited. Useful to keep the PC responsive during big copies."),
        |ui| {
            ui.add(
                egui::DragValue::new(&mut t.speed_limit_mib)
                    .range(0..=10_000)
                    .custom_formatter(|v, _| {
                        if v == 0.0 {
                            tr("Unlimited").into()
                        } else {
                            format!("{v:.0} MiB/s")
                        }
                    }),
            );
        },
    );
    row(
        ui,
        tr("Buffer size"),
        tr("Used by the portable copy loop."),
        |ui| {
            ui.add(
                egui::Slider::new(&mut t.buffer_kib, 256..=65536)
                    .logarithmic(true)
                    .suffix(tr(" KiB")),
            );
        },
    );
    row(
        ui,
        tr("Unbuffered I/O from"),
        tr("Files at least this big skip the Windows file cache."),
        |ui| {
            ui.add(
                egui::Slider::new(&mut t.unbuffered_threshold_mib, 16..=8192)
                    .logarithmic(true)
                    .suffix(tr(" MiB")),
            );
        },
    );
    row(
        ui,
        tr("When a file already exists"),
        tr("\"Ask me\" asks once per job, not once per file."),
        |ui| {
            egui::ComboBox::from_id_salt("conflict")
                .selected_text(t.default_conflict.label())
                .show_ui(ui, |ui| {
                    for p in ConflictPolicy::ALL {
                        ui.selectable_value(&mut t.default_conflict, p, p.label());
                    }
                });
        },
    );
    toggle(
        ui,
        &mut t.preserve_timestamps,
        tr("Keep modification dates"),
        "",
    );
    toggle(
        ui,
        &mut t.use_system_clipboard,
        tr("Share clipboard with Windows Explorer"),
        tr("Copy in Explorer, paste in Oxplorer (and the other way around)."),
    );
    toggle(
        ui,
        &mut t.drag_moves_on_same_drive,
        tr("Dragging moves on the same drive"),
        tr("Like Explorer. Hold Ctrl to copy, Shift to move."),
    );
    row(
        ui,
        tr("Transfer history"),
        tr("Every finished copy and move, with errors."),
        |ui| {
            let log = ox_core::transfer::history::log_path();
            if ui
                .add_enabled(log.exists(), egui::Button::new(tr("📜 Open log")))
                .clicked()
            {
                let _ = ox_core::fsops::open_default(&log);
            }
        },
    );
}

fn search(ui: &mut Ui, s: &mut Settings) {
    let se = &mut s.search;
    toggle(
        ui,
        &mut se.instant_filter,
        tr("Filter while typing"),
        tr("The search box filters the current folder instantly; Enter searches sub-folders."),
    );
    toggle(ui, &mut se.include_hidden, tr("Include hidden files"), "");
    toggle(ui, &mut se.case_sensitive, tr("Case sensitive"), "");
    row(ui, tr("Maximum results"), "", |ui| {
        ui.add(
            egui::DragValue::new(&mut se.max_results)
                .range(100..=1_000_000)
                .speed(100),
        );
    });
    row(
        ui,
        tr("Search threads"),
        tr("0 = one per CPU core."),
        |ui| {
            ui.add(egui::Slider::new(&mut se.threads, 0..=64));
        },
    );
    ui.add_space(8.0);
    ui.label(RichText::new(tr("Query syntax")).strong());
    egui::Grid::new("syntax")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            for (q, d) in QUERY_SYNTAX {
                ui.monospace(q);
                ui.label(tr(d));
                ui.end_row();
            }
        });
}

fn index_section(ui: &mut Ui, app: &mut OxApp) {
    ui.add_space(14.0);
    ui.label(RichText::new(tr("Search index")).size(16.0).strong());
    ui.label(
        RichText::new(tr(
            "Keeps the names of all files in memory (about 50 MB per million files) so searching \
             whole drives takes milliseconds. Built in the background and saved between runs.",
        ))
        .weak(),
    );
    ui.add_space(8.0);
    let se = &mut app.settings.search;
    toggle(ui, &mut se.index_enabled, tr("Index file names"), "");
    if !se.index_enabled {
        return;
    }
    toggle(
        ui,
        &mut se.search_as_you_type,
        tr("Results while typing"),
        tr("On the Home page and in search tabs, results update with every key."),
    );

    let index = app.index.clone();
    row(ui, tr("Status"), "", |ui| {
        ui.vertical(|ui| {
            match index.state() {
                IndexState::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("Loading the saved index…"));
                    });
                }
                IndexState::Building => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(ox_core::trf!(
                            "Indexing… {v1} items so far",
                            v1 = index.progress()
                        ));
                    });
                }
                IndexState::Failed(e) => {
                    ui.colored_label(Color32::from_rgb(0xe5, 0x48, 0x4d), e);
                }
                IndexState::Empty | IndexState::Ready => {}
            }
            if let Some(idx) = index.index() {
                let age = std::time::SystemTime::now()
                    .duration_since(idx.built_at())
                    .unwrap_or_default();
                let mut text = ox_core::trf!(
                    "✔ {v1} items · {v2} in memory · updated {v3} ago",
                    v1 = idx.len(),
                    v2 = ox_core::format::size(idx.memory_bytes() as u64),
                    v3 = ox_core::format::duration(age)
                );
                if let Some((_, took)) = index.last_build() {
                    text.push_str(&ox_core::trf!(
                        " · built in {secs} s",
                        secs = ox_core::format::decimal(took.as_secs_f64(), 1)
                    ));
                }
                ui.label(text);
            } else if index.state() == IndexState::Empty {
                ui.label(tr("Not built yet."));
            }
            ui.horizontal(|ui| {
                if index.is_building() {
                    if ui.button(tr("⏹ Stop")).clicked() {
                        index.cancel();
                    }
                } else if ui.button(tr("🔄 Rebuild now")).clicked() {
                    app.actions.push(Action::RebuildIndex);
                }
            });
        });
    });

    let se = &mut app.settings.search;
    row(
        ui,
        tr("Indexed folders"),
        if cfg!(windows) {
            tr("One per line. Empty = all drives.")
        } else {
            tr("One per line. Empty = your home folder.")
        },
        |ui| {
            let mut text = se
                .index_roots
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let edit = egui::TextEdit::multiline(&mut text)
                .desired_rows(3)
                .desired_width(280.0)
                .hint_text(if cfg!(windows) {
                    tr("C:\\\nD:\\Projects")
                } else {
                    "/home/me"
                });
            if ui.add(edit).changed() {
                se.index_roots = text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(Into::into)
                    .collect();
            }
        },
    );
    row(
        ui,
        tr("Skip folders named"),
        tr("Comma-separated, e.g. node_modules, .git"),
        |ui| {
            let mut text = se.index_exclude.join(", ");
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(280.0))
                .changed()
            {
                se.index_exclude = text
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
            }
        },
    );
    row(ui, tr("Refresh every"), "", |ui| {
        ui.add(
            egui::Slider::new(&mut se.index_refresh_minutes, 0..=240).custom_formatter(|v, _| {
                if v == 0.0 {
                    tr("Only at start").into()
                } else {
                    format!("{v:.0} min")
                }
            }),
        );
    });
}

fn shortcuts(ui: &mut Ui, app: &mut OxApp) {
    use crate::shortcuts::{self, COMMANDS};
    use egui::{Key, KeyboardShortcut, Modifiers};

    // While recording, the next key press (with its modifiers) is the new shortcut.
    if let Some(cmd) = app.ui.recording {
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !shortcuts::is_modifier(*key) => Some((*key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, m)) = pressed {
            if key == Key::Escape && m.is_none() {
                app.ui.recording = None;
            } else {
                let mut mods = Modifiers::NONE;
                if m.ctrl || m.command {
                    mods |= Modifiers::COMMAND;
                }
                if m.alt {
                    mods |= Modifiers::ALT;
                }
                if m.shift {
                    mods |= Modifiers::SHIFT;
                }
                let id = shortcuts::command_info(cmd).id.to_owned();
                let text = shortcuts::to_text(&KeyboardShortcut::new(mods, key));
                app.settings.shortcuts.insert(id, text);
                app.ui.recording = None;
            }
        }
    }

    ui.label(RichText::new(tr("Click a shortcut and press the new keys (Esc cancels).")).weak());
    ui.add_space(8.0);
    egui::Grid::new("shortcuts")
        .striped(true)
        .num_columns(3)
        .spacing(vec2(16.0, 6.0))
        .show(ui, |ui| {
            for c in COMMANDS {
                ui.label(tr(c.label));
                let keys = app.keymap.keys(c.cmd).to_vec();
                let recording = app.ui.recording == Some(c.cmd);
                let text = if recording {
                    tr("Press keys…").to_owned()
                } else if keys.is_empty() {
                    "—".to_owned()
                } else {
                    keys.iter()
                        .map(shortcuts::display)
                        .collect::<Vec<_>>()
                        .join(" / ")
                };
                ui.horizontal(|ui| {
                    let button = egui::Button::new(RichText::new(text).monospace())
                        .selected(recording)
                        .min_size(vec2(170.0, 0.0));
                    if ui.add(button).clicked() {
                        app.ui.recording = if recording { None } else { Some(c.cmd) };
                    }
                    let changed = app.settings.shortcuts.contains_key(c.id);
                    if !keys.is_empty()
                        && ui
                            .small_button("✖")
                            .on_hover_text(tr("No shortcut"))
                            .clicked()
                    {
                        app.settings
                            .shortcuts
                            .insert(c.id.to_owned(), String::new());
                    }
                    if changed
                        && ui
                            .small_button("↺")
                            .on_hover_text(ox_core::trf!(
                                "Back to {v1}",
                                v1 = c.defaults.join(" / ")
                            ))
                            .clicked()
                    {
                        app.settings.shortcuts.remove(c.id);
                    }
                });
                let clash = keys.iter().find_map(|k| app.keymap.conflict(c.cmd, k));
                match clash {
                    Some(other) => ui.colored_label(
                        Color32::from_rgb(0xe5, 0x48, 0x4d),
                        format!(
                            "⚠ {} {}",
                            tr("also"),
                            tr(shortcuts::command_info(other).label)
                        ),
                    ),
                    None => ui.label(""),
                };
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    if !app.settings.shortcuts.is_empty() && ui.button(tr("Reset all shortcuts")).clicked() {
        app.settings.shortcuts.clear();
        app.ui.recording = None;
    }
    ui.add_space(12.0);
    ui.label(RichText::new(tr("Always available")).strong());
    egui::Grid::new("fixed_shortcuts")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            for (k, d) in [
                (tr("Enter"), tr("Open selection")),
                (
                    tr("Arrow keys, Home, End"),
                    tr("Move the selection (Shift to extend)"),
                ),
                (tr("Type letters"), tr("Jump to item")),
                (tr("Mouse side buttons"), tr("Back / forward")),
                (tr("Middle click"), tr("Open folder in new tab / close tab")),
            ] {
                ui.monospace(k);
                ui.label(d);
                ui.end_row();
            }
        });
}

fn about(ui: &mut Ui) {
    ui.label(
        RichText::new(format!(
            "{} {}",
            ox_core::APP_NAME,
            env!("CARGO_PKG_VERSION")
        ))
        .strong(),
    );
    ui.label(tr("A fast file explorer for Windows, written in Rust."));
    ui.add_space(6.0);
    ui.label(
        RichText::new(ox_core::trf!(
            "Config folder: {v1}",
            v1 = ox_core::config_dir().display()
        ))
        .weak(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_are_translated() {
        for s in SECTIONS
            .into_iter()
            .chain(QUERY_SYNTAX.map(|(_, what)| what))
        {
            assert!(ox_core::i18n::german(s).is_some(), "no German for {s:?}");
        }
    }
}
