//! Window chrome: backdrop, title bar and dragging the window by its caption.

use super::*;

impl OxApp {
    /// The backdrop in effect: turning one on needs a restart, as the window
    /// must be created see-through.
    pub(super) fn backdrop(&self) -> Backdrop {
        if self.see_through {
            self.settings.appearance.backdrop
        } else {
            Backdrop::Off
        }
    }

    /// Give the window its Windows 11 look (backdrop, corners, title bar),
    /// and put the tabs into the title bar or take them out again.
    pub(super) fn style_window(&mut self, ctx: &Context, frame: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let tabs_in_title = self.settings.appearance.tabs_in_title_bar;
        let style = (
            self.backdrop(),
            ctx.global_style().visuals.dark_mode,
            tabs_in_title,
        );
        if !ox_core::shell::supported()
            || (self.window_style == Some(style) && self.title_bar_applied == Some(tabs_in_title))
        {
            return;
        }
        let Ok(handle) = frame.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(h) = handle.as_raw() else {
            return;
        };
        let hwnd = h.hwnd.get();
        if self.window_style != Some(style) {
            ox_core::shell::set_window_style(hwnd, style.0, style.1, style.2);
            self.window_style = Some(style);
        }
        if self.title_bar_applied != Some(tabs_in_title) {
            if tabs_in_title {
                let repaint = ctx.clone();
                self.ui.title_bar =
                    ox_core::titlebar::enable(hwnd, move || repaint.request_repaint());
            } else {
                if self.title_bar_applied == Some(true) {
                    ox_core::titlebar::disable(hwnd);
                }
                self.ui.title_bar = false;
            }
            self.title_bar_applied = Some(tabs_in_title);
        }
    }

    /// Tell Windows where the title bar strip has things to click, where
    /// the maximize button is and where it moves the window.
    pub(super) fn publish_title_bar(&self, ctx: &Context) {
        if !self.ui.title_bar {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let px = |r: egui::Rect| {
            [
                (r.min.x * ppp).floor() as i32,
                (r.min.y * ppp).floor() as i32,
                (r.max.x * ppp).ceil() as i32,
                (r.max.y * ppp).ceil() as i32,
            ]
        };
        let mut client: Vec<_> = self.ui.title_hits.iter().copied().map(px).collect();
        // Menus, popups and windows over the strip take the clicks too.
        ctx.memory(|m| {
            for layer in m.areas().visible_layer_ids() {
                let over = !matches!(
                    layer.order,
                    egui::Order::Background | egui::Order::Tooltip | egui::Order::Debug
                ) && layer.id != crate::ui::header::caption_buttons_id();
                if let Some(rect) = m.area_rect(layer.id).filter(|_| over) {
                    client.push(px(rect));
                }
            }
        });
        ox_core::titlebar::set_layout(ox_core::titlebar::Layout {
            height: (self.ui.title_height * ppp).ceil() as i32,
            max_button: self.ui.max_button.map(px),
            client,
        });
    }

    /// Files dragged past the window's edge go on as a Windows drag & drop,
    /// so they can be dropped into other programs (Explorer, mail, editors…).
    pub(super) fn drag_out_of_window(&mut self, ctx: &Context) {
        if !ox_core::shell::supported() {
            return;
        }
        let Some(items) = egui::DragAndDrop::payload::<crate::ui::DragItems>(ctx) else {
            return;
        };
        let window = ctx.content_rect();
        let outside = ctx.input(|i| {
            i.pointer.primary_down()
                && (i.pointer.hover_pos().is_none()
                    || i.pointer.latest_pos().is_some_and(|p| !window.contains(p)))
        });
        // Recycle Bin and archive items aren't files other programs can open.
        let paths: Vec<PathBuf> = items.0.iter().filter(|p| p.exists()).cloned().collect();
        if !outside || paths.is_empty() {
            return;
        }
        egui::DragAndDrop::clear_payload(ctx);
        ox_core::shell::drag_out(&paths);
        self.release_pointer = true;
        ctx.request_repaint();
    }
}
