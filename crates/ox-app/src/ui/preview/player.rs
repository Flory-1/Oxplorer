//! The Player tab: the built-in audio and video player.

use super::*;

/// The Player tab.
#[derive(Default)]
pub(super) struct PlayerState {
    /// The audio or video file being played.
    pub(super) playback: Option<Playback>,
    /// Start playing when the Player tab opens next (the Info tab's ▶ Play).
    pub(super) autoplay: bool,
    /// Kept from file to file.
    pub(super) volume: Option<f32>,
    pub(super) looping: bool,
}

pub(super) struct Playback {
    pub(super) path: PathBuf,
    pub(super) player: Result<ox_core::player::Player, String>,
    /// The last video frame.
    pub(super) texture: Option<TextureHandle>,
    /// Where the seek bar is being dragged to (seconds).
    pub(super) dragging: Option<f32>,
}

/// "1:05" or "1:02:03".
pub(super) fn clock(secs: f32) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The built-in player: the video (or the cover of audio files), a seek
/// bar and the buttons. The file is opened when the tab is shown.
pub(super) fn player_tab(
    ui: &mut Ui,
    state: &mut PlayerState,
    e: &FileEntry,
    cover: Option<&TextureHandle>,
) {
    use ox_core::player::Player;
    let volume = *state.volume.get_or_insert(0.8);
    if state.playback.as_ref().is_none_or(|p| p.path != e.path) {
        let ctx = ui.ctx().clone();
        let player =
            Player::open(&e.path, move || ctx.request_repaint()).map_err(|e| e.to_string());
        if let Ok(p) = &player {
            p.set_volume(volume as f64);
            p.set_looping(state.looping);
            if std::mem::take(&mut state.autoplay) {
                p.play();
            }
        }
        state.playback = Some(Playback {
            path: e.path.clone(),
            player,
            texture: None,
            dragging: None,
        });
    }
    let pb = state.playback.as_mut().expect("opened above");
    let player = match &mut pb.player {
        Ok(p) => p,
        Err(err) => {
            ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), err.as_str());
            return;
        }
    };
    if let Some(frame) = player.frame() {
        let size = [frame.size[0] as usize, frame.size[1] as usize];
        let image = ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
        match &mut pb.texture {
            Some(tex) if tex.size() == size => tex.set(image, TextureOptions::LINEAR),
            _ => {
                pb.texture = Some(ui.ctx().load_texture(
                    "preview_video",
                    image,
                    TextureOptions::LINEAR,
                ))
            }
        }
    }
    let st = player.status();
    let width = ui.available_width();

    // The picture.
    if st.video {
        match &pb.texture {
            Some(tex) => {
                let size = tex.size_vec2();
                ui.add(egui::Image::new((tex.id(), size * (width / size.x))).corner_radius(6.0));
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(width, width * 9.0 / 16.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(rect, 6.0, egui::Color32::BLACK);
            }
        }
    } else if let Some(tex) = cover {
        let size = tex.size_vec2();
        let scale = (width / size.x).min(1.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
        });
    } else {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("🎵").size(56.0).weak());
        });
        ui.add_space(16.0);
    }
    if let Some(err) = &st.error {
        ui.add_space(4.0);
        ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), err.as_str());
    }
    ui.add_space(6.0);

    // Seek bar with the time on either side.
    let total = st.duration.map_or(0.0, |d| d.as_secs_f32());
    let mut pos = pb.dragging.unwrap_or(st.position.as_secs_f32()).min(total);
    ui.horizontal(|ui| {
        let time = |t: f32| RichText::new(clock(t)).monospace().size(11.0);
        ui.label(time(pos));
        let right = ui.fonts_mut(|f| {
            f.layout_no_wrap(
                clock(total),
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
            player.seek(Duration::from_secs_f32(pos));
            pb.dragging = None;
        }
        ui.label(time(total));
    });

    // Buttons.
    ui.horizontal(|ui| {
        let big = |text: &str| {
            egui::Button::new(RichText::new(text).size(16.0)).min_size(egui::vec2(34.0, 28.0))
        };
        if ui
            .add(big("⏪"))
            .on_hover_text(tr("Back 10 seconds"))
            .clicked()
        {
            player.seek(st.position.saturating_sub(Duration::from_secs(10)));
        }
        let (icon, hint) = if st.playing {
            ("⏸", tr("Pause"))
        } else {
            ("▶", tr("Play"))
        };
        if ui.add(big(icon)).on_hover_text(hint).clicked() {
            if st.playing {
                player.pause();
            } else {
                player.play();
            }
        }
        if ui
            .add(big("⏩"))
            .on_hover_text(tr("Forward 10 seconds"))
            .clicked()
        {
            let to = st.position + Duration::from_secs(10);
            player.seek(st.duration.map_or(to, |d| to.min(d)));
        }
        if ui
            .add(egui::Button::selectable(
                state.looping,
                RichText::new("🔁").size(15.0),
            ))
            .on_hover_text(tr("Repeat"))
            .clicked()
        {
            state.looping = !state.looping;
            player.set_looping(state.looping);
        }
        ui.add_space(6.0);
        let mut v = volume;
        let icon = if v <= 0.0 { "🔇" } else { "🔊" };
        ui.label(RichText::new(icon).size(15.0));
        ui.spacing_mut().slider_width = (ui.available_width() - 8.0).clamp(40.0, 110.0);
        if ui
            .add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false))
            .on_hover_text(tr("Volume"))
            .changed()
        {
            state.volume = Some(v);
            player.set_volume(v as f64);
        }
    });
    if st.playing {
        // The time and seek bar move on.
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
}
