//! Cloud files: the status badge on file icons, the status cell for a
//! details column, and the *Always keep on this device* / *Free up space*
//! menu entries.

use egui::{vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Response, Stroke, Ui};
use ox_core::cloud::{CloudStatus, Pin};
use ox_core::i18n::t as tr;
use std::path::PathBuf;

use crate::app::Action;

const BLUE: Color32 = Color32::from_rgb(0x2f, 0x6f, 0xe0);
const GREEN: Color32 = Color32::from_rgb(0x21, 0x9a, 0x5b);
const RED: Color32 = Color32::from_rgb(0xd9, 0x36, 0x3e);

/// Symbol and colour of a status (cloud = online only, check = on this
/// device, check on green = always keep, arrows = syncing, cross = problem).
pub fn symbol(status: CloudStatus) -> (&'static str, Color32) {
    match status {
        CloudStatus::OnlineOnly => ("☁", BLUE),
        CloudStatus::Local => ("✔", GREEN),
        CloudStatus::AlwaysKeep => ("✔", Color32::WHITE),
        CloudStatus::Syncing => ("🔄", BLUE),
        CloudStatus::Error => ("✖", Color32::WHITE),
    }
}

/// Tooltip text: the status and what it means.
pub fn tooltip(status: CloudStatus) -> String {
    format!("{}\n{}", status.label(), status.description())
}

/// Where the badge of an icon in `icon` goes (its bottom-right corner).
pub fn badge_rect(icon: Rect) -> Rect {
    let r = (icon.width() * 0.22).clamp(5.0, 11.0);
    Rect::from_center_size(
        icon.right_bottom() - vec2(r * 0.6, r * 0.6),
        vec2(r, r) * 2.0,
    )
}

/// Paint the status badge over a file icon painted into `icon`.
pub fn paint_badge(p: &Painter, icon: Rect, status: CloudStatus, faded: bool) {
    let rect = badge_rect(icon);
    let (c, r) = (rect.center(), rect.width() / 2.0);
    let fade = |c: Color32| if faded { c.gamma_multiply(0.5) } else { c };
    let fill = match status {
        CloudStatus::AlwaysKeep => GREEN,
        CloudStatus::Error => RED,
        _ => Color32::WHITE,
    };
    p.circle(
        c,
        r,
        fade(fill),
        Stroke::new(1.0, fade(Color32::from_black_alpha(60))),
    );
    paint_symbol(p, c, r * 1.5, status, faded);
}

fn paint_symbol(p: &Painter, center: Pos2, size: f32, status: CloudStatus, faded: bool) {
    let (glyph, color) = symbol(status);
    let color = if faded {
        color.gamma_multiply(0.5)
    } else {
        color
    };
    p.text(
        center,
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(size),
        color,
    );
}

/// Show the badge's tooltip when the pointer is over it.
pub fn badge_tooltip(resp: &Response, icon: Rect, status: CloudStatus) {
    if resp
        .hover_pos()
        .is_some_and(|p| badge_rect(icon).expand(2.0).contains(p))
    {
        resp.clone().on_hover_text_at_pointer(tooltip(status));
    }
}

/// Content of a Status column cell: the symbol (on a green or red disk for
/// always keep and problems), with the tooltip when hovered. For the
/// configurable details columns; sort with `ox_core::cloud::sort_rank`.
#[allow(dead_code)]
pub fn status_cell(ui: &Ui, cell: Rect, status: Option<CloudStatus>, resp: &Response) {
    let Some(status) = status else { return };
    let c = egui::pos2(cell.left() + 14.0, cell.center().y);
    let r = 7.0;
    match status {
        CloudStatus::AlwaysKeep => {
            ui.painter().circle_filled(c, r, GREEN);
        }
        CloudStatus::Error => {
            ui.painter().circle_filled(c, r, RED);
        }
        _ => {}
    }
    paint_symbol(ui.painter(), c, 11.0, status, false);
    if resp
        .hover_pos()
        .is_some_and(|p| Rect::from_center_size(c, vec2(18.0, 18.0)).contains(p))
    {
        resp.clone().on_hover_text_at_pointer(tooltip(status));
    }
}

/// The cloud entries of an item menu. `all_kept`: every selected item is
/// already always kept on this device.
pub fn menu(ui: &mut Ui, sel: &[PathBuf], all_kept: bool, actions: &mut Vec<Action>) {
    let keep = egui::Button::selectable(all_kept, tr("📌 Always keep on this device"));
    if ui.add(keep).clicked() {
        let pin = if all_kept { Pin::Default } else { Pin::Always };
        actions.push(Action::KeepOnDevice(sel.to_vec(), pin));
    }
    if ui
        .button(tr("☁ Free up space"))
        .on_hover_text(tr(
            "Keep it online only; it is downloaded again when you open it",
        ))
        .clicked()
    {
        actions.push(Action::KeepOnDevice(sel.to_vec(), Pin::Online));
    }
}
