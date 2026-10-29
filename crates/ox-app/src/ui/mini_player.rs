//! The mini player: a bar at the bottom of the window while something plays
//! that the preview doesn't show (playing on while browsing, playlists),
//! and the player's entries of the context menus.

use super::*;
use crate::app::PlayerAction;
use std::time::Duration;

/// The bar, when it is needed (call before the side and central panels).
pub fn show(app: &mut OxApp, ui: &mut Ui) {
    if !app.player.mini_shown() {
        return;
    }
    egui::Panel::bottom("mini_player")
        .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(10, 4)))
        .show(ui, |ui| bar(app, ui));
}

fn bar(app: &mut OxApp, ui: &mut Ui) {
    let mut actions = Vec::new();
    let title = app.player.title();
    let queue = &app.player.queue;
    let place = match (queue.len(), queue.current_index()) {
        (n, Some(i)) if n > 1 => format!("{} / {n}", i + 1),
        _ => String::new(),
    };
    let several = queue.len() > 1;
    let Some(pb) = app.player.playback.as_mut() else {
        return;
    };
    let st = pb.status.clone();
    let failed = pb.player.as_ref().err().cloned().or(st.error.clone());
    let mut seek = None;
    ui.horizontal(|ui| {
        const HIGH: f32 = 34.0;
        // A small frame of the video, else a note; click to go to the file.
        let picture = match (&pb.texture, st.video) {
            (Some(tex), true) => {
                let size = tex.size_vec2();
                ui.add(
                    egui::Image::new((tex.id(), size * (HIGH / size.y)))
                        .corner_radius(3.0)
                        .sense(egui::Sense::click()),
                )
            }
            _ => ui.add(
                egui::Label::new(RichText::new("🎵").size(20.0).weak()).sense(egui::Sense::click()),
            ),
        };
        let small = |text: &str| {
            egui::Button::new(RichText::new(text).size(14.0)).min_size(egui::vec2(28.0, 24.0))
        };
        if ui
            .add(small("⏮"))
            .on_hover_text(shortcuts::hint(
                tr("Previous file"),
                Command::PlayerPrevious,
            ))
            .clicked()
        {
            actions.push(Action::Player(PlayerAction::Previous));
        }
        let (icon, hint) = match st.playing {
            true => ("⏸", tr("Pause")),
            false => ("▶", tr("Play")),
        };
        if ui
            .add(small(icon))
            .on_hover_text(shortcuts::hint(hint, Command::PlayerPlayPause))
            .clicked()
        {
            actions.push(Action::Player(PlayerAction::PlayPause));
        }
        if ui
            .add_enabled(several, small("⏭"))
            .on_hover_text(shortcuts::hint(tr("Next file"), Command::PlayerNext))
            .clicked()
        {
            actions.push(Action::Player(PlayerAction::Next));
        }
        ui.add_space(6.0);
        let width = (ui.available_width() * 0.35).clamp(80.0, 280.0);
        let name = ui
            .allocate_ui(egui::vec2(width, HIGH), |ui| {
                ui.vertical(|ui| {
                    let r = ui.add(
                        egui::Label::new(RichText::new(&title).strong())
                            .truncate()
                            .sense(egui::Sense::click()),
                    );
                    match &failed {
                        Some(err) => {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(err.as_str())
                                        .small()
                                        .color(Color32::from_rgb(0xe5, 0x48, 0x4d)),
                                )
                                .truncate(),
                            );
                        }
                        None if !place.is_empty() => {
                            ui.label(RichText::new(&place).small().weak());
                        }
                        None => {}
                    }
                    r
                })
                .inner
            })
            .inner;
        if picture
            .union(name)
            .on_hover_text(tr("Show the playing file"))
            .clicked()
        {
            actions.push(Action::Player(PlayerAction::ShowCurrent));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if close_button(ui)
                .on_hover_text(tr("Stop the player"))
                .clicked()
            {
                actions.push(Action::Player(PlayerAction::Close));
            }
            ui.add_space(6.0);
            let total = st.duration.map_or(0.0, |d| d.as_secs_f32());
            let mut pos = pb.dragging.unwrap_or(st.position.as_secs_f32()).min(total);
            ui.label(
                RichText::new(format!("{} / {}", format::clock(pos), format::clock(total)))
                    .monospace()
                    .size(11.0),
            );
            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(40.0);
            let resp = ui.add_enabled(
                total > 0.0,
                egui::Slider::new(&mut pos, 0.0..=total.max(0.01)).show_value(false),
            );
            if resp.dragged() {
                pb.dragging = Some(pos);
            } else if resp.drag_stopped() || resp.changed() {
                seek = Some(Duration::from_secs_f32(pos));
                pb.dragging = None;
            }
        });
    });
    if let Some(to) = seek {
        app.player.with_player(|p| p.seek(to));
    }
    app.actions.extend(actions);
}

/// "▶ Play all" in an item's menu: for folders, playlists and several
/// audio or video files.
pub fn item_menu(ui: &mut Ui, selection: &[PathBuf], actions: &mut Vec<Action>) {
    use ox_core::player::{playlist, queue};
    if !ox_core::player::supported() {
        return;
    }
    let label = match selection {
        [one] if playlist::is_playlist(one) => tr("▶ Play playlist"),
        _ if selection
            .iter()
            .take(64)
            .any(|p| playlist::is_playlist(p) || p.is_dir()) =>
        {
            tr("▶ Play all")
        }
        _ if selection.iter().filter(|p| queue::is_playable(p)).count() > 1 => tr("▶ Play all"),
        _ => return,
    };
    if ui.button(label).clicked() {
        actions.push(Action::Player(PlayerAction::PlayAll(selection.to_vec())));
    }
}

/// "▶ Play all" in a folder's background menu.
pub fn folder_menu(ui: &mut Ui, folder: &Path, actions: &mut Vec<Action>) {
    if ox_core::player::supported()
        && ui
            .button(tr("▶ Play all"))
            .on_hover_text(tr("Play the audio and video files of this folder"))
            .clicked()
    {
        actions.push(Action::Player(PlayerAction::PlayFolder(
            folder.to_path_buf(),
        )));
    }
}
