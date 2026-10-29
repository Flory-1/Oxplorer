//! The play queue: the files to play, in their order or shuffled, and what
//! comes next with repeat off, repeat all or repeat one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::entry::FileKind;

/// "Previous" within this time of a track's start goes to the previous
/// track; later it starts the track again.
pub const RESTART_AFTER: Duration = Duration::from_secs(3);

/// Whether "previous" at `position` starts the track again (instead of
/// going to the one before).
pub fn previous_restarts(position: Duration) -> bool {
    position > RESTART_AFTER
}

/// Whether the file is audio or video (by its extension).
pub fn is_playable(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        FileKind::from_extension(&ext),
        FileKind::Audio | FileKind::Video
    )
}

/// The audio and video files directly in `dir`, by name (numbers in
/// order: "2" before "10").
pub fn playable_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
                .map(|e| e.path())
                .filter(|p| is_playable(p))
                .collect()
        })
        .unwrap_or_default();
    files.sort_by(|a, b| {
        crate::natural::natural_cmp(
            &crate::entry::display_name(a),
            &crate::entry::display_name(b),
        )
    });
    files
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    /// The whole queue, from the start again.
    All,
    /// The current track, again and again.
    One,
}

impl Repeat {
    /// The mode the repeat button switches to: off → all → one → off.
    pub fn cycle(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

/// One file of the queue.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    /// From a playlist's `#EXTINF` line.
    pub title: Option<String>,
}

impl Item {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            title: None,
        }
    }

    /// The title, else the file name.
    pub fn label(&self) -> String {
        match &self.title {
            Some(t) if !t.trim().is_empty() => t.clone(),
            _ => crate::entry::display_name(&self.path),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    items: Vec<Item>,
    /// The play order: indices into `items` (shuffled or in order).
    order: Vec<usize>,
    /// Where in `order` the current item is.
    pos: usize,
    shuffle: bool,
    /// State of the shuffle's random numbers.
    seed: u64,
}

impl Queue {
    /// A queue of `items` starting at `items[start]`.
    pub fn new(items: Vec<Item>, start: usize) -> Self {
        let order = (0..items.len()).collect();
        Self {
            pos: start.min(items.len().saturating_sub(1)),
            items,
            order,
            shuffle: false,
            seed: 0,
        }
    }

    /// The audio and video files of a folder listing (in the listing's
    /// order), starting at `current` (or the first of them).
    pub fn from_listing<'a>(paths: impl IntoIterator<Item = &'a Path>, current: &Path) -> Self {
        let items: Vec<Item> = paths
            .into_iter()
            .filter(|p| is_playable(p))
            .map(Item::new)
            .collect();
        let start = items.iter().position(|i| i.path == current).unwrap_or(0);
        Self::new(items, start)
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The current item's index in [`items`](Self::items).
    pub fn current_index(&self) -> Option<usize> {
        self.order.get(self.pos).copied()
    }

    pub fn current(&self) -> Option<&Item> {
        self.current_index().and_then(|i| self.items.get(i))
    }

    /// Where `path` is in [`items`](Self::items).
    pub fn position_of(&self, path: &Path) -> Option<usize> {
        self.items.iter().position(|i| i.path == path)
    }

    pub fn shuffled(&self) -> bool {
        self.shuffle
    }

    /// Shuffle on or off. The current item stays current; when shuffled,
    /// the others follow it in a random order, each once.
    pub fn set_shuffle(&mut self, on: bool, seed: u64) {
        let current = self.current_index();
        self.shuffle = on;
        self.order = (0..self.items.len()).collect();
        if on {
            self.seed = seed | 1;
            if let Some(c) = current {
                self.order.retain(|&i| i != c);
                self.shuffle_order();
                self.order.insert(0, c);
            } else {
                self.shuffle_order();
            }
            self.pos = 0;
        } else {
            self.pos = current.unwrap_or(0);
        }
    }

    /// Make `items[index]` the current item.
    pub fn jump(&mut self, index: usize) -> Option<usize> {
        let pos = self.order.iter().position(|&i| i == index)?;
        self.pos = pos;
        Some(index)
    }

    /// The next item ("next" pressed): at the end, the first again unless
    /// repeat is off. A shuffled queue is shuffled anew for the next round.
    pub fn next(&mut self, repeat: Repeat) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        if self.pos + 1 < self.order.len() {
            self.pos += 1;
        } else if repeat == Repeat::Off {
            return None;
        } else {
            if self.shuffle {
                let last = self.current_index();
                self.shuffle_order();
                // Not the same item twice in a row.
                if self.order.len() > 1 && self.order.first().copied() == last {
                    self.order.swap(0, 1);
                }
            }
            self.pos = 0;
        }
        self.current_index()
    }

    /// The item before ("previous" pressed): at the start, the last one
    /// unless repeat is off.
    pub fn previous(&mut self, repeat: Repeat) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        if self.pos > 0 {
            self.pos -= 1;
        } else if repeat == Repeat::Off {
            return None;
        } else {
            self.pos = self.order.len() - 1;
        }
        self.current_index()
    }

    /// What plays when the current item has ended: the same one again
    /// (repeat one), else the next.
    pub fn after_end(&mut self, repeat: Repeat) -> Option<usize> {
        match repeat {
            Repeat::One => self.current_index(),
            _ => self.next(repeat),
        }
    }

    /// Fisher–Yates over `order` with a xorshift generator.
    fn shuffle_order(&mut self) {
        for i in (1..self.order.len()).rev() {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 7;
            self.seed ^= self.seed << 17;
            let j = (self.seed % (i as u64 + 1)) as usize;
            self.order.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(n: usize, start: usize) -> Queue {
        Queue::new(
            (0..n).map(|i| Item::new(format!("{i}.mp3"))).collect(),
            start,
        )
    }

    #[test]
    fn in_order_with_repeat_modes() {
        let mut q = queue(3, 1);
        assert_eq!(q.current_index(), Some(1));
        assert_eq!(q.next(Repeat::Off), Some(2));
        assert_eq!(q.next(Repeat::Off), None);
        assert_eq!(q.current_index(), Some(2));
        assert_eq!(q.next(Repeat::All), Some(0));
        assert_eq!(q.previous(Repeat::Off), None);
        assert_eq!(q.previous(Repeat::All), Some(2));
        // Repeat one plays the same item when it ends, but "next" moves on.
        assert_eq!(q.after_end(Repeat::One), Some(2));
        assert_eq!(q.next(Repeat::One), Some(0));
        assert_eq!(q.after_end(Repeat::Off), Some(1));
        assert_eq!(Queue::default().next(Repeat::All), None);
    }

    #[test]
    fn shuffle_plays_each_once_per_round() {
        let mut q = queue(20, 5);
        q.set_shuffle(true, 12345);
        assert_eq!(q.current_index(), Some(5));
        let mut seen = vec![5];
        while let Some(i) = q.next(Repeat::Off) {
            seen.push(i);
        }
        let mut sorted = seen.clone();
        sorted.sort();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
        assert_ne!(seen, (0..20).collect::<Vec<_>>());
        // The next round is a new order, not starting with the last item.
        let last = *seen.last().unwrap();
        let first = q.next(Repeat::All).unwrap();
        assert_ne!(first, last);
        let mut round = vec![first];
        while let Some(i) = q.next(Repeat::Off) {
            round.push(i);
        }
        round.sort();
        assert_eq!(round, (0..20).collect::<Vec<_>>());
        // Shuffle off: the current item stays, the order is the list's again.
        let cur = q.current_index().unwrap();
        q.set_shuffle(false, 0);
        assert_eq!(q.current_index(), Some(cur));
        assert_eq!(q.next(Repeat::All), Some((cur + 1) % 20));
    }

    #[test]
    fn jump_and_listing() {
        let paths: Vec<PathBuf> = ["a.mp3", "notes.txt", "b.mkv", "sub", "c.flac"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let mut q = Queue::from_listing(paths.iter().map(PathBuf::as_path), Path::new("b.mkv"));
        assert_eq!(q.len(), 3);
        assert_eq!(q.current().unwrap().path, Path::new("b.mkv"));
        assert_eq!(q.next(Repeat::Off), Some(2));
        assert_eq!(q.jump(0), Some(0));
        assert_eq!(q.current().unwrap().label(), "a.mp3");
        assert_eq!(q.jump(7), None);
        assert_eq!(q.position_of(Path::new("c.flac")), Some(2));
        // Not in the listing: from the first.
        let q = Queue::from_listing(paths.iter().map(PathBuf::as_path), Path::new("x.mp3"));
        assert_eq!(q.current_index(), Some(0));
    }

    #[test]
    fn playable_files_of_a_folder_by_name() {
        let dir = std::env::temp_dir().join(format!("ox-player-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub.mp3")).unwrap();
        for name in ["10 b.mp3", "2 a.MP4", "cover.jpg", "1 c.flac"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let names: Vec<String> = playable_in(&dir)
            .iter()
            .map(|p| crate::entry::display_name(p))
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["1 c.flac", "2 a.MP4", "10 b.mp3"]);
    }

    #[test]
    fn previous_restarts_after_a_few_seconds() {
        assert!(!previous_restarts(Duration::from_secs(2)));
        assert!(previous_restarts(Duration::from_secs(10)));
        assert_eq!(Repeat::Off.cycle(), Repeat::All);
        assert_eq!(Repeat::One.cycle(), Repeat::Off);
    }
}
