//! Left sidebar: Home, Favorites, Libraries, Drives and Recent folders.

mod libraries;
mod phones;
mod remote;
mod wsl;

use ox_core::i18n::t as tr;
use std::path::Path;

use egui::{vec2, Align2, Color32, CornerRadius, FontId, RichText, Sense, Stroke, Ui};
use ox_core::entry::display_name;
use ox_core::format;
use ox_core::tags::TagColor;

use crate::app::{Action, OxApp};
use crate::icons::{self, Icon};
use crate::tabs::Location;

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    let current = app.tabs.active().location().clone();
    let accent = crate::theme::accent(&app.settings);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if app.settings.sidebar.show_favorites {
                section(ui, tr("Favorites"));
                if app.quick.favorites.is_empty() {
                    ui.label(
                        RichText::new(tr("  Right-click a folder → Pin to Favorites"))
                            .weak()
                            .small(),
                    );
                }
                for fav in app.quick.favorites.clone() {
                    let selected = current.folder() == Some(fav.path.as_path());
                    // Not asked inside WSL: that would start it.
                    let missing = !ox_core::wsl::is_wsl_path(&fav.path) && !fav.path.exists();
                    let icon = if fav.path.parent().is_none() {
                        Icon::Drive
                    } else {
                        Icon::Folder
                    };
                    let resp = item(ui, icon, &fav.label(), None, selected, accent);
                    let resp = resp.on_hover_text(fav.path.display().to_string());
                    if missing {
                        ui.painter().text(
                            resp.rect.right_center() - vec2(8.0, 0.0),
                            Align2::RIGHT_CENTER,
                            "⚠",
                            FontId::proportional(12.0),
                            Color32::from_rgb(0xe5, 0x48, 0x4d),
                        );
                    }
                    super::open_link(&mut actions, &resp, &fav.path);
                    highlight_drop(
                        ui,
                        &resp,
                        super::drop_target(&mut actions, &resp, &fav.path),
                        accent,
                    );
                    resp.context_menu(|ui| {
                        super::folder_link_menu(ui, &fav.path, true, &mut actions)
                    });
                }
            }

            libraries::show(app, ui, &current, accent, &mut actions);

            if app.settings.sidebar.show_this_pc {
                section(ui, tr("This PC"));
                for drive in app.drives().to_vec() {
                    let selected = current.folder() == Some(drive.mount.as_path());
                    let icon = if drive.remote.is_some() {
                        Icon::Server
                    } else {
                        Icon::Drive
                    };
                    let resp = item(
                        ui,
                        icon,
                        &drive.title(),
                        (drive.total > 0).then(|| drive.used_fraction()),
                        selected,
                        accent,
                    );
                    let resp = match &drive.remote {
                        Some(remote) => resp.on_hover_text(ox_core::trf!(
                            "{v1}\nNetwork drive: {remote}",
                            v1 = drive.mount.display(),
                            remote = remote
                        )),
                        None => resp.on_hover_text(ox_core::trf!(
                            "{v1}\n{v2} free of {v3}",
                            v1 = drive.mount.display(),
                            v2 = format::size(drive.available),
                            v3 = format::size(drive.total)
                        )),
                    };
                    super::open_link(&mut actions, &resp, &drive.mount);
                    highlight_drop(
                        ui,
                        &resp,
                        super::drop_target(&mut actions, &resp, &drive.mount),
                        accent,
                    );
                    let pinned = app.quick.is_favorite(&drive.mount);
                    resp.context_menu(|ui| {
                        super::folder_link_menu(ui, &drive.mount, pinned, &mut actions);
                        network_drive_menu(ui, &mut actions);
                    });
                }
                wsl::show(app, ui, &current, accent, &mut actions);
                phones::show(app, ui, &current, accent, &mut actions);
            }

            remote::show(app, ui, &current, accent, &mut actions);

            let servers = network_servers(app);
            if app.settings.sidebar.show_network && !servers.is_empty() {
                section(ui, tr("Network"));
                for server in servers {
                    let selected = current.folder() == Some(server.as_path());
                    let name = ox_core::network::server_name(&server).unwrap_or_default();
                    let resp = item(ui, Icon::Server, name, None, selected, accent)
                        .on_hover_text(server.display().to_string());
                    super::open_link(&mut actions, &resp, &server);
                    let pinned = app.quick.is_favorite(&server);
                    resp.context_menu(|ui| {
                        super::folder_link_menu(ui, &server, pinned, &mut actions);
                        network_drive_menu(ui, &mut actions);
                    });
                }
            }

            if app.settings.sidebar.show_recent && !app.quick.recent_folders.is_empty() {
                section(ui, tr("Recent"));
                let recent: Vec<_> = app
                    .quick
                    .recent_folders(app.settings.sidebar.recent_count)
                    .map(|r| r.path.clone())
                    .collect();
                for path in recent {
                    let selected = current.folder() == Some(path.as_path());
                    let resp = item(ui, Icon::Folder, &label_for(&path), None, selected, accent)
                        .on_hover_text(path.display().to_string());
                    super::open_link(&mut actions, &resp, &path);
                    let pinned = app.quick.is_favorite(&path);
                    resp.context_menu(|ui| {
                        super::folder_link_menu(ui, &path, pinned, &mut actions);
                        ui.separator();
                        if ui.button(tr("🗑 Remove from Recent")).clicked() {
                            actions.push(Action::ForgetRecent(path.clone()));
                        }
                    });
                }
            }

            let counts = app.tags.counts();
            if counts.iter().any(|&n| n > 0) {
                section(ui, tr("Tags"));
                for color in TagColor::ALL.into_iter().filter(|c| counts[c.index()] > 0) {
                    let loc = Location::Tagged(color);
                    let resp = item(
                        ui,
                        Icon::Dot(super::tag_color(color)),
                        app.tags.label(color),
                        None,
                        current == loc,
                        accent,
                    );
                    ui.painter().text(
                        resp.rect.right_center() - vec2(10.0, 0.0),
                        Align2::RIGHT_CENTER,
                        counts[color.index()].to_string(),
                        FontId::proportional(11.5),
                        ui.visuals().weak_text_color(),
                    );
                    if resp.clicked() {
                        actions.push(Action::Navigate(loc.clone()));
                    }
                    if resp.middle_clicked() {
                        actions.push(Action::OpenInNewTab(loc.clone(), false));
                    }
                    let custom = app.tags.custom_label(color).to_owned();
                    let tagged = app.tags.with_tag(color);
                    // Stays open while the name is being typed.
                    egui::Popup::context_menu(&resp)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            if ui.button(tr("📑 Open in new tab")).clicked() {
                                actions.push(Action::OpenInNewTab(loc.clone(), true));
                                ui.close();
                            }
                            ui.separator();
                            ui.label(RichText::new(tr("Name")).weak().small());
                            let mut name = custom.clone();
                            let edit = egui::TextEdit::singleline(&mut name)
                                .hint_text(color.name())
                                .desired_width(160.0);
                            if ui.add(edit).changed() {
                                actions.push(Action::RenameTag(color, name));
                            }
                            ui.separator();
                            if ui
                                .button(ox_core::trf!(
                                    "✖ Remove from {v1}",
                                    v1 = ox_core::i18n::count(tagged.len() as u64, "item")
                                ))
                                .clicked()
                            {
                                actions.push(Action::ToggleTag(tagged.clone(), color));
                                ui.close();
                            }
                        });
                }
            }

            if !app.quick.saved_searches.is_empty() {
                section(ui, tr("Saved searches"));
                for (i, saved) in app.quick.saved_searches.clone().into_iter().enumerate() {
                    let loc = Location::Search {
                        root: saved.root.clone(),
                        query: saved.query.clone(),
                    };
                    let resp = item(
                        ui,
                        Icon::Glyph("🔍"),
                        &saved.name,
                        None,
                        current == loc,
                        accent,
                    )
                    .on_hover_text(ox_core::trf!(
                        "{v1}\nin {v2}",
                        v1 = saved.query,
                        v2 = saved
                            .root
                            .as_deref()
                            .map(|r| r.display().to_string())
                            .unwrap_or_else(|| tr("This PC").into())
                    ));
                    if resp.clicked() {
                        actions.push(Action::Navigate(loc.clone()));
                    }
                    if resp.middle_clicked() {
                        actions.push(Action::OpenInNewTab(loc.clone(), false));
                    }
                    resp.context_menu(|ui| {
                        if ui.button(tr("📑 Open in new tab")).clicked() {
                            actions.push(Action::OpenInNewTab(loc.clone(), true));
                        }
                        if ui.button(tr("🗑 Remove")).clicked() {
                            actions.push(Action::RemoveSavedSearch(i));
                        }
                    });
                }
            }
        });
    app.actions.extend(actions);
}

fn label_for(path: &Path) -> String {
    let name = display_name(path);
    if name.is_empty() {
        path.display().to_string()
    } else {
        name
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title.to_uppercase()).small().strong().weak());
    ui.add_space(2.0);
}

/// "Map network drive…" / "Disconnect network drive…" (Windows).
fn network_drive_menu(ui: &mut Ui, actions: &mut Vec<Action>) {
    if !ox_core::shell::supported() {
        return;
    }
    ui.separator();
    if ui.button(tr("🌐 Map network drive…")).clicked() {
        actions.push(Action::NetworkDriveDialog(true));
    }
    if ui.button(tr("🌐 Disconnect network drive…")).clicked() {
        actions.push(Action::NetworkDriveDialog(false));
    }
}

/// Servers in use: those behind mapped drives, favorites and recent folders.
fn network_servers(app: &OxApp) -> Vec<std::path::PathBuf> {
    let mapped = app.drives.iter().filter_map(|d| d.remote.as_deref());
    let folders = app
        .quick
        .favorites
        .iter()
        .map(|f| f.path.as_path())
        .chain(app.quick.recent_folders.iter().map(|r| r.path.as_path()));
    let mut servers: Vec<std::path::PathBuf> = mapped
        .filter_map(|r| ox_core::network::server_of(Path::new(r)))
        .chain(folders.filter_map(|p| {
            ox_core::network::server_of(p)
                .or_else(|| ox_core::network::server_name(p).map(|s| format!(r"\\{s}").into()))
        }))
        .collect();
    servers.sort_by_key(|s| s.to_string_lossy().to_lowercase());
    servers.dedup_by_key(|s| s.to_string_lossy().to_lowercase());
    servers
}

/// A full-width sidebar row with optional usage bar (for drives).
fn item(
    ui: &mut Ui,
    icon: Icon<'_>,
    label: &str,
    usage: Option<f32>,
    selected: bool,
    accent: Color32,
) -> egui::Response {
    let height = if usage.is_some() { 38.0 } else { 28.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let v = ui.visuals();
    let fill = if selected {
        accent.gamma_multiply(if v.dark_mode { 0.3 } else { 0.18 })
    } else if resp.hovered() {
        v.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    if selected {
        let bar = egui::Rect::from_min_size(
            rect.left_top() + vec2(0.0, 6.0),
            vec2(3.0, rect.height() - 12.0),
        );
        ui.painter().rect_filled(bar, CornerRadius::same(2), accent);
    }
    let top = if usage.is_some() {
        rect.top() + 14.0
    } else {
        rect.center().y
    };
    let painter = ui.painter().with_clip_rect(rect.shrink(2.0));
    icons::paint_icon(
        &painter,
        icons::icon_rect(rect.left() + 12.0, top, 16.0),
        icon,
        v.text_color(),
        accent,
    );
    painter.text(
        egui::pos2(rect.left() + 34.0, top),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        v.text_color(),
    );
    if let Some(frac) = usage {
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 34.0, rect.bottom() - 10.0),
            vec2(rect.width() - 46.0, 4.0),
        );
        ui.painter()
            .rect_filled(bar, CornerRadius::same(2), v.widgets.inactive.bg_fill);
        let color = if frac > 0.9 {
            Color32::from_rgb(0xe5, 0x48, 0x4d)
        } else {
            accent
        };
        let mut used = bar;
        used.set_width(bar.width() * frac.clamp(0.0, 1.0));
        ui.painter().rect_filled(used, CornerRadius::same(2), color);
    }
    resp
}

fn highlight_drop(ui: &Ui, resp: &egui::Response, hovering: bool, accent: Color32) {
    if hovering {
        ui.painter().rect_stroke(
            resp.rect,
            CornerRadius::same(6),
            Stroke::new(1.5, accent),
            egui::StrokeKind::Inside,
        );
    }
}
