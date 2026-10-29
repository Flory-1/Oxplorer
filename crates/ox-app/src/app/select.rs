//! Selection helpers: select or deselect by pattern, invert the selection,
//! select all items of the same type.

use ox_core::select::Patterns;

use super::*;

pub enum SelectAction {
    /// Ask for a pattern to select (`true`) or deselect matching items.
    PatternDialog(bool),
    /// Select (`true`) or deselect the items matching the patterns.
    Pattern(String, bool),
    Invert,
    SameType,
}

/// The last pattern typed, offered again next time.
fn last_pattern_id() -> egui::Id {
    egui::Id::unique("select_pattern_last")
}

impl OxApp {
    pub(super) fn process_select(&mut self, action: SelectAction) {
        if !self.tabs.active().location().shows_files() {
            return;
        }
        match action {
            SelectAction::PatternDialog(select) => {
                let text = self
                    .ctx
                    .data(|d| d.get_temp::<String>(last_pattern_id()))
                    .unwrap_or_else(|| "*.".to_owned());
                self.dialog = Some(Dialog::SelectPattern {
                    text,
                    select,
                    focus: true,
                });
            }
            SelectAction::Pattern(text, select) => {
                let patterns = Patterns::parse(&text);
                let n = self
                    .tabs
                    .active_mut()
                    .view
                    .select_matching(&patterns, select);
                self.ctx
                    .data_mut(|d| d.insert_temp(last_pattern_id(), text.clone()));
                if n == 0 && !patterns.is_empty() {
                    self.toast(ox_core::trf!("No items match \"{v1}\".", v1 = text.trim()));
                }
            }
            SelectAction::Invert => self.tabs.active_mut().view.invert_selection(),
            SelectAction::SameType => {
                if !self.tabs.active_mut().view.select_same_type() {
                    self.toast(tr("Select an item first."));
                }
            }
        }
    }
}
