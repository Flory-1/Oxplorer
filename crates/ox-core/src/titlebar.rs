//! Tabs in the title bar, like Windows 11's Explorer: the window keeps its
//! frame (borders, shadow, rounded corners, snapping) but loses the title
//! bar, and the app draws the tabs and the caption buttons there.
//!
//! Windows asks the window what is under the mouse (`WM_NCHITTEST`). The app
//! publishes the layout of the strip every frame ([`set_layout`]): where the
//! tabs and buttons are, and where the maximize button is. Everything else in
//! the strip moves the window like a title bar (drag, double-click to
//! maximize, right-click for the window menu), and the maximize button shows
//! Windows 11's snap layouts when hovered.

/// A rectangle in physical pixels of the window's client area:
/// left, top, right, bottom (right and bottom excluded).
pub type PxRect = [i32; 4];

fn contains(r: &PxRect, x: i32, y: i32) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}

/// The title bar strip as the app last drew it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    /// Height of the strip from the top of the window.
    pub height: i32,
    /// Where the maximize button is.
    pub max_button: Option<PxRect>,
    /// Things to click in the strip: tabs, buttons, open menus. Checked
    /// before the maximize button, so a menu over it works.
    pub client: Vec<PxRect>,
}

/// What is at a point of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// The app handles the mouse there.
    Client,
    /// Moves the window.
    Caption,
    MaxButton,
    /// Resize edges where the title bar was.
    Top,
    TopLeft,
    TopRight,
}

/// What is at `(x, y)` of a window `width` pixels wide. `border` is the
/// width of the resize edge along the top (none when maximized).
pub fn hit_test(layout: &Layout, x: i32, y: i32, width: i32, maximized: bool, border: i32) -> Hit {
    if !maximized && y < border {
        // Corners are a bit wider, as in Windows' own frame.
        return if x < border * 2 {
            Hit::TopLeft
        } else if x >= width - border * 2 {
            Hit::TopRight
        } else {
            Hit::Top
        };
    }
    if y >= layout.height || layout.client.iter().any(|r| contains(r, x, y)) {
        return Hit::Client;
    }
    if layout.max_button.is_some_and(|r| contains(&r, x, y)) {
        return Hit::MaxButton;
    }
    Hit::Caption
}

/// Whether the title bar can be replaced here.
pub fn supported() -> bool {
    cfg!(windows)
}

/// Hovered and pressed state of the maximize button (Windows tells the
/// window about the mouse there, not the app).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaxButtonState {
    pub hovered: bool,
    pub pressed: bool,
}

/// Remove the title bar of the window `hwnd`. `repaint` is called when the
/// maximize button's state changes. Returns whether it worked.
pub fn enable(hwnd: isize, repaint: impl Fn() + Send + Sync + 'static) -> bool {
    #[cfg(windows)]
    {
        // SAFETY: called on the window's thread with its handle.
        unsafe { imp::enable(hwnd, Box::new(repaint)) }
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, repaint);
        false
    }
}

/// Give the window its title bar back.
pub fn disable(hwnd: isize) {
    #[cfg(windows)]
    // SAFETY: called on the window's thread with its handle.
    unsafe {
        imp::disable(hwnd)
    }
    #[cfg(not(windows))]
    let _ = hwnd;
}

pub fn set_layout(layout: Layout) {
    #[cfg(windows)]
    imp::set_layout(layout);
    #[cfg(not(windows))]
    let _ = layout;
}

pub fn max_button() -> MaxButtonState {
    #[cfg(windows)]
    {
        imp::max_button()
    }
    #[cfg(not(windows))]
    MaxButtonState::default()
}

#[cfg(windows)]
mod imp {
    use super::{hit_test, Hit, Layout, MaxButtonState};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT,
    };
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, IsZoomed, SetWindowPos, ShowWindow, HTCAPTION, HTCLIENT,
        HTMAXBUTTON, HTTOP, HTTOPLEFT, HTTOPRIGHT, NCCALCSIZE_PARAMS, SM_CXPADDEDBORDER,
        SM_CYFRAME, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        SWP_NOZORDER, SW_MAXIMIZE, SW_RESTORE, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCHITTEST,
        WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE,
        WM_NCRBUTTONDOWN, WM_NCRBUTTONUP,
    };

    const SUBCLASS_ID: usize = 0x6f78_7469; // "oxti"

    struct State {
        layout: Layout,
        repaint: Box<dyn Fn() + Send + Sync>,
    }

    static STATE: Mutex<Option<State>> = Mutex::new(None);
    static HOVERED: AtomicBool = AtomicBool::new(false);
    static PRESSED: AtomicBool = AtomicBool::new(false);

    fn repaint() {
        if let Ok(state) = STATE.lock() {
            if let Some(s) = state.as_ref() {
                (s.repaint)();
            }
        }
    }

    /// Set the maximize button's state; repaint if it changed.
    fn set_button(hovered: bool, pressed: bool) {
        let a = HOVERED.swap(hovered, Ordering::AcqRel);
        let b = PRESSED.swap(pressed, Ordering::AcqRel);
        if a != hovered || b != pressed {
            repaint();
        }
    }

    pub(super) fn set_layout(layout: Layout) {
        if let Ok(mut state) = STATE.lock() {
            if let Some(s) = state.as_mut() {
                s.layout = layout;
            }
        }
    }

    pub(super) fn max_button() -> MaxButtonState {
        MaxButtonState {
            hovered: HOVERED.load(Ordering::Acquire),
            pressed: PRESSED.load(Ordering::Acquire),
        }
    }

    /// Height of the resize edge, which is also how far a maximized window
    /// reaches past the screen at the top.
    unsafe fn frame_height(hwnd: HWND) -> i32 {
        unsafe {
            let dpi = GetDpiForWindow(hwnd);
            GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        }
    }

    unsafe fn client_height(hwnd: HWND) -> i32 {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        rc.bottom - rc.top
    }

    /// Re-run `WM_NCCALCSIZE`, keeping the window the size it had *inside*
    /// (eframe saves and restores that size, so it must not grow or shrink
    /// by the title bar each time).
    unsafe fn frame_changed(hwnd: HWND) {
        unsafe {
            let before = client_height(hwnd);
            let flags =
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER;
            let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, flags | SWP_NOACTIVATE);
            let grown = client_height(hwnd) - before;
            if grown != 0 && !IsZoomed(hwnd).as_bool() {
                let mut rc = RECT::default();
                if GetWindowRect(hwnd, &mut rc).is_ok() {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        rc.right - rc.left,
                        rc.bottom - rc.top - grown,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }

    pub(super) unsafe fn enable(hwnd: isize, repaint: Box<dyn Fn() + Send + Sync>) -> bool {
        let hwnd = HWND(hwnd as *mut core::ffi::c_void);
        if let Ok(mut state) = STATE.lock() {
            *state = Some(State {
                layout: Layout::default(),
                repaint,
            });
        }
        unsafe {
            if !SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0).as_bool() {
                return false;
            }
            frame_changed(hwnd);
        }
        true
    }

    pub(super) unsafe fn disable(hwnd: isize) {
        let hwnd = HWND(hwnd as *mut core::ffi::c_void);
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
            frame_changed(hwnd);
        }
        if let Ok(mut state) = STATE.lock() {
            *state = None;
        }
        HOVERED.store(false, Ordering::Release);
        PRESSED.store(false, Ordering::Release);
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        unsafe {
            let on_max = wparam.0 as u32 == HTMAXBUTTON;
            match msg {
                WM_NCCALCSIZE if wparam.0 != 0 => {
                    // Let Windows work out the frame, then take the title
                    // bar (and top border) back into the client area.
                    let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                    let top = params.rgrc[0].top;
                    let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                    if result.0 != 0 {
                        return result;
                    }
                    // A maximized window hangs past the screen by its frame.
                    let hidden = if IsZoomed(hwnd).as_bool() {
                        frame_height(hwnd)
                    } else {
                        0
                    };
                    params.rgrc[0].top = top + hidden;
                    LRESULT(0)
                }
                WM_NCHITTEST => {
                    // The side and bottom edges stay Windows' own.
                    let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                    if result.0 as u32 != HTCLIENT {
                        return result;
                    }
                    let mut pt = POINT {
                        x: (lparam.0 & 0xFFFF) as i16 as i32,
                        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                    };
                    let _ = ScreenToClient(hwnd, &mut pt);
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let hit = match STATE.lock() {
                        Ok(state) => match state.as_ref() {
                            Some(s) => hit_test(
                                &s.layout,
                                pt.x,
                                pt.y,
                                rc.right,
                                IsZoomed(hwnd).as_bool(),
                                frame_height(hwnd),
                            ),
                            None => Hit::Client,
                        },
                        Err(_) => Hit::Client,
                    };
                    LRESULT(match hit {
                        Hit::Client => HTCLIENT,
                        Hit::Caption => HTCAPTION,
                        Hit::MaxButton => HTMAXBUTTON,
                        Hit::Top => HTTOP,
                        Hit::TopLeft => HTTOPLEFT,
                        Hit::TopRight => HTTOPRIGHT,
                    } as isize)
                }
                WM_NCMOUSEMOVE => {
                    set_button(on_max, on_max && PRESSED.load(Ordering::Acquire));
                    if on_max {
                        // Hear when the mouse leaves the button.
                        let mut track = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE | TME_NONCLIENT,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut track);
                        return LRESULT(0);
                    }
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }
                WM_NCMOUSELEAVE | WM_MOUSEMOVE => {
                    set_button(false, false);
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }
                // Windows would draw an old-style button of its own here.
                WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if on_max => {
                    set_button(true, true);
                    LRESULT(0)
                }
                WM_NCLBUTTONUP if on_max => {
                    if PRESSED.load(Ordering::Acquire) {
                        let cmd = if IsZoomed(hwnd).as_bool() {
                            SW_RESTORE
                        } else {
                            SW_MAXIMIZE
                        };
                        let _ = ShowWindow(hwnd, cmd);
                    }
                    set_button(true, false);
                    LRESULT(0)
                }
                WM_NCRBUTTONDOWN | WM_NCRBUTTONUP if on_max => LRESULT(0),
                _ => DefSubclassProc(hwnd, msg, wparam, lparam),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_testing_the_strip() {
        // A 1000 px wide window: strip 40 px high, a tab at 10..200, the
        // maximize button at 900..950, a menu reaching up over it.
        let mut layout = Layout {
            height: 40,
            max_button: Some([900, 0, 950, 40]),
            client: vec![[10, 6, 200, 40]],
        };
        let hit = |l: &Layout, x, y, max| hit_test(l, x, y, 1000, max, 8);
        assert_eq!(hit(&layout, 100, 20, false), Hit::Client); // the tab
        assert_eq!(hit(&layout, 500, 20, false), Hit::Caption); // empty strip
        assert_eq!(hit(&layout, 920, 20, false), Hit::MaxButton);
        assert_eq!(hit(&layout, 500, 300, false), Hit::Client); // below the strip
                                                                // Resize edges along the top, wider at the corners; none when maximized.
        assert_eq!(hit(&layout, 500, 3, false), Hit::Top);
        assert_eq!(hit(&layout, 5, 3, false), Hit::TopLeft);
        assert_eq!(hit(&layout, 995, 3, false), Hit::TopRight);
        assert_eq!(hit(&layout, 500, 3, true), Hit::Caption);
        assert_eq!(hit(&layout, 920, 3, true), Hit::MaxButton);
        // An open menu over the maximize button gets the clicks.
        layout.client.push([880, 30, 1000, 300]);
        assert_eq!(hit(&layout, 920, 35, false), Hit::Client);
        assert_eq!(hit(&layout, 920, 20, false), Hit::MaxButton);
    }
}
