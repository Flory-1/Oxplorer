//! The action dispatcher: turns each [`Action`] and [`TabAction`] into state changes.

use super::*;

impl OxApp {
    pub(super) fn process_actions(&mut self) {
        let actions = std::mem::take(&mut self.actions);
        for a in actions {
            self.process(a);
        }
    }

    pub(super) fn process(&mut self, action: Action) {
        let Some(action) = self.remote_filter(action) else {
            return;
        };
        let system_clip = self.settings.transfer.use_system_clipboard;
        match action {
            Action::Navigate(loc) => self.navigate(loc),
            Action::OpenInNewTab(page, activate) if page.is_page() => {
                // Reuse its open tab; never put it into a tab group.
                if !self.activate_page(&page) {
                    self.tabs.open(page, None, activate, &self.settings);
                }
            }
            Action::OpenInNewTab(loc, activate) => {
                let after = Some(self.tabs.active);
                self.tabs.open(loc, after, activate, &self.settings);
            }
            Action::Back => self.tabs.active_mut().back(),
            Action::Forward => self.tabs.active_mut().forward(),
            Action::Up => {
                if !self.disk_usage_up() {
                    self.cleanup_aware_up();
                }
            }
            Action::Refresh if self.in_disk_usage() => {
                self.disk_usage_action(DiskUsageAction::Rescan);
            }
            Action::Refresh => {
                if let Some(folder) = self.current_folder() {
                    self.folder_sizes.invalidate_tree(&folder);
                }
                self.tabs.active_mut().stale = true;
                self.drives_at = Instant::now() - Duration::from_secs(60);
            }
            Action::Open(path) if player::opens_playlist(&path) => {
                self.player_action(PlayerAction::Playlist(path));
            }
            Action::Open(path) => {
                let in_archive = archive::split_virtual(&path)
                    .filter(|(_, inner)| !inner.as_os_str().is_empty());
                if ox_core::network::is_dir(&path) {
                    self.navigate(Location::Folder(path));
                } else if self.settings.browsing.open_archives && archive::is_archive(&path) {
                    self.quick.open_file(&path);
                    self.save_quick();
                    self.navigate(Location::Archive {
                        archive: path,
                        inner: PathBuf::new(),
                    });
                } else if let Some((arc, inner)) = in_archive {
                    if archive::index(&arc).is_ok_and(|idx| idx.is_dir(&inner)) {
                        self.navigate(Location::Archive {
                            archive: arc,
                            inner,
                        });
                    } else {
                        self.open_from_archive(arc, inner);
                    }
                } else {
                    match fsops::open_default(&path) {
                        Ok(()) => self.quick.open_file(&path),
                        Err(e) => self.toast(ox_core::trf!(
                            "Couldn't open {v1}: {e}",
                            v1 = path.display(),
                            e = e
                        )),
                    }
                    self.save_quick();
                }
            }
            Action::CopyItems(paths) => {
                let n = paths.len();
                self.clipboard.set(paths, ClipMode::Copy, system_clip);
                self.toast(ox_core::trf!(
                    "{v1} copied",
                    v1 = ox_core::i18n::count(n as u64, "item")
                ));
            }
            Action::CutItems(paths) => {
                let n = paths.len();
                self.clipboard.set(paths, ClipMode::Cut, system_clip);
                self.toast(ox_core::trf!(
                    "{v1} cut",
                    v1 = ox_core::i18n::count(n as u64, "item")
                ));
            }
            Action::Paste(dest) => match self.clipboard.get(system_clip) {
                Some((items, mode)) => {
                    let kind = match mode {
                        ClipMode::Copy => TransferKind::Copy,
                        ClipMode::Cut => TransferKind::Move,
                    };
                    self.start_transfer(kind, items, dest);
                    if mode == ClipMode::Cut {
                        self.clipboard.clear(system_clip);
                    }
                }
                None => self.toast(tr("The clipboard doesn't contain any files.")),
            },
            Action::PasteAfterCurrent(dest) => {
                self.transfer_watch.run_after_next = true;
                self.process(Action::Paste(dest));
                self.transfer_watch.run_after_next = false;
            }
            Action::Transfer(kind, sources, dest) => self.start_transfer(kind, sources, dest),
            Action::Drop(sources, dest) => {
                // Dropping items onto their own folder (or a folder onto itself) is a no-op.
                let sources: Vec<PathBuf> = sources
                    .into_iter()
                    .filter(|s| s != &dest && s.parent() != Some(dest.as_path()))
                    .collect();
                if sources.is_empty() {
                    return;
                }
                let m = self.ctx.input(|i| i.modifiers);
                if (m.ctrl || m.command) && m.shift {
                    // Ctrl+Shift links, like in Explorer.
                    self.link_dropped(sources, dest);
                    return;
                }
                let kind = if m.ctrl || m.command {
                    TransferKind::Copy
                } else if m.shift
                    || (self.settings.transfer.drag_moves_on_same_drive
                        && fsops::same_volume(&sources[0], &dest))
                {
                    TransferKind::Move
                } else {
                    TransferKind::Copy
                };
                self.start_transfer(kind, sources, dest);
            }
            Action::Delete(paths, permanent) => {
                if self.settings.browsing.confirm_delete
                    || permanent
                    || !self.settings.browsing.delete_to_trash
                {
                    self.dialog = Some(Dialog::ConfirmDelete { paths, permanent });
                } else {
                    self.process(Action::DeleteConfirmed(paths, false));
                }
            }
            Action::DeleteConfirmed(paths, permanent) => {
                let to_trash = !permanent && self.settings.browsing.delete_to_trash;
                let dir = paths
                    .first()
                    .and_then(|p| p.parent())
                    .map(Path::to_path_buf);
                let undo = self.undo.recorder();
                self.run_background(tr("Delete"), dir, move || {
                    let since = ox_core::undo::now();
                    let result = fsops::delete(&paths, to_trash);
                    if to_trash {
                        undo.trashed(&paths, since);
                    }
                    result
                });
                self.tabs.active_mut().view.clear_selection();
            }
            Action::Rename(path) => {
                let text = entry::display_name(&path);
                self.dialog = Some(Dialog::Rename {
                    path,
                    text,
                    error: None,
                    focus: true,
                });
            }
            Action::BulkRename(paths) => {
                let items = paths
                    .into_iter()
                    .filter_map(|path| {
                        let meta = std::fs::symlink_metadata(&path).ok()?;
                        Some(ox_core::rename::RenameItem {
                            is_dir: meta.is_dir(),
                            modified: meta.modified().ok(),
                            path,
                        })
                    })
                    .collect();
                self.dialog = Some(Dialog::BulkRename {
                    items,
                    rule: Default::default(),
                    focus: true,
                });
            }
            Action::NewFolder(dir) => match fsops::create_folder(&dir, tr("New folder")) {
                Ok(p) => {
                    self.after_create(&dir, p);
                }
                Err(e) => self.toast(ox_core::trf!("Couldn't create folder: {e}", e = e)),
            },
            Action::NewFile(dir) => match fsops::create_file(&dir, tr("New Text Document.txt")) {
                Ok(p) => self.after_create(&dir, p),
                Err(e) => self.toast(ox_core::trf!("Couldn't create file: {e}", e = e)),
            },
            Action::NewFromItem(dir, i) => {
                let Some(item) = self.settings.new_items.0.get(i).cloned() else {
                    return;
                };
                let name = if item.file_name.trim().is_empty() {
                    tr("New Text Document.txt").to_owned()
                } else {
                    item.file_name.trim().to_owned()
                };
                match fsops::create_file_with(&dir, &name, &item.content_for(&name)) {
                    Ok(p) => self.after_create(&dir, p),
                    Err(e) => self.toast(ox_core::trf!("Couldn't create file: {e}", e = e)),
                }
            }
            Action::Pin(p) => {
                self.quick.pin(&p);
                self.save_quick();
            }
            Action::ToggleTag(paths, color) => {
                self.tags.toggle(&paths, color);
                self.tags_changed();
            }
            Action::ClearTags(paths) => {
                self.tags.clear(&paths);
                self.tags_changed();
            }
            Action::ExtractArchive {
                archive: arc,
                own_folder,
            } => {
                let Some(parent) = arc.parent().map(Path::to_path_buf) else {
                    return;
                };
                let dest = if own_folder {
                    fsops::unique_path(&parent, &archive::base_name(&arc))
                } else {
                    parent
                };
                self.extract(arc, vec![PathBuf::new()], dest);
            }
            Action::Compress(paths) => {
                let Some(dir) = paths
                    .first()
                    .and_then(|p| p.parent())
                    .map(Path::to_path_buf)
                else {
                    return;
                };
                let zip = archive::zip_target(&paths, &dir);
                self.toast(ox_core::trf!(
                    "Compressing to {v1}…",
                    v1 = entry::display_name(&zip)
                ));
                self.tabs.active_mut().view.select_after_load = Some(zip.clone());
                self.run_background(tr("Compressing"), Some(dir), move || {
                    archive::compress_zip(&paths, &zip, &AtomicBool::new(false))
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                });
            }
            Action::InstallTheme(path) => match ox_core::themes::install(&path) {
                Ok(t) => {
                    let a = &mut self.settings.appearance;
                    if t.dark {
                        a.dark_theme = t.name.clone();
                        a.theme = ox_core::settings::Theme::Dark;
                    } else {
                        a.light_theme = t.name.clone();
                        a.theme = ox_core::settings::Theme::Light;
                    }
                    if let Some(accent) = t.accent {
                        a.accent = accent.0;
                    }
                    self.themes = ox_core::themes::load_all();
                    self.themes_changed = true;
                    self.push(Action::SettingsChanged);
                    self.toast(ox_core::trf!(
                        "Theme \"{v1}\" installed and in use.",
                        v1 = t.name
                    ));
                }
                Err(e) => self.toast(ox_core::trf!("This isn't a theme file: {e}", e = e)),
            },
            Action::SetDefaultFolderHandler(on) => {
                match ox_core::default_app::set_enabled(on) {
                    Ok(()) => self.toast(if on {
                        tr("Folders now open in Oxplorer.").to_owned()
                    } else {
                        tr("Folders open in Explorer again.").to_owned()
                    }),
                    Err(e) => self.toast(ox_core::trf!("Couldn't change the default: {e}", e = e)),
                }
                self.ui.default_folder_handler = None;
            }
            Action::NetworkDriveDialog(map) => {
                if map {
                    ox_core::network::map_drive_dialog();
                } else {
                    ox_core::network::disconnect_drive_dialog();
                }
            }
            Action::ToggleTerminal => {
                self.ui.show_terminal = !self.ui.show_terminal;
                self.ui.terminal_focused = false;
                if self.ui.show_terminal {
                    if self.terminal.is_none() {
                        self.start_terminal(); // starts in the current folder
                    } else {
                        self.terminal_to_current_folder();
                    }
                }
            }
            Action::RestartTerminal => self.start_terminal(),
            Action::PathsMoved(pairs) => {
                if !self.tags.is_empty() {
                    for (from, to) in &pairs {
                        self.tags.moved(from, to);
                    }
                    self.tags_changed();
                }
            }
            Action::RenameTag(color, name) => {
                self.tags.set_label(color, &name);
                self.tags_changed();
            }
            Action::Unpin(p) => {
                self.quick.unpin(&p);
                self.save_quick();
            }
            Action::MoveFavorite(p, d) => {
                self.quick.move_favorite(&p, d);
                self.save_quick();
            }
            Action::CopyPathText(paths) => {
                let text = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("\n");
                self.ctx.copy_text(text);
                self.toast(tr("Path copied"));
            }
            Action::CopyPathAs(paths, fmt) => {
                let text = paths
                    .iter()
                    .map(|p| {
                        let root = ox_core::git::repo_root(p);
                        ox_core::pathfmt::format(p, fmt, root.as_deref())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.ctx.copy_text(text);
                self.toast(tr("Path copied"));
            }
            Action::OpenInEditor(paths) => {
                let template = self.settings.developer.editor.clone();
                for path in paths {
                    match fsops::open_in_editor(&template, &path) {
                        Ok(child) => self.in_background(move || match child.wait_with_output() {
                            Ok(out) if !out.status.success() => {
                                // One line, without the final full stop (cmd's message has both).
                                let err = String::from_utf8_lossy(&out.stderr)
                                    .split_whitespace()
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                let err = err.trim_end_matches('.');
                                Notice::Toast(ox_core::trf!(
                                    "The editor didn't start ({e}). Check Settings → Developer.",
                                    e = if err.is_empty() {
                                        out.status.to_string()
                                    } else {
                                        err.to_owned()
                                    }
                                ))
                            }
                            _ => Notice::Nothing,
                        }),
                        Err(e) => self.toast(ox_core::trf!(
                            "The editor didn't start ({e}). Check Settings → Developer.",
                            e = e
                        )),
                    }
                }
            }
            Action::OpenGitRemote(path) => self.in_background(move || {
                let page = || -> Option<String> {
                    let root = ox_core::git::repo_root(&path)?;
                    let remote = ox_core::git::remote_url(&root).ok()??;
                    let repo = ox_core::git::web_url(&remote)?;
                    let branch = ox_core::git::current_branch(&root).ok()?;
                    Some(ox_core::git::web_url_of(
                        &repo,
                        &root,
                        &path,
                        &branch,
                        path.is_dir(),
                    ))
                };
                match page() {
                    Some(url) => Notice::OpenUrl(url),
                    None => Notice::Toast(
                        tr("This repository has no remote on GitHub, GitLab or another website.")
                            .into(),
                    ),
                }
            }),
            Action::CopyGitBranch(path) => {
                self.in_background(move || {
                    match ox_core::git::repo_root(&path).map(|r| ox_core::git::current_branch(&r)) {
                        Some(Ok(branch)) => Notice::Copy(
                            branch.clone(),
                            ox_core::trf!("Copied branch name “{b}”", b = branch),
                        ),
                        Some(Err(e)) => Notice::Toast(e.to_string()),
                        None => Notice::Toast(tr("Not in a Git repository.").into()),
                    }
                })
            }
            Action::Git(a) => self.git_action(a),
            Action::OpenTerminal(dir) => {
                if let Err(e) = fsops::open_terminal(&dir) {
                    self.toast(ox_core::trf!("Couldn't open a terminal: {e}", e = e));
                }
            }
            Action::OpenWslTerminal(dir) => {
                if let Err(e) = ox_core::wsl::open_terminal(&dir) {
                    self.toast(ox_core::trf!("Couldn't open a WSL terminal: {e}", e = e));
                }
            }
            Action::Reveal(p) => {
                if let Err(e) = fsops::reveal_in_system(&p) {
                    self.toast(ox_core::trf!(
                        "Couldn't open the system file manager: {e}",
                        e = e
                    ));
                }
            }
            Action::Properties(paths) => self.open_properties(paths),
            Action::KeepOnDevice(paths, pin) => self.keep_on_device(paths, pin),
            Action::SetAttributes(paths, readonly, hidden) => {
                self.set_attributes(&paths, readonly, hidden)
            }
            Action::ShellMenu(paths) => self.icons.shell.context_menu(paths),
            Action::OpenWith(path) => self.icons.shell.open_with(path),
            Action::Search(root, query) => {
                if query.trim().is_empty() {
                    return;
                }
                self.process_history(HistoryAction::Remember(query.clone()));
                self.navigate(Location::Search { root, query });
            }
            Action::SearchLive(root, query) => {
                let home = self.tabs.home();
                let tab = self.tabs.active_mut();
                let in_search = matches!(tab.location(), Location::Search { .. });
                if query.trim().is_empty() {
                    if in_search && tab.can_go_back() {
                        tab.back();
                    } else if in_search {
                        // A search tab opened from the Home tab: back to Home.
                        let id = tab.id;
                        self.tabs.close(id);
                        self.tabs.activate(home);
                    }
                } else if in_search {
                    tab.replace(Location::Search { root, query });
                } else {
                    self.navigate(Location::Search { root, query });
                }
            }
            Action::SaveSearch(root, query) => {
                self.quick.save_search(&query, root);
                self.save_quick();
                self.toast(tr("Search saved to the sidebar"));
            }
            Action::RemoveSavedSearch(i) => {
                if i < self.quick.saved_searches.len() {
                    self.quick.saved_searches.remove(i);
                    self.save_quick();
                }
            }
            Action::RebuildIndex => {
                self.index_force = true;
            }
            Action::SelectAll => self.tabs.active_mut().view.select_all(),
            Action::Select(a) => self.process_select(a),
            Action::QuickLook => self.toggle_quick_look(),
            Action::ForgetRecent(p) => {
                self.quick.forget(&p);
                self.save_quick();
            }
            Action::ClearRecent => {
                self.quick.clear_recent();
                self.save_quick();
            }
            Action::SettingsChanged => {
                if let Err(e) = self.save_settings() {
                    self.toast(ox_core::trf!("Couldn't save settings: {e}", e = e));
                }
                // Hidden-file setting etc. affect listings.
                self.tabs.mark_stale(None);
            }
            Action::Tab(t) => self.process_tab(t),
            Action::Compare(a) => self.process_compare(a),
            Action::Mirror(a) => self.process_mirror(a),
            Action::Cleanup(a) => self.process_cleanup(a),
            Action::Pictures(a) => self.process_pictures(a),
            Action::CompareTextFiles(a, b) => crate::ui::text_compare::open(self, a, b),
            Action::Command(a) => self.command_action(a),
            Action::ToggleFlatView => self.toggle_flat_view(),
            Action::ShowInFolder(path) => self.show_in_folder(path),
            Action::DiskUsage(a) => self.disk_usage_action(a),
            Action::Player(a) => self.player_action(a),
            Action::Remote(a) => self.remote_action(a),
            Action::NewLink(dir, kind) => self.new_link_dialog(dir, kind),
            Action::ShowLinkTarget(link) => self.show_link_target(link),
            Action::Session(a) => self.process_session(a),
            Action::SearchHistory(a) => self.process_history(a),
            Action::View(a) => self.process_view(a),
            Action::Toast(t) => self.toast(t),
            Action::RestoreFromBin(paths) => {
                let items = self.tabs.active().view.bin.select(&paths);
                self.run_background(tr("Restore"), None, move || recycle::restore(items));
                self.tabs.active_mut().view.clear_selection();
            }
            Action::PurgeFromBin(paths) => {
                let items = self.tabs.active().view.bin.select(&paths);
                if !items.is_empty() {
                    self.dialog = Some(Dialog::ConfirmPurge {
                        items,
                        everything: false,
                    });
                }
            }
            Action::EmptyBin => {
                self.dialog = Some(Dialog::ConfirmPurge {
                    items: Vec::new(),
                    everything: true,
                });
            }
            Action::PurgeConfirmed(items, everything) => {
                if everything {
                    self.run_background(tr("Empty Recycle Bin"), None, recycle::empty);
                } else {
                    self.run_background(tr("Delete"), None, move || recycle::purge(items));
                }
                self.tabs.active_mut().view.clear_selection();
            }
            Action::MoveToBin(paths) => {
                let dir = paths
                    .first()
                    .and_then(|p| p.parent())
                    .map(Path::to_path_buf);
                let undo = self.undo.recorder();
                self.run_background(tr("Move to Recycle Bin"), dir, move || {
                    let since = ox_core::undo::now();
                    let result = fsops::delete(&paths, true);
                    undo.trashed(&paths, since);
                    result
                });
            }
        }
    }

    pub(super) fn process_tab(&mut self, action: TabAction) {
        let s = &self.settings;
        match action {
            TabAction::New => {
                self.tabs.open(Location::Home, None, true, s);
            }
            TabAction::NewWindow => self.new_window(),
            TabAction::ToNewWindow(id, pos) => self.move_to_new_window(id, pos),
            TabAction::Close(id) => self.tabs.close(id),
            TabAction::Activate(id) => self.tabs.activate(id),
            TabAction::ToggleSplit => self.tabs.toggle_split(s),
            TabAction::SwitchPane => self.tabs.switch_pane(),
            TabAction::Duplicate(id) => self.tabs.duplicate(id, s),
            TabAction::Pin(id, true) => self.tabs.pin(id),
            TabAction::Pin(id, false) => self.tabs.unpin(id),
            TabAction::CloseOthers(id) => self.tabs.close_others(id),
            TabAction::NewGroup(id) => {
                let gid = self.tabs.new_group(id);
                self.dialog = Some(Dialog::RenameGroup {
                    id: gid,
                    text: String::new(),
                    focus: true,
                });
            }
            TabAction::AddToGroup(id, g) => self.tabs.add_to_group(id, g),
            TabAction::RemoveFromGroup(id) => self.tabs.remove_from_group(id),
            TabAction::ToggleGroup(g) => self.tabs.toggle_group(g),
            TabAction::CloseGroup(g) => self.tabs.close_group(g),
            TabAction::Ungroup(g) => self.tabs.ungroup(g),
            TabAction::RenameGroup(g) => {
                let text = self
                    .tabs
                    .group(g)
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                self.dialog = Some(Dialog::RenameGroup {
                    id: g,
                    text,
                    focus: true,
                });
            }
            TabAction::SetGroupColor(g, c) => {
                if let Some(g) = self.tabs.group_mut(g) {
                    g.color = c;
                }
            }
            TabAction::NewTabInGroup(g) => {
                let last = self
                    .tabs
                    .tabs
                    .iter()
                    .rev()
                    .find(|t| t.group == Some(g))
                    .map(|t| t.id);
                self.tabs.open(Location::Home, last, true, s);
            }
            TabAction::Move(id, to) => self.tabs.move_tab(id, to),
            TabAction::Cycle(d) => self.tabs.cycle(d),
        }
    }
}
