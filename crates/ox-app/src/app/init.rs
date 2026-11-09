//! Starting the app: everything `OxApp` holds, set up from the settings.

use super::*;

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
        // Before the saved index loads: it catches up through the service.
        ox_core::index::helper::set_enabled(cfg!(windows) && settings.search.index_service);
        let quick = instance.quick.load().unwrap_or_default();

        let engine = TransferEngine::new(settings.transfer.engine_config());
        let repaint = ctx.clone();
        engine.set_notifier(move || repaint.request_repaint());
        let deletes = delete::Deletes::default();
        let repaint = ctx.clone();
        deletes
            .engine
            .set_notifier(move || repaint.request_repaint());

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
            deletes,
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
                transfers_opened: false,
                settings_section: 0,
                sidebar_edge: None,
                preview_edge: None,
                preview_pane: false,
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
            text_compare: None,
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
            mirror: Default::default(),
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
            fulltext: fulltext::FullText::new(&cc.egui_ctx),
            pictures: Default::default(),
            wsl: Default::default(),
            commands: Default::default(),
            remote: Default::default(),
        }
    }
}
