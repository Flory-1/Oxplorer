//! Several Oxplorer windows: each runs as its own process. This is the
//! command line a window is started with, which window owns the saved
//! session, and starting a new window.
//!
//! The saved session (Settings → Start page → Restore session) belongs to
//! the first window started normally (from the Start menu, Win+E, a folder
//! opened in Oxplorer…): it restores the session and saves it when it
//! closes. Windows opened with Ctrl+N or by dragging a tab out, and windows
//! started while another one owns the session, neither restore nor save it,
//! so they can't overwrite the main window's tabs.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// What a window was started for.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Launch {
    /// `--new-window`: opened from another window (Ctrl+N, a tab dragged
    /// out). Doesn't restore or own the session.
    pub new_window: bool,
    /// `--pos x,y`: where the window goes (screen points).
    pub pos: Option<[f32; 2]>,
    /// `--tabs <file>`: tabs handed over by the window that opened this one
    /// (a session file, removed once read).
    pub tabs: Option<PathBuf>,
    /// A folder to open (the first other argument).
    pub folder: Option<String>,
}

/// Read the arguments (without the program's name).
pub fn parse(args: impl IntoIterator<Item = String>) -> Launch {
    let mut launch = Launch::default();
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--new-window" => launch.new_window = true,
            "--pos" => launch.pos = args.next().as_deref().and_then(parse_pos),
            "--tabs" => launch.tabs = args.next().map(PathBuf::from),
            _ if launch.folder.is_none() => launch.folder = Some(a),
            _ => {}
        }
    }
    launch
}

fn parse_pos(text: &str) -> Option<[f32; 2]> {
    let (x, y) = text.split_once(',')?;
    let (x, y) = (x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?);
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

impl Launch {
    /// The command line for this launch (the reverse of [`parse`]).
    pub fn to_args(&self) -> Vec<OsString> {
        let mut out = Vec::new();
        if self.new_window {
            out.push("--new-window".into());
        }
        if let Some([x, y]) = self.pos {
            out.push("--pos".into());
            out.push(format!("{},{}", x.round(), y.round()).into());
        }
        if let Some(t) = &self.tabs {
            out.push("--tabs".into());
            out.push(t.into());
        }
        if let Some(f) = &self.folder {
            out.push(f.into());
        }
        out
    }
}

/// Start another Oxplorer window.
pub fn open_window(launch: &Launch) -> io::Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .args(launch.to_args())
        .spawn()
        .map(drop)
}

/// A file to hand tabs over to a new window in (`--tabs`).
pub fn handoff_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("oxplorer-tabs-{}-{nanos}.toml", std::process::id()))
}

/// Held by the window that owns the saved session, for as long as it runs
/// (Windows lets go of it when the process ends, also after a crash).
pub struct SessionLock {
    _file: Option<File>,
}

/// Become the owner of the saved session, unless another window is. If
/// the lock file can't be used at all, this window owns it (as when there
/// was only ever one window).
pub fn own_session(dir: &Path) -> Option<SessionLock> {
    let file = std::fs::create_dir_all(dir).and_then(|()| {
        File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("session.lock"))
    });
    let file = match file {
        Ok(f) => f,
        Err(e) => {
            log::warn!("no session lock: {e}");
            return Some(SessionLock { _file: None });
        }
    };
    match file.try_lock() {
        Ok(()) => Some(SessionLock { _file: Some(file) }),
        Err(std::fs::TryLockError::WouldBlock) => None,
        Err(std::fs::TryLockError::Error(e)) => {
            log::warn!("no session lock: {e}");
            Some(SessionLock { _file: None })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Launch {
        parse(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn command_lines() {
        assert_eq!(args(&[]), Launch::default());
        let l = args(&["C:\\Users", "extra"]);
        assert_eq!(l.folder.as_deref(), Some("C:\\Users"));
        assert!(!l.new_window);
        let l = args(&["--new-window", "--pos", "100,-20.5", "D:\\x y"]);
        assert!(l.new_window);
        assert_eq!(l.pos, Some([100.0, -20.5]));
        assert_eq!(l.folder.as_deref(), Some("D:\\x y"));
        // A broken position is ignored, the rest still counts.
        let l = args(&["--pos", "nope", "--tabs", "t.toml"]);
        assert_eq!(l.pos, None);
        assert_eq!(l.tabs, Some(PathBuf::from("t.toml")));
        assert_eq!(l.folder, None);
        // There and back.
        let l = Launch {
            new_window: true,
            pos: Some([10.0, 20.0]),
            tabs: Some("a.toml".into()),
            folder: Some("C:\\".into()),
        };
        let back = parse(
            l.to_args()
                .into_iter()
                .map(|a| a.to_string_lossy().into_owned()),
        );
        assert_eq!(back, l);
    }

    #[test]
    fn one_window_owns_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let first = own_session(dir.path()).expect("nobody owns it yet");
        assert!(own_session(dir.path()).is_none(), "the first one does");
        drop(first);
        assert!(own_session(dir.path()).is_some(), "free again");
    }
}
