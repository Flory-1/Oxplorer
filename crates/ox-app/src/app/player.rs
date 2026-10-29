//! The built-in player: the file playing, the queue, repeat, shuffle, speed
//! and subtitles. The app owns it (not the preview), so playback goes on in
//! the mini player while browsing. Drawn by `ui/preview/player.rs` (the
//! Player tab) and `ui/mini_player.rs`.

use super::*;
use egui::{ColorImage, TextureHandle, TextureOptions};
use ox_core::player::playlist;
use ox_core::player::queue::{self, Item, Queue, Repeat};
use ox_core::player::subtitles::{self, Subtitles, Track};
use ox_core::player::{Player, Status};

/// What the player is asked to do (buttons, keys, menus, the palette).
pub enum PlayerAction {
    PlayPause,
    Next,
    Previous,
    /// Play these files as a queue: audio and video, the ones in folders,
    /// the files of playlists.
    PlayAll(Vec<PathBuf>),
    /// Play the audio and video of a folder (in the order shown, if it is
    /// the current folder).
    PlayFolder(PathBuf),
    /// Open an `.m3u` / `.m3u8` playlist.
    Playlist(PathBuf),
    /// Stop and close the player.
    Close,
    /// Go to the playing file (its folder, selected, the Player tab).
    ShowCurrent,
}

/// The speeds offered.
pub const RATES: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// The file being played.
pub struct Playback {
    pub path: PathBuf,
    pub player: Result<Player, String>,
    /// Where playback is (read once a frame).
    pub status: Status,
    /// The last video frame.
    pub texture: Option<TextureHandle>,
    /// Where the seek bar is being dragged to (seconds).
    pub dragging: Option<f32>,
    /// Subtitle files next to the video, the one chosen and its cues.
    pub tracks: Vec<Track>,
    pub track: Option<usize>,
    pub subtitles: Option<Subtitles>,
    /// It was opened to play (not just shown paused).
    started: bool,
    /// Its end (or failure) was dealt with.
    finished: bool,
}

pub struct Session {
    pub playback: Option<Playback>,
    /// Start playing when the Player tab opens next (the Info tab's ▶ Play).
    pub autoplay: bool,
    /// Kept from file to file.
    pub volume: f32,
    pub rate: f64,
    pub repeat: Repeat,
    /// Go on with the next file when one ends.
    pub auto_advance: bool,
    pub subtitles_on: bool,
    pub queue: Queue,
    /// The app's player (Quick Look has its own, without queue or mini player).
    pub main: bool,
    /// Started for the item selected in the preview (not as a playlist).
    pub from_selection: bool,
    /// Settings → Preview → Keep playing while browsing.
    pub keep_playing: bool,
    /// The Player tab showed the playing file this frame, and last frame.
    pub shown: bool,
    pub shown_before: bool,
    /// Moved on to another file: the one before (the selection follows).
    pub moved_from: Option<PathBuf>,
    /// Make the queue from the folder the file is in (next frame).
    pub wants_folder_queue: bool,
    /// The queue list scrolls to the current item.
    pub scroll_queue: bool,
    /// The selection last frame, while it matters (see `poll_player`).
    last_selection: Option<Vec<PathBuf>>,
    /// Files in a row that couldn't be played (to give up on a queue).
    failures: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            playback: None,
            autoplay: false,
            volume: 0.8,
            rate: 1.0,
            repeat: Repeat::Off,
            auto_advance: true,
            subtitles_on: true,
            queue: Queue::default(),
            main: false,
            from_selection: false,
            keep_playing: true,
            shown: false,
            shown_before: false,
            moved_from: None,
            wants_folder_queue: false,
            scroll_queue: false,
            last_selection: None,
            failures: 0,
        }
    }
}

fn is_video(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    entry::FileKind::from_extension(&ext) == entry::FileKind::Video
}

/// Opening the file plays it as a playlist in the built-in player.
pub(super) fn opens_playlist(path: &Path) -> bool {
    ox_core::player::supported()
        && playlist::is_playlist(path)
        && archive::split_virtual(path).is_none()
}

fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

impl Session {
    /// The app's player.
    pub fn main() -> Self {
        Self {
            main: true,
            ..Default::default()
        }
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.playback.as_ref().map(|p| p.path.as_path())
    }

    pub fn is_playing(&self) -> bool {
        self.playback.as_ref().is_some_and(|p| p.status.playing)
    }

    /// The mini player is shown: something is loaded that the preview
    /// doesn't show.
    pub fn mini_shown(&self) -> bool {
        self.main && self.playback.is_some() && !self.shown_before
    }

    /// The title of the playing file (from the playlist, else its name).
    pub fn title(&self) -> String {
        let Some(path) = self.current_path() else {
            return String::new();
        };
        self.queue
            .current()
            .filter(|i| i.path == path)
            .map_or_else(|| entry::display_name(path), Item::label)
    }

    /// Open `path`, paused unless `play`; the one before stops.
    pub fn open(&mut self, ctx: &Context, path: &Path, play: bool) {
        self.playback = None;
        let repaint = ctx.clone();
        let player =
            Player::open(path, move || repaint.request_repaint()).map_err(|e| e.to_string());
        if let Ok(p) = &player {
            p.set_volume(self.volume as f64);
            p.set_rate(self.rate);
            p.set_looping(self.repeat == Repeat::One);
            if play {
                p.play();
            }
        }
        let tracks = match is_video(path) {
            true => subtitles::find_next_to(path),
            false => Vec::new(),
        };
        let track = subtitles::preferred(&tracks, ox_core::i18n::is_german());
        let subtitles = track.and_then(|i| Subtitles::read(&tracks[i].path).ok());
        self.playback = Some(Playback {
            path: path.to_path_buf(),
            player,
            status: Status::default(),
            texture: None,
            dragging: None,
            tracks,
            track,
            subtitles,
            started: play,
            finished: false,
        });
        self.scroll_queue = true;
    }

    /// Open the file selected in the preview: the queue becomes its folder
    /// (unless it is in the queue already).
    pub fn open_selected(&mut self, ctx: &Context, path: &Path, play: bool) {
        self.open(ctx, path, play);
        self.from_selection = true;
        match self.queue.position_of(path) {
            Some(i) => {
                self.queue.jump(i);
            }
            None => self.wants_folder_queue = self.main,
        }
    }

    /// Play `queue` from its current item.
    pub fn start(&mut self, ctx: &Context, mut queue: Queue) {
        if self.queue.shuffled() {
            queue.set_shuffle(true, seed());
        }
        self.queue = queue;
        self.from_selection = false;
        self.failures = 0;
        if let Some(i) = self.queue.current_index() {
            self.play_index(ctx, i);
        }
    }

    /// Play the queue's item `i`.
    fn play_index(&mut self, ctx: &Context, i: usize) {
        let Some(item) = self.queue.items().get(i).cloned() else {
            return;
        };
        let from = self.current_path().map(Path::to_path_buf);
        self.open(ctx, &item.path, true);
        self.moved_from = from.filter(|f| *f != item.path);
    }

    /// Stop, and forget the queue.
    pub fn stop(&mut self) {
        self.playback = None;
        self.queue = Queue::default();
        self.from_selection = false;
    }

    pub fn with_player(&self, f: impl FnOnce(&Player)) {
        if let Some(Ok(p)) = self.playback.as_ref().map(|p| &p.player) {
            f(p);
        }
    }

    pub fn toggle_play(&mut self) {
        let playing = self.is_playing();
        self.with_player(|p| if playing { p.pause() } else { p.play() });
    }

    pub fn pause(&mut self) {
        self.with_player(Player::pause);
    }

    pub fn next(&mut self, ctx: &Context) {
        // "Next" goes on at the end too, unless repeat is off.
        let repeat = match self.repeat {
            Repeat::One => Repeat::All,
            r => r,
        };
        if let Some(i) = self.queue.next(repeat) {
            self.failures = 0;
            self.play_index(ctx, i);
        }
    }

    /// The previous file; a few seconds into a file, its start.
    pub fn previous(&mut self, ctx: &Context) {
        let position = self
            .playback
            .as_ref()
            .map_or(Duration::ZERO, |p| p.status.position);
        let repeat = match self.repeat {
            Repeat::One => Repeat::All,
            r => r,
        };
        if queue::previous_restarts(position) {
            self.with_player(|p| p.seek(Duration::ZERO));
        } else if let Some(i) = self.queue.previous(repeat) {
            self.failures = 0;
            self.play_index(ctx, i);
        } else {
            self.with_player(|p| p.seek(Duration::ZERO));
        }
    }

    pub fn jump(&mut self, ctx: &Context, index: usize) {
        if self.queue.jump(index).is_some() {
            self.failures = 0;
            self.play_index(ctx, index);
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        self.with_player(|p| p.set_volume(volume as f64));
    }

    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        self.with_player(|p| p.set_rate(rate));
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
        self.with_player(|p| p.set_looping(repeat == Repeat::One));
    }

    pub fn toggle_shuffle(&mut self) {
        let on = !self.queue.shuffled();
        self.queue.set_shuffle(on, seed());
        self.scroll_queue = true;
    }

    /// Show the subtitle file `track` (or none).
    pub fn choose_track(&mut self, track: Option<usize>) {
        let Some(pb) = &mut self.playback else {
            return;
        };
        self.subtitles_on = track.is_some();
        if track.is_some() && pb.track != track {
            pb.subtitles = track.and_then(|i| Subtitles::read(&pb.tracks.get(i)?.path).ok());
            pb.track = track;
        }
    }

    /// Every frame: the newest video frame, where playback is, and the next
    /// file when one has ended (or couldn't be played).
    pub fn tick(&mut self, ctx: &Context) {
        let Some(pb) = &mut self.playback else {
            return;
        };
        let failed = match &mut pb.player {
            Ok(player) => {
                if let Some(frame) = player.frame() {
                    let size = [frame.size[0] as usize, frame.size[1] as usize];
                    let image = ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
                    match &mut pb.texture {
                        Some(tex) if tex.size() == size => tex.set(image, TextureOptions::LINEAR),
                        _ => {
                            pb.texture = Some(ctx.load_texture(
                                "preview_video",
                                image,
                                TextureOptions::LINEAR,
                            ))
                        }
                    }
                }
                pb.status = player.status();
                pb.status.error.is_some()
            }
            Err(_) => true,
        };
        if pb.status.playing {
            self.failures = 0;
            // The time, the seek bar and the subtitles move on.
            let every = match pb.subtitles.is_some() && self.subtitles_on {
                true => 100,
                false => 250,
            };
            ctx.request_repaint_after(Duration::from_millis(every));
        }
        // Files that can't be played are skipped in playlists, not when
        // picked in the preview (the message stays).
        let done = pb.status.ended || (failed && pb.started && !self.from_selection);
        if !self.main || !done || pb.finished {
            return;
        }
        pb.finished = true;
        if failed {
            self.failures += 1;
        }
        if self.auto_advance && self.failures < self.queue.len() {
            if let Some(i) = self.queue.after_end(self.repeat) {
                self.play_index(ctx, i);
            }
        }
    }
}

impl OxApp {
    /// Every frame, before drawing: the player moves on, the selection
    /// follows it, its queue is made, and with "keep playing" off it stops
    /// when the selection moves away from it (or the preview is hidden).
    pub(super) fn poll_player(&mut self, ctx: &Context) {
        let keep = !self.settings.preview.stop_player_on_select;
        let p = &mut self.player;
        p.keep_playing = keep;
        p.shown_before = std::mem::take(&mut p.shown);
        p.tick(ctx);
        if let Some(from) = p.moved_from.take() {
            self.follow_player(&from);
        }
        if std::mem::take(&mut self.player.wants_folder_queue) {
            self.folder_queue();
        }
        // One player at a time: this one pauses while Quick Look's plays.
        if self
            .quick_look
            .as_ref()
            .is_some_and(|q| q.preview.playing())
        {
            self.player.pause();
        }
        if keep || !self.player.from_selection || self.player.playback.is_none() {
            self.player.last_selection = None;
            return;
        }
        let tab = self.tabs.active();
        let selection = tab.view.selected_paths();
        let shown = self.settings.browsing.show_preview
            && tab.location().shows_files()
            && self.quick_look.is_none();
        let current = self.player.current_path().map(Path::to_path_buf);
        let away = self
            .player
            .last_selection
            .as_ref()
            .is_some_and(|last| *last != selection)
            && selection.as_slice() != current.as_slice();
        if !shown || away {
            self.player.stop();
        }
        self.player.last_selection = Some(selection);
    }

    /// The player went from `from` to another file: if `from` was selected,
    /// select the new one (when it is in the list).
    fn follow_player(&mut self, from: &Path) {
        let Some(to) = self.player.current_path().map(Path::to_path_buf) else {
            return;
        };
        let view = &mut self.tabs.active_mut().view;
        if !matches!(view.selected_paths().as_slice(), [only] if only == from) {
            return;
        }
        let found =
            (0..view.visible.len()).find(|&i| view.visible_entry(i).is_some_and(|e| e.path == to));
        if let Some(i) = found {
            view.select_only(i);
            view.scroll_to = Some(i);
        }
    }

    /// The queue of a file played from the preview: the audio and video of
    /// its folder, in the order shown.
    fn folder_queue(&mut self) {
        let Some(current) = self.player.current_path().map(Path::to_path_buf) else {
            return;
        };
        let view = &self.tabs.active().view;
        let listing = (0..view.visible.len())
            .filter_map(|i| view.visible_entry(i))
            .filter(|e| !e.is_dir)
            .map(|e| e.path.as_path());
        let mut q = Queue::from_listing(listing, &current);
        if q.position_of(&current).is_none() {
            q = Queue::new(vec![Item::new(current)], 0);
        }
        if self.player.queue.shuffled() {
            q.set_shuffle(true, seed());
        }
        self.player.queue = q;
        self.player.scroll_queue = true;
    }

    pub(super) fn player_action(&mut self, a: PlayerAction) {
        let ctx = self.ctx.clone();
        match a {
            PlayerAction::PlayPause => self.player.toggle_play(),
            PlayerAction::Next => self.player.next(&ctx),
            PlayerAction::Previous => self.player.previous(&ctx),
            PlayerAction::Close => self.player.stop(),
            PlayerAction::PlayAll(paths) => {
                let mut items = Vec::new();
                for p in paths {
                    if ox_core::network::is_dir(&p) {
                        items.extend(queue::playable_in(&p).into_iter().map(Item::new));
                    } else if playlist::is_playlist(&p) {
                        items.extend(playlist::read(&p).unwrap_or_default());
                    } else if queue::is_playable(&p) {
                        items.push(Item::new(p));
                    }
                }
                self.play_items(items);
            }
            PlayerAction::PlayFolder(folder) => {
                let items = match self.current_folder() == Some(folder.clone()) {
                    true => {
                        let view = &self.tabs.active().view;
                        (0..view.visible.len())
                            .filter_map(|i| view.visible_entry(i))
                            .filter(|e| !e.is_dir && queue::is_playable(&e.path))
                            .map(|e| Item::new(e.path.clone()))
                            .collect()
                    }
                    false => queue::playable_in(&folder)
                        .into_iter()
                        .map(Item::new)
                        .collect(),
                };
                self.play_items(items);
            }
            PlayerAction::Playlist(path) => match playlist::read(&path) {
                Ok(items) => {
                    self.quick.open_file(&path);
                    self.save_quick();
                    self.play_items(items);
                }
                Err(e) => self.toast(ox_core::trf!(
                    "Couldn't open {v1}: {e}",
                    v1 = path.display(),
                    e = e
                )),
            },
            PlayerAction::ShowCurrent => {
                let Some(path) = self.player.current_path().map(Path::to_path_buf) else {
                    return;
                };
                let view = &mut self.tabs.active_mut().view;
                let here = (0..view.visible.len())
                    .find(|&i| view.visible_entry(i).is_some_and(|e| e.path == path));
                match here {
                    Some(i) => {
                        view.select_only(i);
                        view.scroll_to = Some(i);
                    }
                    None => self.show_in_folder(path),
                }
                self.preview.show_player();
                if !self.settings.browsing.show_preview {
                    self.settings.browsing.show_preview = true;
                    self.push(Action::SettingsChanged);
                }
            }
        }
    }

    fn play_items(&mut self, items: Vec<Item>) {
        if items.is_empty() {
            self.toast(tr("There is no audio or video to play."));
            return;
        }
        let ctx = self.ctx.clone();
        self.player.start(&ctx, Queue::new(items, 0));
    }
}
