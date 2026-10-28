//! The settings page (opens as a tab).

use egui::{vec2, Color32, RichText, Ui};
use ox_core::entry::{SortKey, SortOrder};
use ox_core::index::IndexState;
use ox_core::settings::{Settings, StartPage, Theme, ViewMode};
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

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let before = app.settings.clone();
    // Section list on the left, content on the right.
    egui::Panel::left("settings_nav")
        .resizable(false)
        .exact_size(210.0)
        .frame(egui::Frame::new().inner_margin(egui::Margin::same(14)))
        .show(ui, |ui| {
            ui.heading(RichText::new("Settings").strong());
            ui.add_space(10.0);
            for (i, name) in SECTIONS.iter().enumerate() {
                let selected = app.ui.settings_section == i;
                let button = egui::Button::new(RichText::new(*name).size(14.0))
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
                    let title = SECTIONS[section];
                    let title = &title[title.find(' ').map_or(0, |i| i + 1)..];
                    ui.heading(RichText::new(title).size(20.0).strong());
                    ui.add_space(12.0);
                    match section {
                        0 => {
                            appearance(ui, app);
                            theme_editor(ui, app);
                        }
                        1 => browsing(ui, &mut app.settings),
                        2 => startup(ui, &mut app.settings),
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
                        if section < 6 && ui.button("Reset this page to defaults").clicked() {
                            reset_section(&mut app.settings, section);
                        }
                        ui.label(
                            RichText::new(format!(
                                "Saved automatically to {}",
                                Settings::path().display()
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
    ui.label(RichText::new("Theme colours").size(16.0).strong());
    let dark = ui.visuals().dark_mode;
    let a = &app.settings.appearance;
    let name = if dark { &a.dark_theme } else { &a.light_theme };
    let mut t = crate::theme::scheme(&app.themes, name, dark).clone();
    ui.label(
        RichText::new(format!(
            "Editing \"{}\" ({} mode). Built-in themes are copied when you change them.",
            t.name,
            if dark { "dark" } else { "light" }
        ))
        .weak(),
    );
    let mut changed = false;
    for (label, color) in [
        ("Panels", &mut t.panel),
        ("Menus and dialogs", &mut t.window),
        ("File list and fields", &mut t.background),
        ("Stripes", &mut t.faint),
    ] {
        row(ui, label, "", |ui| {
            changed |= ui.color_edit_button_srgb(&mut color.0).changed();
        });
    }
    row(ui, "Text", "Buttons and lines are mixed from it.", |ui| {
        let mut own = t.text.is_some();
        if ui.checkbox(&mut own, "Own colour").changed() {
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
    });
    if changed {
        if t.builtin {
            t.builtin = false;
            t.name = themes::free_name(&format!("{} (custom)", t.name), &app.themes);
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
            Err(e) => app.toast(format!("Could not save the theme: {e}")),
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .button("📤 Export")
            .on_hover_text("Save this theme as a file you can share")
            .clicked()
        {
            let mut export = t.clone();
            export.accent = Some(themes::Rgb(app.settings.appearance.accent));
            match themes::save(&export) {
                Ok(path) => {
                    app.toast(format!("Saved as {}", path.display()));
                    app.push(Action::OpenInNewTab(
                        crate::tabs::Location::Folder(themes::dir()),
                        true,
                    ));
                    app.themes = themes::load_all();
                }
                Err(e) => app.toast(format!("Could not save the theme: {e}")),
            }
        }
        if ui.button("📂 Themes folder").clicked() {
            let _ = std::fs::create_dir_all(themes::dir());
            app.push(Action::OpenInNewTab(
                crate::tabs::Location::Folder(themes::dir()),
                true,
            ));
        }
        if !t.builtin && ui.button("🗑 Delete theme").clicked() {
            if let Err(e) = themes::remove(&t.name) {
                app.toast(format!("Could not delete the theme: {e}"));
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
        RichText::new(
            "To use a theme someone sent you, right-click its .toml file → Use as theme.",
        )
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
    row(ui, "Mode", "Follow Windows, or force light/dark.", |ui| {
        ui.selectable_value(&mut a.theme, Theme::System, "System");
        ui.selectable_value(&mut a.theme, Theme::Light, "☀ Light");
        ui.selectable_value(&mut a.theme, Theme::Dark, "🌙 Dark");
    });
    for (label, help, dark) in [
        ("Dark theme", "Colours in dark mode.", true),
        ("Light theme", "Colours in light mode.", false),
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
        "Accent colour",
        "Used for selection, focus and progress.",
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
    row(ui, "UI scale", "Zoom the whole interface.", |ui| {
        ui.add(
            egui::Slider::new(&mut a.ui_scale, 0.75..=2.0)
                .step_by(0.05)
                .custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
        );
    });
    toggle(
        ui,
        &mut a.compact_rows,
        "Compact rows",
        "Smaller row height in the details view.",
    );
    if ox_core::shell::supported() {
        toggle(
            ui,
            &mut a.system_icons,
            "Windows file icons",
            "Show the same icons as Windows Explorer instead of the built-in ones.",
        );
    }
}

fn browsing(ui: &mut Ui, s: &mut Settings) {
    let b = &mut s.browsing;
    toggle(ui, &mut b.show_hidden, "Show hidden items", "");
    toggle(ui, &mut b.show_extensions, "Show file name extensions", "");
    toggle(
        ui,
        &mut b.folders_first,
        "Folders first",
        "Sort folders before files.",
    );
    row(ui, "Default view", "", |ui| {
        ui.selectable_value(&mut b.default_view, ViewMode::Details, "☰ Details");
        ui.selectable_value(&mut b.default_view, ViewMode::Grid, "▦ Grid");
    });
    row(ui, "Default sort", "", |ui| {
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
        "Single-click to open",
        "Open items with one click instead of a double click.",
    );
    toggle(ui, &mut b.confirm_delete, "Confirm before deleting", "");
    toggle(
        ui,
        &mut b.delete_to_trash,
        "Delete to Recycle Bin",
        "Shift+Delete always deletes permanently.",
    );
    toggle(
        ui,
        &mut b.auto_refresh,
        "Refresh automatically",
        "Update the view when files change on disk.",
    );
    toggle(
        ui,
        &mut b.folder_sizes,
        "Show folder sizes",
        "Calculate the size of folders in the background for the Size column.",
    );
    toggle(
        ui,
        &mut b.open_archives,
        "Open archives like folders",
        "Browse zip, 7z and tar files in Oxplorer instead of opening them with another app.",
    );
    toggle(
        ui,
        &mut b.git_status,
        "Git status",
        "In Git repositories, show a Git column and the branch (needs Git installed).",
    );
    toggle(ui, &mut b.show_status_bar, "Show status bar", "");
    row(
        ui,
        "Date format",
        "chrono format, e.g. %d.%m.%Y %H:%M or %Y-%m-%d",
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
    ui.label(RichText::new("Terminal panel").size(16.0).strong());
    let t = &mut s.terminal;
    let default_shell = if cfg!(windows) {
        "powershell.exe -NoLogo"
    } else {
        "$SHELL"
    };
    row(
        ui,
        "Shell",
        "Used the next time the terminal starts. Empty uses the default.",
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
        "Follow the current folder",
        "Change the terminal's folder whenever you open another folder.",
    );
}

fn startup(ui: &mut Ui, s: &mut Settings) {
    let st = &mut s.startup;
    row(ui, "When Oxplorer starts", "", |ui| {
        ui.vertical(|ui| {
            ui.radio_value(&mut st.start_page, StartPage::Home, "Open the Home page");
            ui.radio_value(
                &mut st.start_page,
                StartPage::RestoreSession,
                "Restore tabs and groups from last time",
            );
            ui.radio_value(
                &mut st.start_page,
                StartPage::Folder,
                "Open a specific folder",
            );
        });
    });
    if st.start_page == StartPage::Folder {
        row(ui, "Start folder", "", |ui| {
            let mut text = st
                .start_folder
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(280.0)
                        .hint_text("C:\\Users\\…"),
                )
                .changed()
            {
                st.start_folder = (!text.is_empty()).then(|| text.into());
            }
        });
    }
}

fn sidebar(ui: &mut Ui, s: &mut Settings) {
    let sb = &mut s.sidebar;
    toggle(ui, &mut sb.visible, "Show sidebar", "");
    toggle(ui, &mut sb.show_favorites, "Favorites section", "");
    toggle(ui, &mut sb.show_drives, "Drives section", "");
    toggle(ui, &mut sb.show_recent, "Recent folders section", "");
    row(ui, "Recent folders shown", "", |ui| {
        ui.add(egui::Slider::new(&mut sb.recent_count, 1..=20));
    });
}

fn transfer(ui: &mut Ui, s: &mut Settings) {
    let t = &mut s.transfer;
    ui.label(
        RichText::new(
            "Oxplorer copies files with its own multi-threaded engine instead of the Windows copy dialog. \
             On Windows each file is copied with CopyFileExW (kernel fast path), large files bypass the cache.",
        )
        .weak(),
    );
    ui.add_space(10.0);
    row(
        ui,
        "Parallel file copies",
        "Auto: one file at a time on HDDs, 8 on SSDs. More helps with many small files on SSDs.",
        |ui| {
            ui.add(
                egui::Slider::new(&mut t.workers, 0..=32).custom_formatter(|v, _| {
                    if v == 0.0 {
                        "Auto".into()
                    } else {
                        format!("{v:.0}")
                    }
                }),
            );
        },
    );
    row(
        ui,
        "Parallel jobs",
        "How many copy jobs run at once; the rest wait in a queue.",
        |ui| {
            ui.add(egui::Slider::new(&mut t.max_concurrent_jobs, 1..=8));
        },
    );
    toggle(
        ui,
        &mut t.per_disk_queue,
        "One job per drive at a time",
        "Copies on the same drive wait for each other; different drives still run in parallel.",
    );
    toggle(
        ui,
        &mut t.verify,
        "Verify copied files",
        "Compare every copy with its original (xxh3). Slower, but moves only delete originals that were copied correctly.",
    );
    row(
        ui,
        "Speed limit",
        "Per transfer. 0 = unlimited. Useful to keep the PC responsive during big copies.",
        |ui| {
            ui.add(
                egui::DragValue::new(&mut t.speed_limit_mib)
                    .range(0..=10_000)
                    .custom_formatter(|v, _| {
                        if v == 0.0 {
                            "Unlimited".into()
                        } else {
                            format!("{v:.0} MiB/s")
                        }
                    }),
            );
        },
    );
    row(ui, "Buffer size", "Used by the portable copy loop.", |ui| {
        ui.add(
            egui::Slider::new(&mut t.buffer_kib, 256..=65536)
                .logarithmic(true)
                .suffix(" KiB"),
        );
    });
    row(
        ui,
        "Unbuffered I/O from",
        "Files at least this big skip the Windows file cache.",
        |ui| {
            ui.add(
                egui::Slider::new(&mut t.unbuffered_threshold_mib, 16..=8192)
                    .logarithmic(true)
                    .suffix(" MiB"),
            );
        },
    );
    row(
        ui,
        "When a file already exists",
        "\"Ask me\" asks once per job, not once per file.",
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
        "Keep modification dates",
        "",
    );
    toggle(
        ui,
        &mut t.use_system_clipboard,
        "Share clipboard with Windows Explorer",
        "Copy in Explorer, paste in Oxplorer (and the other way around).",
    );
    toggle(
        ui,
        &mut t.drag_moves_on_same_drive,
        "Dragging moves on the same drive",
        "Like Explorer. Hold Ctrl to copy, Shift to move.",
    );
    row(
        ui,
        "Transfer history",
        "Every finished copy and move, with errors.",
        |ui| {
            let log = ox_core::transfer::history::log_path();
            if ui
                .add_enabled(log.exists(), egui::Button::new("📜 Open log"))
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
        "Filter while typing",
        "The search box filters the current folder instantly; Enter searches sub-folders.",
    );
    toggle(ui, &mut se.include_hidden, "Include hidden files", "");
    toggle(ui, &mut se.case_sensitive, "Case sensitive", "");
    row(ui, "Maximum results", "", |ui| {
        ui.add(
            egui::DragValue::new(&mut se.max_results)
                .range(100..=1_000_000)
                .speed(100),
        );
    });
    row(ui, "Search threads", "0 = one per CPU core.", |ui| {
        ui.add(egui::Slider::new(&mut se.threads, 0..=64));
    });
    ui.add_space(8.0);
    ui.label(RichText::new("Query syntax").strong());
    egui::Grid::new("syntax")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            for (q, d) in [
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
            ] {
                ui.monospace(q);
                ui.label(d);
                ui.end_row();
            }
        });
}

fn index_section(ui: &mut Ui, app: &mut OxApp) {
    ui.add_space(14.0);
    ui.label(RichText::new("Search index").size(16.0).strong());
    ui.label(
        RichText::new(
            "Keeps the names of all files in memory (about 50 MB per million files) so searching \
             whole drives takes milliseconds. Built in the background and saved between runs.",
        )
        .weak(),
    );
    ui.add_space(8.0);
    let se = &mut app.settings.search;
    toggle(ui, &mut se.index_enabled, "Index file names", "");
    if !se.index_enabled {
        return;
    }
    toggle(
        ui,
        &mut se.search_as_you_type,
        "Results while typing",
        "On the Home page and in search tabs, results update with every key.",
    );

    let index = app.index.clone();
    row(ui, "Status", "", |ui| {
        ui.vertical(|ui| {
            match index.state() {
                IndexState::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading the saved index…");
                    });
                }
                IndexState::Building => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Indexing… {} items so far", index.progress()));
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
                let mut text = format!(
                    "✔ {} items · {} in memory · updated {} ago",
                    idx.len(),
                    ox_core::format::size(idx.memory_bytes() as u64),
                    ox_core::format::duration(age)
                );
                if let Some((_, took)) = index.last_build() {
                    text.push_str(&format!(" · built in {:.1}s", took.as_secs_f32()));
                }
                ui.label(text);
            } else if index.state() == IndexState::Empty {
                ui.label("Not built yet.");
            }
            ui.horizontal(|ui| {
                if index.is_building() {
                    if ui.button("⏹ Stop").clicked() {
                        index.cancel();
                    }
                } else if ui.button("🔄 Rebuild now").clicked() {
                    app.actions.push(Action::RebuildIndex);
                }
            });
        });
    });

    let se = &mut app.settings.search;
    row(
        ui,
        "Indexed folders",
        if cfg!(windows) {
            "One per line. Empty = all drives."
        } else {
            "One per line. Empty = your home folder."
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
                    "C:\\\nD:\\Projects"
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
        "Skip folders named",
        "Comma-separated, e.g. node_modules, .git",
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
    row(ui, "Refresh every", "", |ui| {
        ui.add(
            egui::Slider::new(&mut se.index_refresh_minutes, 0..=240).custom_formatter(|v, _| {
                if v == 0.0 {
                    "Only at start".into()
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

    ui.label(RichText::new("Click a shortcut and press the new keys (Esc cancels).").weak());
    ui.add_space(8.0);
    egui::Grid::new("shortcuts")
        .striped(true)
        .num_columns(3)
        .spacing(vec2(16.0, 6.0))
        .show(ui, |ui| {
            for c in COMMANDS {
                ui.label(c.label);
                let keys = app.keymap.keys(c.cmd).to_vec();
                let recording = app.ui.recording == Some(c.cmd);
                let text = if recording {
                    "Press keys…".to_owned()
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
                        && ui.small_button("✖").on_hover_text("No shortcut").clicked()
                    {
                        app.settings
                            .shortcuts
                            .insert(c.id.to_owned(), String::new());
                    }
                    if changed
                        && ui
                            .small_button("↺")
                            .on_hover_text(format!("Back to {}", c.defaults.join(" / ")))
                            .clicked()
                    {
                        app.settings.shortcuts.remove(c.id);
                    }
                });
                let clash = keys.iter().find_map(|k| app.keymap.conflict(c.cmd, k));
                match clash {
                    Some(other) => ui.colored_label(
                        Color32::from_rgb(0xe5, 0x48, 0x4d),
                        format!("⚠ also {}", shortcuts::command_info(other).label),
                    ),
                    None => ui.label(""),
                };
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    if !app.settings.shortcuts.is_empty() && ui.button("Reset all shortcuts").clicked() {
        app.settings.shortcuts.clear();
        app.ui.recording = None;
    }
    ui.add_space(12.0);
    ui.label(RichText::new("Always available").strong());
    egui::Grid::new("fixed_shortcuts")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            for (k, d) in [
                ("Enter", "Open selection"),
                (
                    "Arrow keys, Home, End",
                    "Move the selection (Shift to extend)",
                ),
                ("Type letters", "Jump to item"),
                ("Mouse side buttons", "Back / forward"),
                ("Middle click", "Open folder in new tab / close tab"),
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
    ui.label("A fast file explorer for Windows, written in Rust.");
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!(
            "Config folder: {}",
            ox_core::config_dir().display()
        ))
        .weak(),
    );
}
