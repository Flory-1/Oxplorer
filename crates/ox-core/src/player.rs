//! Audio and video playback for the preview pane.
//!
//! On Windows the system's media engine (`Windows.Media.Playback.MediaPlayer`)
//! plays the file: every format Windows can play, hardware decoding, sound
//! and picture in sync. Video frames are copied into a small Direct3D
//! texture and read back as RGBA pixels for the UI to draw.
//! Elsewhere there is no player ([`supported`] is false).
//!
//! What plays next ([`queue`]), playlists ([`playlist`]) and subtitles
//! ([`subtitles`]) are worked out here too, the same on every system.

pub mod playlist;
pub mod queue;
pub mod subtitles;

use std::io;
use std::path::Path;
use std::time::Duration;

/// Whether files can be played here.
pub fn supported() -> bool {
    cfg!(windows)
}

/// Video frames are scaled down to fit this size before they are read back.
pub const MAX_FRAME: [u32; 2] = [960, 540];

/// Where playback is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub position: Duration,
    /// Known once the file is open.
    pub duration: Option<Duration>,
    pub playing: bool,
    /// Playback reached the end.
    pub ended: bool,
    /// The file has a picture (known once it is open).
    pub video: bool,
    /// Why the file can't be played.
    pub error: Option<String>,
}

/// One video frame: width, height and RGBA pixels.
pub struct Frame {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}

/// The size a `[w, h]` video is shown at: scaled down to fit `max`, even.
pub fn fit(size: [u32; 2], max: [u32; 2]) -> [u32; 2] {
    let [w, h] = size;
    if w == 0 || h == 0 {
        return [0, 0];
    }
    let scale = (max[0] as f64 / w as f64)
        .min(max[1] as f64 / h as f64)
        .min(1.0);
    let even = |v: f64| ((v.round() as u32) & !1).max(2);
    [even(w as f64 * scale), even(h as f64 * scale)]
}

/// Swap BGRA pixels (Direct3D's order) to RGBA, rows `pitch` bytes apart,
/// and make them opaque.
pub fn bgra_to_rgba(src: &[u8], pitch: usize, size: [u32; 2]) -> Vec<u8> {
    let [w, h] = [size[0] as usize, size[1] as usize];
    let mut out = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let line = &src[row * pitch..row * pitch + w * 4];
        for px in line.as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    out
}

pub struct Player {
    #[cfg(windows)]
    imp: imp::Player,
}

impl Player {
    /// Open `path`, paused at the start. `repaint` is called (from other
    /// threads) when there is something new to show.
    pub fn open(path: &Path, repaint: impl Fn() + Send + Sync + 'static) -> io::Result<Player> {
        #[cfg(windows)]
        {
            Ok(Player {
                imp: imp::Player::open(path, std::sync::Arc::new(repaint))?,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = (path, repaint);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "playback is only built in on Windows",
            ))
        }
    }

    pub fn status(&self) -> Status {
        #[cfg(windows)]
        {
            self.imp.status()
        }
        #[cfg(not(windows))]
        Status::default()
    }

    pub fn play(&self) {
        #[cfg(windows)]
        self.imp.play();
    }

    pub fn pause(&self) {
        #[cfg(windows)]
        self.imp.pause();
    }

    pub fn seek(&self, to: Duration) {
        #[cfg(windows)]
        self.imp.seek(to);
        #[cfg(not(windows))]
        let _ = to;
    }

    /// 0.0 to 1.0.
    pub fn set_volume(&self, volume: f64) {
        #[cfg(windows)]
        self.imp.set_volume(volume);
        #[cfg(not(windows))]
        let _ = volume;
    }

    pub fn set_looping(&self, on: bool) {
        #[cfg(windows)]
        self.imp.set_looping(on);
        #[cfg(not(windows))]
        let _ = on;
    }

    /// Playback speed: 1.0 is normal, 0.5 half, 2.0 double.
    pub fn set_rate(&self, rate: f64) {
        #[cfg(windows)]
        self.imp.set_rate(rate);
        #[cfg(not(windows))]
        let _ = rate;
    }

    /// The newest video frame, if there is one since the last call.
    pub fn frame(&mut self) -> Option<Frame> {
        #[cfg(windows)]
        {
            self.imp.frame()
        }
        #[cfg(not(windows))]
        None
    }
}

#[cfg(windows)]
mod imp {
    use super::{Frame, Status, MAX_FRAME};
    use std::io;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use windows::core::{Interface, HSTRING};
    use windows::Foundation::{TimeSpan, TypedEventHandler};
    use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
    use windows::Media::Core::MediaSource;
    use windows::Media::Playback::{MediaPlaybackState, MediaPlayer};
    use windows::Storage::StorageFile;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
        D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::IDXGISurface;
    use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11SurfaceFromDXGISurface;

    fn err(e: windows::core::Error) -> io::Error {
        io::Error::other(e.message())
    }

    /// 100-nanosecond units.
    fn to_duration(t: TimeSpan) -> Duration {
        Duration::from_nanos(t.Duration.max(0) as u64 * 100)
    }

    /// What the event handlers report (they run on other threads).
    #[derive(Default)]
    struct Events {
        opened: AtomicBool,
        ended: AtomicBool,
        new_frame: AtomicBool,
        error: Mutex<Option<String>>,
    }

    /// Where video frames are copied to and read back from.
    struct Surfaces {
        size: [u32; 2],
        context: ID3D11DeviceContext,
        target: ID3D11Texture2D,
        staging: ID3D11Texture2D,
        surface: IDirect3DSurface,
    }

    pub struct Player {
        player: MediaPlayer,
        events: Arc<Events>,
        tokens: [i64; 4],
        device: Option<ID3D11Device>,
        surfaces: Option<Surfaces>,
    }

    impl Player {
        pub fn open(path: &Path, repaint: Arc<dyn Fn() + Send + Sync>) -> io::Result<Player> {
            let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))
                .and_then(|op| op.join())
                .map_err(err)?;
            let source = MediaSource::CreateFromStorageFile(&file).map_err(err)?;
            let player = MediaPlayer::new().map_err(err)?;
            player.SetAutoPlay(false).map_err(err)?;
            player.SetIsVideoFrameServerEnabled(true).map_err(err)?;
            let events = Arc::new(Events::default());

            let (ev, rp) = (events.clone(), repaint.clone());
            let opened = player
                .MediaOpened(&TypedEventHandler::new(move |_, _| {
                    ev.opened.store(true, Ordering::Release);
                    rp();
                    Ok(())
                }))
                .map_err(err)?;
            let (ev, rp) = (events.clone(), repaint.clone());
            let ended = player
                .MediaEnded(&TypedEventHandler::new(move |_, _| {
                    ev.ended.store(true, Ordering::Release);
                    rp();
                    Ok(())
                }))
                .map_err(err)?;
            let (ev, rp) = (events.clone(), repaint.clone());
            let failed = player
                .MediaFailed(&TypedEventHandler::new(
                    move |_,
                          args: windows::core::Ref<
                        windows::Media::Playback::MediaPlayerFailedEventArgs,
                    >| {
                        let message = args
                            .as_ref()
                            .and_then(|a| a.ErrorMessage().ok())
                            .map(|m| m.to_string())
                            .filter(|m| !m.trim().is_empty())
                            .unwrap_or_else(|| {
                                crate::i18n::t("Windows can't play this file.").into()
                            });
                        if let Ok(mut e) = ev.error.lock() {
                            *e = Some(message);
                        }
                        rp();
                        Ok(())
                    },
                ))
                .map_err(err)?;
            let (ev, rp) = (events.clone(), repaint);
            let frame = player
                .VideoFrameAvailable(&TypedEventHandler::new(move |_, _| {
                    ev.new_frame.store(true, Ordering::Release);
                    rp();
                    Ok(())
                }))
                .map_err(err)?;
            player.SetSource(&source).map_err(err)?;
            Ok(Player {
                player,
                events,
                tokens: [opened, ended, failed, frame],
                device: None,
                surfaces: None,
            })
        }

        pub fn status(&self) -> Status {
            let session = self.player.PlaybackSession().ok();
            let opened = self.events.opened.load(Ordering::Acquire);
            let get = |f: &dyn Fn(
                &windows::Media::Playback::MediaPlaybackSession,
            ) -> Option<Duration>| { session.as_ref().and_then(f) };
            Status {
                position: get(&|s| s.Position().ok().map(to_duration)).unwrap_or_default(),
                duration: get(&|s| s.NaturalDuration().ok().map(to_duration))
                    .filter(|d| opened && !d.is_zero()),
                playing: session
                    .as_ref()
                    .and_then(|s| s.PlaybackState().ok())
                    .is_some_and(|s| {
                        s == MediaPlaybackState::Playing
                            || s == MediaPlaybackState::Buffering
                            || s == MediaPlaybackState::Opening
                    }),
                ended: self.events.ended.load(Ordering::Acquire),
                video: opened
                    && session
                        .as_ref()
                        .and_then(|s| s.NaturalVideoWidth().ok())
                        .is_some_and(|w| w > 0),
                error: self.events.error.lock().ok().and_then(|e| e.clone()),
            }
        }

        pub fn play(&self) {
            if self.events.ended.swap(false, Ordering::AcqRel) {
                self.seek(Duration::ZERO);
            }
            let _ = self.player.Play();
        }

        pub fn pause(&self) {
            let _ = self.player.Pause();
        }

        pub fn seek(&self, to: Duration) {
            self.events.ended.store(false, Ordering::Release);
            if let Ok(session) = self.player.PlaybackSession() {
                let _ = session.SetPosition(TimeSpan {
                    Duration: (to.as_nanos() / 100) as i64,
                });
            }
        }

        pub fn set_volume(&self, volume: f64) {
            let _ = self.player.SetVolume(volume.clamp(0.0, 1.0));
        }

        pub fn set_looping(&self, on: bool) {
            let _ = self.player.SetIsLoopingEnabled(on);
        }

        pub fn set_rate(&self, rate: f64) {
            if let Ok(session) = self.player.PlaybackSession() {
                let _ = session.SetPlaybackRate(rate.clamp(0.25, 4.0));
            }
        }

        pub fn frame(&mut self) -> Option<Frame> {
            if !self.events.new_frame.swap(false, Ordering::AcqRel) {
                return None;
            }
            let session = self.player.PlaybackSession().ok()?;
            let natural = [
                session.NaturalVideoWidth().ok()?,
                session.NaturalVideoHeight().ok()?,
            ];
            let size = super::fit(natural, MAX_FRAME);
            if size[0] == 0 {
                return None;
            }
            if self.surfaces.as_ref().is_none_or(|s| s.size != size) {
                self.surfaces = self.create_surfaces(size).ok();
            }
            let s = self.surfaces.as_ref()?;
            self.player.CopyFrameToVideoSurface(&s.surface).ok()?;
            // SAFETY: both textures are `size`, BGRA; the mapping is read
            // within its pitch and released before returning.
            unsafe {
                s.context.CopyResource(&s.staging, &s.target);
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                s.context
                    .Map(&s.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                    .ok()?;
                let pitch = mapped.RowPitch as usize;
                let bytes =
                    std::slice::from_raw_parts(mapped.pData as *const u8, pitch * size[1] as usize);
                let rgba = super::bgra_to_rgba(bytes, pitch, size);
                s.context.Unmap(&s.staging, 0);
                Some(Frame { size, rgba })
            }
        }

        fn create_surfaces(&mut self, size: [u32; 2]) -> windows::core::Result<Surfaces> {
            if self.device.is_none() {
                let mut device = None;
                // SAFETY: plain device creation with out pointers to locals.
                unsafe {
                    D3D11CreateDevice(
                        None,
                        D3D_DRIVER_TYPE_HARDWARE,
                        HMODULE::default(),
                        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                        None,
                        D3D11_SDK_VERSION,
                        Some(&mut device),
                        None,
                        None,
                    )?;
                }
                self.device = device;
            }
            let device = self
                .device
                .as_ref()
                .ok_or_else(windows::core::Error::empty)?;
            let mut desc = D3D11_TEXTURE2D_DESC {
                Width: size[0],
                Height: size[1],
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let (mut target, mut staging) = (None, None);
            // SAFETY: `desc` outlives the calls; results are checked below.
            unsafe {
                device.CreateTexture2D(&desc, None, Some(&mut target))?;
                desc.Usage = D3D11_USAGE_STAGING;
                desc.BindFlags = 0;
                desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                device.CreateTexture2D(&desc, None, Some(&mut staging))?;
            }
            let target = target.ok_or_else(windows::core::Error::empty)?;
            let staging = staging.ok_or_else(windows::core::Error::empty)?;
            let dxgi: IDXGISurface = target.cast()?;
            // SAFETY: wraps the texture's DXGI surface for WinRT.
            let surface: IDirect3DSurface =
                unsafe { CreateDirect3D11SurfaceFromDXGISurface(&dxgi)? }.cast()?;
            // SAFETY: the device's own immediate context.
            let context = unsafe { device.GetImmediateContext()? };
            Ok(Surfaces {
                size,
                context,
                target,
                staging,
                surface,
            })
        }
    }

    impl Drop for Player {
        fn drop(&mut self) {
            let _ = self.player.Pause();
            let [opened, ended, failed, frame] = self.tokens;
            let _ = self.player.RemoveMediaOpened(opened);
            let _ = self.player.RemoveMediaEnded(ended);
            let _ = self.player.RemoveMediaFailed(failed);
            let _ = self.player.RemoveVideoFrameAvailable(frame);
            let _ = self.player.Close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fit_the_preview() {
        assert_eq!(fit([1920, 1080], MAX_FRAME), [960, 540]);
        assert_eq!(fit([640, 480], MAX_FRAME), [640, 480]);
        // Portrait videos are limited by the height; sizes stay even.
        assert_eq!(fit([1080, 1920], MAX_FRAME), [304, 540]);
        assert_eq!(fit([0, 0], MAX_FRAME), [0, 0]);
    }

    #[test]
    fn bgra_rows_with_padding() {
        // 2×1 pixels, rows padded to 12 bytes.
        let src = [1, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9];
        assert_eq!(bgra_to_rgba(&src, 12, [2, 1]), [3, 2, 1, 255, 6, 5, 4, 255]);
    }
}
