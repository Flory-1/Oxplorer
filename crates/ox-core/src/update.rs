//! Update check: is there a newer release on GitHub?
//!
//! At most once a day, asks GitHub where `releases/latest` redirects to: the
//! page of the newest release, whose last part is its tag (`v1.2.3`). No API
//! and no JSON. The answer and a version the user dismissed are kept in
//! `update.toml` in the config folder.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const CHECK_EVERY_SECS: i64 = 24 * 60 * 60;

/// A release newer than this build.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// The Git tag, e.g. `v1.2.3`.
    pub tag: String,
}

impl Release {
    /// The version without the `v`.
    pub fn version(&self) -> &str {
        self.tag.trim_start_matches(['v', 'V'])
    }

    /// The release's page on GitHub, with its notes.
    pub fn notes_url(&self) -> String {
        format!("{REPOSITORY}/releases/tag/{}", self.tag)
    }
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct State {
    /// When GitHub was last asked (Unix seconds).
    checked: i64,
    /// The newest release's tag then (none: no release yet).
    latest: Option<String>,
    /// The tag whose note the user closed.
    dismissed: Option<String>,
}

static OFFER: Mutex<Option<Release>> = Mutex::new(None);

/// Check in the background (at most once a day; otherwise the last answer is
/// used). Calls `found` when a newer release is there to offer.
pub fn start(found: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("update check".into())
        .spawn(move || {
            let path = state_path();
            let mut state = load(&path);
            let now = chrono::Utc::now().timestamp();
            if due(&state, now) {
                match latest_tag() {
                    Ok(latest) => {
                        state.checked = now;
                        state.latest = latest;
                        if let Err(e) = save(&path, &state) {
                            log::warn!("could not save the update check: {e}");
                        }
                    }
                    // Offline, a proxy, GitHub down: try again next start.
                    Err(e) => log::info!("update check failed: {e}"),
                }
            }
            if let Some(release) = offer(&state, CURRENT) {
                log::info!("update available: {}", release.tag);
                *OFFER.lock().unwrap_or_else(|e| e.into_inner()) = Some(release);
                found();
            }
        });
    if let Err(e) = spawned {
        log::warn!("could not start the update check: {e}");
    }
}

/// The newer release found by [`start`], unless its note was closed.
pub fn available() -> Option<Release> {
    OFFER.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Don't offer this release again (a newer one will be).
pub fn dismiss() {
    let Some(release) = OFFER.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let path = state_path();
    let mut state = load(&path);
    state.dismissed = Some(release.tag);
    if let Err(e) = save(&path, &state) {
        log::warn!("could not save the update check: {e}");
    }
}

fn state_path() -> PathBuf {
    crate::config_dir().join("update.toml")
}

fn load(path: &Path) -> State {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(path: &Path, state: &State) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string(state).map_err(io::Error::other)?;
    std::fs::write(path, text)
}

/// Time to ask again (also when the clock went back).
fn due(state: &State, now: i64) -> bool {
    now - state.checked >= CHECK_EVERY_SECS || now < state.checked
}

/// The release to offer: newer than `current` and not dismissed.
fn offer(state: &State, current: &str) -> Option<Release> {
    let tag = state.latest.as_ref()?;
    (is_newer(tag, current) && state.dismissed.as_ref() != Some(tag))
        .then(|| Release { tag: tag.clone() })
}

/// `1.2.3` from `v1.2.3`, `1.2` or `1.2.3-beta` (missing parts are 0).
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().transpose().ok()?.unwrap_or(0);
    let patch = parts.next().transpose().ok()?.unwrap_or(0);
    parts.next().is_none().then_some((major, minor, patch))
}

fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(new), Some(old)) => new > old,
        _ => false,
    }
}

/// The tag in a release page's address (`…/releases/tag/v1.2.3`).
fn tag_from_location(location: &str) -> Option<String> {
    let (_, tag) = location.trim().rsplit_once("/releases/tag/")?;
    let tag = tag.split(['?', '#', '/']).next()?;
    parse_version(tag).map(|_| tag.to_string())
}

/// The newest release's tag; `None` if there is no release yet.
fn latest_tag() -> io::Result<Option<String>> {
    let rest = REPOSITORY
        .strip_prefix("https://")
        .ok_or_else(|| io::Error::other("the repository isn't an https address"))?;
    let (host, repo) = rest.split_once('/').unwrap_or((rest, ""));
    let location = redirect_of(host, &format!("/{repo}/releases/latest"))?;
    Ok(location.as_deref().and_then(tag_from_location))
}

/// Where `https://host/path` redirects to, if it does.
#[cfg(windows)]
fn redirect_of(host: &str, path: &str) -> io::Result<Option<String>> {
    use std::ffi::c_void;
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Networking::WinHttp::*;

    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
    fn handle(h: *mut c_void) -> io::Result<Handle> {
        if h.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Handle(h))
        }
    }

    let agent = HSTRING::from(format!("{}/{CURRENT}", crate::APP_NAME));
    // SAFETY: every handle is checked and closed when dropped; the buffers
    // outlive the calls that fill them.
    unsafe {
        let session = handle(WinHttpOpen(
            &agent,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 10_000)?;
        let connect = handle(WinHttpConnect(
            session.0,
            &HSTRING::from(host),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ))?;
        let request = handle(WinHttpOpenRequest(
            connect.0,
            w!("HEAD"),
            &HSTRING::from(path),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(&WINHTTP_DISABLE_REDIRECTS.to_ne_bytes()),
        )?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())?;

        let mut status = 0u32;
        let mut len = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status).cast()),
            &mut len,
            std::ptr::null_mut(),
        )?;
        if !(300..400).contains(&status) {
            return match status {
                200..300 => Ok(None),
                _ => Err(io::Error::other(format!("GitHub answered {status}"))),
            };
        }
        let mut buf = [0u16; 2048];
        let mut len = size_of_val(&buf) as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_LOCATION,
            PCWSTR::null(),
            Some(buf.as_mut_ptr().cast()),
            &mut len,
            std::ptr::null_mut(),
        )?;
        Ok(Some(String::from_utf16_lossy(&buf[..len as usize / 2])))
    }
}

#[cfg(not(windows))]
fn redirect_of(_host: &str, _path: &str) -> io::Result<Option<String>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the update check is only on Windows",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_by_number() {
        assert!(is_newer("v1.0.10", "1.0.9"));
        assert!(is_newer("v2.0", "1.9.9"));
        assert!(!is_newer("v1.0.2", "1.0.2"));
        assert!(!is_newer("v1.0.1", "1.0.2"));
        assert!(!is_newer("nightly", "1.0.2"));
        assert_eq!(parse_version("v1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("1..2"), None);
    }

    #[test]
    fn the_tag_comes_from_the_release_page_address() {
        let loc = "https://github.com/Flory-1/Oxplorer/releases/tag/v1.0.3";
        assert_eq!(tag_from_location(loc).as_deref(), Some("v1.0.3"));
        // Without any release, GitHub sends you to the release list.
        assert_eq!(
            tag_from_location("https://github.com/Flory-1/Oxplorer/releases"),
            None
        );
        assert_eq!(tag_from_location("https://x/releases/tag/latest"), None);
    }

    #[test]
    fn asks_at_most_once_a_day() {
        let state = State {
            checked: 1_000_000,
            ..State::default()
        };
        assert!(!due(&state, 1_000_000 + CHECK_EVERY_SECS - 1));
        assert!(due(&state, 1_000_000 + CHECK_EVERY_SECS));
        assert!(due(&state, 999_999), "clock went back");
        assert!(due(&State::default(), 1_000_000));
    }

    #[test]
    fn a_dismissed_release_is_not_offered_again() {
        let mut state = State {
            latest: Some("v1.1.0".into()),
            ..State::default()
        };
        let offered = offer(&state, "1.0.2").unwrap();
        assert_eq!(offered.version(), "1.1.0");
        assert!(offered.notes_url().ends_with("/releases/tag/v1.1.0"));
        assert_eq!(offer(&state, "1.1.0"), None);
        state.dismissed = Some("v1.1.0".into());
        assert_eq!(offer(&state, "1.0.2"), None);
        state.latest = Some("v1.2.0".into());
        assert!(offer(&state, "1.0.2").is_some());
    }

    /// `cargo test -p ox-core update -- --ignored` (asks GitHub).
    #[test]
    #[ignore = "needs the network"]
    #[cfg(windows)]
    fn github_redirects_to_the_newest_release() {
        let location = redirect_of("github.com", "/rust-lang/rust/releases/latest")
            .unwrap()
            .expect("a redirect");
        assert!(tag_from_location(&location).is_some(), "{location}");
    }

    #[test]
    fn state_round_trips_and_a_broken_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.toml");
        assert_eq!(load(&path), State::default());
        let state = State {
            checked: 42,
            latest: Some("v1.0.3".into()),
            dismissed: None,
        };
        save(&path, &state).unwrap();
        assert_eq!(load(&path), state);
        std::fs::write(&path, "checked = \"soon\"").unwrap();
        assert_eq!(load(&path), State::default());
    }
}
