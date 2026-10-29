//! Quick Look: opening and closing it, and its keys while it is open
//! (drawn in `ui/quick_look.rs`).

use super::*;
use crate::ui::quick_look::{current, QuickLook};

impl OxApp {
    /// Open Quick Look on the selected item, or close it.
    pub fn toggle_quick_look(&mut self) {
        if self.quick_look.is_some() {
            self.close_quick_look();
            return;
        }
        let tab = self.tabs.active();
        if !tab.location().shows_files() || tab.location().is_bin() {
            return;
        }
        if current(&tab.view).is_none() {
            return;
        }
        // One player at a time: the pane's stops.
        self.preview.stop_playback();
        self.quick_look = Some(QuickLook::new(tab.id));
    }

    pub fn close_quick_look(&mut self) {
        if let Some(mut ql) = self.quick_look.take() {
            ql.preview.release(&self.ctx);
        }
    }

    /// While Quick Look is open: the arrow keys go to the previous or next
    /// item (moving the selection along), Enter opens it, Esc closes.
    pub(super) fn quick_look_keys(&mut self, ctx: &Context) {
        let (step, close, open) = ctx.input_mut(|i| {
            let mut step = 0isize;
            for (key, d) in [
                (Key::ArrowLeft, -1),
                (Key::ArrowUp, -1),
                (Key::ArrowRight, 1),
                (Key::ArrowDown, 1),
            ] {
                while i.consume_key(Modifiers::NONE, key) {
                    step += d;
                }
            }
            (
                step,
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if close {
            self.close_quick_look();
            return;
        }
        let view = &mut self.tabs.active_mut().view;
        let Some(cur) = current(view) else {
            return;
        };
        if open {
            if let Some(e) = view.visible_entry(cur) {
                self.actions.push(Action::Open(e.path.clone()));
            }
            self.close_quick_look();
            return;
        }
        if step != 0 {
            let last = view.visible.len().saturating_sub(1) as isize;
            let to = (cur as isize + step).clamp(0, last) as usize;
            view.select_only(to);
            view.scroll_to = Some(to);
        }
    }
}
