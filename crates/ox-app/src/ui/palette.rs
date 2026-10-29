//! Command palette (Ctrl+Shift+P): run any command or jump to a place by typing.

use egui::{Context, Id, Key, RichText};
use ox_core::entry::display_name;
use ox_core::i18n::t as tr;
use ox_core::settings::{Theme, ViewMode};
use ox_core::tags::TagColor;

use crate::app::{Action, OxApp, TabAction};
use crate::shortcuts::{self, Command};
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
        tr("Go to Home"),
        "",
        act(|| Action::Navigate(Location::Home)),
    );
    cmd(
        "🗑",
        tr("Open Recycle Bin"),
        "",
        act(|| Action::Navigate(Location::RecycleBin)),
    );
    cmd(
        "⚙",
        tr("Open Settings"),
        &shortcuts::label(Command::Settings),
        act(|| Action::OpenInNewTab(Location::Settings, true)),
    );
    cmd(
        "❓",
        tr("Open Help"),
        &shortcuts::label(Command::Help),
        act(|| Action::OpenInNewTab(Location::Help, true)),
    );
    cmd(
        "⬅",
        tr("Back"),
        &shortcuts::label(Command::Back),
        act(|| Action::Back),
    );
    cmd(
        "➡",
        tr("Forward"),
        &shortcuts::label(Command::Forward),
        act(|| Action::Forward),
    );
    cmd(
        "⬆",
        tr("Up one folder"),
        &shortcuts::label(Command::Up),
        act(|| Action::Up),
    );
    cmd(
        "🔄",
        tr("Refresh"),
        &shortcuts::label(Command::Refresh),
        act(|| Action::Refresh),
    );
    cmd(
        "➕",
        tr("New tab"),
        &shortcuts::label(Command::NewTab),
        act(|| Action::Tab(TabAction::New)),
    );
    cmd(
        "✖",
        tr("Close tab"),
        &shortcuts::label(Command::CloseTab),
        act(move || Action::Tab(TabAction::Close(tab_id))),
    );
    cmd(
        "📑",
        tr("Duplicate tab"),
        "",
        act(move || Action::Tab(TabAction::Duplicate(tab_id))),
    );
    cmd(
        "🔖",
        tr("Add tab to new group"),
        "",
        act(move || Action::Tab(TabAction::NewGroup(tab_id))),
    );
    cmd(
        "🔍",
        tr("Search…"),
        &shortcuts::label(Command::Search),
        Box::new(|app: &mut OxApp| app.ui.focus_search = true),
    );
    if let Some(e) = app.undo.history.peek(false) {
        cmd(
            "↺",
            &ox_core::trf!("Undo {what}", what = e.describe()),
            &shortcuts::label(Command::Undo),
            Box::new(|app: &mut OxApp| app.undo_redo(false)),
        );
    }
    if let Some(e) = app.undo.history.peek(true) {
        cmd(
            "↻",
            &ox_core::trf!("Redo {what}", what = e.describe()),
            &shortcuts::label(Command::Redo),
            Box::new(|app: &mut OxApp| app.undo_redo(true)),
        );
    }

    if let Some(f) = folder.clone() {
        let (f1, f2, f3, f4, f5) = (f.clone(), f.clone(), f.clone(), f.clone(), f.clone());
        cmd(
            "📁",
            tr("New folder"),
            &shortcuts::label(Command::NewFolder),
            act(move || Action::NewFolder(f1)),
        );
        cmd(
            "📄",
            tr("New text document"),
            "",
            act(move || Action::NewFile(f2)),
        );
        for (i, item) in app.settings.new_items.0.iter().enumerate() {
            let dir = f.clone();
            cmd(
                "📄",
                &ox_core::trf!("New {what}", what = ox_core::i18n::t_any(&item.label)),
                &item.file_name,
                act(move || Action::NewFromItem(dir.clone(), i)),
            );
        }
        use ox_core::links::LinkKind;
        for (kind, label) in [
            (LinkKind::Symbolic, tr("New symbolic link…")),
            (LinkKind::Junction, tr("New junction…")),
            (LinkKind::Hard, tr("New hard link…")),
        ] {
            let dir = f.clone();
            cmd("🔗", label, "", act(move || Action::NewLink(dir, kind)));
        }
        cmd(
            "📥",
            tr("Paste"),
            &shortcuts::label(Command::Paste),
            act(move || Action::Paste(f3)),
        );
        if app.engine.has_active() {
            let dir = f.clone();
            cmd(
                "📥",
                tr("Paste after current transfers"),
                "",
                act(move || Action::PasteAfterCurrent(dir)),
            );
        }
        cmd(
            "🖳",
            tr("Open terminal here"),
            "",
            act(move || Action::OpenTerminal(f4)),
        );
        cmd(
            "🖥",
            tr("Open in Windows Explorer"),
            "",
            act(move || Action::Reveal(f5)),
        );
        let f6 = f.clone();
        cmd(
            "📝",
            tr("Open folder in editor"),
            &shortcuts::label(Command::OpenInEditor),
            act(move || Action::OpenInEditor(vec![f6])),
        );
        if let Some(root) = ox_core::git::repo_root(&f) {
            let (r1, r2) = (root.clone(), root);
            cmd(
                "🌐",
                tr("Open repository on GitHub / GitLab"),
                "",
                act(move || Action::OpenGitRemote(r1)),
            );
            cmd(
                "🌿",
                tr("Copy Git branch name"),
                "",
                act(move || Action::CopyGitBranch(r2)),
            );
        }
        if app.quick.is_favorite(&f) {
            cmd(
                "☆",
                tr("Unpin folder from Favorites"),
                "",
                act(move || Action::Unpin(f)),
            );
        } else {
            cmd(
                "⭐",
                tr("Pin folder to Favorites"),
                "",
                act(move || Action::Pin(f)),
            );
        }
    }
    if tab.location().shows_files() {
        cmd(
            "☑",
            tr("Select all"),
            &shortcuts::label(Command::SelectAll),
            act(|| Action::SelectAll),
        );
    }
    if !selection.is_empty() && !tab.location().is_bin() && !tab.location().is_read_only() {
        let (s1, s2, s3) = (selection.clone(), selection.clone(), selection.clone());
        let rename = if selection.len() == 1 {
            act(move || Action::Rename(s1[0].clone()))
        } else {
            act(move || Action::BulkRename(s1))
        };
        cmd(
            "✏",
            tr("Rename selection"),
            &shortcuts::label(Command::Rename),
            rename,
        );
        cmd(
            "🔗",
            tr("Copy path of selection"),
            &shortcuts::label(Command::CopyPath),
            act(move || Action::CopyPathText(s2)),
        );
        let ed = selection.clone();
        cmd(
            "📝",
            tr("Open selection in editor"),
            &shortcuts::label(Command::OpenInEditor),
            act(move || Action::OpenInEditor(ed)),
        );
        cmd(
            "ℹ",
            tr("Properties of selection"),
            &shortcuts::label(Command::Properties),
            act(move || Action::Properties(s3)),
        );
        let zip = selection.clone();
        cmd(
            "📦",
            tr("Compress selection to ZIP"),
            "",
            act(move || Action::Compress(zip)),
        );
        if selection.len() == 1 && ox_core::archive::is_archive(&selection[0]) {
            let (a1, a2) = (selection[0].clone(), selection[0].clone());
            cmd(
                "📦",
                &ox_core::trf!(
                    "Extract to \"{v1}\"",
                    v1 = ox_core::archive::base_name(&selection[0])
                ),
                "",
                act(move || Action::ExtractArchive {
                    archive: a1,
                    own_folder: true,
                }),
            );
            cmd(
                "📦",
                tr("Extract here"),
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
                &ox_core::trf!("Tag selection: {v1}", v1 = app.tags.label(color)),
                "",
                act(move || Action::ToggleTag(sel, color)),
            );
        }
    }
    let counts = app.tags.counts();
    for color in TagColor::ALL.into_iter().filter(|c| counts[c.index()] > 0) {
        cmd(
            "🏷",
            &ox_core::trf!("Show items tagged {v1}", v1 = app.tags.label(color)),
            &ox_core::i18n::count(counts[color.index()] as u64, "item"),
            act(move || Action::Navigate(Location::Tagged(color))),
        );
    }

    if ox_core::shell::supported() {
        cmd(
            "🌐",
            tr("Map network drive…"),
            "",
            act(|| Action::NetworkDriveDialog(true)),
        );
        cmd(
            "🌐",
            tr("Disconnect network drive…"),
            "",
            act(|| Action::NetworkDriveDialog(false)),
        );
    }
    cmd(
        "▭",
        tr("Toggle terminal panel"),
        &shortcuts::label(Command::ToggleTerminal),
        act(|| Action::ToggleTerminal),
    );
    cmd(
        "◫",
        tr("Toggle split view"),
        &shortcuts::label(Command::ToggleSplit),
        act(|| Action::Tab(TabAction::ToggleSplit)),
    );
    cmd(
        "⇄",
        tr("Compare folders"),
        &shortcuts::label(Command::CompareFolders),
        act(|| Action::Compare(crate::app::CompareAction::Start)),
    );
    if app.tabs.panes.is_some() {
        cmd(
            "⇆",
            tr("Switch pane"),
            &shortcuts::label(Command::SwitchPane),
            act(|| Action::Tab(TabAction::SwitchPane)),
        );
    }
    cmd(
        "👁",
        tr("Toggle preview pane"),
        &shortcuts::label(Command::TogglePreview),
        setting(|a| a.settings.browsing.show_preview ^= true),
    );
    cmd(
        "◧",
        tr("Toggle sidebar"),
        "",
        setting(|a| a.settings.sidebar.visible ^= true),
    );
    cmd(
        "👻",
        tr("Toggle hidden items"),
        "",
        setting(|a| a.settings.browsing.show_hidden ^= true),
    );
    cmd(
        "🔤",
        tr("Toggle file name extensions"),
        "",
        setting(|a| a.settings.browsing.show_extensions ^= true),
    );
    cmd(
        "▬",
        tr("Toggle status bar"),
        "",
        setting(|a| a.settings.browsing.show_status_bar ^= true),
    );
    if app.tabs.active().can_flat() {
        cmd(
            "🗂",
            tr("Toggle flat view (all files in sub-folders)"),
            &shortcuts::label(Command::FlatView),
            act(|| Action::ToggleFlatView),
        );
    }
    cmd(
        "☰",
        tr("Details view"),
        "",
        Box::new(|a: &mut OxApp| a.tabs.active_mut().view_mode = ViewMode::Details),
    );
    cmd(
        "▦",
        tr("Grid view"),
        "",
        Box::new(|a: &mut OxApp| a.tabs.active_mut().view_mode = ViewMode::Grid),
    );
    for (theme, name) in [
        (Theme::Dark, tr("dark")),
        (Theme::Light, tr("light")),
        (Theme::System, tr("system")),
    ] {
        cmd(
            "🎨",
            &ox_core::trf!("Theme: {name}", name = name),
            "",
            setting(move |a| a.settings.appearance.theme = theme),
        );
    }
    cmd(
        "⚡",
        tr("Rebuild search index"),
        "",
        act(|| Action::RebuildIndex),
    );
    cmd(
        "⇄",
        tr("Show transfers"),
        "",
        Box::new(|a: &mut OxApp| a.ui.show_transfers = true),
    );
    cmd(
        "🗑",
        tr("Clear finished transfers"),
        "",
        Box::new(|a: &mut OxApp| a.engine.clear_finished()),
    );
    cmd("🗑", tr("Empty Recycle Bin"), "", act(|| Action::EmptyBin));
    cmd(
        "🕘",
        tr("Clear recent history"),
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
            label: ox_core::trf!("Search: {v1}", v1 = s.name),
            detail: tr("saved search").into(),
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
            .hint_text(tr("Type a command or a place…"))
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
            ui.label(RichText::new(tr("Nothing found.")).weak());
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
            RichText::new(tr("↑↓ to choose · Enter to run · Esc to close"))
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
