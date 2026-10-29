//! The suggestions under the search box: earlier searches and completions of
//! the query syntax (from `ox_core::search_history`). Up/Down choose one,
//! Enter or a click takes it, Tab completes, Escape closes the list. While
//! it's closed the box handles its keys as before.

use egui::{vec2, Align2, EventFilter, FontId, Id, Key, Rect, RectAlign, Response, Sense, Ui};
use ox_core::i18n::t as tr;
use ox_core::search_history::{Suggestion, SuggestionKind};

/// Suggestions shown at most.
pub(super) const MAX: usize = 12;
const ROW_H: f32 = 24.0;

/// Kept between frames.
#[derive(Clone, Default)]
struct State {
    open: bool,
    /// Chosen with the arrow keys or the mouse. Enter only takes a chosen
    /// one, so that typing and pressing Enter still searches for the text.
    selected: Option<usize>,
    /// The text the selection was made for; a new text starts over.
    text: String,
    /// The text the list was closed for with Escape (until it changes).
    dismissed: Option<String>,
    /// Where the list was, to keep the box focused while clicking it.
    rect: Option<Rect>,
}

/// What the user did in the list.
pub(super) enum Picked {
    /// Put this suggestion into the box; `run` = search for it right away.
    Take(Suggestion, bool),
    /// Forget this earlier search.
    Remove(String),
    ClearHistory,
    /// Enter was taken for the list, but there was nothing to take: search.
    Submit,
}

/// Keys pressed for the list, taken before the text field sees them.
#[derive(Default)]
struct Keys {
    down: usize,
    up: usize,
    enter: bool,
    tab: bool,
    escape: bool,
}

pub(super) struct Dropdown {
    field: Id,
    id: Id,
    state: State,
    keys: Keys,
}

/// Take presses of `key` without modifiers; how many there were.
fn take_key(i: &mut egui::InputState, key: Key) -> usize {
    let before = i.events.len();
    i.events.retain(|e| {
        !matches!(e, egui::Event::Key { key: k, pressed: true, modifiers, .. }
            if *k == key && modifiers.is_none())
    });
    before - i.events.len()
}

fn filter(open: bool) -> EventFilter {
    EventFilter {
        tab: open,
        escape: open,
        horizontal_arrows: true,
        vertical_arrows: true,
    }
}

impl Dropdown {
    /// Before the text field: takes the list's keys while it is open.
    pub(super) fn before(ui: &mut Ui, field: Id) -> Self {
        let id = field.with("suggestions");
        let state: State = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        let mut keys = Keys::default();
        if state.open {
            ui.input_mut(|i| {
                keys.down = take_key(i, Key::ArrowDown);
                keys.up = take_key(i, Key::ArrowUp);
                keys.tab = take_key(i, Key::Tab) > 0;
                keys.escape = take_key(i, Key::Escape) > 0;
                if state.selected.is_some() {
                    keys.enter = take_key(i, Key::Enter) > 0;
                }
            });
        }
        Self {
            field,
            id,
            state,
            keys,
        }
    }

    /// For the text field: while the list is open, Tab and Escape act on it
    /// instead of moving or dropping the focus.
    pub(super) fn event_filter(&self) -> EventFilter {
        filter(self.state.open)
    }

    /// Show the text field with `add`, keeping its focus while the pointer
    /// is on the list (a click there would take it away otherwise).
    pub(super) fn add_field(&self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> Response) -> Response {
        let on_list = self.state.open
            && self.state.rect.is_some_and(|r| {
                ui.ctx()
                    .pointer_interact_pos()
                    .is_some_and(|p| r.contains(p))
            });
        let before = ui.ctx().options(|o| o.input_options.surrender_focus_on);
        if on_list {
            ui.ctx().options_mut(|o| {
                o.input_options.surrender_focus_on = egui::SurrenderFocusOn::Never;
            });
        }
        let resp = add(ui);
        ui.ctx()
            .options_mut(|o| o.input_options.surrender_focus_on = before);
        resp
    }

    /// After the text field: show `suggestions` for `text` under `anchor`
    /// while `edit` has focus.
    pub(super) fn show(
        mut self,
        ui: &mut Ui,
        anchor: &Response,
        edit: &Response,
        text: &str,
        suggestions: Vec<Suggestion>,
    ) -> Option<Picked> {
        let st = &mut self.state;
        if !edit.has_focus() {
            // Closed with Escape: open again the next time the box is used.
            st.dismissed = None;
        }
        if st.text != text {
            st.text = text.to_owned();
            st.selected = None;
            if st.dismissed.as_deref() != Some(text) {
                st.dismissed = None;
            }
        }
        if self.keys.escape {
            st.dismissed = Some(text.to_owned());
        }
        let n = suggestions.len();
        if n > 0 {
            let mut sel = st.selected;
            for _ in 0..self.keys.down {
                sel = Some(sel.map_or(0, |s| (s + 1) % n));
            }
            for _ in 0..self.keys.up {
                sel = Some(sel.map_or(n - 1, |s| (s + n - 1) % n));
            }
            st.selected = sel.filter(|&s| s < n);
        } else {
            st.selected = None;
        }

        let mut picked = None;
        if self.keys.enter {
            picked = Some(match st.selected {
                Some(i) => {
                    let run = suggestions[i].kind == SuggestionKind::History;
                    Picked::Take(suggestions[i].clone(), run)
                }
                None => Picked::Submit,
            });
        } else if self.keys.tab {
            if let Some(s) = suggestions.get(st.selected.unwrap_or(0)) {
                picked = Some(Picked::Take(s.clone(), false));
            }
        }

        let open = edit.has_focus() && n > 0 && st.dismissed.is_none() && picked.is_none();
        let width = anchor.rect.width().max(360.0);
        let shown = egui::Popup::from_response(anchor)
            .id(self.id.with("popup"))
            .kind(egui::PopupKind::Popup)
            .open(open)
            .align(RectAlign::BOTTOM_END)
            .align_alternatives(&[RectAlign::BOTTOM_START])
            .gap(4.0)
            .width(width)
            .show(|ui| list(ui, &suggestions, &mut st.selected));
        st.rect = shown.as_ref().map(|r| r.response.rect);
        if let Some(p) = shown.and_then(|r| r.inner) {
            picked = Some(p);
        }
        // Taking one closes the list; removing earlier searches doesn't.
        let taken = matches!(picked, Some(Picked::Take(..) | Picked::Submit));
        st.open = open && !taken;
        if taken {
            st.selected = None;
        }
        // Right away for the next frame (the field sets it only when it's shown again).
        let field = self.field;
        if st.open {
            ui.memory_mut(|m| m.set_focus_lock_filter(field, filter(true)));
        }
        ui.data_mut(|d| d.insert_temp(self.id, self.state));
        picked
    }
}

fn list(ui: &mut Ui, suggestions: &[Suggestion], selected: &mut Option<usize>) -> Option<Picked> {
    let mut picked = None;
    let moved = ui.input(|i| i.pointer.is_moving());
    let inner_w = ui.available_width();
    ui.spacing_mut().item_spacing.y = 0.0;
    for (i, s) in suggestions.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(inner_w, ROW_H), Sense::click());
        let hovered = ui.rect_contains_pointer(rect);
        if hovered && moved {
            *selected = Some(i);
        }
        let v = ui.visuals();
        if *selected == Some(i) {
            ui.painter()
                .rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
        }
        let history = s.kind == SuggestionKind::History;
        let icon = if history { "🕘" } else { "🔣" };
        let weak = v.weak_text_color();
        let strong = v.text_color();
        let font = FontId::proportional(13.0);
        // The remove button takes the right end of earlier searches.
        let right = if history {
            rect.right() - 26.0
        } else {
            rect.right() - 6.0
        };
        let painter = ui.painter_at(Rect::from_x_y_ranges(rect.left()..=right, rect.y_range()));
        painter.text(
            rect.left_center() + vec2(6.0, 0.0),
            Align2::LEFT_CENTER,
            icon,
            font.clone(),
            weak,
        );
        let label = painter.text(
            rect.left_center() + vec2(26.0, 0.0),
            Align2::LEFT_CENTER,
            &s.label,
            font,
            strong,
        );
        if !s.detail.is_empty() && label.right() + 16.0 < right {
            painter.text(
                egui::pos2(right, rect.center().y),
                Align2::RIGHT_CENTER,
                s.detail,
                FontId::proportional(11.5),
                weak,
            );
        }
        let mut removed = false;
        if history && (hovered || *selected == Some(i)) {
            let x = Rect::from_center_size(
                egui::pos2(rect.right() - 13.0, rect.center().y),
                vec2(20.0, 20.0),
            );
            removed = ui
                .put(
                    x,
                    egui::Button::new(egui::RichText::new("✖").size(11.0)).frame(false),
                )
                .on_hover_text(tr("Remove from history"))
                .clicked();
        }
        if removed {
            picked = Some(Picked::Remove(s.text.clone()));
        } else if resp.clicked() {
            picked = Some(Picked::Take(s.clone(), history));
        }
    }
    if suggestions
        .iter()
        .any(|s| s.kind == SuggestionKind::History)
    {
        ui.add_space(4.0);
        ui.separator();
        let clear = ui.add(
            egui::Button::new(egui::RichText::new(tr("Clear search history")).small()).frame(false),
        );
        if clear.clicked() {
            picked = Some(Picked::ClearHistory);
        }
    }
    picked
}

/// Put the text cursor of the field `id` at the end of `text` (after a
/// suggestion replaced it).
pub(super) fn cursor_to_end(ui: &Ui, id: Id, text: &str) {
    use egui::text::{CCursor, CCursorRange};
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
    let end = CCursor::new(text.chars().count());
    state.cursor.set_char_range(Some(CCursorRange::one(end)));
    state.store(ui.ctx(), id);
}
