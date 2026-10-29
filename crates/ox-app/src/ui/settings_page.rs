//! The settings page (opens as a tab).

use egui::{vec2, Color32, RichText, Ui};
use ox_core::entry::{SortKey, SortOrder};
use ox_core::i18n::t as tr;
use ox_core::settings::{Backdrop, Settings, StartPage, Theme, ViewMode};

mod commands;
mod keys;
mod search;
mod theme;
mod transfer;
mod views;

use keys::shortcuts;
use search::{history_section, index_section, search};
use theme::{swatch, theme_editor};
use transfer::transfer;

use crate::app::{Action, OxApp};
use crate::shortcuts::{self, Command};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Appearance,
    Browsing,
    Startup,
    Sidebar,
    Preview,
    NewMenu,
    Developer,
    Commands,
    Transfer,
    Search,
    Shortcuts,
    About,
}

/// The pages in the order of the list on the left, with their names.
const SECTIONS: [(Section, &str); 12] = [
    (Section::Appearance, "🎨 Appearance"),
    (Section::Browsing, "📂 Browsing"),
    (Section::Startup, "🚀 Startup"),
    (Section::Sidebar, "◧ Sidebar"),
    (Section::Preview, "◨ Preview pane"),
    (Section::NewMenu, "➕ New menu"),
    (Section::Developer, "🛠 Developer"),
    (Section::Commands, "⚡ Custom commands"),
    (Section::Transfer, "⇄ Copy engine"),
    (Section::Search, "🔍 Search"),
    (Section::Shortcuts, "⌨ Shortcuts"),
    (Section::About, "ℹ About"),
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
            for (i, (_, name)) in SECTIONS.iter().enumerate() {
                let selected = app.ui.settings_section == i;
                if super::nav_item(ui, tr(name), selected).clicked() {
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
                    let (section, name) = SECTIONS[app.ui.settings_section.min(SECTIONS.len() - 1)];
                    let title = tr(name);
                    let title = &title[title.find(' ').map_or(0, |i| i + 1)..];
                    ui.heading(RichText::new(title).size(20.0).strong());
                    ui.add_space(12.0);
                    match section {
                        Section::Appearance => {
                            appearance(ui, app);
                            theme_editor(ui, app);
                        }
                        Section::Browsing => {
                            browsing(ui, &mut app.settings);
                            views::folder_views(ui, app);
                        }
                        Section::Startup => {
                            startup(ui, &mut app.settings);
                            default_folder_handler(ui, app);
                        }
                        Section::Sidebar => sidebar(ui, &mut app.settings),
                        Section::Preview => preview(ui, &mut app.settings),
                        Section::NewMenu => new_menu(ui, &mut app.settings),
                        Section::Developer => developer(ui, &mut app.settings),
                        Section::Commands => commands::commands(ui, app),
                        Section::Transfer => transfer(ui, &mut app.settings),
                        Section::Search => {
                            search(ui, &mut app.settings);
                            history_section(ui, app);
                            index_section(ui, app);
                        }
                        Section::Shortcuts => shortcuts(ui, app),
                        Section::About => about(ui),
                    }
                    ui.add_space(24.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        let resettable = !matches!(
                            section,
                            Section::Shortcuts | Section::Commands | Section::About
                        );
                        if resettable && ui.button(tr("Reset this page to defaults")).clicked() {
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

fn reset_section(s: &mut Settings, section: Section) {
    let d = Settings::default();
    match section {
        Section::Appearance => s.appearance = d.appearance,
        Section::Browsing => {
            s.browsing = d.browsing;
            s.terminal = d.terminal;
        }
        Section::Startup => s.startup = d.startup,
        Section::Sidebar => s.sidebar = d.sidebar,
        Section::Preview => {
            s.preview = d.preview;
            s.browsing.show_preview = d.browsing.show_preview;
        }
        Section::NewMenu => s.new_items = d.new_items,
        Section::Developer => s.developer = d.developer,
        Section::Transfer => s.transfer = d.transfer,
        Section::Search => s.search = d.search,
        Section::Shortcuts | Section::Commands | Section::About => {}
    }
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
    }
    if ox_core::titlebar::supported() {
        toggle(
            ui,
            &mut a.tabs_in_title_bar,
            tr("Tabs in the title bar"),
            tr("Like Windows 11: the tabs take the place of the title bar, with the minimize, maximize and close buttons next to them. Drag the free space beside the tabs to move the window. Off: Windows' title bar above the tabs."),
        );
    }
    if ox_core::shell::supported() {
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
            .selected_text(tr(b.default_sort.label()))
            .show_ui(ui, |ui| {
                for k in SortKey::ALL {
                    ui.selectable_value(&mut b.default_sort, k, tr(k.label()));
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
    toggle(
        ui,
        &mut st.check_updates,
        tr("Check for updates"),
        tr("Once a day, ask GitHub for the newest version of Oxplorer and say so on the Home page when there's a newer one."),
    );
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
    toggle(
        ui,
        &mut sb.show_libraries,
        tr("Libraries section"),
        tr("The Gallery and the Windows libraries (Documents, Music, Pictures, Videos and your own)"),
    );
    row(ui, tr("Recent folders shown"), "", |ui| {
        ui.add(egui::Slider::new(&mut sb.recent_count, 1..=20));
    });
}

fn preview(ui: &mut Ui, s: &mut Settings) {
    toggle(
        ui,
        &mut s.browsing.show_preview,
        tr("Show the preview pane"),
        &ox_core::trf!(
            "Also with {keys} or View → Preview pane.",
            keys = shortcuts::label(Command::TogglePreview)
        ),
    );
    let p = &mut s.preview;
    toggle(
        ui,
        &mut p.as_window,
        tr("Open as a window"),
        tr("Show the preview in its own window that you can move and resize, instead of a pane on the right."),
    );
    if ox_core::player::supported() {
        let mut keep = !p.stop_player_on_select;
        toggle(
            ui,
            &mut keep,
            tr("Keep playing while browsing"),
            tr("Audio and video go on playing in the mini player when you select another item or go to another folder. Off: selecting another item stops the player."),
        );
        p.stop_player_on_select = !keep;
    }
    if ox_core::preview_handler::supported() {
        toggle(
            ui,
            &mut p.documents,
            tr("Office documents and mails"),
            tr("Show Word, Excel, PowerPoint, Outlook and other files with the preview handlers Windows uses in Explorer (the Document tab)."),
        );
    }
    toggle(
        ui,
        &mut p.reencode_rotated_jpegs,
        tr("Re-encode JPEGs instead (for programs that ignore the orientation)"),
        tr("Rotating and flipping a JPEG only changes its EXIF orientation: lossless, and every viewer that honours the tag shows it turned. On: the picture is turned and saved again at quality 95."),
    );
}

/// Editors offered on the Developer page: name and command line.
const EDITORS: [(&str, &str); 7] = [
    ("Visual Studio Code", "code \"{path}\""),
    ("VS Code Insiders", "code-insiders \"{path}\""),
    ("Cursor", "cursor \"{path}\""),
    ("Zed", "zed \"{path}\""),
    ("Sublime Text", "subl \"{path}\""),
    ("Notepad++", "notepad++ \"{path}\""),
    ("Notepad", "notepad \"{path}\""),
];

fn developer(ui: &mut Ui, s: &mut Settings) {
    let d = &mut s.developer;
    row(
        ui,
        tr("Editor"),
        &ox_core::trf!(
            "Opens files and folders with “Open in editor” ({keys}). {path} in the command is replaced with the file or folder.",
            keys = shortcuts::label(Command::OpenInEditor),
            path = "{path}"
        ),
        |ui| {
            ui.vertical(|ui| {
                let current = EDITORS
                    .iter()
                    .find(|(_, cmd)| *cmd == d.editor)
                    .map_or(tr("Custom"), |(name, _)| name);
                egui::ComboBox::from_id_salt("editor")
                    .selected_text(current)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for (name, cmd) in EDITORS {
                            ui.selectable_value(&mut d.editor, cmd.to_owned(), name);
                        }
                    });
                ui.add(
                    egui::TextEdit::singleline(&mut d.editor)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(220.0),
                );
            });
        },
    );
    toggle(
        ui,
        &mut d.context_menus,
        tr("Developer entries in menus"),
        tr("“Open in editor”, “Copy path as” (forward slashes, quoted, relative to the Git repository, file:// URL, name) and, in Git repositories, “Git” (open on GitHub or GitLab, copy the branch name) in the right-click menus."),
    );
    ui.add_space(4.0);
    ui.label(RichText::new(tr("Also for developers")).strong());
    ui.add_space(2.0);
    for line in [
        tr("• Preview pane → Git: the commit graph of the repository a folder is in."),
        tr("• Preview pane → Info: encoding, line endings and indentation of text files, the last commit that changed a file, and SHA-256 and MD5 checksums."),
        tr("• Preview pane → Markdown: Markdown files formatted."),
        tr("• Settings → Browsing → Git status: the Git column and the branch in the status bar."),
    ] {
        ui.label(RichText::new(line).weak());
    }
}

/// The user's entries of the "New" menus.
fn new_menu(ui: &mut Ui, s: &mut Settings) {
    ui.label(
        RichText::new(tr(
            "Extra entries of ➕ New (toolbar and right-click menu) and the command palette, after Folder and Text document. In the content, {name} becomes the file name without extension and {date} today's date.",
        ))
        .weak(),
    );
    ui.add_space(8.0);
    let items = &mut s.new_items.0;
    let mut remove = None;
    let mut swap = None;
    let count = items.len();
    for (i, item) in items.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new(("new_item", i))
                .num_columns(2)
                .spacing(vec2(12.0, 6.0))
                .show(ui, |ui| {
                    ui.label(tr("Name in the menu"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut item.label).desired_width(220.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("🗑").on_hover_text(tr("Remove")).clicked() {
                                remove = Some(i);
                            }
                            if ui
                                .add_enabled(i + 1 < count, egui::Button::new("⬇").small())
                                .on_hover_text(tr("Move down"))
                                .clicked()
                            {
                                swap = Some((i, i + 1));
                            }
                            if ui
                                .add_enabled(i > 0, egui::Button::new("⬆").small())
                                .on_hover_text(tr("Move up"))
                                .clicked()
                            {
                                swap = Some((i - 1, i));
                            }
                        });
                    });
                    ui.end_row();
                    ui.label(tr("File name"));
                    ui.add(
                        egui::TextEdit::singleline(&mut item.file_name)
                            .hint_text("script.py")
                            .desired_width(220.0),
                    );
                    ui.end_row();
                    ui.label(tr("Content"));
                    ui.add(
                        egui::TextEdit::multiline(&mut item.content)
                            .code_editor()
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                });
        });
        ui.add_space(4.0);
    }
    if let Some(i) = remove {
        items.remove(i);
    }
    if let Some((a, b)) = swap {
        items.swap(a, b);
    }
    if ui.button(tr("➕ Add entry")).clicked() {
        items.push(ox_core::settings::NewItem {
            label: tr("New entry").to_owned(),
            file_name: "file.txt".to_owned(),
            content: String::new(),
        });
    }
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
        for s in SECTIONS.map(|(_, name)| name) {
            assert!(ox_core::i18n::german(s).is_some(), "no German for {s:?}");
        }
    }
}
