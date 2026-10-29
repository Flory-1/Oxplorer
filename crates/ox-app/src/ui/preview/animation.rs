//! Animated GIF and WebP pictures: decoded frame by frame in the
//! background, played while they are on screen (each frame as long as the
//! file says), standing still while they aren't.

use std::io::{BufReader, Cursor};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{ColorImage, Context, TextureHandle, TextureOptions, Ui};
use image::{AnimationDecoder, DynamicImage, Frame, ImageResult};
use ox_core::i18n::t as tr;
use ox_core::picture::{self, Source};

use super::Loaded;

/// Frames kept at most, and their pixels' bytes: longer animations play
/// the beginning.
const MAX_FRAMES: usize = 500;
const MAX_BYTES: usize = 192 * 1024 * 1024;

/// The frames of an animated `src` scaled to `side`, or `None` for a
/// still picture (and anything that isn't a GIF/WebP).
pub(super) fn decode(src: Source, ext: &str, side: u32) -> Option<Loaded> {
    let frames: Box<dyn Iterator<Item = ImageResult<Frame>>> = match (ext, src) {
        ("gif", Source::File(path)) => {
            let file = BufReader::new(std::fs::File::open(path).ok()?);
            Box::new(
                image::codecs::gif::GifDecoder::new(file)
                    .ok()?
                    .into_frames(),
            )
        }
        ("gif", Source::Bytes(bytes)) => Box::new(
            image::codecs::gif::GifDecoder::new(Cursor::new(bytes))
                .ok()?
                .into_frames(),
        ),
        ("webp", Source::File(path)) => {
            let file = BufReader::new(std::fs::File::open(path).ok()?);
            let decoder = image::codecs::webp::WebPDecoder::new(file).ok()?;
            if !decoder.has_animation() {
                return None;
            }
            Box::new(decoder.into_frames())
        }
        ("webp", Source::Bytes(bytes)) => {
            let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
            if !decoder.has_animation() {
                return None;
            }
            Box::new(decoder.into_frames())
        }
        _ => return None,
    };
    let mut out = Vec::new();
    let mut bytes = 0;
    let mut full = [0, 0];
    for frame in frames {
        // A broken frame ends it; what came before still plays.
        let Ok(frame) = frame else {
            break;
        };
        let (num, den) = frame.delay().numer_denom_ms();
        let delay = picture::frame_delay(num / den.max(1));
        let buffer = frame.into_buffer();
        full = [buffer.width(), buffer.height()];
        let buffer = if buffer.width().max(buffer.height()) > side {
            DynamicImage::ImageRgba8(buffer)
                .thumbnail(side, side)
                .into_rgba8()
        } else {
            buffer
        };
        let size = [buffer.width() as usize, buffer.height() as usize];
        bytes += buffer.as_raw().len();
        out.push((
            ColorImage::from_rgba_unmultiplied(size, buffer.as_raw()),
            delay,
        ));
        if out.len() >= MAX_FRAMES || bytes >= MAX_BYTES {
            break;
        }
    }
    (out.len() > 1).then_some(Loaded::Animation(out, full))
}

/// A playing animation: one texture showing the frame due.
pub(super) struct Animation {
    texture: TextureHandle,
    frames: Vec<Arc<ColorImage>>,
    delays: Vec<Duration>,
    /// Its size (each frame's).
    pub(super) size: [u32; 2],
    clock: Mutex<Clock>,
}

#[derive(Default)]
struct Clock {
    /// How far it has played.
    played: Duration,
    /// When it was drawn last, and how long until the next frame then.
    drawn: Option<(Instant, Duration)>,
    shown: usize,
}

impl Animation {
    pub(super) fn new(ctx: &Context, frames: Vec<(ColorImage, Duration)>, size: [u32; 2]) -> Self {
        let (frames, delays): (Vec<_>, Vec<_>) =
            frames.into_iter().map(|(f, d)| (Arc::new(f), d)).unzip();
        let first = frames.first().cloned().unwrap_or_default();
        Self {
            texture: ctx.load_texture("preview", first, TextureOptions::LINEAR),
            frames,
            delays,
            size,
            clock: Mutex::default(),
        }
    }

    /// The texture with the frame due now, asking for a repaint when the
    /// next one is. Only called while it is drawn, so a hidden animation
    /// costs nothing and goes on where it stopped.
    pub(super) fn texture(&self, ctx: &Context) -> &TextureHandle {
        let mut clock = self.clock.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        if let Some((at, due)) = clock.drawn {
            let since = now - at;
            // Not drawn for longer than asked: it was out of sight.
            if since <= due + Duration::from_millis(250) {
                clock.played += since;
            }
        }
        let (i, left) = picture::frame_at(&self.delays, clock.played);
        if i != clock.shown {
            if let Some(frame) = self.frames.get(i) {
                let delta = egui::epaint::ImageDelta::full(frame.clone(), TextureOptions::LINEAR);
                ctx.tex_manager().write().set(self.texture.id(), delta);
            }
            clock.shown = i;
        }
        clock.drawn = Some((now, left));
        ctx.request_repaint_after(left);
        &self.texture
    }

    /// The Info tab's rows: size, frames and how long one loop lasts.
    pub(super) fn facts(&self) -> Vec<(&'static str, String)> {
        let [w, h] = self.size;
        let total: Duration = self.delays.iter().sum();
        vec![
            (tr("Dimensions"), format!("{w} × {h} px")),
            (
                tr("Frames"),
                ox_core::trf!(
                    "{v1} ({v2} s per loop)",
                    v1 = self.frames.len(),
                    v2 = format!("{:.1}", total.as_secs_f32())
                ),
            ),
        ]
    }

    /// Drawn as wide as the pane (like a still picture).
    pub(super) fn show(&self, ui: &mut Ui) {
        let tex = self.texture(ui.ctx());
        let size = tex.size_vec2();
        let scale = (ui.available_width() / size.x).min(1.5);
        ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Rgba, RgbaImage};

    fn gif(frames: &[(u8, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            for &(red, ms) in frames {
                let buffer = RgbaImage::from_pixel(40, 20, Rgba([red, 0, 0, 255]));
                let delay = Delay::from_numer_denom_ms(ms, 1);
                encoder
                    .encode_frame(Frame::from_parts(buffer, 0, 0, delay))
                    .unwrap();
            }
        }
        out
    }

    #[test]
    fn animated_gifs_keep_their_frames_and_delays() {
        let data = gif(&[(255, 200), (0, 100)]);
        let Some(Loaded::Animation(frames, size)) = decode(Source::Bytes(&data), "gif", 10) else {
            panic!("not an animation");
        };
        assert_eq!(size, [40, 20]);
        assert_eq!(frames.len(), 2);
        // Scaled down to the side asked for.
        assert_eq!(frames[0].0.size, [10, 5]);
        assert_eq!(frames[0].1, Duration::from_millis(200));
        assert_eq!(frames[1].1, Duration::from_millis(100));
        let ctx = Context::default();
        let a = Animation::new(&ctx, frames, size);
        assert_eq!(a.facts()[1].1, "2 (0.3 s per loop)");
        assert_eq!(a.texture(&ctx).size(), [10, 5]);
        // One frame: a still picture.
        assert!(decode(Source::Bytes(&gif(&[(255, 100)])), "gif", 10).is_none());
        assert!(decode(Source::Bytes(&data), "png", 10).is_none());
    }
}
