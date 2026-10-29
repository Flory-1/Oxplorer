//! Windows Shell integration: real file icons, thumbnails, the native context
//! menu ("Show more options"), the native Properties dialog and "Open with…".
//!
//! All Shell calls run on one dedicated background thread that owns a hidden
//! helper window and pumps its messages. The UI thread only sends requests,
//! so native menus and dialogs never re-enter the egui event loop.
//!
//! On other platforms [`supported`] is `false` and requests are ignored.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, Sender};

/// What an icon is looked up by. Most files share the icon of their
/// extension; some file types carry their own icon.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IconKey {
    Folder,
    /// Lower-case extension without the dot ("" for files without one).
    Extension(String),
    /// Files with their own icon (programs, shortcuts, .ico files, drives).
    Path(PathBuf),
}

/// Extensions whose icon depends on the individual file.
const PER_FILE_ICONS: &[&str] = &[
    "exe",
    "ico",
    "lnk",
    "url",
    "cur",
    "ani",
    "msc",
    "scr",
    "cpl",
    "appref-ms",
    "dll",
];

impl IconKey {
    pub fn for_path(path: &Path, is_dir: bool) -> Self {
        if is_dir {
            // Drive roots have their own icons; other folders share one.
            return if path.parent().is_none() {
                IconKey::Path(path.to_path_buf())
            } else {
                IconKey::Folder
            };
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if PER_FILE_ICONS.contains(&ext.as_str()) {
            IconKey::Path(path.to_path_buf())
        } else {
            IconKey::Extension(ext)
        }
    }
}

/// A decoded icon: square RGBA (not premultiplied).
#[derive(Debug, Clone)]
pub struct IconImage {
    pub size: u32,
    pub rgba: Vec<u8>,
}

/// A thumbnail: RGBA with premultiplied alpha.
#[derive(Debug, Clone)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The Shell's thumbnail of a file (pictures, videos, PDFs, … — whatever has
/// a thumbnail handler), at most `size` pixels on the long side. Blocks, so
/// call it from a background thread. `None` on other platforms, for files
/// without a thumbnail and on errors.
pub fn thumbnail(path: &Path, size: u32) -> Option<Thumbnail> {
    #[cfg(windows)]
    {
        // SAFETY: COM is set up for the calling thread inside.
        unsafe { imp::thumbnail(path, size) }
    }
    #[cfg(not(windows))]
    {
        let _ = (path, size);
        None
    }
}

/// Start a Windows drag & drop of `paths` into other programs, as when files
/// are dragged out of Explorer. Blocks until they are dropped (or the drag is
/// cancelled) and returns whether they were dropped somewhere. Call it on the
/// UI thread while the mouse button is held. Does nothing on other platforms.
pub fn drag_out(paths: &[PathBuf]) -> bool {
    #[cfg(windows)]
    {
        // SAFETY: called on the window's (OLE) thread, see above.
        unsafe { imp::drag_out(paths) }
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        false
    }
}

/// Window look on Windows 11: the backdrop material behind see-through
/// parts of the window (Mica, Acrylic or none), rounded corners and a dark
/// or light title bar. `hwnd` is the window's handle; `no_title_bar` says
/// the tabs took the title bar's place (see `titlebar`). Older Windows
/// ignore what they don't know; other platforms ignore it all.
pub fn set_window_style(
    hwnd: isize,
    backdrop: crate::settings::Backdrop,
    dark: bool,
    no_title_bar: bool,
) {
    #[cfg(windows)]
    {
        // SAFETY: only window attributes of the given window are set.
        unsafe { imp::set_window_style(hwnd, backdrop, dark, no_title_bar) }
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, backdrop, dark, no_title_bar);
    }
}

/// Is the Windows Shell available?
pub fn supported() -> bool {
    cfg!(windows)
}

#[cfg_attr(not(windows), allow(dead_code))]
enum Request {
    Icon(IconKey),
    ContextMenu(Vec<PathBuf>),
    Properties(PathBuf),
    OpenWith(PathBuf),
}

/// Handle to the shell thread. Cheap to keep around for the whole app.
pub struct Shell {
    tx: Option<Sender<Request>>,
    icons: Receiver<(IconKey, Option<IconImage>)>,
    requested: std::cell::RefCell<HashSet<IconKey>>,
}

impl Shell {
    /// Start the shell thread. `notify` is called when an icon is ready.
    pub fn start(notify: impl Fn() + Send + 'static) -> Self {
        let (icon_tx, icon_rx) = crossbeam_channel::unbounded();
        let tx = if supported() {
            let (tx, rx) = crossbeam_channel::unbounded();
            let spawned = std::thread::Builder::new()
                .name("shell".into())
                .spawn(move || imp::run(rx, icon_tx, Box::new(notify)));
            spawned.ok().map(|_| tx)
        } else {
            drop(icon_tx);
            None
        };
        Self {
            tx,
            icons: icon_rx,
            requested: Default::default(),
        }
    }

    fn send(&self, r: Request) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(r);
        }
    }

    /// Ask for an icon; it arrives later through [`Shell::poll_icons`]. Each key is only loaded once.
    pub fn request_icon(&self, key: &IconKey) {
        if self.tx.is_some() && self.requested.borrow_mut().insert(key.clone()) {
            self.send(Request::Icon(key.clone()));
        }
    }

    /// Icons loaded since the last call (`None` if a lookup failed).
    pub fn poll_icons(&self) -> Vec<(IconKey, Option<IconImage>)> {
        self.icons.try_iter().collect()
    }

    /// Show the Explorer context menu for `paths` at the mouse cursor.
    pub fn context_menu(&self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.send(Request::ContextMenu(paths));
        }
    }

    /// Open the Windows Properties dialog of a file or folder.
    pub fn properties(&self, path: PathBuf) {
        self.send(Request::Properties(path));
    }

    /// Show the "Open with" dialog for a file.
    pub fn open_with(&self, path: PathBuf) {
        self.send(Request::OpenWith(path));
    }
}

#[cfg(windows)]
mod imp {
    use super::{IconImage, IconKey, Request, Thumbnail};
    use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use windows::core::{w, Interface, PCSTR, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
    };
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
    use windows::Win32::System::Com::IDataObject;
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Ole::{
        IDropSource, OleInitialize, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE,
        DROPEFFECT_NONE,
    };
    use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        BHID_DataObject, IContextMenu, IContextMenu2, IContextMenu3, IShellFolder, IShellItemArray,
        IShellItemImageFactory, SHBindToParent, SHCreateItemFromParsingName,
        SHCreateShellItemArrayFromIDLists, SHDoDragDrop, SHGetFileInfoW, SHGetImageList,
        SHOpenWithDialog, SHParseDisplayName, ShellExecuteExW, CMF_EXPLORE, CMF_NORMAL,
        CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OPENASINFO,
        SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW, SHFILEINFOW, SHGFI_SYSICONINDEX,
        SHGFI_USEFILEATTRIBUTES, SHIL_EXTRALARGE, SIIGBF_THUMBNAILONLY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
        DispatchMessageW, GetCursorPos, GetForegroundWindow, GetIconInfo, PeekMessageW,
        PostMessageW, RegisterClassW, SetForegroundWindow, TrackPopupMenuEx, TranslateMessage,
        HICON, ICONINFO, MSG, PM_REMOVE, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON,
        WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR, WM_NULL, WNDCLASSW, WS_POPUP,
    };

    const CMIC_MASK_UNICODE: u32 = 0x0000_4000;
    const CMIC_MASK_PTINVOKE: u32 = 0x2000_0000;
    const FIRST_CMD: u32 = 1;
    const LAST_CMD: u32 = 0x7FFF;

    thread_local! {
        /// The context menu being shown, so the helper window can forward
        /// owner-drawn submenu messages ("Send to", "Open with", …) to it.
        static ACTIVE_MENU: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
    }

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn run(
        rx: Receiver<Request>,
        icons: Sender<(IconKey, Option<IconImage>)>,
        notify: Box<dyn Fn() + Send>,
    ) {
        // SAFETY: plain COM/Win32 calls on this thread only.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let window = unsafe { create_helper_window() };
        loop {
            pump_messages();
            match rx.recv_timeout(Duration::from_millis(30)) {
                Ok(Request::Icon(key)) => {
                    let image = unsafe { load_icon(&key) };
                    let _ = icons.send((key, image));
                    notify();
                }
                Ok(Request::ContextMenu(paths)) => {
                    if let Some(hwnd) = window {
                        unsafe { show_context_menu(hwnd, &paths) };
                    }
                }
                Ok(Request::Properties(path)) => unsafe { properties(&path) },
                Ok(Request::OpenWith(path)) => unsafe { open_with(window, &path) },
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn pump_messages() {
        let mut msg = MSG::default();
        // SAFETY: standard message loop on the thread that owns the windows.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe extern "system" fn helper_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if matches!(
            msg,
            WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
        ) {
            let handled = ACTIVE_MENU.with(|m| {
                let menu = m.borrow();
                let menu = menu.as_ref()?;
                let mut result = LRESULT(0);
                if let Ok(m3) = menu.cast::<IContextMenu3>() {
                    // SAFETY: forwarding the message we received unchanged.
                    unsafe { m3.HandleMenuMsg2(msg, wparam, lparam, Some(&mut result)) }.ok()?;
                    return Some(result);
                }
                if let Ok(m2) = menu.cast::<IContextMenu2>() {
                    unsafe { m2.HandleMenuMsg(msg, wparam, lparam) }.ok()?;
                    return Some(LRESULT(0));
                }
                None
            });
            if let Some(r) = handled {
                return r;
            }
        }
        // SAFETY: default handling for everything else.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    unsafe fn create_helper_window() -> Option<HWND> {
        unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            let class = w!("OxplorerShellHelper");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(helper_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc);
            CreateWindowExW(
                Default::default(),
                class,
                w!(""),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .ok()
        }
    }

    // ---- icons ---------------------------------------------------------------

    unsafe fn load_icon(key: &IconKey) -> Option<IconImage> {
        unsafe {
            let (name, attrs, flags) = match key {
                IconKey::Folder => (
                    wide("folder"),
                    FILE_ATTRIBUTE_DIRECTORY,
                    SHGFI_USEFILEATTRIBUTES,
                ),
                IconKey::Extension(ext) if ext.is_empty() => {
                    (wide("file"), FILE_ATTRIBUTE_NORMAL, SHGFI_USEFILEATTRIBUTES)
                }
                IconKey::Extension(ext) => (
                    wide(format!("file.{ext}")),
                    FILE_ATTRIBUTE_NORMAL,
                    SHGFI_USEFILEATTRIBUTES,
                ),
                IconKey::Path(p) => (wide(p), Default::default(), Default::default()),
            };
            let mut info = SHFILEINFOW::default();
            let ok = SHGetFileInfoW(
                PCWSTR(name.as_ptr()),
                attrs,
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                flags | SHGFI_SYSICONINDEX,
            );
            if ok == 0 {
                return None;
            }
            let list: IImageList = SHGetImageList(SHIL_EXTRALARGE as i32).ok()?;
            let icon = list.GetIcon(info.iIcon, ILD_TRANSPARENT.0).ok()?;
            let image = icon_to_rgba(icon);
            let _ = DestroyIcon(icon);
            image
        }
    }

    /// Read the pixels of an HICON as RGBA.
    unsafe fn icon_to_rgba(icon: HICON) -> Option<IconImage> {
        unsafe {
            let mut info = ICONINFO::default();
            GetIconInfo(icon, &mut info).ok()?;
            let result = (|| {
                if info.hbmColor.is_invalid() {
                    return None; // monochrome icon
                }
                let mut bm = BITMAP::default();
                let n = GetObjectW(
                    HGDIOBJ(info.hbmColor.0),
                    std::mem::size_of::<BITMAP>() as i32,
                    Some(&mut bm as *mut BITMAP as *mut c_void),
                );
                if n == 0 || bm.bmWidth <= 0 || bm.bmWidth != bm.bmHeight {
                    return None;
                }
                let size = bm.bmWidth;
                let mut pixels = read_bitmap(info.hbmColor, size, size)?;
                if pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
                    // Old-style icon without alpha: use the AND mask instead.
                    let mask = read_bitmap(info.hbmMask, size, size)?;
                    let mask = mask.as_chunks::<4>().0;
                    for (p, m) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(mask) {
                        p[3] = if m[0] == 0 { 255 } else { 0 };
                    }
                }
                for p in pixels.as_chunks_mut::<4>().0 {
                    p.swap(0, 2); // BGRA -> RGBA
                }
                Some(IconImage {
                    size: size as u32,
                    rgba: pixels,
                })
            })();
            if !info.hbmColor.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
            }
            if !info.hbmMask.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
            }
            result
        }
    }

    /// 32-bit top-down BGRA pixels of a bitmap.
    unsafe fn read_bitmap(bitmap: HBITMAP, width: i32, height: i32) -> Option<Vec<u8>> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut buf = vec![0u8; (width * height * 4) as usize];
            let lines = GetDIBits(
                dc,
                bitmap,
                0,
                height as u32,
                Some(buf.as_mut_ptr() as *mut c_void),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            let _ = DeleteDC(dc);
            (lines == height).then_some(buf)
        }
    }

    // ---- thumbnails --------------------------------------------------------------

    pub(super) unsafe fn thumbnail(path: &Path, size: u32) -> Option<Thumbnail> {
        unsafe {
            // Once per thread; later calls just return S_FALSE.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let name = wide(std::path::absolute(path).ok()?);
            let factory: IShellItemImageFactory =
                SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None).ok()?;
            let side = size as i32;
            let bitmap = factory
                .GetImage(SIZE { cx: side, cy: side }, SIIGBF_THUMBNAILONLY)
                .ok()?;
            let result = (|| {
                let mut bm = BITMAP::default();
                let n = GetObjectW(
                    HGDIOBJ(bitmap.0),
                    std::mem::size_of::<BITMAP>() as i32,
                    Some(&mut bm as *mut BITMAP as *mut c_void),
                );
                let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
                if n == 0 || w <= 0 || h <= 0 || w > 4096 || h > 4096 {
                    return None;
                }
                let mut pixels = read_bitmap(bitmap, w, h)?;
                // Photos come without alpha (all zero): make them opaque.
                let opaque = pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
                for p in pixels.as_chunks_mut::<4>().0 {
                    p.swap(0, 2); // BGRA -> RGBA
                    if opaque {
                        p[3] = 255;
                    }
                }
                Some(Thumbnail {
                    width: w as u32,
                    height: h as u32,
                    rgba: pixels,
                })
            })();
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            result
        }
    }

    // ---- context menu, properties, open with ------------------------------------

    /// Absolute PIDLs, freed on drop.
    struct Pidls(Vec<*mut ITEMIDLIST>);

    impl Drop for Pidls {
        fn drop(&mut self) {
            for p in &self.0 {
                // SAFETY: allocated by SHParseDisplayName.
                unsafe { CoTaskMemFree(Some(*p as *const c_void)) };
            }
        }
    }

    unsafe fn show_context_menu(hwnd: HWND, paths: &[PathBuf]) {
        unsafe {
            // The Shell can only build one menu for items in the same folder.
            let parent = paths[0].parent().map(Path::to_path_buf);
            let paths: Vec<&PathBuf> = paths
                .iter()
                .filter(|p| p.parent().map(Path::to_path_buf) == parent)
                .collect();
            let mut pidls = Pidls(Vec::new());
            for p in &paths {
                let name = wide(p);
                let mut pidl = std::ptr::null_mut();
                if SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut pidl, 0, None).is_ok() {
                    pidls.0.push(pidl);
                }
            }
            let Some(&first) = pidls.0.first() else {
                return;
            };
            let Ok(folder) = SHBindToParent::<IShellFolder>(first, None) else {
                return;
            };
            // Child (last) ids of every item, relative to the shared parent.
            let children: Vec<*const ITEMIDLIST> = pidls
                .0
                .iter()
                .filter_map(|&p| {
                    let mut child = std::ptr::null_mut();
                    SHBindToParent::<IShellFolder>(p, Some(&mut child))
                        .ok()
                        .map(|_| child as *const ITEMIDLIST)
                })
                .collect();
            let Ok(menu) = folder.GetUIObjectOf::<IContextMenu>(hwnd, &children, None) else {
                return;
            };
            let Ok(hmenu) = CreatePopupMenu() else { return };
            if menu
                .QueryContextMenu(hmenu, 0, FIRST_CMD, LAST_CMD, CMF_NORMAL | CMF_EXPLORE)
                .is_err()
            {
                let _ = DestroyMenu(hmenu);
                return;
            }
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // The owner must be in the foreground or the menu won't close on outside clicks.
            let _ = SetForegroundWindow(hwnd);
            ACTIVE_MENU.with(|m| *m.borrow_mut() = Some(menu.clone()));
            let cmd = TrackPopupMenuEx(
                hmenu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
                pt.x,
                pt.y,
                hwnd,
                None,
            )
            .0 as u32;
            ACTIVE_MENU.with(|m| *m.borrow_mut() = None);
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            if cmd >= FIRST_CMD {
                let offset = (cmd - FIRST_CMD) as usize;
                let info = CMINVOKECOMMANDINFOEX {
                    cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                    fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
                    hwnd,
                    // MAKEINTRESOURCE: the verb is passed as a command offset.
                    lpVerb: PCSTR(offset as *const u8),
                    lpVerbW: PCWSTR(offset as *const u16),
                    nShow: SW_SHOWNORMAL.0,
                    ptInvoke: pt,
                    ..Default::default()
                };
                let _ = menu.InvokeCommand(
                    &info as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO,
                );
            }
            let _ = DestroyMenu(hmenu);
        }
    }

    // ---- window style ----------------------------------------------------------------

    pub(super) unsafe fn set_window_style(
        hwnd: isize,
        backdrop: crate::settings::Backdrop,
        dark: bool,
        no_title_bar: bool,
    ) {
        use crate::settings::Backdrop;
        use windows::Win32::Graphics::Dwm::{
            DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMSBT_NONE,
            DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWMWINDOWATTRIBUTE,
        };
        use windows::Win32::UI::Controls::MARGINS;
        unsafe {
            let hwnd = HWND(hwnd as *mut c_void);
            let set = |attr: DWMWINDOWATTRIBUTE, value: i32| {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    attr,
                    &value as *const i32 as *const c_void,
                    std::mem::size_of::<i32>() as u32,
                );
            };
            set(DWMWA_USE_IMMERSIVE_DARK_MODE, dark as i32);
            set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND.0);
            let kind = match backdrop {
                Backdrop::Off => DWMSBT_NONE,
                Backdrop::Mica => DWMSBT_MAINWINDOW,
                Backdrop::Acrylic => DWMSBT_TRANSIENTWINDOW,
            };
            set(DWMWA_SYSTEMBACKDROP_TYPE, kind.0);
            // The material shows through the client area only where the
            // frame is extended into it (all of it, with -1 margins). Where
            // it reaches over the top edge, DWM also draws the title bar's
            // caption buttons; without a title bar the app draws its own, so
            // then the frame is extended from the bottom up over the whole
            // window instead.
            let m = if backdrop == Backdrop::Off { 0 } else { -1 };
            let margins = if m != 0 && no_title_bar {
                MARGINS {
                    cxLeftWidth: 0,
                    cxRightWidth: 0,
                    cyTopHeight: 0,
                    cyBottomHeight: 0x7fff,
                }
            } else {
                MARGINS {
                    cxLeftWidth: m,
                    cxRightWidth: m,
                    cyTopHeight: m,
                    cyBottomHeight: m,
                }
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        }
    }

    // ---- drag out ------------------------------------------------------------------

    pub(super) unsafe fn drag_out(paths: &[PathBuf]) -> bool {
        unsafe {
            // winit has set up OLE on this thread already; this only adds a reference.
            let _ = OleInitialize(None);
            let mut pidls = Pidls(Vec::new());
            for p in paths {
                let Ok(abs) = std::path::absolute(p) else {
                    continue;
                };
                let name = wide(abs);
                let mut pidl = std::ptr::null_mut();
                if SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut pidl, 0, None).is_ok() {
                    pidls.0.push(pidl);
                }
            }
            if pidls.0.is_empty() {
                return false;
            }
            let ids: Vec<*const ITEMIDLIST> = pidls.0.iter().map(|&p| p as *const _).collect();
            let Ok(items) = SHCreateShellItemArrayFromIDLists(&ids) else {
                return false;
            };
            let items: IShellItemArray = items;
            let Ok(data) = items.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else {
                return false;
            };
            // No drop source of our own: the Shell's default one also draws
            // the drag image. The target decides between copy and move.
            let window = GetForegroundWindow();
            let effect = SHDoDragDrop(
                (!window.is_invalid()).then_some(window),
                &data,
                None::<&IDropSource>,
                DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK,
            );
            matches!(effect, Ok(e) if e != DROPEFFECT_NONE)
        }
    }

    unsafe fn properties(path: &Path) {
        unsafe {
            let file = wide(path);
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_INVOKEIDLIST,
                lpVerb: w!("properties"),
                lpFile: PCWSTR(file.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            let _ = ShellExecuteExW(&mut info);
        }
    }

    unsafe fn open_with(owner: Option<HWND>, path: &Path) {
        unsafe {
            let file = wide(path);
            let info = OPENASINFO {
                pcszFile: PCWSTR(file.as_ptr()),
                pcszClass: PCWSTR::null(),
                oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_EXEC,
            };
            let _ = SHOpenWithDialog(owner, &info);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{IconImage, IconKey, Request};
    use crossbeam_channel::{Receiver, Sender};

    pub(super) fn run(
        _rx: Receiver<Request>,
        _icons: Sender<(IconKey, Option<IconImage>)>,
        _notify: Box<dyn Fn() + Send>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_keys() {
        assert_eq!(IconKey::for_path(Path::new("/a/b"), true), IconKey::Folder);
        assert_eq!(
            IconKey::for_path(Path::new("/a/Report.PDF"), false),
            IconKey::Extension("pdf".into())
        );
        assert_eq!(
            IconKey::for_path(Path::new("/a/setup.exe"), false),
            IconKey::Path("/a/setup.exe".into())
        );
        assert_eq!(
            IconKey::for_path(Path::new("/a/README"), false),
            IconKey::Extension(String::new())
        );
    }

    #[test]
    fn unsupported_platforms_do_nothing() {
        let shell = Shell::start(|| {});
        shell.request_icon(&IconKey::Folder);
        shell.context_menu(vec!["/x".into()]);
        if !supported() {
            assert!(shell.poll_icons().is_empty());
        }
    }
}
