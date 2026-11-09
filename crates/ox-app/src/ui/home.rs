//! Home / Quick Access start page: favorites, drives and recent items.

mod phones;
mod wsl;

use ox_core::i18n::t as tr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use egui::{vec2, Align2, Color32, CornerRadius, FontId, RichText, Sense, Stroke, Ui};
use ox_core::entry::{display_name, FileKind};
use ox_core::format;

use crate::app::{Action, OxApp};
use crate::icons::{self, Icon};
use crate::tabs::Location;

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    let accent = crate::theme::accent(&app.settings);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(28, 20)).show(ui, |ui| {
            ui.heading(RichText::new(tr("Quick access")).size(24.0).strong());
            ui.label(RichText::new(tr("Your pinned folders, drives and recent places.")).weak());
            if app.settings.startup.check_updates {
                update_note(ui, accent);
            }
            ui.add_space(14.0);

            // Favorites
            heading(ui, tr("⭐ Favorites"));
            if app.quick.favorites.is_empty() {
                ui.label(
                    RichText::new(tr(
                        "Nothing pinned yet. Right-click any folder and choose \"Pin to Favorites\".",
                    ))
                    .weak(),
                );
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                for fav in app.quick.favorites.clone() {
                    let resp = card(ui, Icon::Folder, &fav.label(), &fav.path.display().to_string(), None, accent);
                    super::open_link(&mut actions, &resp, &fav.path);
                    let hovering = super::drop_target(&mut actions, &resp, &fav.path);
                    if hovering {
                        ui.painter().rect_stroke(resp.rect, CornerRadius::same(10), Stroke::new(2.0, accent), egui::StrokeKind::Inside);
                    }
                    resp.context_menu(|ui| super::folder_link_menu(ui, &fav.path, true, &mut actions));
                }
            });
            ui.add_space(22.0);

            // Drives
            heading(ui, tr("🖴 Drives"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                for d in app.drives().to_vec() {
                    // Network drives: no space shown (an offline server can take long to answer).
                    let resp = match &d.remote {
                        Some(remote) => card(ui, Icon::Server, &d.title(), remote, None, accent),
                        None => {
                            let sub = ox_core::trf!("{v1} free of {v2}", v1 = format::size(d.available), v2 = format::size(d.total));
                            card(ui, Icon::Drive, &d.title(), &sub, Some(d.used_fraction()), accent)
                        }
                    };
                    super::open_link(&mut actions, &resp, &d.mount);
                    super::drop_target(&mut actions, &resp, &d.mount);
                    let pinned = app.quick.is_favorite(&d.mount);
                    resp.context_menu(|ui| super::folder_link_menu(ui, &d.mount, pinned, &mut actions));
                }
                wsl::cards(app, ui, accent, &mut actions);
                phones::cards(app, ui, accent, &mut actions);
            });
            ui.add_space(22.0);

            // Recent
            ui.columns(2, |cols| {
                heading(&mut cols[0], tr("🕘 Recent folders"));
                let folders: Vec<_> = app.quick.recent_folders(12).cloned().collect();
                if folders.is_empty() {
                    cols[0].label(RichText::new(tr("Folders you open will show up here.")).weak());
                }
                for r in folders {
                    let resp = recent_row(&mut cols[0], Icon::Folder, &r.path, r.last_used);
                    super::open_link(&mut actions, &resp, &r.path);
                    let pinned = app.quick.is_favorite(&r.path);
                    resp.context_menu(|ui| {
                        super::folder_link_menu(ui, &r.path, pinned, &mut actions);
                        if ui.button(tr("🗑 Remove from Recent")).clicked() {
                            actions.push(Action::ForgetRecent(r.path.clone()));
                        }
                    });
                }

                heading(&mut cols[1], tr("📄 Recent files"));
                let files: Vec<_> = app.quick.recent_files.iter().take(12).cloned().collect();
                if files.is_empty() {
                    cols[1].label(RichText::new(tr("Files you open will show up here.")).weak());
                }
                for r in files {
                    let kind = FileKind::from_extension(
                        &r.path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default(),
                    );
                    let resp = recent_row(&mut cols[1], Icon::File(kind), &r.path, r.last_used);
                    if resp.clicked() {
                        actions.push(Action::Open(r.path.clone()));
                    }
                    resp.context_menu(|ui| {
                        if ui.button(tr("📂 Open")).clicked() {
                            actions.push(Action::Open(r.path.clone()));
                        }
                        if let Some(parent) = r.path.parent() {
                            if ui.button(tr("📁 Open file location")).clicked() {
                                actions.push(Action::Navigate(Location::Folder(parent.to_path_buf())));
                            }
                        }
                        if ui.button(tr("🔗 Copy path")).clicked() {
                            actions.push(Action::CopyPathText(vec![r.path.clone()]));
                        }
                        if ui.button(tr("🗑 Remove from Recent")).clicked() {
                            actions.push(Action::ForgetRecent(r.path.clone()));
                        }
                    });
                }
            });
            if !app.quick.recent_folders.is_empty() || !app.quick.recent_files.is_empty() {
                ui.add_space(10.0);
                if ui.button(tr("Clear recent history")).clicked() {
                    actions.push(Action::ClearRecent);
                }
            }
        });
    });
    app.actions.extend(actions);
}

/// A newer release is on GitHub: a quiet line with a link to its notes, and
/// ✖ to stop showing it for that version.
fn update_note(ui: &mut Ui, accent: Color32) {
    let Some(release) = ox_core::update::available() else {
        return;
    };
    ui.add_space(10.0);
    egui::Frame::new()
        .stroke(Stroke::new(1.0, accent.gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(ox_core::trf!(
                    "✨ Oxplorer {v1} is available.",
                    v1 = release.version()
                ));
                ui.hyperlink_to(tr("What's new"), release.notes_url());
                ui.add_space(6.0);
                if super::close_button(ui)
                    .on_hover_text(tr("Don't show this again for this version"))
                    .clicked()
                {
                    ox_core::update::dismiss();
                }
            });
        });
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(15.0).strong());
    ui.add_space(6.0);
}

fn card(
    ui: &mut Ui,
    icon: Icon<'_>,
    title: &str,
    sub: &str,
    usage: Option<f32>,
    accent: Color32,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(230.0, 68.0), Sense::click());
    let v = ui.visuals();
    let fill = if resp.hovered() {
        v.widgets.hovered.weak_bg_fill
    } else {
        v.faint_bg_color
    };
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    icons::paint_icon(
        ui.painter(),
        icons::icon_rect(rect.left() + 14.0, rect.center().y, 32.0),
        icon,
        v.text_color(),
        accent,
    );
    let painter = ui.painter().with_clip_rect(rect.shrink(4.0));
    let x = rect.left() + 58.0;
    painter.text(
        egui::pos2(x, rect.top() + 20.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(14.0),
        v.strong_text_color(),
    );
    painter.text(
        egui::pos2(x, rect.top() + 39.0),
        Align2::LEFT_CENTER,
        sub,
        FontId::proportional(11.5),
        v.weak_text_color(),
    );
    if let Some(frac) = usage {
        let bar = egui::Rect::from_min_size(
            egui::pos2(x, rect.bottom() - 15.0),
            vec2(rect.width() - 74.0, 5.0),
        );
        ui.painter()
            .rect_filled(bar, CornerRadius::same(3), v.widgets.inactive.bg_fill);
        let mut used = bar;
        used.set_width(bar.width() * frac.clamp(0.0, 1.0));
        let color = if frac > 0.9 {
            Color32::from_rgb(0xe5, 0x48, 0x4d)
        } else {
            accent
        };
        ui.painter().rect_filled(used, CornerRadius::same(3), color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn recent_row(ui: &mut Ui, icon: Icon<'_>, path: &Path, last_used: u64) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let v = ui.visuals();
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), v.widgets.hovered.weak_bg_fill);
    }
    let painter = ui.painter().with_clip_rect(rect);
    icons::paint_icon(
        &painter,
        icons::icon_rect(rect.left() + 8.0, rect.center().y, 16.0),
        icon,
        v.text_color(),
        v.hyperlink_color,
    );
    painter.text(
        rect.left_center() + vec2(32.0, 0.0),
        Align2::LEFT_CENTER,
        display_name(path),
        FontId::proportional(13.5),
        v.text_color(),
    );
    painter.text(
        rect.right_center() - vec2(8.0, 0.0),
        Align2::RIGHT_CENTER,
        ago(last_used),
        FontId::proportional(11.5),
        v.weak_text_color(),
    );
    resp.on_hover_text(parent_text(path))
}

fn parent_text(path: &Path) -> String {
    path.parent()
        .map(PathBuf::from)
        .unwrap_or_default()
        .display()
        .to_string()
}

fn ago(ts: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let d = now.saturating_sub(ts);
    match d {
        0..60 => tr("just now").into(),
        60..3600 => ox_core::trf!("{v1} min ago", v1 = d / 60),
        3600..86400 => ox_core::trf!("{v1} h ago", v1 = d / 3600),
        _ => ox_core::trf!("{v1} d ago", v1 = d / 86400),
    }
}
