//! Core, UI-independent building blocks for Oxplorer.
//!
//! Everything in here is plain Rust with no GUI dependency so it can be unit
//! tested (and reused by a future CLI or alternative front-end).

pub mod archive;
pub mod clipboard;
pub mod compare;
pub mod default_app;
pub mod details;
pub mod dirsize;
pub mod disk;
pub mod entry;
pub mod flat;
pub mod format;
pub mod fsops;
pub mod fuzzy;
pub mod git;
pub mod i18n;
mod i18n_de;
pub mod index;
pub mod links;
pub mod logfile;
pub mod natural;
pub mod network;
pub mod pathfmt;
pub mod pdf;
pub mod places;
pub mod player;
pub mod power;
pub mod recycle;
pub mod rename;
pub mod search;
pub mod settings;
pub mod shell;
pub mod tags;
pub mod textinfo;
pub mod themes;
pub mod titlebar;
pub mod transfer;
pub mod undo;
pub mod update;
pub mod watcher;

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
