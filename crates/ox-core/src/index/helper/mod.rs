//! The Oxplorer index service: a small Windows service (`oxplorer-indexer.exe`,
//! LocalSystem, started on demand) that opens the NTFS volumes for the app,
//! so the index is read from the master file table and follows the change
//! journal without running Oxplorer as administrator.
//!
//! It is a privilege boundary, so it does as little as possible:
//! - It serves one named pipe ([`PIPE_NAME`]) whose security descriptor
//!   (see [`security::pipe_sddl`]) lets in only the user the installer
//!   recorded, SYSTEM and Administrators; network logons are denied and the
//!   pipe rejects remote clients. Clients can't create pipe instances, and
//!   the app refuses a pipe not owned by SYSTEM/Administrators (squatting).
//! - The protocol ([`proto`]) is fixed and versioned, with tiny fixed-size
//!   requests that only name a drive letter, checked against the fixed NTFS
//!   volumes. Nothing the client sends is ever opened as a path or run.
//! - Answers are metadata the master file table holds (file reference,
//!   parent reference, name, attributes, size, last write time) and change
//!   journal records. No file contents are read for the client.
//! - It keeps no state between requests and stops itself when idle.
//!
//! What this means: the allowed user can see the names (and sizes and
//! dates) of every file on the fixed NTFS drives, including in folders they
//! can't open themselves, as with other MFT-based search tools. That is why
//! it is off by default: the installer task is unchecked and the app only
//! uses the service when "Use the Oxplorer index service" is turned on.

pub mod client;
pub mod proto;
pub mod security;
pub mod server;

#[cfg(windows)]
mod local;
#[cfg(windows)]
mod pipe;

#[cfg(windows)]
pub use local::LocalVolumes;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The service's name (`sc.exe query OxplorerIndex`).
pub const SERVICE_NAME: &str = "OxplorerIndex";
pub const PIPE_NAME: &str = r"\\.\pipe\OxplorerIndex";
/// Where the installer records the allowed user (under `HKEY_LOCAL_MACHINE`,
/// writable only by administrators).
pub const PARAMETERS_KEY: &str = r"SYSTEM\CurrentControlSet\Services\OxplorerIndex\Parameters";
/// The allowed user's SID (`REG_SZ`).
pub const ALLOWED_USER_VALUE: &str = "AllowedUser";

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Use the service when it is installed (the setting; off by default).
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// How the index service is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Not on Windows.
    Unsupported,
    NotInstalled,
    /// Installed and stopped; it starts when needed.
    Stopped,
    /// Running and reading these drives.
    Running(Vec<char>),
    /// Installed for another user.
    NoAccess,
    /// Installed, but it doesn't answer.
    Unavailable,
}

/// A connection to the service if it is enabled, installed and allowed,
/// starting it if needed.
#[cfg(windows)]
pub(crate) fn connect() -> Option<client::Client<std::fs::File>> {
    if !enabled() {
        return None;
    }
    pipe::connect_raw(true).ok()
}

/// A connection to the service if it is running, regardless of the setting
/// and without starting it (for the end-to-end test and diagnostics). `Err`
/// is the Windows error code.
#[cfg(windows)]
pub fn connect_running() -> Result<client::Client<std::fs::File>, u32> {
    pipe::connect_raw(false)
}

struct Cached {
    at: Option<Instant>,
    status: Option<Status>,
    refreshing: bool,
}

static CACHE: Mutex<Cached> = Mutex::new(Cached {
    at: None,
    status: None,
    refreshing: false,
});

/// The service's status as last checked (`None` until the first check is
/// done). Checks again in the background when the last check is older
/// than a few seconds, so this is cheap to call every frame.
pub fn status() -> Option<Status> {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let stale = c.at.is_none_or(|t| t.elapsed() > Duration::from_secs(3));
    if stale && !c.refreshing {
        c.refreshing = true;
        let spawned = std::thread::Builder::new()
            .name("index-service-status".into())
            .spawn(|| {
                #[cfg(windows)]
                let s = pipe::query_status();
                #[cfg(not(windows))]
                let s = Status::Unsupported;
                let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
                c.status = Some(s);
                c.at = Some(Instant::now());
                c.refreshing = false;
            });
        if spawned.is_err() {
            c.refreshing = false;
        }
    }
    c.status.clone()
}
