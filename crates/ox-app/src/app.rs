//! Application state and the action dispatcher.
//!
//! UI code never mutates state directly in complex ways; instead it pushes
//! [`Action`]s which are processed at the end of each frame. This keeps the
//! borrow checker happy and makes every user command go through one place.

mod action;
mod cleanup;
mod cloud;
mod commands;
mod compare;
mod disk_usage;
mod flat;
mod input;
mod instances;
mod library;
mod links;
mod picture_tools;
mod player;
mod poll;
mod process;
mod properties;
mod quick_look;
mod search_history;
mod select;
mod sessions;
mod transfer_watch;
mod undo;
mod view;
mod window;
mod wsl;
pub use action::{Action, TabAction};
pub use cleanup::{CleanupAction, CleanupView, ConfirmCleanup};
pub use commands::{shortcut_of, CommandAction, CustomCommands};
pub use compare::{CompareAction, CompareState};
pub use disk_usage::{DiskUsageAction, DiskUsageScans};
pub use instances::Start;
pub use picture_tools::{doing as doing_pictures, PictureAction, PictureTools};
pub use player::{PlayerAction, Session as PlayerSession, RATES};
pub use properties::Properties;
pub use search_history::{HistoryAction, RecentSearches};
pub use select::SelectAction;
pub use sessions::{SavedSessions, SessionAction};
pub use transfer_watch::TransferWatch;
pub use view::{ColumnEdit, ViewAction};

use ox_core::i18n::t as tr;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use egui::{Context, Key, Modifiers};
use ox_core::archive;
use ox_core::clipboard::{ClipMode, FileClipboard};
use ox_core::dirsize::FolderSizes;
use ox_core::entry;
use ox_core::index::{IndexService, IndexState};
use ox_core::places::{self, Drive, QuickAccess};
use ox_core::recycle::{self, TrashItem};
use ox_core::search::{self, Query, SearchEvent, SearchOptions};
use ox_core::settings::{Backdrop, Settings, StartPage};
use ox_core::tags::TagStore;
use ox_core::transfer::{TransferEngine, TransferKind, TransferRequest};
use ox_core::watcher::DirWatcher;
use ox_core::{format, fsops};

use crate::tabs::{GroupId, Listing, Location, SearchState, Session, TabId, Tabs};
use crate::{theme, ui};

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
    Properties(Box<Properties>),
    RenameGroup {
        id: GroupId,
        text: String,
        focus: bool,
    },
    NewLink(Box<crate::ui::link_dialog::LinkDialog>),
    /// Name for saving the open tabs as a session, or (`old`) a new name for one.
    SessionName {
        old: Option<String>,
        text: String,
        focus: bool,
    },
    /// Select (`select`) or deselect the items matching typed patterns.
    SelectPattern {
        text: String,
        select: bool,
        focus: bool,
    },
    /// Convert or resize pictures.
    Pictures(Box<crate::ui::picture_tools::PictureDialog>),
}

/// A file operation running on a helper thread (label, result, folder to refresh).
type BackgroundOp = (String, Receiver<Result<(), String>>, Option<PathBuf>);

pub struct Toast {
    pub text: String,
    pub until: Instant,
}

/// Transient UI state that doesn't belong to a tab.
pub struct UiState {
    /// The tabs are in the title bar (it was removed; see `ox_core::titlebar`).
    pub title_bar: bool,
    /// Where the title bar strip has things to click this frame, its height
    /// and the maximize button (all in points).
    pub title_hits: Vec<egui::Rect>,
    pub title_height: f32,
    pub max_button: Option<egui::Rect>,
    pub address_editing: bool,
    pub address_text: String,
    pub focus_address: bool,
    pub focus_search: bool,
    pub show_transfers: bool,
    pub settings_section: usize,
    /// Where the sidebar's and the preview pane's edges go (x in points),
    /// measured in the header this frame: under the command bar's separator
    /// after the edit buttons, and in the gap before the search box.
    pub sidebar_edge: Option<f32>,
    pub preview_edge: Option<f32>,
    /// Whether folders open in Oxplorer (read from the registry when needed).
    pub default_folder_handler: Option<bool>,
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
    pub transfer_watch: TransferWatch,
    text_clipboard: Option<arboard::Clipboard>,
    applied_settings: Option<Settings>,
    pub icons: crate::icons::IconCache,
    pub thumbs: crate::thumbs::Thumbnails,
    /// Values of the extra details columns, read in the background.
    pub column_values: crate::column_values::ColumnValues,
    /// The view each folder was last shown with.
    pub folder_views: ox_core::folder_views::FolderViews,
    folder_views_saved: Instant,
    pub index: IndexService,
    pub preview: crate::ui::preview::PreviewState,
    /// The built-in player (plays on while browsing; see `app/player.rs`).
    pub player: PlayerSession,
    /// Quick Look, while it is open (Space).
    pub quick_look: Option<crate::ui::quick_look::QuickLook>,
    pub folder_sizes: FolderSizes,
    /// The trees scanned for disk usage views.
    pub disk_usage: DiskUsageScans,
    /// Keyboard shortcuts in effect (defaults + the user's changes).
    pub keymap: crate::shortcuts::Keymap,
    /// Colour schemes: built-in ones and the user's (see `ox_core::themes`).
    pub themes: Vec<ox_core::themes::ColorTheme>,
    /// A scheme was edited; apply the look again.
    pub themes_changed: bool,
    /// A native drag ended inside its own message loop, so egui never saw
    /// the mouse button go up; tell it on the next frame.
    release_pointer: bool,
    /// The window is transparent, so a backdrop can show through it.
    pub(crate) see_through: bool,
    /// Backdrop, dark mode and "no title bar" last given to the window.
    window_style: Option<(Backdrop, bool, bool)>,
    /// Whether the window's title bar was last removed (`Some(true)`) or given back.
    title_bar_applied: Option<bool>,
    /// The terminal panel's shell, started the first time the panel opens.
    pub terminal: Option<crate::terminal::Terminal>,
    /// Folder the terminal was last sent to (for "follow the folder").
    terminal_dir: Option<PathBuf>,
    pub tags: TagStore,
    /// Compare folders: the comparison of the split view's panes.
    pub compare: Option<CompareState>,
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
    /// Rebuild the index now ("Rebuild now", or its settings changed).
    index_force: bool,
    /// Index-related settings the current index was built with.
    index_settings: Option<(Vec<PathBuf>, Vec<String>)>,
    /// Results of background work started by actions (Git, the editor).
    notices: (Sender<Notice>, Receiver<Notice>),
    /// What Ctrl+Z / Ctrl+Y can undo and redo.
    pub undo: undo::UndoState,
    /// Named sessions (sets of tabs saved to open again).
    pub sessions: SavedSessions,
    /// Searches that were run, suggested under the search box.
    pub search_history: RecentSearches,
    /// This window among other Oxplorer windows (files they share).
    instance: instances::Instance,
    /// When the panes were last checked for syncing cloud files.
    cloud_check: Option<Instant>,
    /// The Windows libraries (for the sidebar and drops into a library).
    pub libraries: Vec<ox_core::library::Library>,
    /// Reading them in the background, and when they were last read.
    libraries_rx: Option<Receiver<Vec<ox_core::library::Library>>>,
    libraries_at: Option<Instant>,
    /// Running Gallery walks: the tab and the flag that stops it.
    gallery_walks: Vec<(TabId, Arc<AtomicBool>)>,
    /// Find duplicates / empty folders: the pages' scans and ticks.
    pub cleanups: cleanup::Cleanups,
    /// Picture tools: the running job (rotate, convert, …).
    pub pictures: PictureTools,
    /// The WSL distributions (for the sidebar and the Home page).
    pub wsl: wsl::Wsl,
    /// Custom commands: the question before one runs, output windows.
    pub commands: CustomCommands,
}

/// What background work started by an action reports back.
pub enum Notice {
    Nothing,
    Toast(String),
    OpenUrl(String),
    /// Put text on the clipboard and say so.
    Copy(String, String),
    /// These folders changed: list them again, and show the text if any.
    Changed(Vec<PathBuf>, Option<String>),
}

impl OxApp {
    /// `see_through`: the window was created transparent (for Mica/Acrylic).
    pub fn new(cc: &eframe::CreationContext<'_>, start: Start, see_through: bool) -> Self {
        let ctx = cc.egui_ctx.clone();
        // Images in the Markdown preview (local files).
        egui_extras::install_image_loaders(&ctx);
        install_icon_font(&ctx);
        let mut instance = instances::Instance::new(start.session_lock);
        let settings = instance.settings.load().unwrap_or_default();
        // Before anything is named (e.g. the default favorites on first start).
        ox_core::i18n::set_language(settings.appearance.language);
        let quick = instance.quick.load().unwrap_or_default();

        let engine = TransferEngine::new(settings.transfer.engine_config());
        let repaint = ctx.clone();
        engine.set_notifier(move || repaint.request_repaint());

        // Tabs handed over by another window, else the start page (only
        // one window restores the saved session).
        let handed = start.tabs.and_then(|s| Tabs::from_session(s, &settings));
        let mut tabs = match settings.startup.start_page {
            _ if handed.is_some() => handed,
            StartPage::RestoreSession if instance.owns_session() => {
                Session::load().and_then(|s| Tabs::from_session(s, &settings))
            }
            StartPage::RestoreSession => None,
            StartPage::Folder => settings
                .startup
                .start_folder
                .clone()
                .filter(|p| p.is_dir())
                .map(|p| Tabs::new(Location::Folder(p), &settings)),
            StartPage::Home => None,
        }
        .unwrap_or_else(|| Tabs::new(Location::Home, &settings));
        if let Some(path) = start.open {
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
                title_bar: false,
                title_hits: Vec::new(),
                title_height: 0.0,
                max_button: None,
                address_editing: false,
                address_text: String::new(),
                focus_address: false,
                focus_search: false,
                show_transfers: true,
                settings_section: 0,
                sidebar_edge: None,
                preview_edge: None,
                default_folder_handler: None,
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
            transfer_watch: Default::default(),
            text_clipboard: None,
            applied_settings: None,
            icons: crate::icons::IconCache::new(&cc.egui_ctx),
            thumbs: crate::thumbs::Thumbnails::new(&cc.egui_ctx),
            column_values: crate::column_values::ColumnValues::new(&cc.egui_ctx),
            folder_views: ox_core::folder_views::FolderViews::load(),
            folder_views_saved: Instant::now(),
            index: {
                let repaint = cc.egui_ctx.clone();
                IndexService::start(ox_core::index::default_cache_path(), move || {
                    repaint.request_repaint()
                })
            },
            preview: Default::default(),
            player: PlayerSession::main(),
            quick_look: None,
            disk_usage: Default::default(),
            folder_sizes: {
                let repaint = cc.egui_ctx.clone();
                FolderSizes::new(move || repaint.request_repaint())
            },
            tags: {
                let mut tags = TagStore::from_file(instance.tags.load().unwrap_or_default());
                tags.prune();
                tags
            },
            compare: None,
            keymap: Default::default(),
            themes: ox_core::themes::load_all(),
            themes_changed: false,
            release_pointer: false,
            see_through,
            window_style: None,
            title_bar_applied: None,
            terminal: None,
            terminal_dir: None,
            sizes_key: None,
            git_key: Vec::new(),
            git_pending: Vec::new(),
            notices: crossbeam_channel::unbounded(),
            sizes_applied: 0,
            index_started: false,
            index_next: None,
            index_force: false,
            index_settings: None,
            undo: Default::default(),
            sessions: Default::default(),
            search_history: Default::default(),
            instance,
            cloud_check: None,
            libraries: Vec::new(),
            libraries_rx: None,
            libraries_at: None,
            gallery_walks: Vec::new(),
            cleanups: Default::default(),
            pictures: Default::default(),
            wsl: Default::default(),
            commands: Default::default(),
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

    /// The folder of the active tab; for a library, its save folder (where
    /// pasted, new and dropped files go).
    pub fn current_folder(&self) -> Option<PathBuf> {
        let location = self.tabs.active().location();
        if let Location::Library { file, .. } = location {
            return self.library_save_folder(file);
        }
        location.folder().map(Path::to_path_buf)
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
            ox_core::i18n::set_language(self.settings.appearance.language);
            theme::apply(
                ctx,
                &self.settings,
                &self.themes,
                self.backdrop() != Backdrop::Off,
            );
            self.keymap = crate::shortcuts::Keymap::new(&self.settings.shortcuts);
            crate::shortcuts::set_current(&self.keymap);
            crate::ui::commands::set_current(&self.settings.commands.0);
            self.engine
                .set_config(self.settings.transfer.engine_config());
            self.applied_settings = Some(self.settings.clone());
        }
        self.remap_text_clipboard_keys(ctx);
        self.poll_shared_files();
        self.icons.poll(ctx);
        self.thumbs.poll(ctx);
        self.column_values.poll();
        self.manage_index();
        self.tabs.fix_panes();
        self.sync_folder_views();
        self.load_active_tab();
        if let Some(other) = self.tabs.split_partner() {
            // Load the other pane's tab too (`load_active_tab` works on the active one).
            let focused = self.tabs.active;
            self.tabs.active = other;
            self.load_active_tab();
            self.tabs.active = focused;
        }
        self.poll_tabs();
        self.poll_compare();
        self.poll_cleanups();
        self.poll_folder_sizes();
        self.poll_disk_usage();
        self.poll_git();
        self.poll_cloud();
        self.poll_libraries();
        self.poll_wsl();
        self.poll_player(ctx);
        self.poll_watcher();
        self.poll_jobs();
        self.poll_undo();
        self.poll_pictures();
        self.poll_transfer_watch(ctx);
        self.poll_background();
        self.poll_notices();
        self.poll_dropped_files(ctx);
        self.follow_folder_in_terminal();
        self.sync_groups();
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
            && !self.compare.as_ref().is_some_and(|c| c.confirm.is_some())
            && !ctx.text_edit_focused()
            // Not while a new shortcut is being recorded in the settings.
            && self.ui.recording.is_none()
            && !self.commands.takes_keys()
        {
            self.handle_shortcuts(ctx);
        }
        self.toasts.retain(|t| t.until > Instant::now());
        if !self.toasts.is_empty() || self.engine.has_active() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
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
        let drives = self.index_drives();
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
            Location::Folder(path) if tab.is_flat() => {
                let opts = ox_core::flat::FlatOptions {
                    show_hidden,
                    ..Default::default()
                };
                tab.start_flat(path, opts, move || ctx.request_repaint());
            }
            Location::Folder(path) => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Listing::folder(&path, show_hidden).map_err(|e| e.to_string()));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Search { root, query } => {
                let mut q = Query::parse(&query, search_opts.case_sensitive);
                q.resolve_tags(&self.tags);
                tab.view = Default::default();
                let repaint = move || ctx.request_repaint();
                // Answer from the index when it covers the place we search in
                // (it only knows file system names, not what's in archives).
                let handle = match index {
                    Some(idx)
                        if !q.in_archives() && root.as_deref().is_none_or(|r| idx.covers(r)) =>
                    {
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
            Location::Library { file, .. } => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Listing::library(&file, show_hidden));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Gallery => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                let cancel = Arc::new(AtomicBool::new(false));
                let flag = cancel.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Ok(Listing::gallery(show_hidden, &flag)));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
                // A walk started earlier for this tab isn't needed any more.
                self.gallery_walks.retain(|(id, old)| {
                    let same = *id == tab.id;
                    if same {
                        old.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    !same
                });
                self.gallery_walks.push((tab.id, cancel));
            }
            Location::Cleanup { root, mode } => {
                self.cleanups.start(tab.id, root, mode, show_hidden, ctx);
            }
            // Disk usage views get their scan in `poll_disk_usage`.
            Location::Home | Location::Settings | Location::Help | Location::DiskUsage(_) => {}
        }
    }

    /// Switch to the tab showing `page` (Settings, Help, Recycle Bin), if
    /// one does.
    fn activate_page(&mut self, page: &Location) -> bool {
        let tab = self.tabs.tabs.iter().find(|t| t.location() == page);
        let Some(id) = tab.map(|t| t.id) else {
            return false;
        };
        self.tabs.activate(id);
        true
    }

    /// Go to `loc` in the active tab. Home is the Home tab; what is opened
    /// from the Home tab opens in a new tab, so it stays Home.
    pub fn navigate(&mut self, loc: Location) {
        self.ui.address_editing = false;
        if loc == Location::Home {
            self.tabs.activate(self.tabs.home());
            return;
        }
        if loc.is_page() && self.activate_page(&loc) {
            return;
        }
        if let Location::Folder(p) = &loc {
            if !ox_core::network::is_dir(p) {
                self.toast(ox_core::trf!(
                    "\"{v1}\" doesn't exist or isn't a folder.",
                    v1 = p.display()
                ));
                return;
            }
            self.quick.visit_folder(p);
            self.save_quick();
        }
        if self.tabs.is_home(self.tabs.active) {
            // The text typed into its search box went with the search.
            self.tabs.active_mut().filter.clear();
            self.tabs.open(loc, None, true, &self.settings);
        } else {
            self.tabs.active_mut().navigate(loc);
        }
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
        let req =
            TransferRequest::new(kind, sources, dest, self.settings.transfer.default_conflict);
        if std::mem::take(&mut self.transfer_watch.run_after_next) {
            self.engine.submit_after_current(req);
        } else {
            self.engine.submit(req);
        }
        self.ui.show_transfers = true;
    }

    /// Keep the search index fresh: rebuild on start, on schedule and when
    /// the index settings change; drop it when indexing is turned off.
    fn manage_index(&mut self) {
        let s = &self.settings.search;
        if !s.index_enabled {
            self.index.set_live(None);
            if self.index.index().is_some() || self.index.is_building() {
                self.index.clear();
            }
            self.index_started = false;
            return;
        }
        let wanted = (s.index_roots.clone(), s.index_exclude.clone());
        if self.index_settings.as_ref() != Some(&wanted) {
            if self.index_settings.is_some() {
                self.index_force = true; // settings changed: rebuild
            }
            self.index_settings = Some(wanted);
        }
        let drives = self.index_drives();
        let config = self.settings.search.index_config(&drives);
        // Live updates follow the same folders (and restart when they change).
        let live = self.settings.search.live_config(&config, &self.drives);
        self.index.set_live(live);
        if self.index.state() == IndexState::Loading || self.index.is_building() {
            return;
        }
        let minutes = self.settings.search.index_refresh_minutes;
        let interval = Duration::from_secs(u64::from(minutes) * 60);
        if std::mem::take(&mut self.index_force) {
            self.index_started = true;
        } else if !self.index_started {
            self.index_started = true;
            // A saved index of the same folders that isn't due yet is used as
            // is; the refresh waits until it is. ("Only at start" = 0 always
            // refreshes at start.)
            let age = self
                .index
                .index()
                .filter(|idx| idx.roots() == config.roots.as_slice())
                .and_then(|idx| idx.built_at().elapsed().ok());
            if let Some(age) = age.filter(|age| minutes > 0 && *age < interval) {
                self.index_next = Some(Instant::now() + (interval - age));
                return;
            }
        } else if !self.index_next.is_some_and(|t| Instant::now() >= t) {
            return;
        }
        self.index.rebuild(config);
        self.index_next = (minutes > 0).then(|| Instant::now() + interval);
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
                self.toast(ox_core::trf!("Couldn't start the shell: {e}", e = e));
            }
        }
    }

    /// With "follow the current folder" on, `cd` the terminal along.
    fn follow_folder_in_terminal(&mut self) {
        if self.settings.terminal.follow_folder && self.ui.show_terminal {
            self.terminal_to_current_folder();
        }
    }

    /// `cd` the running terminal to the folder on screen (if it isn't there).
    fn terminal_to_current_folder(&mut self) {
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
        self.toast(ox_core::trf!(
            "Extracting {v1} from {v2}…",
            v1 = if items.iter().any(|i| i.as_os_str().is_empty()) {
                tr("everything").to_owned()
            } else {
                ox_core::i18n::count(items.len() as u64, "item")
            },
            v2 = entry::display_name(&arc)
        ));
        let target = dest.clone();
        self.run_background(tr("Extracting"), Some(dest), move || {
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
        self.run_background(tr("Opening"), None, move || {
            let made = archive::extract(&arc, &[inner], &dir, &AtomicBool::new(false))
                .map_err(|e| e.to_string())?;
            let file = made.first().ok_or(tr("Nothing was extracted."))?;
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
        self.undo.created(&created);
        self.mark_changed(Some(dir));
        self.tabs.active_mut().view.select_after_load = Some(created.clone());
        self.process(Action::Rename(created));
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
            ox_core::i18n::count(active.len() as u64, "transfer"),
            progress * 100.0,
            format::rate(speed)
        ))
    }
}

/// The icons in labels and menus ("📂 Open") are drawn by Lucide: a font
/// made from it (see `tools/icon-font`) comes right after the text font, so
/// it answers for those symbols before the emoji fonts do.
fn install_icon_font(ctx: &Context) {
    use egui::{FontData, FontDefinitions, FontFamily};
    const NAME: &str = "oxplorer-icons";
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        NAME.to_owned(),
        std::sync::Arc::new(FontData::from_static(include_bytes!("../assets/icons.ttf"))),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        list.insert(list.len().min(1), NAME.to_owned());
    }
    ctx.set_fonts(fonts);
}

impl eframe::App for OxApp {
    /// eframe only keeps the window's geometry; everything else lives in
    /// Oxplorer's own settings and session files.
    fn persist_egui_memory(&self) -> bool {
        false
    }

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

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.backdrop() != Backdrop::Off {
            [0.0; 4] // let the backdrop show through
        } else {
            visuals.panel_fill.to_normalized_gamma_f32()
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pre_frame(&ctx);
        self.style_window(&ctx, frame);
        ui::show(self, ui);
        ui::preview::place_documents(self, &ctx, frame);
        self.publish_title_bar(&ctx);
        self.drag_out_of_window(&ctx);
        self.drag_tab_out(&ctx);
        self.process_actions();
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_session();
            self.save_folder_views();
            self.save_quick();
        }
    }
}
