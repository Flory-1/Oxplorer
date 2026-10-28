//! Command palette (Ctrl+Shift+P): run any command or jump to a place by typing.

use egui::{Context, Id, Key, RichText};
use ox_core::entry::display_name;
use ox_core::format;
use ox_core::settings::{Theme, ViewMode};
use ox_core::tags::TagColor;

use crate::app::{Action, OxApp, TabAction};
use crate::tabs::Location;

#[derive(Default)]
pub struct PaletteState {
    pub query: String,
    selected: usize,
    focused: bool,
}

type Run = Box<dyn FnOnce(&mut OxApp)>;

struct Entry {
    icon: &'static str,
    label: String,
    detail: String,
    run: Run,
}

fn act(a: impl FnOnce() -> Action + 'static) -> Run {
    Box::new(move |app: &mut OxApp| app.push(a()))
}

fn setting(f: impl FnOnce(&mut OxApp) + 'static) -> Run {
    Box::new(move |app: &mut OxApp| {
        f(app);
        app.push(Action::SettingsChanged);
    })
}

/// Everything the palette offers right now.
fn entries(app: &OxApp) -> Vec<Entry> {
    let mut v = Vec::new();
    let mut cmd = |icon, label: &str, detail: &str, run: Run| {
        v.push(Entry {
            icon,
            label: label.to_owned(),
            detail: detail.to_owned(),
            run,
        })
    };
    let tab = app.tabs.active();
    let tab_id = tab.id;
    let folder = app.current_folder();
    let selection = tab.view.selected_paths();

    cmd(
        "🏠",
        "Go to Home",
        "",
        act(|| Action::Navigate(Location::Home)),
    );
    cmd(
        "🗑",
        "Open Recycle Bin",
        "",
        act(|| Action::Navigate(Location::RecycleBin)),
    );
    cmd(
        "⚙",
        "Open Settings",
        "Ctrl+,",
        act(|| Action::OpenInNewTab(Location::Settings, true)),
    );
    cmd("⬅", "Back", "Alt+Left", act(|| Action::Back));
    cmd("➡", "Forward", "Alt+Right", act(|| Action::Forward));
    cmd("⬆", "Up one folder", "Alt+Up", act(|| Action::Up));
    cmd("🔄", "Refresh", "F5", act(|| Action::Refresh));
    cmd(
        "➕",
        "New tab",
        "Ctrl+T",
        act(|| Action::Tab(TabAction::New)),
    );
    cmd(
        "✖",
        "Close tab",
        "Ctrl+W",
        act(move || Action::Tab(TabAction::Close(tab_id))),
    );
    cmd(
        "📑",
        "Duplicate tab",
        "",
        act(move || Action::Tab(TabAction::Duplicate(tab_id))),
    );
    cmd(
        "🗂",
        "Add tab to new group",
        "",
        act(move || Action::Tab(TabAction::NewGroup(tab_id))),
    );
    cmd(
        "🔍",
        "Search…",
        "Ctrl+F",
        Box::new(|app: &mut OxApp| app.ui.focus_search = true),
    );

    if let Some(f) = folder.clone() {
        let (f1, f2, f3, f4, f5) = (f.clone(), f.clone(), f.clone(), f.clone(), f.clone());
        cmd(
            "📁",
            "New folder",
            "Ctrl+Shift+N",
            act(move || Action::NewFolder(f1)),
        );
        cmd(
            "📄",
            "New text document",
            "",
            act(move || Action::NewFile(f2)),
        );
        cmd("📥", "Paste", "Ctrl+V", act(move || Action::Paste(f3)));
        cmd(
            "🖳",
            "Open terminal here",
            "",
            act(move || Action::OpenTerminal(f4)),
        );
        cmd(
            "🗔",
            "Open in Windows Explorer",
            "",
            act(move || Action::Reveal(f5)),
        );
        if app.quick.is_favorite(&f) {
            cmd(
                "☆",
                "Unpin folder from Favorites",
                "",
                act(move || Action::Unpin(f)),
            );
        } else {
            cmd(
                "⭐",
                "Pin folder to Favorites",
                "",
                act(move || Action::Pin(f)),
            );
        }
    }
    if tab.location().shows_files() {
        cmd("☑", "Select all", "Ctrl+A", act(|| Action::SelectAll));
    }
    if !selection.is_empty() && !tab.location().is_bin() && !tab.location().is_read_only() {
        let (s1, s2, s3) = (selection.clone(), selection.clone(), selection.clone());
        let rename = if selection.len() == 1 {
            act(move || Action::Rename(s1[0].clone()))
        } else {
            act(move || Action::BulkRename(s1))
        };
        cmd("✏", "Rename selection", "F2", rename);
        cmd(
            "🔗",
            "Copy path of selection",
            "Ctrl+Shift+C",
            act(move || Action::CopyPathText(s2)),
        );
        cmd(
            "ℹ",
            "Properties of selection",
            "Alt+Enter",
            act(move || Action::Properties(s3)),
        );
        let zip = selection.clone();
        cmd(
            "🗜",
            "Compress selection to ZIP",
            "",
            act(move || Action::Compress(zip)),
        );
        if selection.len() == 1 && ox_core::archive::is_archive(&selection[0]) {
            let (a1, a2) = (selection[0].clone(), selection[0].clone());
            cmd(
                "📦",
                &format!(
                    "Extract to \"{}\"",
                    ox_core::archive::base_name(&selection[0])
                ),
                "",
                act(move || Action::ExtractArchive {
                    archive: a1,
                    own_folder: true,
                }),
            );
            cmd(
                "📦",
                "Extract here",
                "",
                act(move || Action::ExtractArchive {
                    archive: a2,
                    own_folder: false,
                }),
            );
        }
        for color in TagColor::ALL {
            let sel = selection.clone();
            cmd(
                "🏷",
                &format!("Tag selection: {}", app.tags.label(color)),
                "",
                act(move || Action::ToggleTag(sel, color)),
            );
        }
    }
    let counts = app.tags.counts();
    for color in TagColor::ALL.into_iter().filter(|c| counts[c.index()] > 0) {
        cmd(
            "🏷",
            &format!("Show items tagged {}", app.tags.label(color)),
            &format::count(counts[color.index()] as u64, "item"),
            act(move || Action::Navigate(Location::Tagged(color))),
        );
    }

    cmd(
        "◫",
        "Toggle split view",
        "Ctrl+\\",
        act(|| Action::Tab(TabAction::ToggleSplit)),
    );
    if app.tabs.panes.is_some() {
        cmd(
            "⇆",
            "Switch pane",
            "F6",
            act(|| Action::Tab(TabAction::SwitchPane)),
        );
    }
    cmd(
        "👁",
        "Toggle preview pane",
        "Alt+P",
        setting(|a| a.settings.browsing.show_preview ^= true),
    );
    cmd(
        "◧",
        "Toggle sidebar",
        "",
        setting(|a| a.settings.sidebar.visible ^= true),
    );
    cmd(
        "👻",
        "Toggle hidden items",
        "",
        setting(|a| a.settings.browsing.show_hidden ^= true),
    );
    cmd(
        "🔤",
        "Toggle file name extensions",
        "",
        setting(|a| a.settings.browsing.show_extensions ^= true),
    );
    cmd(
        "▬",
        "Toggle status bar",
        "",
        setting(|a| a.settings.browsing.show_status_bar ^= true),
    );
    cmd(
        "☰",
        "Details view",
        "",
        Box::new(|a: &mut OxApp| a.tabs.active_mut().view_mode = ViewMode::Details),
    );
    cmd(
        "▦",
        "Grid view",
        "",
        Box::new(|a: &mut OxApp| a.tabs.active_mut().view_mode = ViewMode::Grid),
    );
    for (theme, name) in [
        (Theme::Dark, "dark"),
        (Theme::Light, "light"),
        (Theme::System, "system"),
    ] {
        cmd(
            "🎨",
            &format!("Theme: {name}"),
            "",
            setting(move |a| a.settings.appearance.theme = theme),
        );
    }
    cmd(
        "⚡",
        "Rebuild search index",
        "",
        act(|| Action::RebuildIndex),
    );
    cmd(
        "⇄",
        "Show transfers",
        "",
        Box::new(|a: &mut OxApp| a.ui.show_transfers = true),
    );
    cmd(
        "🧹",
        "Clear finished transfers",
        "",
        Box::new(|a: &mut OxApp| a.engine.clear_finished()),
    );
    cmd("🗑", "Empty Recycle Bin", "", act(|| Action::EmptyBin));
    cmd(
        "🕘",
        "Clear recent history",
        "",
        act(|| Action::ClearRecent),
    );

    // Places
    let mut place = |icon, label: String, path: std::path::PathBuf| {
        let detail = path.display().to_string();
        v.push(Entry {
            icon,
            label,
            detail,
            run: act(move || Action::Navigate(Location::Folder(path))),
        });
    };
    for f in &app.quick.favorites {
        place("⭐", format!("Go to {}", f.label()), f.path.clone());
    }
    for d in &app.drives {
        place("🖴", format!("Go to {}", d.title()), d.mount.clone());
    }
    for r in app.quick.recent_folders(10) {
        place(
            "🕘",
            format!("Go to {}", display_name(&r.path)),
            r.path.clone(),
        );
    }
    for s in &app.quick.saved_searches {
        let loc = Location::Search {
            root: s.root.clone(),
            query: s.query.clone(),
        };
        v.push(Entry {
            icon: "🔍",
            label: format!("Search: {}", s.name),
            detail: "saved search".into(),
            run: act(move || Action::Navigate(loc)),
        });
    }
    v
}

pub fn show(app: &mut OxApp, ctx: &Context) {
    let Some(mut state) = app.ui.palette.take() else {
        return;
    };
    let all = entries(app);
    // Best matches first; ties keep the natural order.
    let mut ranked: Vec<(i32, usize)> = all
        .iter()
        .enumerate()
        .filter_map(|(i, e)| ox_core::fuzzy::score(&state.query, &e.label).map(|s| (s, i)))
        .collect();
    if !state.query.trim().is_empty() {
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    let visible: Vec<usize> = ranked.into_iter().map(|(_, i)| i).take(12).collect();

    let (up, down, enter) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, Key::Enter),
        )
    });
    if down {
        state.selected = (state.selected + 1).min(visible.len().saturating_sub(1));
    }
    if up {
        state.selected = state.selected.saturating_sub(1);
    }
    state.selected = state.selected.min(visible.len().saturating_sub(1));

    let mut chosen = enter
        .then(|| visible.get(state.selected).copied())
        .flatten();
    let mut close = false;
    let modal = egui::Modal::new(Id::unique("command_palette")).show(ctx, |ui| {
        ui.set_width(560.0);
        let edit = egui::TextEdit::singleline(&mut state.query)
            .hint_text("Type a command or a place…")
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Heading);
        let resp = ui.add(edit);
        if !state.focused {
            resp.request_focus();
            state.focused = true;
        }
        if resp.changed() {
            state.selected = 0;
        }
        ui.add_space(6.0);
        if visible.is_empty() {
            ui.label(RichText::new("Nothing found.").weak());
        }
        for (row, &i) in visible.iter().enumerate() {
            let e = &all[i];
            let selected = row == state.selected;
            let resp = ui
                .horizontal(|ui| {
                    let label = format!("{}  {}", e.icon, e.label);
                    let r = ui.add_sized(
                        [ui.available_width() - 150.0, 26.0],
                        egui::Button::selectable(selected, label),
                    );
                    ui.label(RichText::new(&e.detail).weak().small());
                    r
                })
                .inner;
            if resp.clicked() {
                chosen = Some(i);
            }
            if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                state.selected = row;
            }
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new("↑↓ to choose · Enter to run · Esc to close")
                .weak()
                .small(),
        );
    });
    if modal.should_close() {
        close = true;
    }

    if let Some(i) = chosen {
        let run = all.into_iter().nth(i).map(|e| e.run);
        if let Some(run) = run {
            run(app);
        }
        close = true;
    }
    if !close {
        app.ui.palette = Some(state);
    }
}
