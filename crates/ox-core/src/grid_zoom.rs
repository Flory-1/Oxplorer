//! Tile sizes of the grid view: Ctrl+mouse wheel steps through [`STEPS`],
//! the size slider sets any size in between. The size is the side of the
//! icon (or thumbnail) box in points; the tile around it follows.

/// Smallest and largest icon box (points).
pub const MIN: u32 = 48;
pub const MAX: u32 = 256;
/// The size before zooming existed: 48 px icons in 108 px wide tiles.
pub const DEFAULT: u32 = MIN;

/// Where one wheel notch goes, from small icons to large thumbnails.
pub const STEPS: [u32; 8] = [48, 64, 80, 96, 128, 160, 208, 256];

/// Wheel scrolling in points (touchpads) that counts as one notch.
const POINTS_PER_STEP: f32 = 50.0;

/// What a zoom step leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    /// The grid with this icon size.
    Grid(u32),
    /// Zoomed out past the smallest icons: the details view.
    Details,
}

pub fn clamp(size: u32) -> u32 {
    size.clamp(MIN, MAX)
}

/// One step bigger (`up`) or smaller from `size`. Sizes between steps (from
/// the slider) go to the next step in that direction; zooming out of the
/// smallest size switches to the details view.
pub fn step(size: u32, up: bool) -> Zoom {
    let size = clamp(size);
    if up {
        Zoom::Grid(STEPS.iter().copied().find(|&s| s > size).unwrap_or(MAX))
    } else if size <= MIN {
        Zoom::Details
    } else {
        Zoom::Grid(
            STEPS
                .iter()
                .rev()
                .copied()
                .find(|&s| s < size)
                .unwrap_or(MIN),
        )
    }
}

/// Zooming in from the details view: the grid with the smallest icons
/// (the step before it was [`Zoom::Details`]).
pub fn from_details() -> Zoom {
    Zoom::Grid(MIN)
}

/// Turns wheel movement into whole steps, so one notch is one step however
/// the wheel delivers it (lines from a mouse, points from a touchpad).
#[derive(Debug, Default, Clone, Copy)]
pub struct WheelSteps {
    /// Notches not yet turned into a step (−1 < rest < 1).
    rest: f32,
}

impl WheelSteps {
    /// Wheel movement in lines (one per notch); positive is up (zoom in).
    /// Returns the steps to take, positive for bigger.
    pub fn lines(&mut self, lines: f32) -> i32 {
        if lines == 0.0 || !lines.is_finite() {
            return 0;
        }
        if self.rest != 0.0 && self.rest.signum() != lines.signum() {
            self.rest = 0.0; // turned the other way: start over
        }
        self.rest += lines;
        let steps = self.rest.trunc();
        self.rest -= steps;
        steps as i32
    }

    /// Wheel movement in points (touchpads); positive is up.
    pub fn points(&mut self, points: f32) -> i32 {
        self.lines(points / POINTS_PER_STEP)
    }
}

/// Layout of one grid tile for an icon size, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub width: f32,
    pub height: f32,
    /// Side of the icon box.
    pub icon: f32,
    /// Space above the icon box.
    pub top: f32,
    /// Font size of the name below it.
    pub label: f32,
}

impl Tile {
    pub fn new(size: u32) -> Self {
        let icon = clamp(size) as f32;
        // Larger sizes get a slightly larger name; small ones keep 12.5.
        let label = 12.5 + 1.0 * ((icon - 96.0) / (MAX as f32 - 96.0)).clamp(0.0, 1.0);
        Tile {
            // Wide enough for two short lines of name even with small icons.
            width: (icon + 36.0).max(108.0),
            height: icon + 56.0,
            icon,
            top: 8.0,
            label,
        }
    }

    /// Top of the name, below the icon box.
    pub fn label_top(&self) -> f32 {
        self.top + self.icon + 6.0
    }

    /// Size of the box a thumbnail is fitted into (a bit wider than the icon).
    pub fn thumb_area(&self) -> (f32, f32) {
        (self.width - 16.0, self.icon + 4.0)
    }
}

/// Thumbnail resolutions that are loaded (long side in pixels). A tile asks
/// for the smallest one that is sharp enough, so the cache holds a few
/// sizes instead of one per wheel tick.
pub const THUMB_BUCKETS: [u32; 3] = [128, 256, 512];

/// The thumbnail resolution for a box whose long side is `pixels` on screen.
/// A bucket serves up to 1.25× its size (thumbnails may be scaled up a little).
pub fn thumb_bucket(pixels: f32) -> u32 {
    THUMB_BUCKETS
        .iter()
        .copied()
        .find(|&b| b as f32 * 1.25 >= pixels)
        .unwrap_or(THUMB_BUCKETS[THUMB_BUCKETS.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_go_up_and_down() {
        assert_eq!(step(48, true), Zoom::Grid(64));
        assert_eq!(step(64, false), Zoom::Grid(48));
        assert_eq!(step(256, true), Zoom::Grid(256));
        assert_eq!(step(208, true), Zoom::Grid(256));
        // In between (from the slider): the next step that way.
        assert_eq!(step(100, true), Zoom::Grid(128));
        assert_eq!(step(100, false), Zoom::Grid(96));
        // Out of range values are clamped first.
        assert_eq!(step(10, true), Zoom::Grid(64));
        assert_eq!(step(999, false), Zoom::Grid(208));
    }

    #[test]
    fn zooming_out_of_the_smallest_size_shows_details() {
        assert_eq!(step(48, false), Zoom::Details);
        assert_eq!(step(49, false), Zoom::Grid(48));
        assert_eq!(from_details(), Zoom::Grid(MIN));
    }

    #[test]
    fn one_notch_is_one_step() {
        let mut w = WheelSteps::default();
        assert_eq!(w.lines(1.0), 1);
        assert_eq!(w.lines(-2.0), -2);
        assert_eq!(w.lines(0.0), 0);
        // Touchpads: points add up to notches.
        assert_eq!(w.points(20.0), 0);
        assert_eq!(w.points(20.0), 0);
        assert_eq!(w.points(20.0), 1);
        // Changing direction drops what was left over.
        assert_eq!(w.points(20.0), 0);
        assert_eq!(w.points(-20.0), 0);
        assert_eq!(w.points(-40.0), -1);
        assert_eq!(w.lines(f32::NAN), 0);
    }

    #[test]
    fn tiles_grow_with_the_icon() {
        let small = Tile::new(DEFAULT);
        // The old fixed tile.
        assert_eq!((small.width, small.height), (108.0, 104.0));
        assert_eq!(small.label_top(), 62.0);
        assert_eq!(small.label, 12.5);
        let big = Tile::new(MAX);
        assert_eq!(big.icon, 256.0);
        assert!(big.width > big.icon && big.height > big.icon);
        assert_eq!(big.label, 13.5);
        assert_eq!(Tile::new(1000), big);
        let (w, h) = big.thumb_area();
        assert!(w > h * 0.9 && h > big.icon);
    }

    #[test]
    fn thumbnails_come_in_a_few_sizes() {
        assert_eq!(thumb_bucket(92.0), 128);
        assert_eq!(thumb_bucket(160.0), 128);
        assert_eq!(thumb_bucket(184.0), 256);
        assert_eq!(thumb_bucket(552.0), 512);
        assert_eq!(thumb_bucket(5000.0), 512);
    }
}
