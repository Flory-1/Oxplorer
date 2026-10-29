//! Header bar: tab strip with tab groups, navigation/address/search row and command bar.

mod caption;
mod command_bar;
mod nav;
mod tab_strip;
use caption::caption_buttons;
pub use caption::caption_buttons_id;
use command_bar::command_bar;
use nav::nav_row;
use ox_core::i18n::t as tr;
use std::path::{Path, PathBuf};
use tab_strip::tab_strip;

use egui::{
    vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke, Ui,
};
use ox_core::entry::{display_name, SortKey, SortOrder};
use ox_core::settings::ViewMode;

use super::{DragItems, DragTab};
use crate::app::{Action, OxApp, TabAction};
use crate::icons::Icon;
use crate::shortcuts::{self, Command};
use crate::tabs::{Location, TabGroup, GROUP_COLORS};

const TAB_HEIGHT: f32 = 32.0;
/// Caption buttons (minimize, maximize, close), as wide as Windows 11's.
const CAPTION_W: f32 = 46.0;
/// The fixed Home tab shows only its icon.
const HOME_TAB_W: f32 = 40.0;
/// Free space after the tabs to grab the window by.
const DRAG_GAP: f32 = 40.0;

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    tab_strip(app, ui);
    ui.add_space(4.0);
    nav_row(app, ui);
    ui.add_space(2.0);
    command_bar(app, ui);
}

pub(super) fn group_color(g: &TabGroup) -> Color32 {
    let [r, gg, b] = GROUP_COLORS[g.color % GROUP_COLORS.len()].1;
    Color32::from_rgb(r, gg, b)
}
