//! The Windows side: the registry, the host window and the COM thread.
//!
//! The host window (a child of the app's window) belongs to the UI thread,
//! so moving and hiding it never waits for another thread. The handler is
//! created, initialised and told about size changes on a thread of its own;
//! the UI thread only posts to it. Host windows are never destroyed while a
//! handler may still have a window in them: the thread hands its host back
//! to [`POOL`] once the handler is unloaded, and a host whose thread got
//! stuck is just left hidden.

use super::{colorref, Classes, Colors, Handler, PxRect};
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{
    COLORREF, ERROR_MORE_DATA, E_NOINTERFACE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, FillRect, HDC};
use windows::Win32::System::Com::{
    CLSIDFromString, CoCreateInstance, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, STGM_READ,
    STGM_SHARE_DENY_NONE,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CLASSES_ROOT, REG_DWORD, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RRF_RT_ANY,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
use windows::Win32::UI::Shell::{
    IInitializeWithItem, IPreviewHandler, IPreviewHandlerVisuals, IShellItem,
    SHCreateItemFromParsingName, SHCreateStreamOnFileEx,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, IsWindow, PeekMessageW, PostThreadMessageW, RegisterClassExW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    GWLP_USERDATA, LWA_ALPHA, MSG, PM_NOREMOVE, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SW_HIDE, WM_APP, WM_ERASEBKGND, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOPARENTNOTIFY,
};

/// Wakes the COM thread up to look at its commands.
const WM_WAKE: u32 = WM_APP + 0x5a;

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// `HKEY_CLASSES_ROOT` (the machine's and the user's classes together).
pub(super) struct Registry;

impl Classes for Registry {
    fn value(&self, key: &str, name: &str) -> Option<String> {
        let (key, name) = (wide(key), wide(name));
        let mut buf = [0u16; 1024];
        let mut size = std::mem::size_of_val(&buf) as u32;
        let mut kind = REG_VALUE_TYPE::default();
        // SAFETY: `buf` is `size` bytes; the strings are null-terminated.
        let err = unsafe {
            RegGetValueW(
                HKEY_CLASSES_ROOT,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_ANY,
                Some(&mut kind),
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if err == ERROR_MORE_DATA {
            return Some(String::new()); // there, but longer than we need
        }
        if !err.is_ok() {
            return None;
        }
        match kind {
            REG_SZ | REG_EXPAND_SZ => {
                let len = (size as usize / 2).min(buf.len());
                let s = String::from_utf16_lossy(&buf[..len]);
                Some(s.trim_end_matches('\0').to_owned())
            }
            REG_DWORD if size >= 4 => {
                Some((u32::from(buf[0]) | u32::from(buf[1]) << 16).to_string())
            }
            _ => Some(String::new()),
        }
    }
}

/// Host windows given back by their threads: parent, layered, window.
static POOL: Mutex<Vec<(isize, bool, isize)>> = Mutex::new(Vec::new());

/// The host window's background colour (`COLORREF` + 1) is kept in its
/// user data; it fills what the handler doesn't cover (yet).
unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_ERASEBKGND {
        // SAFETY: our own window; `wp` is the device context to erase.
        unsafe {
            let color = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if color != 0 {
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let brush = CreateSolidBrush(COLORREF((color - 1) as u32));
                FillRect(HDC(wp.0 as _), &rect, brush);
                let _ = DeleteObject(brush.into());
                return LRESULT(1);
            }
        }
    }
    // SAFETY: default handling of a message to our window.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// A host window in `parent`: one given back, or a new one (hidden).
fn host_window(parent: isize, layered: bool) -> std::io::Result<isize> {
    let reused = {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let i = pool
            .iter()
            .position(|&(p, l, _)| p == parent && l == layered);
        i.map(|i| pool.swap_remove(i).2)
    };
    // SAFETY: the window was ours; it may have gone with its parent.
    if let Some(h) = reused.filter(|&h| unsafe { IsWindow(Some(HWND(h as _))) }.as_bool()) {
        return Ok(h);
    }
    static CLASS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    // SAFETY: plain window creation with a registered class; the strings
    // live in statics or until the calls return.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = CLASS.get_or_init(|| {
            let name = wide("OxplorerPreviewHost");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(host_proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(name.as_ptr()),
                ..Default::default()
            };
            RegisterClassExW(&wc);
            name
        });
        let mut ex = WS_EX_NOPARENTNOTIFY;
        if layered {
            ex |= WS_EX_LAYERED;
        }
        let hwnd = CreateWindowExW(
            ex,
            PCWSTR(class.as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            Some(HWND(parent as _)),
            None,
            Some(instance.into()),
            None,
        )?;
        if layered {
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        }
        Ok(hwnd.0 as isize)
    }
}

enum Cmd {
    Load {
        generation: u64,
        path: PathBuf,
        handler: Handler,
        size: [i32; 2],
        colors: Colors,
    },
    Resize([i32; 2]),
    Unload,
}

type Report = (u64, Result<(), String>);

/// The COM thread of one host window.
struct Worker {
    tx: Sender<Cmd>,
    /// Its thread ID (to wake it), once it has a message queue.
    tid: Option<u32>,
    tid_rx: Receiver<u32>,
    reports: Receiver<Report>,
}

impl Worker {
    fn start(host: isize, key: (isize, bool), repaint: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (tid_tx, tid_rx) = crossbeam_channel::bounded(1);
        let (report_tx, reports) = crossbeam_channel::unbounded();
        std::thread::Builder::new()
            .name("preview-handler".into())
            .spawn(move || run(host, key, rx, tid_tx, report_tx, repaint))
            .expect("spawn preview handler thread");
        Worker {
            tx,
            tid: None,
            tid_rx,
            reports,
        }
    }

    fn send(&mut self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
        self.wake();
    }

    /// Without a thread ID the thread hasn't looked at its commands yet.
    fn wake(&mut self) {
        if self.tid.is_none() {
            self.tid = self.tid_rx.try_recv().ok();
        }
        if let Some(tid) = self.tid {
            // SAFETY: posting a message never waits for the thread.
            let _ = unsafe { PostThreadMessageW(tid, WM_WAKE, WPARAM(0), LPARAM(0)) };
        }
    }

    /// Let the thread finish: it unloads the handler and gives the host back.
    fn stop(self) {
        let Worker {
            tx, tid, tid_rx, ..
        } = self;
        drop(tx);
        if let Some(tid) = tid.or_else(|| tid_rx.try_recv().ok()) {
            // SAFETY: posting a message never waits for the thread.
            let _ = unsafe { PostThreadMessageW(tid, WM_WAKE, WPARAM(0), LPARAM(0)) };
        }
    }
}

/// The UI thread's side: the host window and its COM thread.
pub(super) struct Host {
    parent: isize,
    layered: bool,
    hwnd: isize,
    worker: Option<Worker>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    /// Where the window is, and whether it is shown.
    placed: Option<PxRect>,
    visible: bool,
    /// The size the handler was last given.
    size: [i32; 2],
}

impl Host {
    pub(super) fn new(
        parent: isize,
        layered: bool,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        let hwnd = host_window(parent, layered)?;
        Ok(Host {
            parent,
            layered,
            hwnd,
            worker: Some(Worker::start(hwnd, (parent, layered), repaint.clone())),
            repaint,
            placed: None,
            visible: false,
            size: [1, 1],
        })
    }

    fn worker(&mut self) -> &mut Worker {
        self.worker.as_mut().expect("there is always a worker")
    }

    /// The thread is stuck in a handler: leave it (and its window) alone
    /// and carry on with new ones.
    pub(super) fn restart(&mut self) {
        self.hide();
        let old = self.worker.take().expect("there is always a worker");
        old.stop();
        match host_window(self.parent, self.layered) {
            Ok(hwnd) => self.hwnd = hwnd,
            Err(e) => log::warn!("preview host window: {e}"),
        }
        self.placed = None;
        let key = (self.parent, self.layered);
        self.worker = Some(Worker::start(self.hwnd, key, self.repaint.clone()));
    }

    pub(super) fn load(&mut self, generation: u64, path: &Path, handler: &Handler, colors: Colors) {
        // SAFETY: our own window.
        unsafe {
            SetWindowLongPtrW(
                HWND(self.hwnd as _),
                GWLP_USERDATA,
                colorref(colors.background) as isize + 1,
            );
        }
        let size = self.size;
        self.worker().send(Cmd::Load {
            generation,
            path: path.to_owned(),
            handler: handler.clone(),
            size,
            colors,
        });
    }

    pub(super) fn unload(&mut self) {
        self.hide();
        self.worker().send(Cmd::Unload);
    }

    pub(super) fn reports(&mut self) -> Vec<Report> {
        self.worker().reports.try_iter().collect()
    }

    fn hide(&mut self) {
        if self.visible {
            // SAFETY: our own window (owned by this thread: nothing waits).
            let _ = unsafe { ShowWindow(HWND(self.hwnd as _), SW_HIDE) };
            self.visible = false;
        }
    }

    pub(super) fn place(&mut self, rect: Option<PxRect>, shown: bool) {
        let Some(r) = rect.filter(|r| r.w > 0 && r.h > 0) else {
            self.hide();
            return;
        };
        if self.placed != Some(r) || self.visible != shown {
            let flags = SWP_NOZORDER
                | SWP_NOACTIVATE
                | if shown {
                    SWP_SHOWWINDOW
                } else {
                    SWP_HIDEWINDOW
                };
            // SAFETY: our own window (owned by this thread: nothing waits).
            let _ = unsafe { SetWindowPos(HWND(self.hwnd as _), None, r.x, r.y, r.w, r.h, flags) };
            self.placed = Some(r);
            self.visible = shown;
        }
        if self.size != [r.w, r.h] {
            self.size = [r.w, r.h];
            self.worker().send(Cmd::Resize([r.w, r.h]));
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.hide();
        if let Some(worker) = self.worker.take() {
            worker.stop();
        }
    }
}

/// The COM thread: a single-threaded apartment that pumps messages (for
/// in-process handlers' windows and calls back from `prevhost.exe`) and
/// looks at its commands between them, never in the middle of a call.
fn run(
    host: isize,
    key: (isize, bool),
    rx: Receiver<Cmd>,
    tid_tx: Sender<u32>,
    reports: Sender<Report>,
    repaint: Arc<dyn Fn() + Send + Sync>,
) {
    // SAFETY: COM is set up for this thread and taken down at its end; the
    // message calls only use this thread's queue.
    unsafe {
        let ole = OleInitialize(None).is_ok();
        let mut msg = MSG::default();
        // Create the message queue before anyone posts to it.
        let _ = PeekMessageW(&mut msg, None, WM_WAKE, WM_WAKE, PM_NOREMOVE);
        let _ = tid_tx.send(GetCurrentThreadId());
        let mut current: Option<IPreviewHandler> = None;
        'run: loop {
            let mut cmds = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(cmd) => cmds.push(cmd),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'run,
                }
            }
            // Only the last size counts.
            let last_resize = cmds.iter().rposition(|c| matches!(c, Cmd::Resize(_)));
            for (i, cmd) in cmds.into_iter().enumerate() {
                match cmd {
                    Cmd::Load {
                        generation,
                        path,
                        handler,
                        size,
                        colors,
                    } => {
                        unload(&mut current);
                        let result = load(host, &path, &handler, size, colors);
                        let result = match result {
                            Ok(h) => {
                                current = Some(h);
                                Ok(())
                            }
                            Err(e) => Err(message(&e)),
                        };
                        let _ = reports.send((generation, result));
                        repaint();
                    }
                    Cmd::Resize([w, h]) if Some(i) == last_resize => {
                        if let Some(handler) = &current {
                            let _ = handler.SetRect(&RECT {
                                left: 0,
                                top: 0,
                                right: w,
                                bottom: h,
                            });
                        }
                    }
                    Cmd::Resize(_) => {}
                    Cmd::Unload => unload(&mut current),
                }
            }
            if GetMessageW(&mut msg, None, 0, 0).0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        unload(&mut current);
        POOL.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((key.0, key.1, host));
        if ole {
            OleUninitialize();
        }
    }
}

fn unload(current: &mut Option<IPreviewHandler>) {
    if let Some(handler) = current.take() {
        // SAFETY: a handler created on this thread.
        let _ = unsafe { handler.Unload() };
    }
}

/// Create the handler, give it the file and the window, and let it draw.
fn load(
    host: isize,
    path: &Path,
    handler: &Handler,
    [w, h]: [i32; 2],
    colors: Colors,
) -> windows::core::Result<IPreviewHandler> {
    let clsid = wide(&handler.clsid);
    let context = if handler.in_process {
        CLSCTX_INPROC_SERVER
    } else {
        CLSCTX_LOCAL_SERVER
    };
    // SAFETY: COM calls on this thread's apartment; the strings outlive them.
    unsafe {
        let clsid = CLSIDFromString(PCWSTR(clsid.as_ptr()))?;
        let preview: IPreviewHandler = CoCreateInstance(&clsid, None, context)?;
        let shown = (|| {
            initialize(&preview, path)?;
            if let Ok(visuals) = preview.cast::<IPreviewHandlerVisuals>() {
                let _ = visuals.SetBackgroundColor(COLORREF(colorref(colors.background)));
                let _ = visuals.SetTextColor(COLORREF(colorref(colors.text)));
            }
            let rect = RECT {
                left: 0,
                top: 0,
                right: w.max(1),
                bottom: h.max(1),
            };
            preview.SetWindow(HWND(host as _), &rect)?;
            preview.DoPreview()
        })();
        match shown {
            Ok(()) => Ok(preview),
            Err(e) => {
                let _ = preview.Unload();
                Err(e)
            }
        }
    }
}

/// Hand the file to the handler: as a stream where it takes one (what
/// Explorer prefers), else as a shell item or a path.
unsafe fn initialize(preview: &IPreviewHandler, path: &Path) -> windows::core::Result<()> {
    let wpath = wide(path);
    let path = PCWSTR(wpath.as_ptr());
    // SAFETY: COM calls on this thread's apartment; `wpath` outlives them.
    unsafe {
        if let Ok(init) = preview.cast::<IInitializeWithStream>() {
            let mode = (STGM_READ | STGM_SHARE_DENY_NONE).0;
            let stream = SHCreateStreamOnFileEx(path, mode, 0, false, None)?;
            return init.Initialize(&stream, STGM_READ.0);
        }
        if let Ok(init) = preview.cast::<IInitializeWithItem>() {
            let item: IShellItem = SHCreateItemFromParsingName(path, None)?;
            return init.Initialize(&item, STGM_READ.0);
        }
        if let Ok(init) = preview.cast::<IInitializeWithFile>() {
            return init.Initialize(path, STGM_READ.0);
        }
    }
    Err(E_NOINTERFACE.into())
}

/// Windows' text for the error, or its code.
fn message(e: &windows::core::Error) -> String {
    let text = e.message();
    let text = text.trim();
    if text.is_empty() {
        format!("{:#010x}", e.code().0)
    } else {
        text.to_owned()
    }
}

/// A top-level window to host previews in (for tests).
#[cfg(test)]
pub(super) fn test_window() -> isize {
    use windows::Win32::UI::WindowsAndMessaging::{
        WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    let class = wide("Static");
    // SAFETY: a plain window of a system class.
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR::null(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
            100,
            100,
            500,
            400,
            None,
            None,
            None,
            None,
        )
        .expect("test window")
        .0 as isize
    }
}

/// Handle this thread's window messages for a while (for tests).
#[cfg(test)]
pub(super) fn pump(time: std::time::Duration) {
    use windows::Win32::UI::WindowsAndMessaging::PM_REMOVE;
    let end = std::time::Instant::now() + time;
    let mut msg = MSG::default();
    while std::time::Instant::now() < end {
        // SAFETY: this thread's own queue.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
