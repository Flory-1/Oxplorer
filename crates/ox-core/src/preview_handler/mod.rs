//! Office documents, mails and everything else Explorer can preview, shown
//! through Windows' preview handlers (`IPreviewHandler`).
//!
//! The handler of a file type is found as Explorer finds it ([`find`]). It
//! runs out of process in Windows' `prevhost.exe` surrogate where it is
//! registered so (a crashing handler can't take Oxplorer down), in process
//! only if it has to. All COM work happens on a thread of its own (a
//! single-threaded apartment with a message loop), so a slow handler never
//! holds up the UI; the handler draws into a child window of Oxplorer's
//! window that the UI moves over the preview tab ([`DocumentPreview::place`]).
//! Elsewhere there are no preview handlers ([`supported`] is false).

#[cfg(windows)]
mod win;

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The shell extension key preview handlers are registered under (the IID
/// of `IPreviewHandler`).
pub const SHELLEX_KEY: &str = "{8895b1c6-b41f-4c1c-a562-0d564250836f}";

/// A load taking longer than this is given up on: the tab says so, and the
/// next file gets a fresh thread (the stuck one is left to finish alone).
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// Whether preview handlers exist here.
pub fn supported() -> bool {
    cfg!(windows)
}

/// A registered preview handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handler {
    /// Its class ID, `{…}`.
    pub clsid: String,
    /// It has to run inside Oxplorer (it opts out of the surrogate with
    /// `DisableProcessIsolation`, or is registered only as an in-process DLL).
    pub in_process: bool,
}

/// Read access to `HKEY_CLASSES_ROOT`: the value `name` of `key` (`""` for
/// the default value) as text (numbers in decimal), `None` if missing.
pub trait Classes {
    fn value(&self, key: &str, name: &str) -> Option<String>;
}

/// The preview handler registered for files ending in `.ext`, looked up as
/// Explorer does: on the extension's key, its ProgID, under
/// `SystemFileAssociations` (for the extension and its perceived type) and
/// for all files (`*`). `None` if there is none, or it can't run here.
pub fn find(classes: &impl Classes, ext: &str) -> Option<Handler> {
    let ext = format!(".{}", ext.trim_start_matches('.').to_lowercase());
    if ext.len() < 2 {
        return None;
    }
    let shellex = |base: &str| {
        classes
            .value(&format!(r"{base}\ShellEx\{SHELLEX_KEY}"), "")
            .map(|v| v.trim().to_owned())
            .filter(|v| is_guid(v))
    };
    let nonempty = |v: Option<String>| v.filter(|v| !v.trim().is_empty());
    let progid = nonempty(classes.value(&ext, ""));
    let perceived = nonempty(classes.value(&ext, "PerceivedType"));
    let clsid = shellex(&ext)
        .or_else(|| progid.as_deref().and_then(shellex))
        .or_else(|| shellex(&format!(r"SystemFileAssociations\{ext}")))
        .or_else(|| perceived.and_then(|p| shellex(&format!(r"SystemFileAssociations\{p}"))))
        .or_else(|| shellex("*"))?;
    registered(classes, &clsid)
}

/// How the class `clsid` can be created: out of process where its
/// registration allows it (an `AppID`, e.g. `prevhost.exe`, or a local
/// server), in process only if it opts out of that or is registered only as
/// a 64-bit DLL. 32-bit DLLs can only run out of process.
fn registered(classes: &impl Classes, clsid: &str) -> Option<Handler> {
    let has = |key: &str, sub: &str| classes.value(&format!(r"{key}\{sub}"), "").is_some();
    let native = format!(r"CLSID\{clsid}");
    let wow = format!(r"WOW6432Node\CLSID\{clsid}");
    let (key, dll) = if has(&native, "InprocServer32") || has(&native, "LocalServer32") {
        let dll = has(&native, "InprocServer32");
        (native, dll)
    } else if has(&wow, "InprocServer32") || has(&wow, "LocalServer32") {
        (wow, false)
    } else {
        return None;
    };
    let isolated = classes
        .value(&key, "DisableProcessIsolation")
        .is_none_or(|v| v.trim() != "1");
    let surrogate = classes.value(&key, "AppID").is_some() || has(&key, "LocalServer32");
    if isolated && surrogate {
        Some(Handler {
            clsid: clsid.to_owned(),
            in_process: false,
        })
    } else {
        dll.then(|| Handler {
            clsid: clsid.to_owned(),
            in_process: true,
        })
    }
}

/// `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`.
fn is_guid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 38
        && b[0] == b'{'
        && b[37] == b'}'
        && b[1..37].iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// The preview handler for `path` (by its extension), from the registry.
pub fn handler_for(path: &Path) -> Option<Handler> {
    let ext = path.extension()?.to_str()?;
    #[cfg(windows)]
    {
        find(&win::Registry, ext)
    }
    #[cfg(not(windows))]
    {
        let _ = ext;
        None
    }
}

/// A rectangle in physical pixels of the parent window's client area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The pixels the rectangle `min`–`max` (in points, as the UI measures)
/// covers with `pixels_per_point` (the display scale and zoom). Edges are
/// rounded, so neighbouring rectangles don't overlap or leave a gap.
pub fn to_pixels(min: [f32; 2], max: [f32; 2], pixels_per_point: f32) -> PxRect {
    let px = |v: f32| (v * pixels_per_point).round() as i32;
    let (x, y) = (px(min[0]), px(min[1]));
    PxRect {
        x,
        y,
        w: (px(max[0]) - x).max(0),
        h: (px(max[1]) - y).max(0),
    }
}

/// Colours handlers that support it (`IPreviewHandlerVisuals`) draw with,
/// to match the theme: sRGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Colors {
    pub background: [u8; 3],
    pub text: [u8; 3],
}

/// Windows' `COLORREF` of an sRGB colour: `0x00BBGGRR`.
pub fn colorref([r, g, b]: [u8; 3]) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// What the preview shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Nothing (yet).
    Idle,
    Loading,
    /// The handler draws the file.
    Shown,
    /// The handler couldn't show it: why.
    Failed(String),
    /// The handler took longer than [`TIMEOUT`].
    TimedOut,
}

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Idle,
    Loading(Instant),
    Shown,
    Failed(String),
}

/// Bookkeeping of what is loaded, kept apart from COM so it can be tested.
/// Every load gets a new generation; reports of older ones (finished after
/// the selection moved on) are ignored.
#[derive(Debug, Default)]
pub struct Tracker {
    generation: u64,
    path: Option<PathBuf>,
    phase: Phase,
    /// The latest load sent to the thread, until it reports back: its
    /// generation and when it was sent.
    busy: Option<(u64, Instant)>,
}

impl Tracker {
    /// Start showing `path` unless it is already: the generation of the
    /// load to start.
    pub fn open(&mut self, path: &Path, now: Instant) -> Option<u64> {
        if self.path.as_deref() == Some(path) {
            return None;
        }
        self.generation += 1;
        self.path = Some(path.to_owned());
        self.phase = Phase::Loading(now);
        self.busy = Some((self.generation, now));
        Some(self.generation)
    }

    /// Stop showing the file. True if there was one.
    pub fn unload(&mut self) -> bool {
        self.generation += 1;
        self.phase = Phase::Idle;
        self.path.take().is_some()
    }

    /// The load of `generation` is done.
    pub fn finished(&mut self, generation: u64, result: Result<(), String>) {
        if self.busy.is_some_and(|(g, _)| g == generation) {
            self.busy = None;
        }
        if generation == self.generation && matches!(self.phase, Phase::Loading(_)) {
            self.phase = match result {
                Ok(()) => Phase::Shown,
                Err(e) => Phase::Failed(e),
            };
        }
    }

    pub fn status(&self, now: Instant) -> Status {
        match &self.phase {
            Phase::Idle => Status::Idle,
            Phase::Loading(since) if now.duration_since(*since) >= TIMEOUT => Status::TimedOut,
            Phase::Loading(_) => Status::Loading,
            Phase::Shown => Status::Shown,
            Phase::Failed(e) => Status::Failed(e.clone()),
        }
    }

    /// The thread has been busy with a load for longer than [`TIMEOUT`].
    pub fn stuck(&self, now: Instant) -> bool {
        self.busy
            .is_some_and(|(_, since)| now.duration_since(since) >= TIMEOUT)
    }

    /// A fresh thread took over: nothing is loaded or loading any more.
    pub fn restarted(&mut self) {
        self.unload();
        self.busy = None;
    }

    /// The file shown or being loaded.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

/// The preview of one file at a time through its preview handler, drawn in
/// a child window of the app's window.
pub struct DocumentPreview {
    tracker: Tracker,
    #[cfg(windows)]
    host: win::Host,
}

impl DocumentPreview {
    /// A preview in a child window of `parent` (a window handle). `layered`:
    /// the parent is see-through (Mica/Acrylic), so the child gets a
    /// surface of its own to stay opaque. `repaint` is called (from another
    /// thread) when a load is done.
    pub fn new(
        parent: isize,
        layered: bool,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> io::Result<Self> {
        #[cfg(windows)]
        {
            Ok(Self {
                tracker: Tracker::default(),
                host: win::Host::new(parent, layered, std::sync::Arc::new(repaint))?,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = (parent, layered, repaint);
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    /// Show `path` with `handler` (unloading the file shown before).
    pub fn open(&mut self, path: &Path, handler: &Handler, colors: Colors) {
        let now = Instant::now();
        if self.tracker.path() == Some(path) {
            return;
        }
        #[cfg(windows)]
        if self.tracker.stuck(now) {
            self.host.restart();
            self.tracker.restarted();
        }
        if let Some(generation) = self.tracker.open(path, now) {
            #[cfg(windows)]
            self.host.load(generation, path, handler, colors);
            #[cfg(not(windows))]
            let _ = (generation, handler, colors);
        }
    }

    /// Let go of the file (the handler is unloaded).
    pub fn unload(&mut self) {
        if self.tracker.unload() {
            #[cfg(windows)]
            self.host.unload();
        }
    }

    pub fn status(&mut self) -> Status {
        #[cfg(windows)]
        for (generation, result) in self.host.reports() {
            self.tracker.finished(generation, result);
        }
        self.tracker.status(Instant::now())
    }

    /// The file shown or being loaded.
    pub fn path(&self) -> Option<&Path> {
        self.tracker.path()
    }

    /// Put the preview at `rect` of the parent window, or hide it (`None`).
    /// Call it every frame; it is only shown once the handler draws.
    pub fn place(&mut self, rect: Option<PxRect>) {
        let shown = self.status() == Status::Shown;
        #[cfg(windows)]
        self.host.place(rect, shown);
        #[cfg(not(windows))]
        let _ = (rect, shown);
    }
}

#[cfg(test)]
mod tests;
