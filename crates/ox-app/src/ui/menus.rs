//! Context menus: items, backgrounds, folder links, tags and the New submenu.

use super::*;
use crate::app::SelectAction;
use ox_core::links::LinkKind;

/// Context information for the item context menu.
pub struct MenuTarget<'a> {
    pub selection: &'a [PathBuf],
    /// Items inside an archive: only open, copy (extract) and copy path.
    pub read_only: bool,
    pub tags: &'a TagStore,
    pub single_dir: Option<&'a Path>,
    pub pinned: bool,
    pub can_paste: bool,
    pub destinations: &'a [(String, PathBuf)],
    /// The Windows Shell is available ("Open with", "Show more options").
    pub shell: bool,
    /// Show the developer entries (Settings → Developer).
    pub dev: bool,
    /// Items in a cloud folder (OneDrive, …): whether all of them are
    /// always kept on this device. `None` elsewhere.
    pub cloud: Option<bool>,
}

/// Folders offered in "Copy to" / "Move to": favorites and other open tabs.
pub fn quick_destinations(app: &OxApp) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = app
        .quick
        .favorites
        .iter()
        .map(|f| (format!("⭐ {}", f.label()), f.path.clone()))
        .collect();
    for t in &app.tabs.tabs {
        if let Some(f) = t.location().folder() {
            if !out.iter().any(|(_, p)| p == f) {
                out.push((format!("📑 {}", display_name(f)), f.to_path_buf()));
            }
        }
    }
    out
}

pub fn tag_color(c: TagColor) -> Color32 {
    let [r, g, b] = c.rgb();
    Color32::from_rgb(r, g, b)
}

/// Tick a colour to tag the selection, again to remove the tag.
pub fn tag_menu(ui: &mut Ui, tags: &TagStore, sel: &[PathBuf], actions: &mut Vec<Action>) {
    for color in TagColor::ALL {
        let all = sel.iter().all(|p| tags.has(p, color));
        let label = RichText::new(format!("●  {}", tags.label(color))).color(tag_color(color));
        if ui.add(egui::Button::selectable(all, label)).clicked() {
            actions.push(Action::ToggleTag(sel.to_vec(), color));
        }
    }
    if sel.iter().any(|p| !tags.tags(p).is_empty()) {
        ui.separator();
        if ui.button(tr("✖ Remove all tags")).clicked() {
            actions.push(Action::ClearTags(sel.to_vec()));
        }
    }
}

/// Menu for items inside an archive: open, extract, copy path.
fn archive_item_menu(ui: &mut Ui, t: &MenuTarget<'_>, actions: &mut Vec<Action>) {
    let sel = t.selection.to_vec();
    if ui.button(tr("📂 Open")).clicked() {
        for p in &sel {
            actions.push(Action::Open(p.clone()));
        }
    }
    if ui
        .add(egui::Button::new(tr("📋 Copy")).shortcut_text(shortcuts::label(Command::Copy)))
        .clicked()
    {
        actions.push(Action::CopyItems(sel.clone()));
    }
    ui.separator();
    let next_to = sel
        .first()
        .and_then(|p| ox_core::archive::split_virtual(p))
        .and_then(|(arc, _)| arc.parent().map(Path::to_path_buf));
    if let Some(dir) = next_to {
        if ui.button(tr("📦 Extract next to the archive")).clicked() {
            actions.push(Action::Transfer(TransferKind::Copy, sel.clone(), dir));
        }
    }
    if !t.destinations.is_empty() {
        ui.menu_button(tr("📦 Extract to"), |ui| {
            for (label, dest) in t.destinations {
                if ui.button(label).clicked() {
                    actions.push(Action::Transfer(
                        TransferKind::Copy,
                        sel.clone(),
                        dest.clone(),
                    ));
                }
            }
        });
    }
    ui.separator();
    if ui
        .add(
            egui::Button::new(tr("🔗 Copy path"))
                .shortcut_text(shortcuts::label(Command::CopyPath)),
        )
        .clicked()
    {
        actions.push(Action::CopyPathText(sel.clone()));
    }
}

pub fn item_menu(ui: &mut Ui, t: &MenuTarget<'_>, actions: &mut Vec<Action>) {
    if t.read_only {
        archive_item_menu(ui, t, actions);
        return;
    }
    let sel = t.selection.to_vec();
    if let Some(dir) = t.single_dir {
        if ui.button(tr("📂 Open")).clicked() {
            actions.push(Action::Open(dir.to_path_buf()));
        }
        if ui.button(tr("📑 Open in new tab")).clicked() {
            actions.push(Action::OpenInNewTab(
                Location::Folder(dir.to_path_buf()),
                false,
            ));
        }
    } else if ui.button(tr("📂 Open")).clicked() {
        for p in &sel {
            actions.push(Action::Open(p.clone()));
        }
    }
    if t.shell
        && t.single_dir.is_none()
        && sel.len() == 1
        && ui.button(tr("🗗 Open with…")).clicked()
    {
        actions.push(Action::OpenWith(sel[0].clone()));
    }
    if sel.len() == 1
        && ui
            .add(
                egui::Button::new(tr("👁 Quick Look"))
                    .shortcut_text(shortcuts::label(Command::QuickLook)),
            )
            .clicked()
    {
        actions.push(Action::QuickLook);
    }
    super::mini_player::item_menu(ui, &sel, actions);
    ui.separator();
    if ui
        .add(egui::Button::new(tr("✂ Cut")).shortcut_text(shortcuts::label(Command::Cut)))
        .clicked()
    {
        actions.push(Action::CutItems(sel.clone()));
    }
    if ui
        .add(egui::Button::new(tr("📋 Copy")).shortcut_text(shortcuts::label(Command::Copy)))
        .clicked()
    {
        actions.push(Action::CopyItems(sel.clone()));
    }
    if let Some(dir) = t.single_dir {
        if ui
            .add_enabled(t.can_paste, egui::Button::new(tr("📥 Paste into folder")))
            .clicked()
        {
            actions.push(Action::Paste(dir.to_path_buf()));
        }
    }
    if !t.destinations.is_empty() {
        ui.menu_button(tr("➡ Copy to"), |ui| {
            for (label, dest) in t.destinations {
                if ui.button(label).clicked() {
                    actions.push(Action::Transfer(
                        TransferKind::Copy,
                        sel.clone(),
                        dest.clone(),
                    ));
                }
            }
        });
        ui.menu_button(tr("➡ Move to"), |ui| {
            for (label, dest) in t.destinations {
                if ui.button(label).clicked() {
                    actions.push(Action::Transfer(
                        TransferKind::Move,
                        sel.clone(),
                        dest.clone(),
                    ));
                }
            }
        });
    }
    ui.separator();
    if sel.len() == 1
        && ui
            .add(egui::Button::new(tr("✏ Rename")).shortcut_text(shortcuts::label(Command::Rename)))
            .clicked()
    {
        actions.push(Action::Rename(sel[0].clone()));
    }
    if sel.len() > 1
        && ui
            .add(
                egui::Button::new(ox_core::trf!("✏ Rename {v1} items…", v1 = sel.len()))
                    .shortcut_text(shortcuts::label(Command::Rename)),
            )
            .clicked()
    {
        actions.push(Action::BulkRename(sel.clone()));
    }
    if ui
        .add(egui::Button::new(tr("🗑 Delete")).shortcut_text(shortcuts::label(Command::Delete)))
        .clicked()
    {
        actions.push(Action::Delete(sel.clone(), false));
    }
    if ui
        .add(
            egui::Button::new(tr("⛔ Delete permanently"))
                .shortcut_text(shortcuts::label(Command::DeletePermanently)),
        )
        .clicked()
    {
        actions.push(Action::Delete(sel.clone(), true));
    }
    ui.separator();
    ui.menu_button(tr("🏷 Tags"), |ui| tag_menu(ui, t.tags, &sel, actions));
    if sel.len() == 1 && ox_core::archive::is_archive(&sel[0]) {
        let name = ox_core::archive::base_name(&sel[0]);
        if ui
            .button(ox_core::trf!("📦 Extract to \"{name}\"", name = name))
            .clicked()
        {
            actions.push(Action::ExtractArchive {
                archive: sel[0].clone(),
                own_folder: true,
            });
        }
        if ui.button(tr("📦 Extract here")).clicked() {
            actions.push(Action::ExtractArchive {
                archive: sel[0].clone(),
                own_folder: false,
            });
        }
    }
    if ui.button(tr("📦 Compress to ZIP")).clicked() {
        actions.push(Action::Compress(sel.clone()));
    }
    let theme_file = sel.len() == 1
        && sel[0]
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
        && ox_core::themes::read(&sel[0]).is_ok();
    if theme_file && ui.button(tr("🎨 Use as theme")).clicked() {
        actions.push(Action::InstallTheme(sel[0].clone()));
    }
    if let Some(dir) = t.single_dir {
        if t.pinned {
            if ui.button(tr("☆ Unpin from Favorites")).clicked() {
                actions.push(Action::Unpin(dir.to_path_buf()));
            }
        } else if ui.button(tr("⭐ Pin to Favorites")).clicked() {
            actions.push(Action::Pin(dir.to_path_buf()));
        }
        if ui.button(tr("🖳 Open in Terminal")).clicked() {
            actions.push(Action::OpenTerminal(dir.to_path_buf()));
        }
        super::cleanup::menu(ui, dir, actions);
        disk_usage_button(ui, dir, actions);
    }
    if ui
        .add(
            egui::Button::new(tr("🔗 Copy path"))
                .shortcut_text(shortcuts::label(Command::CopyPath)),
        )
        .clicked()
    {
        actions.push(Action::CopyPathText(sel.clone()));
    }
    if sel.len() == 1 && ui.button(tr("🖥 Show in Windows Explorer")).clicked() {
        actions.push(Action::Reveal(sel[0].clone()));
    }
    if let Some(all_kept) = t.cloud {
        ui.separator();
        super::cloud::menu(ui, &sel, all_kept, actions);
    }
    if t.dev {
        ui.separator();
        dev_menu(ui, &sel, actions);
    }
    ui.separator();
    if ui
        .add(
            egui::Button::new(tr("ℹ Properties"))
                .shortcut_text(shortcuts::label(Command::Properties)),
        )
        .clicked()
    {
        actions.push(Action::Properties(sel.clone()));
    }
    if t.shell {
        ui.separator();
        if ui
            .add(
                egui::Button::new(tr("⋯ Show more options"))
                    .shortcut_text(shortcuts::label(Command::ShellMenu)),
            )
            .on_hover_text(tr(
                "The Windows Explorer menu, with entries from other apps",
            ))
            .clicked()
        {
            actions.push(Action::ShellMenu(sel));
        }
    }
}

/// Developer entries (Settings → Developer): open in the editor, copy the
/// path in other forms and, in Git repositories, Git actions.
fn dev_menu(ui: &mut Ui, paths: &[PathBuf], actions: &mut Vec<Action>) {
    use ox_core::pathfmt::PathFormat;
    if ui
        .add(
            egui::Button::new(tr("📝 Open in editor"))
                .shortcut_text(shortcuts::label(Command::OpenInEditor)),
        )
        .clicked()
    {
        actions.push(Action::OpenInEditor(paths.to_vec()));
    }
    let repo = paths.first().and_then(|p| ox_core::git::repo_root(p));
    ui.menu_button(tr("🔗 Copy path as"), |ui| {
        for fmt in PathFormat::ALL {
            if fmt == PathFormat::GitRelative && repo.is_none() {
                continue;
            }
            if ui.button(fmt.label()).clicked() {
                actions.push(Action::CopyPathAs(paths.to_vec(), fmt));
            }
        }
    });
    if let (Some(root), [path]) = (repo, paths) {
        ui.menu_button(tr("🌿 Git"), |ui| {
            if ui.button(tr("🌐 Open on GitHub / GitLab")).clicked() {
                actions.push(Action::OpenGitRemote(path.clone()));
            }
            if ui.button(tr("📋 Copy branch name")).clicked() {
                actions.push(Action::CopyGitBranch(root.clone()));
            }
        });
    }
}

/// The entries of the "New" menus: Folder, Text document and the user's own
/// (Settings → New menu).
pub fn new_menu(ui: &mut Ui, folder: &Path, new_items: &[NewItem], actions: &mut Vec<Action>) {
    if ui
        .add(egui::Button::new(tr("📁 Folder")).shortcut_text(shortcuts::label(Command::NewFolder)))
        .clicked()
    {
        actions.push(Action::NewFolder(folder.to_path_buf()));
    }
    if ui.button(tr("📄 Text document")).clicked() {
        actions.push(Action::NewFile(folder.to_path_buf()));
    }
    ui.separator();
    for (kind, label) in [
        (LinkKind::Symbolic, tr("🔗 Symbolic link…")),
        (LinkKind::Junction, tr("🔗 Junction…")),
        (LinkKind::Hard, tr("🔗 Hard link…")),
    ] {
        if ui.button(label).clicked() {
            actions.push(Action::NewLink(folder.to_path_buf(), kind));
        }
    }
    if !new_items.is_empty() {
        ui.separator();
    }
    for (i, item) in new_items.iter().enumerate() {
        let text = format!("📄 {}", ox_core::i18n::t_any(&item.label));
        if ui.button(text).on_hover_text(&item.file_name).clicked() {
            actions.push(Action::NewFromItem(folder.to_path_buf(), i));
        }
    }
}

/// Context menu for the empty area of a folder.
#[allow(clippy::too_many_arguments)]
pub fn background_menu(
    ui: &mut Ui,
    folder: &Path,
    can_paste: bool,
    transfers_running: bool,
    pinned: bool,
    new_items: &[NewItem],
    dev: bool,
    actions: &mut Vec<Action>,
) {
    ui.menu_button(tr("➕ New"), |ui| new_menu(ui, folder, new_items, actions));
    if ui
        .add_enabled(
            can_paste,
            egui::Button::new(tr("📥 Paste")).shortcut_text(shortcuts::label(Command::Paste)),
        )
        .clicked()
    {
        actions.push(Action::Paste(folder.to_path_buf()));
    }
    if transfers_running
        && ui
            .add_enabled(
                can_paste,
                egui::Button::new(tr("📥 Paste after current transfers")),
            )
            .on_hover_text(tr("Starts when the running transfers are done."))
            .clicked()
    {
        actions.push(Action::PasteAfterCurrent(folder.to_path_buf()));
    }
    if ui
        .add(egui::Button::new(tr("🔄 Refresh")).shortcut_text(shortcuts::label(Command::Refresh)))
        .clicked()
    {
        actions.push(Action::Refresh);
    }
    super::mini_player::folder_menu(ui, folder, actions);
    if ui
        .add(
            egui::Button::new(tr("☑ Select all"))
                .shortcut_text(shortcuts::label(Command::SelectAll)),
        )
        .clicked()
    {
        actions.push(Action::SelectAll);
    }
    select_menu(ui, actions);
    ui.separator();
    if pinned {
        if ui.button(tr("☆ Unpin from Favorites")).clicked() {
            actions.push(Action::Unpin(folder.to_path_buf()));
        }
    } else if ui.button(tr("⭐ Pin to Favorites")).clicked() {
        actions.push(Action::Pin(folder.to_path_buf()));
    }
    if ui.button(tr("🖳 Open in Terminal")).clicked() {
        actions.push(Action::OpenTerminal(folder.to_path_buf()));
    }
    disk_usage_button(ui, folder, actions);
    if ui.button(tr("🖥 Open in Windows Explorer")).clicked() {
        actions.push(Action::Reveal(folder.to_path_buf()));
    }
    super::cleanup::menu(ui, folder, actions);
    if dev {
        ui.separator();
        dev_menu(ui, &[folder.to_path_buf()], actions);
    }
    ui.separator();
    if ui.button(tr("ℹ Properties")).clicked() {
        actions.push(Action::Properties(vec![folder.to_path_buf()]));
    }
}

/// Context menu for a folder link (sidebar, home page cards).
/// Select by pattern, invert, same type: a submenu next to Select all.
pub fn select_menu(ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.menu_button(tr("☑ Select"), |ui| {
        for (label, cmd, action) in [
            (
                tr("Select by pattern…"),
                Command::SelectPattern,
                SelectAction::PatternDialog(true),
            ),
            (
                tr("Deselect by pattern…"),
                Command::DeselectPattern,
                SelectAction::PatternDialog(false),
            ),
            (
                tr("Invert selection"),
                Command::InvertSelection,
                SelectAction::Invert,
            ),
            (
                tr("Select all of the same type"),
                Command::SelectSameType,
                SelectAction::SameType,
            ),
        ] {
            if ui
                .add(egui::Button::new(label).shortcut_text(shortcuts::label(cmd)))
                .clicked()
            {
                actions.push(Action::Select(action));
            }
        }
    });
}

pub fn folder_link_menu(ui: &mut Ui, path: &Path, pinned: bool, actions: &mut Vec<Action>) {
    if ui.button(tr("📂 Open")).clicked() {
        actions.push(Action::Navigate(Location::Folder(path.to_path_buf())));
    }
    if ui.button(tr("📑 Open in new tab")).clicked() {
        actions.push(Action::OpenInNewTab(
            Location::Folder(path.to_path_buf()),
            false,
        ));
    }
    ui.separator();
    if pinned {
        if ui.button(tr("☆ Unpin from Favorites")).clicked() {
            actions.push(Action::Unpin(path.to_path_buf()));
        }
        if ui.button(tr("⬆ Move up")).clicked() {
            actions.push(Action::MoveFavorite(path.to_path_buf(), -1));
        }
        if ui.button(tr("⬇ Move down")).clicked() {
            actions.push(Action::MoveFavorite(path.to_path_buf(), 1));
        }
    } else if ui.button(tr("⭐ Pin to Favorites")).clicked() {
        actions.push(Action::Pin(path.to_path_buf()));
    }
    ui.separator();
    if ui.button(tr("📥 Paste into folder")).clicked() {
        actions.push(Action::Paste(path.to_path_buf()));
    }
    if ui.button(tr("🖳 Open in Terminal")).clicked() {
        actions.push(Action::OpenTerminal(path.to_path_buf()));
    }
    disk_usage_button(ui, path, actions);
    if ui.button(tr("🔗 Copy path")).clicked() {
        actions.push(Action::CopyPathText(vec![path.to_path_buf()]));
    }
    if ui.button(tr("ℹ Properties")).clicked() {
        actions.push(Action::Properties(vec![path.to_path_buf()]));
    }
}

/// "Disk usage": what takes up the space in a folder or drive.
fn disk_usage_button(ui: &mut Ui, folder: &Path, actions: &mut Vec<Action>) {
    if ui
        .button(tr("🖴 Disk usage"))
        .on_hover_text(tr(
            "What takes up the space in here, as a map of rectangles",
        ))
        .clicked()
    {
        actions.push(Action::Navigate(Location::DiskUsage(folder.to_path_buf())));
    }
}

/// Middle-click opens a folder link in a background tab.
pub fn open_link(actions: &mut Vec<Action>, response: &Response, path: &Path) {
    if response.clicked() {
        actions.push(Action::Navigate(Location::Folder(path.to_path_buf())));
    } else if response.middle_clicked() {
        actions.push(Action::OpenInNewTab(
            Location::Folder(path.to_path_buf()),
            false,
        ));
    }
}
