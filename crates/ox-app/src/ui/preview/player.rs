//! The Player tab: the built-in audio and video player. The pane plays with
//! the app's player (`app/player.rs`, swapped in while the tab is drawn), so
//! it plays on when another item is selected; Quick Look has its own.

use super::*;
use crate::app::{PlayerSession, RATES};
use crate::shortcuts::{self, Command};
use ox_core::player::queue::Repeat;

mod queue;
pub use queue::rate_label;
use queue::{queue_list, subtitle_overlay};

pub(super) type PlayerState = PlayerSession;

impl PreviewState {
    /// Its player is playing (Quick Look's).
    pub fn playing(&self) -> bool {
        self.play.is_playing()
    }

    /// Open on the Player tab (going back to the playing file).
    pub fn show_player(&mut self) {
        self.wanted = Some(PreviewTab::Player);
    }
}

/// The built-in player: the video (or the cover of audio files), a seek
/// bar and the buttons, and the queue. The file is opened when the tab is
/// shown. The picture is at most `max_h` high (Quick Look; unlimited in the
/// pane).
pub(super) fn player_tab(
    ui: &mut Ui,
    state: &mut PlayerState,
    e: &FileEntry,
    cover: Option<&TextureHandle>,
    max_h: f32,
) {
    let ctx = ui.ctx().clone();
    if state.current_path() != Some(e.path.as_path()) {
        let autoplay = std::mem::take(&mut state.autoplay);
        // Something else plays on (or a playlist waits): not replaced
        // unless asked to.
        let busy =
            state.main && state.playback.is_some() && (state.is_playing() || !state.from_selection);
        if busy && !autoplay {
            other_playing(ui, state, e, cover);
            return;
        }
        state.open_selected(&ctx, &e.path, autoplay);
    } else if std::mem::take(&mut state.autoplay) {
        state.with_player(|p| p.play());
    }
    if !state.main {
        state.tick(&ctx);
    }
    state.shown = true;

    let pb = state.playback.as_ref().expect("opened above");
    let st = pb.status.clone();
    if let Err(err) = &pb.player {
        ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), err.as_str());
    } else {
        picture(ui, state, cover, max_h);
    }
    if let Some(err) = &st.error {
        ui.add_space(4.0);
        ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), err.as_str());
    }
    ui.add_space(6.0);
    seek_bar(ui, state, &st);
    buttons(ui, state, &st);
    if state.main && state.queue.len() > 1 {
        ui.add_space(6.0);
        queue_list(ui, state);
    }
}

/// The video with its subtitles, the cover of audio files, or a note.
fn picture(ui: &mut Ui, state: &PlayerState, cover: Option<&TextureHandle>, max_h: f32) {
    let Some(pb) = &state.playback else {
        return;
    };
    let width = ui.available_width();
    // Fits the picture into the width and `max_h`, centred.
    let fit = |ui: &mut Ui, tex: &TextureHandle, most: f32| {
        let size = tex.size_vec2();
        let scale = (width / size.x).min(max_h / size.y).min(most);
        ui.vertical_centered(|ui| {
            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0))
                .rect
        })
        .inner
    };
    if pb.status.video {
        let rect = match &pb.texture {
            Some(tex) => fit(ui, tex, f32::INFINITY),
            None => {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(width, (width * 9.0 / 16.0).min(max_h)),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(rect, 6.0, egui::Color32::BLACK);
                rect
            }
        };
        if let Some(subs) = pb.subtitles.as_ref().filter(|_| state.subtitles_on) {
            subtitle_overlay(ui, rect, subs, pb.status.position);
        }
    } else if let Some(tex) = cover {
        fit(ui, tex, 1.0);
    } else {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("🎵").size(56.0).weak());
        });
        ui.add_space(16.0);
    }
}

/// The Player tab of a file while another one plays: play this one instead.
fn other_playing(
    ui: &mut Ui,
    state: &mut PlayerState,
    e: &FileEntry,
    cover: Option<&TextureHandle>,
) {
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        match cover {
            Some(tex) => {
                let size = tex.size_vec2();
                let scale = (ui.available_width() / size.x).min(160.0 / size.y).min(1.0);
                ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
            }
            None => {
                ui.label(RichText::new("🎵").size(40.0).weak());
            }
        }
        ui.add_space(8.0);
        let label = match state.is_playing() {
            true => tr("Playing:"),
            false => tr("Paused:"),
        };
        ui.label(RichText::new(label).weak());
        ui.label(RichText::new(state.title()).strong());
        ui.add_space(8.0);
        if ui.button(tr("▶ Play this file")).clicked() {
            let ctx = ui.ctx().clone();
            state.open_selected(&ctx, &e.path, true);
        }
    });
}

/// Seek bar with the time on either side.
fn seek_bar(ui: &mut Ui, state: &mut PlayerState, st: &ox_core::player::Status) {
    let Some(pb) = &mut state.playback else {
        return;
    };
    let total = st.duration.map_or(0.0, |d| d.as_secs_f32());
    let mut pos = pb.dragging.unwrap_or(st.position.as_secs_f32()).min(total);
    let mut seek = None;
    ui.horizontal(|ui| {
        let time = |t: f32| {
            RichText::new(ox_core::format::clock(t))
                .monospace()
                .size(11.0)
        };
        ui.label(time(pos));
        let right = ui.fonts_mut(|f| {
            f.layout_no_wrap(
                ox_core::format::clock(total),
                egui::FontId::monospace(11.0),
                egui::Color32::WHITE,
            )
            .size()
            .x
        });
        ui.spacing_mut().slider_width = (ui.available_width() - right - 12.0).max(40.0);
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
        ui.label(time(total));
    });
    if let Some(to) = seek {
        state.with_player(|p| p.seek(to));
    }
}

fn big(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).size(16.0)).min_size(egui::vec2(34.0, 28.0))
}

fn small(selected: bool, text: &str) -> egui::Button<'_> {
    egui::Button::selectable(selected, RichText::new(text).size(15.0))
}

/// Previous, back, play/pause, forward, next; then repeat, shuffle,
/// subtitles, speed and volume.
fn buttons(ui: &mut Ui, state: &mut PlayerState, st: &ox_core::player::Status) {
    let ctx = ui.ctx().clone();
    let main = state.main;
    ui.horizontal(|ui| {
        if main
            && ui
                .add(big("⏮"))
                .on_hover_text(shortcuts::hint(
                    tr("Previous file"),
                    Command::PlayerPrevious,
                ))
                .clicked()
        {
            state.previous(&ctx);
        }
        if ui
            .add(big("⏪"))
            .on_hover_text(tr("Back 10 seconds"))
            .clicked()
        {
            let to = st.position.saturating_sub(Duration::from_secs(10));
            state.with_player(|p| p.seek(to));
        }
        let (icon, hint) = match st.playing {
            true => ("⏸", tr("Pause")),
            false => ("▶", tr("Play")),
        };
        let hint = match main {
            true => shortcuts::hint(hint, Command::PlayerPlayPause),
            false => hint.to_owned(),
        };
        if ui.add(big(icon)).on_hover_text(hint).clicked() {
            state.toggle_play();
        }
        if ui
            .add(big("⏩"))
            .on_hover_text(tr("Forward 10 seconds"))
            .clicked()
        {
            let to = st.position + Duration::from_secs(10);
            state.with_player(|p| p.seek(st.duration.map_or(to, |d| to.min(d))));
        }
        if main
            && ui
                .add_enabled(state.queue.len() > 1, big("⏭"))
                .on_hover_text(shortcuts::hint(tr("Next file"), Command::PlayerNext))
                .clicked()
        {
            state.next(&ctx);
        }
    });
    ui.horizontal_wrapped(|ui| {
        // Quick Look: repeat is on or off; the pane's player also repeats
        // the whole queue.
        let (icon, hint) = match state.repeat {
            Repeat::Off => ("🔁", tr("Repeat: off")),
            Repeat::All => ("🔁", tr("Repeat: all")),
            Repeat::One => ("🔂", tr("Repeat: this file")),
        };
        if ui
            .add(small(state.repeat != Repeat::Off, icon))
            .on_hover_text(hint)
            .clicked()
        {
            let next = match (main, state.repeat) {
                (false, Repeat::Off) => Repeat::One,
                (false, _) => Repeat::Off,
                (true, r) => r.cycle(),
            };
            state.set_repeat(next);
        }
        if main
            && ui
                .add(small(state.queue.shuffled(), "🔀"))
                .on_hover_text(tr("Shuffle"))
                .clicked()
        {
            state.toggle_shuffle();
        }
        subtitle_menu(ui, state);
        let rate = state.rate;
        ui.menu_button(RichText::new(rate_label(rate)).size(13.0), |ui| {
            for r in RATES {
                if ui
                    .add(egui::Button::selectable(r == rate, rate_label(r)))
                    .clicked()
                {
                    state.set_rate(r);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(tr("Playback speed"));
        ui.add_space(6.0);
        let mut v = state.volume;
        let icon = if v <= 0.0 { "🔇" } else { "🔊" };
        ui.label(RichText::new(icon).size(15.0));
        ui.spacing_mut().slider_width = (ui.available_width() - 8.0).clamp(40.0, 110.0);
        if ui
            .add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false))
            .on_hover_text(tr("Volume"))
            .changed()
        {
            state.set_volume(v);
        }
    });
}

/// 💬 with the subtitle files of the video (or off); only when there are any.
fn subtitle_menu(ui: &mut Ui, state: &mut PlayerState) {
    let Some(pb) = &state.playback else {
        return;
    };
    if pb.tracks.is_empty() || !pb.status.video {
        return;
    }
    let names: Vec<String> = pb
        .tracks
        .iter()
        .map(|t| match t.label.as_str() {
            "" => ox_core::entry::display_name(&t.path),
            label => label.to_owned(),
        })
        .collect();
    let chosen = pb.track.filter(|_| state.subtitles_on);
    let mut pick = None;
    let text = RichText::new("💬").size(15.0);
    let resp = match names.len() {
        // One file: on or off.
        1 => {
            let r = ui.add(small(chosen.is_some(), "💬"));
            if r.clicked() {
                pick = Some(chosen.is_none().then_some(0));
            }
            r
        }
        _ => {
            ui.menu_button(text, |ui| {
                if ui
                    .add(egui::Button::selectable(chosen.is_none(), tr("Off")))
                    .clicked()
                {
                    pick = Some(None);
                }
                for (i, name) in names.iter().enumerate() {
                    if ui
                        .add(egui::Button::selectable(chosen == Some(i), name))
                        .clicked()
                    {
                        pick = Some(Some(i));
                    }
                }
            })
            .response
        }
    };
    resp.on_hover_text(tr("Subtitles"));
    if let Some(track) = pick {
        state.choose_track(track);
    }
}
