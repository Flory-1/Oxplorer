//! Header navigation row: back/forward, address bar, breadcrumbs and search box.

mod suggest;

use super::*;
use crate::app::HistoryAction;
use suggest::{Dropdown, Picked};

pub(super) fn nav_row(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        let tab = app.tabs.active();
        let nav = |ui: &mut Ui, icon: &str, enabled: bool, tip: &str| {
            ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(icon).size(15.0))
                    .frame(false)
                    .min_size(vec2(28.0, 28.0)),
            )
            .on_hover_text(tip)
            .clicked()
        };
        if nav(
            ui,
            "⬅",
            tab.can_go_back(),
            &shortcuts::hint(tr("Back"), Command::Back),
        ) {
            actions.push(Action::Back);
        }
        if nav(
            ui,
            "➡",
            tab.can_go_forward(),
            &shortcuts::hint(tr("Forward"), Command::Forward),
        ) {
            actions.push(Action::Forward);
        }
        let can_up = !matches!(tab.location(), Location::Home);
        if nav(ui, "⬆", can_up, &shortcuts::hint(tr("Up"), Command::Up)) {
            actions.push(Action::Up);
        }
        if nav(
            ui,
            "🔄",
            true,
            &shortcuts::hint(tr("Refresh"), Command::Refresh),
        ) {
            actions.push(Action::Refresh);
        }

        let search_w = 260.0_f32.min(ui.available_width() * 0.35);
        let addr_w = ui.available_width() - search_w - 8.0;
        let address = ui
            .allocate_ui(vec2(addr_w, 30.0), |ui| address_bar(app, ui, &mut actions))
            .inner;
        let search = search_box(app, ui, search_w, &mut actions);
        // The preview pane's edge goes in the middle of the gap between them.
        app.ui.preview_edge = Some((address.right() + search.left()) / 2.0);
    });
    app.actions.extend(actions);
}

fn field_frame(ui: &Ui, focused: bool, accent: Color32) -> egui::Frame {
    let v = ui.visuals();
    egui::Frame::new()
        .fill(v.extreme_bg_color)
        .stroke(if focused {
            Stroke::new(1.5, accent)
        } else {
            v.widgets.noninteractive.bg_stroke
        })
        .corner_radius(CornerRadius::same(7))
        .inner_margin(egui::Margin::symmetric(6, 2))
}

/// Returns the field's rectangle.
fn address_bar(app: &mut OxApp, ui: &mut Ui, actions: &mut Vec<Action>) -> Rect {
    let accent = crate::theme::accent(&app.settings);
    let location = app.tabs.active().location().clone();
    let width = ui.available_width();
    let field = field_frame(ui, app.ui.address_editing, accent).show(ui, |ui| {
        ui.set_width(width - 14.0);
        ui.set_height(24.0);
        if app.ui.address_editing {
            let edit = egui::TextEdit::singleline(&mut app.ui.address_text)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .align(Align2::LEFT_CENTER)
                .font(FontId::proportional(13.5));
            let resp = ui.add(edit);
            if app.ui.focus_address {
                resp.request_focus();
                app.ui.focus_address = false;
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                    let all = egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(app.ui.address_text.chars().count()),
                    );
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), resp.id);
                }
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let text = app.ui.address_text.trim().trim_matches('"').to_owned();
                    let path = PathBuf::from(expand_env(&text));
                    if text.is_empty() {
                    } else if ox_core::network::is_dir(&path) {
                        actions.push(Action::Navigate(Location::Folder(path)));
                    } else if path.is_file() {
                        actions.push(Action::Open(path));
                    } else {
                        actions.push(Action::Toast(ox_core::trf!(
                            "Can't find \"{text}\".",
                            text = text
                        )));
                    }
                }
                app.ui.address_editing = false;
            }
            return;
        }

        app.ui.address_text = match &location {
            Location::Folder(p) => p.display().to_string(),
            _ => String::new(),
        };
        let tag_label = match &location {
            Location::Tagged(c) => app.tags.label(*c).to_owned(),
            _ => String::new(),
        };
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            match &location {
                Location::Tagged(_) => {
                    ui.label(ox_core::trf!(
                        "🏷  Items tagged \"{tag_label}\"",
                        tag_label = tag_label
                    ));
                }
                Location::Folder(p) => breadcrumbs(ui, p, actions),
                Location::Home => {
                    ui.label(ox_core::trf!("{v1}  Home", v1 = crate::icons::HOME));
                }
                Location::Settings => {
                    ui.label(tr("⚙  Settings"));
                }
                Location::Help => {
                    ui.label(tr("❓  Help"));
                }
                Location::RecycleBin => {
                    ui.label(tr("🗑  Recycle Bin"));
                }
                Location::Library { name, .. } => {
                    ui.label(format!("📚  {name}"));
                }
                Location::Gallery => {
                    ui.label(tr("🎞  Gallery"));
                }
                Location::DiskUsage(p) => {
                    ui.label(ox_core::trf!("🖴  Disk usage of {path}", path = p.display()));
                }
                Location::Archive { archive, inner } => {
                    if let Some(parent) = archive.parent() {
                        breadcrumbs(ui, parent, actions);
                    }
                    archive_crumbs(ui, archive, inner, actions);
                }
                Location::Cleanup { root, mode } => {
                    let icon = Location::cleanup_icon(*mode);
                    let place = root.display();
                    ui.label(match mode {
                        ox_core::cleanup::CleanupMode::Duplicates => ox_core::trf!(
                            "{icon}  Duplicates in {place}",
                            icon = icon,
                            place = place
                        ),
                        ox_core::cleanup::CleanupMode::EmptyFolders => ox_core::trf!(
                            "{icon}  Empty folders in {place}",
                            icon = icon,
                            place = place
                        ),
                    });
                }
                Location::Search { root, query } => {
                    let place = root
                        .as_deref()
                        .map(display_name)
                        .unwrap_or_else(|| tr("This PC").into());
                    ui.label(ox_core::trf!(
                        "🔍  Search results for \"{query}\" in {place}",
                        query = query,
                        place = place
                    ));
                }
            }
            // Clicking the empty part of the bar switches to text editing.
            let rest = ui.available_size();
            if ui
                .allocate_response(rest, Sense::click())
                .on_hover_cursor(egui::CursorIcon::Text)
                .clicked()
            {
                app.ui.address_editing = true;
                app.ui.focus_address = true;
            }
        });
    });
    field.response.rect
}

/// `%USERPROFILE%\Desktop` → `C:\Users\me\Desktop`, and `~` → home on all platforms.
fn expand_env(text: &str) -> String {
    let mut out = text.to_owned();
    if let Some(rest) = out.strip_prefix('~') {
        out = format!("{}{rest}", ox_core::places::home_dir().display());
    }
    while let Some(start) = out.find('%') {
        let Some(len) = out[start + 1..].find('%') else {
            break;
        };
        let name = &out[start + 1..start + 1 + len];
        match std::env::var(name) {
            Ok(v) => out.replace_range(start..start + len + 2, &v),
            Err(_) => break,
        }
    }
    out
}

fn breadcrumbs(ui: &mut Ui, path: &Path, actions: &mut Vec<Action>) {
    let server = ox_core::network::server_of(path);
    let mut parts: Vec<&Path> = path.ancestors().collect();
    // `\\server\share` paths start at the server, which lists its shares.
    parts.extend(server.as_deref());
    parts.reverse();
    const MAX: usize = 5;
    let hidden = parts.len().saturating_sub(MAX);
    if hidden > 0 {
        crumb_menu(ui, RichText::new("«"), |ui| {
            for p in &parts[..hidden] {
                if ui.button(format!("📁 {}", crumb_name(p))).clicked() {
                    actions.push(Action::Navigate(Location::Folder(p.to_path_buf())));
                }
            }
        });
    }
    for (i, p) in parts.iter().enumerate().skip(hidden) {
        let last = i + 1 == parts.len();
        let text = RichText::new(crumb_name(p)).size(13.5);
        let resp = ui.add(egui::Button::new(if last { text.strong() } else { text }).frame(false));
        if resp.clicked() {
            actions.push(Action::Navigate(Location::Folder(p.to_path_buf())));
        }
        if resp.middle_clicked() {
            actions.push(Action::OpenInNewTab(
                Location::Folder(p.to_path_buf()),
                false,
            ));
        }
        super::super::drop_target(actions, &resp, p);
        // "›" opens a menu of the sub-folders, like Explorer's address bar.
        crumb_menu(ui, RichText::new("›").size(14.0), |ui| {
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    for sub in subfolders(p) {
                        if ui.button(format!("📁 {}", display_name(&sub))).clicked() {
                            actions.push(Action::Navigate(Location::Folder(sub)));
                        }
                    }
                });
        });
    }
}

/// A frameless menu button for the address bar's `«` and `›` arrows.
fn crumb_menu(ui: &mut Ui, text: RichText, content: impl FnOnce(&mut Ui)) {
    egui::containers::menu::MenuButton::from_button(
        egui::Button::new(text)
            .frame(false)
            .min_size(egui::vec2(18.0, 0.0)),
    )
    .ui(ui, content);
}

/// The archive and the folders inside it, after the folder crumbs.
fn archive_crumbs(ui: &mut Ui, archive: &Path, inner: &Path, actions: &mut Vec<Action>) {
    let mut parts = vec![PathBuf::new()];
    for c in inner.components() {
        let next = parts.last().cloned().unwrap_or_default().join(c);
        parts.push(next);
    }
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let name = if i == 0 {
            format!("📦 {}", display_name(archive))
        } else {
            display_name(part)
        };
        let text = RichText::new(name).size(13.5);
        let text = if last { text.strong() } else { text };
        if ui.add(egui::Button::new(text).frame(false)).clicked() {
            actions.push(Action::Navigate(Location::Archive {
                archive: archive.to_path_buf(),
                inner: part.clone(),
            }));
        }
        // "›" lists the folders inside, like for ordinary folders.
        crumb_menu(ui, RichText::new("›").size(14.0), |ui| {
            let mut dirs: Vec<_> = ox_core::archive::list(archive, part)
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.is_dir)
                .collect();
            dirs.sort_by(|a, b| ox_core::natural::natural_cmp(&a.name, &b.name));
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    for d in dirs {
                        if ui.button(format!("📁 {}", d.name)).clicked() {
                            actions.push(Action::Navigate(Location::Archive {
                                archive: archive.to_path_buf(),
                                inner: part.join(&d.name),
                            }));
                        }
                    }
                });
        });
    }
}

fn crumb_name(p: &Path) -> String {
    if let Some(server) = ox_core::network::server_name(p) {
        return server.to_owned();
    }
    if p.parent().is_none() && ox_core::network::server_of(p).is_some() {
        // A share's root: just the share, after the server crumb.
        let s = p.to_string_lossy();
        return s
            .trim_end_matches('\\')
            .rsplit('\\')
            .next()
            .unwrap_or("")
            .to_owned();
    }
    if p.parent().is_none() {
        let s = p.display().to_string();
        let s = s.trim_end_matches(['\\', '/']);
        if s.is_empty() {
            "/".into()
        } else {
            s.to_owned()
        }
    } else {
        display_name(p)
    }
}

fn subfolders(p: &Path) -> Vec<PathBuf> {
    if let Some(server) = ox_core::network::server_name(p) {
        let shares = ox_core::network::shares(server).unwrap_or_default();
        return shares.into_iter().map(|e| e.path).collect();
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .map(|e| e.path())
                .take(300)
                .collect()
        })
        .unwrap_or_default();
    dirs.sort_by(|a, b| ox_core::natural::natural_cmp(&display_name(a), &display_name(b)));
    dirs
}

/// Returns the field's rectangle.
fn search_box(app: &mut OxApp, ui: &mut Ui, width: f32, actions: &mut Vec<Action>) -> Rect {
    let accent = crate::theme::accent(&app.settings);
    let folders_first = app.settings.browsing.folders_first;
    let case = app.settings.search.case_sensitive;
    let instant = app.settings.search.instant_filter;
    let remember = app.settings.search.remember_searches;
    let focus = std::mem::take(&mut app.ui.focus_search);
    // With the index, Home and search tabs show results while typing.
    let live = app.settings.search.search_as_you_type && app.index_ready();
    let id = ui.make_persistent_id("search_box");
    let focused = ui.memory(|m| m.has_focus(id));
    if focused && remember {
        app.search_history.refresh();
    }
    let history: &[String] = if remember {
        app.search_history.entries()
    } else {
        &[]
    };
    let tab = app.tabs.active_mut();
    let live_here = live && matches!(tab.location(), Location::Home | Location::Search { .. });
    let in_search = matches!(tab.location(), Location::Search { .. });
    let (root, place) = match tab.location() {
        Location::Folder(p) => (Some(p.clone()), display_name(p)),
        Location::Search { root, .. } => (
            root.clone(),
            root.as_deref()
                .map(display_name)
                .unwrap_or(tr("This PC").into()),
        ),
        Location::RecycleBin => (None, tr("Recycle Bin").into()),
        _ => (None, tr("This PC").into()),
    };
    let in_bin = tab.location().is_bin();
    let dropdown = Dropdown::before(ui, id);
    ui.allocate_ui(vec2(width, 30.0), |ui| {
        let mut edit_resp = None;
        let field = field_frame(ui, focused, accent).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new("🔍").weak());
                let edit = egui::TextEdit::singleline(&mut tab.filter)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .hint_text(ox_core::trf!("Search {place}", place = place))
                    .desired_width(ui.available_width() - 4.0)
                    .align(Align2::LEFT_CENTER)
                    .event_filter(dropdown.event_filter());
                let resp = dropdown.add_field(ui, |ui| ui.add(edit));
                if focus {
                    resp.request_focus();
                }
                if resp.changed() && instant {
                    tab.refresh_visible(folders_first, case);
                }
                if resp.changed() && live_here {
                    actions.push(Action::SearchLive(root.clone(), tab.filter.clone()));
                }
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if resp.lost_focus() && enter && !in_bin {
                    actions.push(Action::Search(root.clone(), tab.filter.clone()));
                } else if resp.lost_focus() && escape {
                    tab.filter.clear();
                    tab.refresh_visible(folders_first, case);
                } else if resp.lost_focus() && live_here && in_search {
                    // Left a search made while typing (e.g. for its results): it counts as run.
                    actions.push(Action::SearchHistory(HistoryAction::Remember(
                        tab.filter.clone(),
                    )));
                }
                edit_resp = Some(resp.clone());
                resp.on_hover_text(tr(
                    "Type to filter this folder, press Enter to search all sub-folders.\n\
                     On Home, results appear while you type (with the search index).\n\
                     Filters: *.ext  /regex/  ext:pdf,docx  kind:image  type:folder  size:>10mb  modified:7d  content:\"text\"  -exclude",
                ));
            });
        });
        let edit = edit_resp.expect("the search field was shown");
        let suggestions = ox_core::search_history::suggest(&tab.filter, history, suggest::MAX);
        match dropdown.show(ui, &field.response, &edit, &tab.filter, suggestions) {
            Some(Picked::Take(s, run)) => {
                tab.filter = s.text;
                if run && !in_bin {
                    ui.memory_mut(|m| m.surrender_focus(id));
                    actions.push(Action::Search(root, tab.filter.clone()));
                } else {
                    suggest::cursor_to_end(ui, id, &tab.filter);
                    if instant || run {
                        tab.refresh_visible(folders_first, case);
                    }
                    if live_here {
                        actions.push(Action::SearchLive(root, tab.filter.clone()));
                    }
                }
            }
            Some(Picked::Submit) if !in_bin => {
                ui.memory_mut(|m| m.surrender_focus(id));
                actions.push(Action::Search(root, tab.filter.clone()));
            }
            Some(Picked::Submit) | None => {}
            Some(Picked::Remove(q)) => {
                actions.push(Action::SearchHistory(HistoryAction::Remove(q)));
            }
            Some(Picked::ClearHistory) => {
                actions.push(Action::SearchHistory(HistoryAction::Clear));
            }
        }
        field.response.rect
    })
    .inner
}
