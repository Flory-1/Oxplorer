//! Core, UI-independent building blocks for Oxplorer.
//!
//! Everything in here is plain Rust with no GUI dependency so it can be unit
//! tested (and reused by a future CLI or alternative front-end).

pub mod archive;
pub mod cleanup;
pub mod clipboard;
pub mod cloud;
pub mod columns;
pub mod commands;
pub mod compare;
pub mod datatable;
pub mod datatree;
pub mod default_app;
pub mod details;
pub mod dirsize;
pub mod disk;
pub mod disk_usage;
pub mod entry;
pub mod exif_info;
pub mod flat;
pub mod folder_views;
pub mod format;
pub mod fsops;
pub mod fuzzy;
pub mod gallery;
pub mod git;
pub mod grid_zoom;
pub mod grouping;
pub mod hexview;
pub mod i18n;
mod i18n_de;
pub mod index;
pub mod instance;
pub mod library;
pub mod links;
pub mod logfile;
pub mod natural;
pub mod network;
pub mod pathfmt;
pub mod pdf;
pub mod picture;
pub mod picture_tools;
pub mod places;
pub mod player;
pub mod power;
pub mod preview_handler;
pub mod props;
pub mod recycle;
pub mod remote;
pub mod rename;
pub mod search;
pub mod search_history;
pub mod select;
pub mod sessions;
pub mod settings;
pub mod shared;
pub mod shell;
pub mod sqlite_view;
pub mod tags;
pub mod textinfo;
pub mod themes;
pub mod titlebar;
pub mod toast;
pub mod transfer;
pub mod undo;
pub mod update;
pub mod watcher;
pub mod wsl;

pub use entry::{FileEntry, SortKey, SortOrder};
pub use settings::Settings;
pub use transfer::{ConflictPolicy, TransferEngine, TransferKind, TransferRequest};

/// Name used for config directories and window titles.
pub const APP_NAME: &str = "Oxplorer";

/// Directory where Oxplorer stores its configuration files.
pub fn config_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(APP_NAME)
}
