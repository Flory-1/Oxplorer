//! Application state and the action dispatcher.
//!
//! UI code never mutates state directly in complex ways; instead it pushes
//! [`Action`]s which are processed at the end of each frame. This keeps the
//! borrow checker happy and makes every user command go through one place.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use egui::{Context, Key, Modifiers};
use ox_core::archive;
use ox_core::clipboard::{ClipMode, FileClipboard};
use ox_core::dirsize::FolderSizes;
use ox_core::entry::{self, FileEntry};
use ox_core::index::{IndexService, IndexState};
use ox_core::places::{self, Drive, QuickAccess};
use ox_core::recycle::{self, TrashItem};
use ox_core::search::{self, Query, SearchEvent, SearchOptions};
use ox_core::settings::{Settings, StartPage};
use ox_core::tags::{TagColor, TagStore};
use ox_core::transfer::{TransferEngine, TransferKind, TransferRequest};
use ox_core::watcher::DirWatcher;
use ox_core::{format, fsops};

use crate::tabs::{GroupId, Listing, Location, SearchState, Session, TabId, Tabs};
use crate::{theme, ui};

pub enum Action {
    Navigate(Location),
    OpenInNewTab(Location, bool),
    Back,
    Forward,
    Up,
    Refresh,
    /// Open a folder (navigate) or file (default app).
    Open(PathBuf),
    CopyItems(Vec<PathBuf>),
    CutItems(Vec<PathBuf>),
    Paste(PathBuf),
    Transfer(TransferKind, Vec<PathBuf>, PathBuf),
    /// Items dropped onto a folder; the kind is decided from modifiers and drives.
    Drop(Vec<PathBuf>, PathBuf),
    Delete(Vec<PathBuf>, bool),
    DeleteConfirmed(Vec<PathBuf>, bool),
    Rename(PathBuf),
    /// Rename several items at once with a pattern.
    BulkRename(Vec<PathBuf>),
    /// Add the tag to all items, or remove it if all have it.
    ToggleTag(Vec<PathBuf>, TagColor),
    ClearTags(Vec<PathBuf>),
    /// Extract a whole archive next to it: into a new folder named after it,
    /// or straight into the archive's folder.
    ExtractArchive {
        archive: PathBuf,
        own_folder: bool,
    },
    /// Put these items into a new zip file next to them.
    Compress(Vec<PathBuf>),
    /// Copy a theme file into the themes folder and use it.
    InstallTheme(PathBuf),
    /// Show or hide the terminal panel (starting the shell the first time).
    ToggleTerminal,
    RestartTerminal,
    /// Type `cd <folder>` into the terminal.
    TerminalCd(PathBuf),
    /// Items were renamed or moved (old path, new path): tags go along.
    PathsMoved(Vec<(PathBuf, PathBuf)>),
    /// Give a tag colour a custom name (empty = default).
    RenameTag(TagColor, String),
    NewFolder(PathBuf),
    NewFile(PathBuf),
    Pin(PathBuf),
    Unpin(PathBuf),
    MoveFavorite(PathBuf, isize),
    CopyPathText(Vec<PathBuf>),
    OpenTerminal(PathBuf),
    Reveal(PathBuf),
    Properties(Vec<PathBuf>),
    Search(Option<PathBuf>, String),
    /// Search while typing: updates the current search tab instead of adding history.
    SearchLive(Option<PathBuf>, String),
    SaveSearch(Option<PathBuf>, String),
    RemoveSavedSearch(usize),
    RebuildIndex,
    SelectAll,
    ForgetRecent(PathBuf),
    ClearRecent,
    SettingsChanged,
    Tab(TabAction),
    Toast(String),
    /// Windows: show the Explorer context menu ("Show more options").
    ShellMenu(Vec<PathBuf>),
    /// Windows: "Open with…" dialog.
    OpenWith(PathBuf),
    /// Recycle Bin: put entries (by their bin path) back where they came from.
    RestoreFromBin(Vec<PathBuf>),
    /// Recycle Bin: delete entries for good (asks first).
    PurgeFromBin(Vec<PathBuf>),
    EmptyBin,
    PurgeConfirmed(Vec<TrashItem>, bool),
    /// Move items to the Recycle Bin without asking (e.g. dropped onto it).
    MoveToBin(Vec<PathBuf>),
}

pub enum TabAction {
    New,
    Close(TabId),
    Activate(TabId),
    Duplicate(TabId),
    CloseOthers(TabId),
    NewGroup(TabId),
    AddToGroup(TabId, GroupId),
    RemoveFromGroup(TabId),
    ToggleGroup(GroupId),
    CloseGroup(GroupId),
    Ungroup(GroupId),
    RenameGroup(GroupId),
    SetGroupColor(GroupId, usize),
    NewTabInGroup(GroupId),
    Move(TabId, usize),
    Cycle(isize),
    /// Turn the split view (two panes) on or off.
    ToggleSplit,
    /// Focus the other pane.
    SwitchPane,
}

pub enum Dialog {
    BulkRename {
        items: Vec<ox_core::rename::RenameItem>,
        rule: ox_core::rename::RenameRule,
        focus: bool,
    },
    Rename {
        path: PathBuf,
        text: String,
        error: Option<String>,
        focus: bool,
    },
    /// Permanently delete Recycle Bin items, or everything if `everything` is set.
    ConfirmPurge {
        items: Vec<TrashItem>,
        everything: bool,
    },
    ConfirmDelete {
        paths: Vec<PathBuf>,
        permanent: bool,
    },
    Properties(Properties),
    RenameGroup {
        id: GroupId,
        text: String,
        focus: bool,
    },
}

pub struct Properties {
    pub paths: Vec<PathBuf>,
    pub entries: Vec<FileEntry>,
    /// (bytes, files, folders), computed in the background.
    pub totals: Option<(u64, u64, u64)>,
    pub rx: Option<Receiver<(u64, u64, u64)>>,
    pub cancel: Arc<AtomicBool>,
}

/// A file operation running on a helper thread (label, result, folder to refresh).
type BackgroundOp = (String, Receiver<Result<(), String>>, Option<PathBuf>);

pub struct Toast {
    pub text: String,
    pub until: Instant,
}

/// Transient UI state that doesn't belong to a tab.
pub struct UiState {
    pub address_editing: bool,
    pub address_text: String,
    pub focus_address: bool,
    pub focus_search: bool,
    pub show_transfers: bool,
    pub settings_section: usize,
    pub palette: Option<crate::ui::palette::PaletteState>,
    /// The settings page is waiting for keys for this command.
    pub recording: Option<crate::shortcuts::Command>,
    pub show_terminal: bool,
    pub help: crate::ui::help_page::HelpState,
    /// The terminal has the keyboard (app shortcuts are off then).
    pub terminal_focused: bool,
    type_ahead: String,
    type_ahead_at: Instant,
}

pub struct OxApp {
    pub settings: Settings,
    pub quick: QuickAccess,
    pub engine: TransferEngine,
    pub tabs: Tabs,
    pub clipboard: FileClipboard,
    pub drives: Vec<Drive>,
    pub actions: Vec<Action>,
    pub dialog: Option<Dialog>,
    pub toasts: Vec<Toast>,
    pub ui: UiState,
    pub ctx: Context,
    /// Watch the folders of the focused and the other pane.
    watchers: [DirWatcher; 2],
    drives_at: Instant,
    refresh_at: [Option<Instant>; 2],
    background: Vec<BackgroundOp>,
    seen_jobs: HashSet<u64>,
    text_clipboard: Option<arboard::Clipboard>,
    applied_settings: Option<Settings>,
    pub icons: crate::icons::IconCache,
    pub thumbs: crate::thumbs::Thumbnails,
    pub index: IndexService,
    pub preview: crate::ui::preview::PreviewState,
    pub folder_sizes: FolderSizes,
    /// Keyboard shortcuts in effect (defaults + the user's changes).
    pub keymap: crate::shortcuts::Keymap,
    /// Colour schemes: built-in ones and the user's (see `ox_core::themes`).
    pub themes: Vec<ox_core::themes::ColorTheme>,
    /// A scheme was edited; apply the look again.
    pub themes_changed: bool,
    /// A native drag ended inside its own message loop, so egui never saw
    /// the mouse button go up; tell it on the next frame.
    release_pointer: bool,
    /// The terminal panel's shell, started the first time the panel opens.
    pub terminal: Option<crate::terminal::Terminal>,
    /// Folder the terminal was last sent to (for "follow the folder").
    terminal_dir: Option<PathBuf>,
    pub tags: TagStore,
    /// (tabs, listing versions, cache epoch) the folder sizes were requested for.
    sizes_key: Option<(Vec<(TabId, u64)>, u64)>,
    /// `folder_sizes.version()` last copied into the listing.
    sizes_applied: u64,
    /// Listings (tab, version) the Git status was requested for.
    git_key: Vec<(TabId, u64)>,
    /// Running `git status` calls: tab, repository root, result.
    git_pending: Vec<(
        TabId,
        PathBuf,
        Receiver<std::io::Result<ox_core::git::RepoStatus>>,
    )>,
    /// The index was (re)built at least once this session.
    index_started: bool,
    index_next: Option<Instant>,
    /// Index-related settings the current index was built with.
    index_settings: Option<(Vec<PathBuf>, Vec<String>)>,
}

impl OxApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let settings = Settings::load();
        let quick = QuickAccess::load();

        let engine = TransferEngine::new(settings.transfer.engine_config());
        let repaint = ctx.clone();
        engine.set_notifier(move || repaint.request_repaint());

        let mut tabs = match settings.startup.start_page {
            StartPage::RestoreSession => {
                Session::load().and_then(|s| Tabs::from_session(s, &settings))
            }
            StartPage::Folder => settings
                .startup
                .start_folder
                .clone()
                .filter(|p| p.is_dir())
                .map(|p| Tabs::new(Location::Folder(p), &settings)),
            StartPage::Home => None,
        }
        .unwrap_or_else(|| Tabs::new(Location::Home, &settings));
        if let Some(path) = open {
            tabs.open(Location::Folder(path), None, true, &settings);
        }

        let watcher = || {
            let repaint = ctx.clone();
            DirWatcher::new(move || repaint.request_repaint())
        };
        let watchers = [watcher(), watcher()];

        Self {
            settings,
            quick,
            engine,
            tabs,
            clipboard: FileClipboard::default(),
            drives: places::drives(),
            actions: Vec::new(),
            dialog: None,
            toasts: Vec::new(),
            ui: UiState {
                address_editing: false,
                address_text: String::new(),
                focus_address: false,
                focus_search: false,
                show_transfers: true,
                settings_section: 0,
                palette: None,
                recording: None,
                show_terminal: false,
                help: Default::default(),
                terminal_focused: false,
                type_ahead: String::new(),
                type_ahead_at: Instant::now(),
            },
            ctx,
            watchers,
            drives_at: Instant::now(),
            refresh_at: [None; 2],
            background: Vec::new(),
            seen_jobs: HashSet::new(),
            text_clipboard: None,
            applied_settings: None,
            icons: crate::icons::IconCache::new(&cc.egui_ctx),
            thumbs: crate::thumbs::Thumbnails::new(&cc.egui_ctx),
            index: {
                let repaint = cc.egui_ctx.clone();
                IndexService::start(ox_core::index::default_cache_path(), move || {
                    repaint.request_repaint()
                })
            },
            preview: Default::default(),
            folder_sizes: {
                let repaint = cc.egui_ctx.clone();
                FolderSizes::new(move || repaint.request_repaint())
            },
            tags: TagStore::load(),
            keymap: Default::default(),
            themes: ox_core::themes::load_all(),
            themes_changed: false,
            release_pointer: false,
            terminal: None,
            terminal_dir: None,
            sizes_key: None,
            git_key: Vec::new(),
            git_pending: Vec::new(),
            sizes_applied: 0,
            index_started: false,
            index_next: None,
            index_settings: None,
        }
    }

    pub fn push(&mut self, a: Action) {
        self.actions.push(a);
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            until: Instant::now() + Duration::from_secs(4),
        });
    }

    pub fn current_folder(&self) -> Option<PathBuf> {
        self.tabs
            .active()
            .location()
            .folder()
            .map(Path::to_path_buf)
    }

    pub fn drives(&mut self) -> &[Drive] {
        if self.drives_at.elapsed() > Duration::from_secs(5) {
            self.drives = places::drives();
            self.drives_at = Instant::now();
        }
        &self.drives
    }

    // ---- per-frame housekeeping --------------------------------------------

    fn pre_frame(&mut self, ctx: &Context) {
        if self.applied_settings.as_ref() != Some(&self.settings) || self.themes_changed {
            self.themes_changed = false;
            theme::apply(ctx, &self.settings, &self.themes);
            self.keymap = crate::shortcuts::Keymap::new(&self.settings.shortcuts);
            crate::shortcuts::set_current(&self.keymap);
            self.engine
                .set_config(self.settings.transfer.engine_config());
            self.applied_settings = Some(self.settings.clone());
        }
        self.remap_text_clipboard_keys(ctx);
        self.icons.poll(ctx);
        self.thumbs.poll(ctx);
        self.manage_index();
        self.tabs.fix_panes();
        self.load_active_tab();
        if let Some(other) = self.tabs.split_partner() {
            // Load the other pane's tab too (`load_active_tab` works on the active one).
            let focused = self.tabs.active;
            self.tabs.active = other;
            self.load_active_tab();
            self.tabs.active = focused;
        }
        self.poll_tabs();
        self.poll_folder_sizes();
        self.poll_git();
        self.poll_watcher();
        self.poll_jobs();
        self.poll_background();
        self.poll_dropped_files(ctx);
        self.follow_folder_in_terminal();
        if self.ui.show_terminal && self.ui.terminal_focused {
            // The terminal gets every key except its own toggle.
            let toggle = ctx.input_mut(|i| {
                !self
                    .keymap
                    .pressed(i, |c| c == crate::shortcuts::Command::ToggleTerminal)
                    .is_empty()
            });
            if toggle {
                self.push(Action::ToggleTerminal);
            }
        } else if self.dialog.is_none()
            && !ctx.text_edit_focused()
            // Not while a new shortcut is being recorded in the settings.
            && self.ui.recording.is_none()
        {
            self.handle_shortcuts(ctx);
        }
        self.toasts.retain(|t| t.until > Instant::now());
        if !self.toasts.is_empty() || self.engine.has_active() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// Clipboard shortcuts are disabled in eframe so that Ctrl+V works even when
    /// the clipboard holds files instead of text. Text fields still need them,
    /// so translate the keys back into clipboard events while one has focus.
    fn remap_text_clipboard_keys(&mut self, ctx: &Context) {
        if !ctx.text_edit_focused() {
            return;
        }
        let wants_paste = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command_only())
            })
        });
        let paste_text = if wants_paste {
            if self.text_clipboard.is_none() {
                self.text_clipboard = arboard::Clipboard::new().ok();
            }
            self.text_clipboard.as_mut().and_then(|c| c.get_text().ok())
        } else {
            None
        };
        ctx.input_mut(|i| {
            for ev in &mut i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = ev
                {
                    if !modifiers.command_only() {
                        continue;
                    }
                    match key {
                        Key::C => *ev = egui::Event::Copy,
                        Key::X => *ev = egui::Event::Cut,
                        Key::V => {
                            if let Some(t) = &paste_text {
                                *ev = egui::Event::Paste(t.replace("\r\n", "\n"));
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    fn load_active_tab(&mut self) {
        let show_hidden = self.settings.browsing.show_hidden;
        let ctx = self.ctx.clone();
        let search_opts = SearchOptions {
            include_hidden: self.settings.search.include_hidden,
            case_sensitive: self.settings.search.case_sensitive,
            max_results: self.settings.search.max_results,
            threads: self.settings.search.threads,
        };
        let drives: Vec<PathBuf> = self.drives.iter().map(|d| d.mount.clone()).collect();
        let index = self
            .settings
            .search
            .index_enabled
            .then(|| self.index.index())
            .flatten();
        let tab = self.tabs.active_mut();
        if !tab.stale {
            return;
        }
        tab.stale = false;
        match tab.location().clone() {
            Location::Folder(path) => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Listing::folder(&path, show_hidden).map_err(|e| e.to_string()));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Search { root, query } => {
                let q = Query::parse(&query, search_opts.case_sensitive);
                tab.view = Default::default();
                let repaint = move || ctx.request_repaint();
                // Answer from the index when it covers the place we search in.
                let handle = match index {
                    Some(idx) if root.as_deref().is_none_or(|r| idx.covers(r)) => {
                        search::start_indexed(idx, root, q, search_opts, repaint)
                    }
                    _ => {
                        let roots = root.map(|r| vec![r]).unwrap_or(drives);
                        search::start(roots, q, search_opts, repaint)
                    }
                };
                tab.search = Some(SearchState {
                    handle: Some(handle),
                    done: None,
                });
            }
            Location::RecycleBin => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Listing::recycle_bin());
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Archive { archive, inner } => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Listing::archive(&archive, &inner).map_err(|e| e.to_string()));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Tagged(color) => {
                let paths = self.tags.with_tag(color);
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Ok(Listing::items(&paths)));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Home | Location::Settings | Location::Help => {}
        }
    }

    fn poll_tabs(&mut self) {
        let folders_first = self.settings.browsing.folders_first;
        let case = self.settings.search.case_sensitive;
        for tab in &mut self.tabs.tabs {
            if let Some(rx) = &tab.view.loading {
                if let Ok(result) = rx.try_recv() {
                    tab.view.loading = None;
                    match result {
                        Ok(listing) => {
                            let existing: HashSet<&PathBuf> =
                                listing.entries.iter().map(|e| &e.path).collect();
                            tab.view.selection.retain(|p| existing.contains(p));
                            tab.view.entries = listing.entries;
                            tab.view.bin = listing.bin;
                            tab.view.version += 1;
                            tab.view.error = None;
                        }
                        Err(e) => {
                            tab.view.entries.clear();
                            tab.view.error = Some(e);
                        }
                    }
                    tab.refresh_visible(folders_first, case);
                }
            }
            let mut changed = false;
            if let Some(search) = &mut tab.search {
                if let Some(handle) = &search.handle {
                    for ev in handle.events.try_iter() {
                        match ev {
                            SearchEvent::Results(mut r) => {
                                tab.view.entries.append(&mut r);
                                changed = true;
                            }
                            SearchEvent::Done {
                                elapsed,
                                scanned,
                                truncated,
                                from_index,
                            } => {
                                let limit = if truncated { " · limit reached" } else { "" };
                                let results =
                                    format::count(tab.view.entries.len() as u64, "result");
                                search.done = Some(if from_index {
                                    format!(
                                        "{results} in {} ms · ⚡ from the index ({} items){limit}",
                                        elapsed.as_millis(),
                                        scanned
                                    )
                                } else {
                                    format!(
                                        "{results} · {scanned} items scanned in {:.2}s{limit}",
                                        elapsed.as_secs_f32()
                                    )
                                });
                                changed = true;
                            }
                        }
                    }
                }
                if search.done.is_some() {
                    search.handle = None;
                }
            }
            if changed {
                tab.refresh_visible(folders_first, case);
            }
        }
    }

    fn poll_watcher(&mut self) {
        let panes = [Some(self.tabs.active), self.tabs.split_partner()];
        for (i, id) in panes.into_iter().enumerate() {
            let folder = id
                .and_then(|id| self.tabs.get(id))
                .and_then(|t| t.location().folder().map(Path::to_path_buf))
                .filter(|_| self.settings.browsing.auto_refresh);
            self.watchers[i].watch(folder.as_deref());
            if self.watchers[i].take_dirty() && self.refresh_at[i].is_none() {
                // Debounce bursts of file system events.
                self.refresh_at[i] = Some(Instant::now() + Duration::from_millis(250));
            }
            if let Some(at) = self.refresh_at[i] {
                if Instant::now() >= at {
                    self.refresh_at[i] = None;
                    if let Some(folder) = &folder {
                        self.folder_sizes.invalidate(folder);
                        self.tabs.mark_stale(Some(folder));
                    }
                } else {
                    self.ctx.request_repaint_after(at - Instant::now());
                }
            }
        }
    }

    fn poll_jobs(&mut self) {
        for job in self.engine.jobs() {
            if !job.state().is_finished() || !self.seen_jobs.insert(job.id) {
                continue;
            }
            if let Err(e) = ox_core::transfer::history::append(&job) {
                eprintln!("could not write transfer log: {e}");
            }
            self.mark_changed(Some(&job.destination));
            if job.kind == TransferKind::Move && !self.tags.is_empty() {
                // Tags follow moved items (those that left their old place).
                let moved = job
                    .sources
                    .iter()
                    .filter_map(|src| {
                        let to = job.destination.join(src.file_name()?);
                        (!src.exists() && to.exists()).then(|| (src.clone(), to))
                    })
                    .collect();
                self.push(Action::PathsMoved(moved));
            }
            for src in &job.sources {
                if let Some(parent) = src.parent() {
                    self.mark_changed(Some(parent));
                }
            }
            let errors = job.error_count();
            if errors > 0 {
                self.toast(format!(
                    "Transfer finished with {}. See the transfer panel.",
                    format::count(errors as u64, "error")
                ));
                self.ui.show_transfers = true;
            }
        }
    }

    fn poll_background(&mut self) {
        let mut finished = Vec::new();
        self.background
            .retain(|(label, rx, dir)| match rx.try_recv() {
                Ok(result) => {
                    finished.push((label.clone(), result, dir.clone()));
                    false
                }
                Err(crossbeam_channel::TryRecvError::Empty) => true,
                Err(_) => false,
            });
        for (label, result, dir) in finished {
            if let Err(e) = result {
                self.toast(format!("{label} failed: {e}"));
            }
            self.mark_changed(dir.as_deref());
        }
    }

    /// Save the tag store and reload tag views.
    pub fn tags_changed(&mut self) {
        if let Err(e) = self.tags.save() {
            self.toast(format!("Could not save tags: {e}"));
        }
        for t in &mut self.tabs.tabs {
            if matches!(t.location(), Location::Tagged(_)) {
                t.stale = true;
            }
        }
    }

    /// Files in `dir` changed: reload tabs showing it and forget folder sizes
    /// that included it.
    pub fn mark_changed(&mut self, dir: Option<&Path>) {
        if let Some(d) = dir {
            self.folder_sizes.invalidate(d);
        }
        self.tabs.mark_stale(dir);
    }

    /// Ask for the sizes of the folders listed in the active tab, and copy
    /// finished sizes into the listing (so sorting by size works).
    fn poll_folder_sizes(&mut self) {
        // The focused pane first, so its sizes come first.
        let panes: Vec<TabId> = [Some(self.tabs.active), self.tabs.split_partner()]
            .into_iter()
            .flatten()
            .filter(|&id| {
                self.tabs.get(id).is_some_and(|t| {
                    matches!(t.location(), Location::Folder(_)) && t.view.loading.is_none()
                })
            })
            .collect();
        if !self.settings.browsing.folder_sizes || panes.is_empty() {
            if self.sizes_key.take().is_some() {
                self.folder_sizes.clear_requests();
            }
            return;
        }
        let versions: Vec<(TabId, u64)> = panes
            .iter()
            .filter_map(|&id| self.tabs.get(id).map(|t| (id, t.view.version)))
            .collect();
        let key = (versions, self.folder_sizes.epoch());
        if self.sizes_key.as_ref() != Some(&key) {
            self.sizes_key = Some(key);
            let dirs = panes
                .iter()
                .filter_map(|&id| self.tabs.get(id))
                .flat_map(|t| t.view.entries.iter())
                .filter(|e| e.is_dir && !e.is_symlink)
                .map(|e| e.path.clone())
                .collect();
            self.folder_sizes.request(dirs);
            self.sizes_applied = u64::MAX;
        }
        let version = self.folder_sizes.version();
        if version == self.sizes_applied {
            return;
        }
        self.sizes_applied = version;
        let (folders_first, case) = (
            self.settings.browsing.folders_first,
            self.settings.search.case_sensitive,
        );
        for id in panes {
            let Some(tab) = self.tabs.get_mut(id) else {
                continue;
            };
            let mut changed = false;
            for e in tab.view.entries.iter_mut().filter(|e| e.is_dir) {
                if let Some(s) = self.folder_sizes.get(&e.path) {
                    changed |= e.size != s.bytes;
                    e.size = s.bytes;
                }
            }
            if changed && tab.sort_key == entry::SortKey::Size {
                tab.refresh_visible(folders_first, case);
            }
        }
    }

    /// Ask Git for the status of the repositories the panes show, whenever
    /// their listings reload, and hand the results to the tabs.
    fn poll_git(&mut self) {
        let panes: Vec<TabId> = [Some(self.tabs.active), self.tabs.split_partner()]
            .into_iter()
            .flatten()
            .collect();
        let key: Vec<(TabId, u64)> = panes
            .iter()
            .filter_map(|&id| self.tabs.get(id).map(|t| (id, t.view.version)))
            .collect();
        if key != self.git_key {
            self.git_key = key;
            for &id in &panes {
                let Some(tab) = self.tabs.get_mut(id) else {
                    continue;
                };
                let root = tab
                    .location()
                    .folder()
                    .filter(|_| self.settings.browsing.git_status)
                    .and_then(ox_core::git::repo_root);
                let Some(root) = root else {
                    tab.view.git = None;
                    continue;
                };
                if tab.view.git.as_ref().is_some_and(|g| g.root != root) {
                    tab.view.git = None;
                }
                if self
                    .git_pending
                    .iter()
                    .any(|(t, r, _)| *t == id && *r == root)
                {
                    continue;
                }
                let (tx, rx) = crossbeam_channel::bounded(1);
                let ctx = self.ctx.clone();
                let dir = root.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(ox_core::git::status(&dir));
                    ctx.request_repaint();
                });
                self.git_pending.push((id, root, rx));
            }
        }
        let mut done = Vec::new();
        self.git_pending
            .retain(|(id, root, rx)| match rx.try_recv() {
                Ok(result) => {
                    done.push((*id, root.clone(), result));
                    false
                }
                Err(crossbeam_channel::TryRecvError::Empty) => true,
                Err(_) => false,
            });
        for (id, root, result) in done {
            let Some(tab) = self.tabs.get_mut(id) else {
                continue;
            };
            // Only if the tab is still in that repository.
            let same = tab
                .location()
                .folder()
                .is_some_and(|f| f.starts_with(&root));
            tab.view.git = match result {
                Ok(status) if same => Some(Arc::new(status)),
                _ => None,
            };
        }
    }

    /// Files dropped onto the window from other applications.
    fn poll_dropped_files(&mut self, ctx: &Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        match self.current_folder() {
            Some(dest) => self.push(Action::Drop(dropped, dest)),
            None => self.toast("Open a folder to drop files into it."),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        use crate::shortcuts::Command as C;
        use egui::PointerButton;
        let folder = self.current_folder();
        let selection = self.tabs.active().view.selected_paths();
        let shows_files = self.tabs.active().location().shows_files();
        let in_bin = self.tabs.active().location().is_bin();
        let read_only = self.tabs.active().location().is_read_only();
        let has_sel = !selection.is_empty();

        // Which commands make sense here; only their keys are taken.
        let available = |c: C| match c {
            C::SelectAll => shows_files,
            // The Recycle Bin purges on Delete; archives are read-only.
            C::Delete | C::DeletePermanently => shows_files && has_sel && !read_only,
            C::Copy | C::CopyPath => shows_files && has_sel && !in_bin,
            C::Cut | C::Rename | C::Properties => shows_files && has_sel && !in_bin && !read_only,
            C::ShellMenu => {
                shows_files && has_sel && !in_bin && !read_only && ox_core::shell::supported()
            }
            C::Paste | C::NewFolder => shows_files && folder.is_some(),
            _ => true,
        };
        let (pressed, back, forward) = ctx.input_mut(|i| {
            (
                self.keymap.pressed(i, available),
                i.pointer.button_pressed(PointerButton::Extra1),
                i.pointer.button_pressed(PointerButton::Extra2),
            )
        });
        let mut actions = Vec::new();
        if back {
            actions.push(Action::Back);
        }
        if forward {
            actions.push(Action::Forward);
        }
        for c in pressed {
            match c {
                C::NewTab => actions.push(Action::Tab(TabAction::New)),
                C::CloseTab => actions.push(Action::Tab(TabAction::Close(self.tabs.active))),
                C::NextTab => actions.push(Action::Tab(TabAction::Cycle(1))),
                C::PrevTab => actions.push(Action::Tab(TabAction::Cycle(-1))),
                C::ToggleSplit => actions.push(Action::Tab(TabAction::ToggleSplit)),
                C::SwitchPane => actions.push(Action::Tab(TabAction::SwitchPane)),
                C::Back => actions.push(Action::Back),
                C::Forward => actions.push(Action::Forward),
                C::Up => actions.push(Action::Up),
                C::Refresh => actions.push(Action::Refresh),
                C::EditAddress => {
                    self.ui.address_editing = true;
                    self.ui.focus_address = true;
                }
                C::Search => self.ui.focus_search = true,
                C::Palette => self.ui.palette = Some(Default::default()),
                C::TogglePreview => {
                    self.settings.browsing.show_preview = !self.settings.browsing.show_preview;
                    actions.push(Action::SettingsChanged);
                }
                C::Settings => actions.push(Action::OpenInNewTab(Location::Settings, true)),
                C::Help => actions.push(Action::OpenInNewTab(Location::Help, true)),
                C::SelectAll => actions.push(Action::SelectAll),
                C::Copy => actions.push(Action::CopyItems(selection.clone())),
                C::Cut => actions.push(Action::CutItems(selection.clone())),
                C::CopyPath => actions.push(Action::CopyPathText(selection.clone())),
                C::Paste => actions.extend(folder.clone().map(Action::Paste)),
                C::NewFolder => actions.extend(folder.clone().map(Action::NewFolder)),
                C::Delete | C::DeletePermanently if in_bin => {
                    actions.push(Action::PurgeFromBin(selection.clone()));
                }
                C::Delete => actions.push(Action::Delete(selection.clone(), false)),
                C::DeletePermanently => actions.push(Action::Delete(selection.clone(), true)),
                C::Rename => match selection.len() {
                    1 => actions.push(Action::Rename(selection[0].clone())),
                    _ => actions.push(Action::BulkRename(selection.clone())),
                },
                C::Properties => actions.push(Action::Properties(selection.clone())),
                C::ShellMenu => actions.push(Action::ShellMenu(selection.clone())),
                C::ToggleTerminal => actions.push(Action::ToggleTerminal),
            }
        }
        self.actions.extend(actions);
        if shows_files {
            self.keyboard_navigation(ctx);
        }
    }

    /// Arrow keys, Home/End, Enter and type-to-select in the file view.
    fn keyboard_navigation(&mut self, ctx: &Context) {
        let tab = self.tabs.active_mut();
        let n = tab.view.visible.len();
        if n == 0 {
            return;
        }
        let mut target: Option<usize> = None;
        let mut extend = false;
        let mut open = false;
        let mut typed = String::new();
        let grid_cols = ctx
            .data(|d| d.get_temp::<usize>(egui::Id::unique(("grid_columns", tab.id))))
            .unwrap_or(1)
            .max(1);
        let step = if tab.view_mode == ox_core::settings::ViewMode::Grid {
            grid_cols
        } else {
            1
        };
        ctx.input_mut(|i| {
            let cur = tab.view.cursor;
            extend = i.modifiers.shift;
            for (key, delta) in [
                (Key::ArrowDown, step as isize),
                (Key::ArrowUp, -(step as isize)),
                (Key::ArrowRight, if step > 1 { 1 } else { 0 }),
                (Key::ArrowLeft, if step > 1 { -1 } else { 0 }),
                (Key::PageDown, 20),
                (Key::PageUp, -20),
            ] {
                if delta != 0
                    && (i.consume_key(Modifiers::NONE, key) || i.consume_key(Modifiers::SHIFT, key))
                {
                    target = Some(match cur {
                        Some(c) => (c as isize + delta).clamp(0, n as isize - 1) as usize,
                        None => 0,
                    });
                }
            }
            if i.consume_key(Modifiers::NONE, Key::Home)
                || i.consume_key(Modifiers::SHIFT, Key::Home)
            {
                target = Some(0);
            }
            if i.consume_key(Modifiers::NONE, Key::End) || i.consume_key(Modifiers::SHIFT, Key::End)
            {
                target = Some(n - 1);
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                open = true;
            }
            if !i.modifiers.command && !i.modifiers.alt {
                for ev in &i.events {
                    if let egui::Event::Text(t) = ev {
                        typed.push_str(t);
                    }
                }
            }
        });

        if !typed.trim().is_empty() {
            if self.ui.type_ahead_at.elapsed() > Duration::from_millis(900) {
                self.ui.type_ahead.clear();
            }
            self.ui.type_ahead.push_str(&typed.to_lowercase());
            self.ui.type_ahead_at = Instant::now();
            let prefix = self.ui.type_ahead.clone();
            target = (0..n).find(|&i| {
                tab.view
                    .visible_entry(i)
                    .is_some_and(|e| e.name.to_lowercase().starts_with(&prefix))
            });
            extend = false;
        }

        if let Some(t) = target {
            if extend {
                if tab.view.anchor.is_none() {
                    tab.view.anchor = tab.view.cursor.or(Some(t));
                }
                tab.view.select_range(t, false);
            } else {
                tab.view.select_only(t);
            }
            tab.view.scroll_to = Some(t);
        }
        if open && !tab.location().is_bin() {
            let selected = tab.view.selected_paths();
            for p in selected {
                self.actions.push(Action::Open(p));
            }
        }
    }

    // ---- action processing ---------------------------------------------------

    fn process_actions(&mut self) {
        let actions = std::mem::take(&mut self.actions);
        for a in actions {
            self.process(a);
        }
    }

    fn process(&mut self, action: Action) {
        let system_clip = self.settings.transfer.use_system_clipboard;
        match action {
            Action::Navigate(loc) => self.navigate(loc),
            Action::OpenInNewTab(page @ (Location::Settings | Location::Help), _) => {
                // Reuse an open Settings/Help tab; never put it into a tab group.
                match self.tabs.tabs.iter().find(|t| *t.location() == page) {
                    Some(t) => {
                        let id = t.id;
                        self.tabs.activate(id);
                    }
                    None => {
                        self.tabs.open(page, None, true, &self.settings);
                    }
                }
            }
            Action::OpenInNewTab(loc, activate) => {
                let after = Some(self.tabs.active);
                self.tabs.open(loc, after, activate, &self.settings);
            }
            Action::Back => self.tabs.active_mut().back(),
            Action::Forward => self.tabs.active_mut().forward(),
            Action::Up => self.tabs.active_mut().up(),
            Action::Refresh => {
                if let Some(folder) = self.current_folder() {
                    self.folder_sizes.invalidate_tree(&folder);
                }
                self.tabs.active_mut().stale = true;
                self.drives_at = Instant::now() - Duration::from_secs(60);
            }
            Action::Open(path) => {
                let in_archive = archive::split_virtual(&path)
                    .filter(|(_, inner)| !inner.as_os_str().is_empty());
                if path.is_dir() {
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
                        Err(e) => self.toast(format!("Couldn't open {}: {e}", path.display())),
                    }
                    self.save_quick();
                }
            }
            Action::CopyItems(paths) => {
                let n = paths.len();
                self.clipboard.set(paths, ClipMode::Copy, system_clip);
                self.toast(format!("{} copied", format::count(n as u64, "item")));
            }
            Action::CutItems(paths) => {
                let n = paths.len();
                self.clipboard.set(paths, ClipMode::Cut, system_clip);
                self.toast(format!("{} cut", format::count(n as u64, "item")));
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
                None => self.toast("The clipboard doesn't contain any files."),
            },
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
                self.run_background("Delete", dir, move || fsops::delete(&paths, to_trash));
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
            Action::NewFolder(dir) => match fsops::create_folder(&dir, "New folder") {
                Ok(p) => {
                    self.after_create(&dir, p);
                }
                Err(e) => self.toast(format!("Couldn't create folder: {e}")),
            },
            Action::NewFile(dir) => match fsops::create_file(&dir, "New Text Document.txt") {
                Ok(p) => self.after_create(&dir, p),
                Err(e) => self.toast(format!("Couldn't create file: {e}")),
            },
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
                self.toast(format!("Compressing to {}…", entry::display_name(&zip)));
                self.tabs.active_mut().view.select_after_load = Some(zip.clone());
                self.run_background("Compressing", Some(dir), move || {
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
                    self.toast(format!("Theme \"{}\" installed and in use.", t.name));
                }
                Err(e) => self.toast(format!("This isn't a theme file: {e}")),
            },
            Action::ToggleTerminal => {
                self.ui.show_terminal = !self.ui.show_terminal;
                self.ui.terminal_focused = false;
                if self.ui.show_terminal && self.terminal.is_none() {
                    self.start_terminal();
                }
            }
            Action::RestartTerminal => self.start_terminal(),
            Action::TerminalCd(dir) => {
                if let Some(t) = &self.terminal {
                    t.cd(&dir);
                    self.terminal_dir = Some(dir);
                }
            }
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
                self.toast("Path copied");
            }
            Action::OpenTerminal(dir) => {
                if let Err(e) = fsops::open_terminal(&dir) {
                    self.toast(format!("Couldn't open a terminal: {e}"));
                }
            }
            Action::Reveal(p) => {
                if let Err(e) = fsops::reveal_in_system(&p) {
                    self.toast(format!("Couldn't open the system file manager: {e}"));
                }
            }
            Action::Properties(paths) => match paths.as_slice() {
                // The native dialog shows everything Explorer shows (security, versions, …).
                [one] if ox_core::shell::supported() => self.icons.shell.properties(one.clone()),
                _ => self.open_properties(paths),
            },
            Action::ShellMenu(paths) => self.icons.shell.context_menu(paths),
            Action::OpenWith(path) => self.icons.shell.open_with(path),
            Action::Search(root, query) => {
                if query.trim().is_empty() {
                    return;
                }
                self.navigate(Location::Search { root, query });
            }
            Action::SearchLive(root, query) => {
                let tab = self.tabs.active_mut();
                let in_search = matches!(tab.location(), Location::Search { .. });
                if query.trim().is_empty() {
                    if in_search {
                        tab.back();
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
                self.toast("Search saved to the sidebar");
            }
            Action::RemoveSavedSearch(i) => {
                if i < self.quick.saved_searches.len() {
                    self.quick.saved_searches.remove(i);
                    self.save_quick();
                }
            }
            Action::RebuildIndex => {
                self.index_started = false;
            }
            Action::SelectAll => self.tabs.active_mut().view.select_all(),
            Action::ForgetRecent(p) => {
                self.quick.forget(&p);
                self.save_quick();
            }
            Action::ClearRecent => {
                self.quick.clear_recent();
                self.save_quick();
            }
            Action::SettingsChanged => {
                if let Err(e) = self.settings.save() {
                    self.toast(format!("Couldn't save settings: {e}"));
                }
                // Hidden-file setting etc. affect listings.
                self.tabs.mark_stale(None);
            }
            Action::Tab(t) => self.process_tab(t),
            Action::Toast(t) => self.toast(t),
            Action::RestoreFromBin(paths) => {
                let items = self.tabs.active().view.bin.select(&paths);
                self.run_background("Restore", None, move || recycle::restore(items));
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
                    self.run_background("Empty Recycle Bin", None, recycle::empty);
                } else {
                    self.run_background("Delete", None, move || recycle::purge(items));
                }
                self.tabs.active_mut().view.clear_selection();
            }
            Action::MoveToBin(paths) => {
                let dir = paths
                    .first()
                    .and_then(|p| p.parent())
                    .map(Path::to_path_buf);
                self.run_background("Move to Recycle Bin", dir, move || {
                    fsops::delete(&paths, true)
                });
            }
        }
    }

    fn process_tab(&mut self, action: TabAction) {
        let s = &self.settings;
        match action {
            TabAction::New => {
                self.tabs.open(Location::Home, None, true, s);
            }
            TabAction::Close(id) => self.tabs.close(id, s),
            TabAction::Activate(id) => self.tabs.activate(id),
            TabAction::ToggleSplit => self.tabs.toggle_split(s),
            TabAction::SwitchPane => self.tabs.switch_pane(),
            TabAction::Duplicate(id) => self.tabs.duplicate(id, s),
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
            TabAction::CloseGroup(g) => self.tabs.close_group(g, s),
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

    pub fn navigate(&mut self, loc: Location) {
        if let Location::Folder(p) = &loc {
            if !p.is_dir() {
                self.toast(format!(
                    "\"{}\" doesn't exist or isn't a folder.",
                    p.display()
                ));
                return;
            }
            self.quick.visit_folder(p);
            self.save_quick();
        }
        self.ui.address_editing = false;
        self.tabs.active_mut().navigate(loc);
    }

    fn start_transfer(&mut self, kind: TransferKind, sources: Vec<PathBuf>, dest: PathBuf) {
        // Items inside archives are extracted instead (archives are read-only,
        // so a move is an extraction too).
        let mut from_archives: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
        let mut sources_on_disk = Vec::new();
        for src in sources {
            match archive::split_virtual(&src) {
                Some((arc, inner)) if !inner.as_os_str().is_empty() => {
                    match from_archives.iter_mut().find(|(a, _)| *a == arc) {
                        Some((_, items)) => items.push(inner),
                        None => from_archives.push((arc, vec![inner])),
                    }
                }
                _ => sources_on_disk.push(src),
            }
        }
        for (arc, items) in from_archives {
            self.extract(arc, items, dest.clone());
        }
        let sources = sources_on_disk;
        if sources.is_empty() {
            return;
        }
        self.engine.submit(TransferRequest::new(
            kind,
            sources,
            dest,
            self.settings.transfer.default_conflict,
        ));
        self.ui.show_transfers = true;
    }

    /// Keep the search index fresh: rebuild on start, on schedule and when
    /// the index settings change; drop it when indexing is turned off.
    fn manage_index(&mut self) {
        let s = &self.settings.search;
        if !s.index_enabled {
            if self.index.index().is_some() || self.index.is_building() {
                self.index.clear();
            }
            self.index_started = false;
            return;
        }
        let wanted = (s.index_roots.clone(), s.index_exclude.clone());
        if self.index_settings.as_ref() != Some(&wanted) {
            if self.index_settings.is_some() {
                self.index_started = false; // settings changed: rebuild
            }
            self.index_settings = Some(wanted);
        }
        if self.index.state() == IndexState::Loading || self.index.is_building() {
            return;
        }
        let due = !self.index_started || self.index_next.is_some_and(|t| Instant::now() >= t);
        if due {
            let drives: Vec<PathBuf> = self.drives.iter().map(|d| d.mount.clone()).collect();
            self.index
                .rebuild(self.settings.search.index_config(&drives));
            self.index_started = true;
            let minutes = self.settings.search.index_refresh_minutes;
            self.index_next = (minutes > 0)
                .then(|| Instant::now() + Duration::from_secs(u64::from(minutes) * 60));
        }
    }

    /// Is a ready-to-use index available for instant search?
    pub fn index_ready(&self) -> bool {
        self.settings.search.index_enabled && self.index.index().is_some()
    }

    /// Run `op` on a helper thread; when it finishes, errors become a toast and
    /// tabs showing `dir` (all tabs if `None`) are refreshed.
    fn start_terminal(&mut self) {
        let dir = crate::terminal::start_dir(self.current_folder().as_deref());
        match crate::terminal::Terminal::spawn(&self.ctx, &dir, &self.settings.terminal.shell) {
            Ok(t) => {
                self.terminal = Some(t);
                self.terminal_dir = Some(dir);
            }
            Err(e) => {
                self.terminal = None;
                self.toast(format!("Couldn't start the shell: {e}"));
            }
        }
    }

    /// With "follow the current folder" on, `cd` the terminal along.
    fn follow_folder_in_terminal(&mut self) {
        if !self.settings.terminal.follow_folder || !self.ui.show_terminal {
            return;
        }
        let Some(folder) = self.current_folder() else {
            return;
        };
        if self.terminal_dir.as_ref() != Some(&folder) {
            if let Some(t) = self.terminal.as_ref().filter(|t| !t.exited) {
                t.cd(&folder);
                self.terminal_dir = Some(folder);
            }
        }
    }

    /// Extract `items` (paths inside `arc`; empty = everything) into `dest`.
    fn extract(&mut self, arc: PathBuf, items: Vec<PathBuf>, dest: PathBuf) {
        self.toast(format!(
            "Extracting {} from {}…",
            if items.iter().any(|i| i.as_os_str().is_empty()) {
                "everything".to_owned()
            } else {
                format::count(items.len() as u64, "item")
            },
            entry::display_name(&arc)
        ));
        let target = dest.clone();
        self.run_background("Extracting", Some(dest), move || {
            archive::extract(&arc, &items, &target, &AtomicBool::new(false))
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
    }

    /// Open a file from an archive: extract it to a temporary folder first.
    fn open_from_archive(&mut self, arc: PathBuf, inner: PathBuf) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir()
            .join("oxplorer-open")
            .join(stamp.to_string());
        self.run_background("Opening", None, move || {
            let made = archive::extract(&arc, &[inner], &dir, &AtomicBool::new(false))
                .map_err(|e| e.to_string())?;
            let file = made.first().ok_or("Nothing was extracted.")?;
            fsops::open_default(file).map_err(|e| e.to_string())
        });
    }

    fn run_background(
        &mut self,
        label: &str,
        dir: Option<PathBuf>,
        op: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(op());
            ctx.request_repaint();
        });
        self.background.push((label.to_owned(), rx, dir));
    }

    fn after_create(&mut self, dir: &Path, created: PathBuf) {
        self.mark_changed(Some(dir));
        self.tabs.active_mut().view.select_after_load = Some(created.clone());
        self.process(Action::Rename(created));
    }

    fn open_properties(&mut self, paths: Vec<PathBuf>) {
        let entries: Vec<FileEntry> = paths
            .iter()
            .filter_map(|p| FileEntry::from_path(p).ok())
            .collect();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let worker_paths = paths.clone();
        let worker_cancel = cancel.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut totals = (0, 0, 0);
            for p in &worker_paths {
                if p.is_dir() {
                    let (b, f, d) = fsops::folder_size(p, &worker_cancel);
                    totals.0 += b;
                    totals.1 += f;
                    totals.2 += d + 1;
                } else if let Ok(m) = std::fs::metadata(p) {
                    totals.0 += m.len();
                    totals.1 += 1;
                }
            }
            let _ = tx.send(totals);
            ctx.request_repaint();
        });
        self.dialog = Some(Dialog::Properties(Properties {
            paths,
            entries,
            totals: None,
            rx: Some(rx),
            cancel,
        }));
    }

    pub fn save_quick(&mut self) {
        if let Err(e) = self.quick.save() {
            eprintln!("could not save quick access: {e}");
        }
    }

    fn save_session(&self) {
        if let Err(e) = self.tabs.to_session().save() {
            eprintln!("could not save session: {e}");
        }
    }

    /// Short status text for the transfers button.
    pub fn transfer_summary(&self) -> Option<String> {
        let jobs = self.engine.jobs();
        let active: Vec<_> = jobs.iter().filter(|j| !j.state().is_finished()).collect();
        if active.is_empty() {
            return None;
        }
        let speed: f64 = active.iter().map(|j| j.speed()).sum();
        let progress = active.iter().map(|j| j.progress()).sum::<f32>() / active.len() as f32;
        Some(format!(
            "{} · {:.0}% · {}",
            format::count(active.len() as u64, "transfer"),
            progress * 100.0,
            format::rate(speed)
        ))
    }
}

impl OxApp {
    /// Files dragged past the window's edge go on as a Windows drag & drop,
    /// so they can be dropped into other programs (Explorer, mail, editors…).
    fn drag_out_of_window(&mut self, ctx: &Context) {
        if !ox_core::shell::supported() {
            return;
        }
        let Some(items) = egui::DragAndDrop::payload::<crate::ui::DragItems>(ctx) else {
            return;
        };
        let window = ctx.content_rect();
        let outside = ctx.input(|i| {
            i.pointer.primary_down()
                && (i.pointer.hover_pos().is_none()
                    || i.pointer.latest_pos().is_some_and(|p| !window.contains(p)))
        });
        // Recycle Bin and archive items aren't files other programs can open.
        let paths: Vec<PathBuf> = items.0.iter().filter(|p| p.exists()).cloned().collect();
        if !outside || paths.is_empty() {
            return;
        }
        egui::DragAndDrop::clear_payload(ctx);
        ox_core::shell::drag_out(&paths);
        self.release_pointer = true;
        ctx.request_repaint();
    }
}

impl eframe::App for OxApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if std::mem::take(&mut self.release_pointer) {
            let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap_or_default();
            raw_input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: ctx.input(|i| i.modifiers),
            });
            raw_input.events.push(egui::Event::PointerGone);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pre_frame(&ctx);
        ui::show(self, ui);
        self.drag_out_of_window(&ctx);
        self.process_actions();
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_session();
            self.save_quick();
        }
    }
}
