//! Pictures the `image` crate can't read: SVG drawings (rendered with
//! `resvg`) and HEIC/HEIF, AVIF, JPEG XR/XL and camera RAW files (decoded
//! by the codecs installed in Windows). Also the helpers for turning a
//! picture as the camera says and for playing animations.

mod svg;
mod wic;

use std::path::Path;
use std::time::Duration;

/// A decoded picture: RGBA pixels (not premultiplied), at most the size
/// asked for, and the picture's own size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Width and height of the picture itself (turned as it is shown).
    pub full: [u32; 2],
}

/// Where a picture is read from.
#[derive(Clone, Copy)]
pub enum Source<'a> {
    File(&'a Path),
    /// The file's bytes (a picture inside an archive).
    Bytes(&'a [u8]),
}

/// SVG drawings (`svgz`: gzip-compressed).
pub fn is_svg(ext: &str) -> bool {
    matches!(ext, "svg" | "svgz")
}

/// Formats decoded by Windows' codecs (HEIF, AV1, Raw Image Extension …).
pub fn is_wic(ext: &str) -> bool {
    matches!(
        ext,
        "heic"
            | "heif"
            | "hif"
            | "avif"
            | "jxr"
            | "wdp"
            | "hdp"
            | "jxl"
            | "dng"
            | "cr2"
            | "cr3"
            | "crw"
            | "nef"
            | "nrw"
            | "arw"
            | "srf"
            | "sr2"
            | "orf"
            | "rw2"
            | "raf"
            | "pef"
            | "srw"
            | "3fr"
            | "erf"
            | "kdc"
            | "mrw"
            | "mos"
            | "rwl"
            | "iiq"
            | "x3f"
    )
}

/// Pictures read here rather than with the `image` crate: SVGs anywhere,
/// the Windows codecs' formats on Windows.
pub fn own_decoder(ext: &str) -> bool {
    is_svg(ext) || (wic::supported() && is_wic(ext))
}

/// Decode `src` (an `ext` file) with its long side at most `max_side`.
/// The error is meant for the user.
pub fn decode(src: Source, ext: &str, max_side: u32) -> Result<Bitmap, String> {
    if is_svg(ext) {
        return svg::render(src, max_side);
    }
    let mut bitmap = wic::decode(src, ext, max_side)?;
    // Codecs don't turn the picture: the turn the codec reports (HEIF's),
    // else the EXIF one (RAW files; tried with the Raw Image Extension).
    let orientation = bitmap.1.or_else(|| match src {
        Source::File(path) => crate::exif_info::orientation(path),
        Source::Bytes(bytes) => crate::exif_info::orientation_in(bytes),
    });
    if let Some(o) = orientation {
        orient(&mut bitmap.0, o);
    }
    Ok(bitmap.0)
}

/// The size of the picture in `path` without decoding it (as it is shown).
pub fn dimensions(path: &Path, ext: &str) -> Option<[u32; 2]> {
    if is_svg(ext) {
        return svg::size(Source::File(path));
    }
    wic::dimensions(path)
}

/// Whether the shown picture is turned on its side by EXIF orientation `o`.
pub fn swaps_sides(o: u16) -> bool {
    (5..=8).contains(&o)
}

/// Turn and mirror `b` as EXIF orientation `o` (1–8) says: 2 mirrored,
/// 3 upside down, 4 flipped, 5 transposed, 6 turned clockwise, 7
/// transversed, 8 turned counter-clockwise.
pub fn orient(b: &mut Bitmap, o: u16) {
    if !(2..=8).contains(&o) {
        return;
    }
    let (w, h) = (b.width as usize, b.height as usize);
    if b.rgba.len() != w * h * 4 {
        return;
    }
    let (out_w, out_h) = if swaps_sides(o) { (h, w) } else { (w, h) };
    let mut out = vec![0u8; b.rgba.len()];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = match o {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                _ => (y, w - 1 - x),
            };
            let from = (y * w + x) * 4;
            let to = (dy * out_w + dx) * 4;
            out[to..to + 4].copy_from_slice(&b.rgba[from..from + 4]);
        }
    }
    b.rgba = out;
    b.width = out_w as u32;
    b.height = out_h as u32;
    if swaps_sides(o) {
        b.full = [b.full[1], b.full[0]];
    }
}

// ---- animations ------------------------------------------------------------

/// How long a GIF/WebP frame stays. Like browsers, very short delays
/// (often 0, written by old tools) mean a tenth of a second.
pub fn frame_delay(ms: u32) -> Duration {
    Duration::from_millis(if ms < 20 { 100 } else { ms.into() })
}

/// The frame shown at `pos` into an animation of frames lasting `delays`
/// (it loops), and how long until the next one.
pub fn frame_at(delays: &[Duration], pos: Duration) -> (usize, Duration) {
    let total: Duration = delays.iter().sum();
    if total.is_zero() {
        return (0, Duration::from_millis(100));
    }
    let mut t = Duration::from_nanos((pos.as_nanos() % total.as_nanos()) as u64);
    for (i, d) in delays.iter().enumerate() {
        if t < *d {
            return (i, *d - t);
        }
        t -= *d;
    }
    (0, delays[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 × 1 picture: a red and a green pixel.
    fn two() -> Bitmap {
        Bitmap {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
            full: [2, 1],
        }
    }

    #[test]
    fn orientation_turns_and_mirrors() {
        const RED: [u8; 4] = [255, 0, 0, 255];
        const GREEN: [u8; 4] = [0, 255, 0, 255];
        let pixels = |b: &Bitmap| -> Vec<[u8; 4]> {
            b.rgba.chunks(4).map(|c| c.try_into().unwrap()).collect()
        };
        for (o, size, expect) in [
            (1, (2, 1), [RED, GREEN]),
            (2, (2, 1), [GREEN, RED]),
            (3, (2, 1), [GREEN, RED]),
            (4, (2, 1), [RED, GREEN]),
            // Turned on the side: one column, top to bottom.
            (5, (1, 2), [RED, GREEN]),
            (6, (1, 2), [RED, GREEN]),
            (7, (1, 2), [GREEN, RED]),
            (8, (1, 2), [GREEN, RED]),
        ] {
            let mut b = two();
            orient(&mut b, o);
            assert_eq!((b.width, b.height), size, "orientation {o}");
            assert_eq!(pixels(&b), expect, "orientation {o}");
        }
        // A 2 × 2 picture turned clockwise: the top left goes to the top right.
        let mut b = Bitmap {
            width: 2,
            height: 2,
            rgba: (0..4u8).flat_map(|i| [i, 0, 0, 255]).collect(),
            full: [2, 2],
        };
        orient(&mut b, 6);
        let reds: Vec<u8> = b.rgba.chunks(4).map(|c| c[0]).collect();
        assert_eq!(reds, [2, 0, 3, 1]);
        assert!(swaps_sides(6) && swaps_sides(8) && !swaps_sides(3) && !swaps_sides(1));
    }

    #[test]
    fn formats_are_routed() {
        assert!(is_svg("svg") && is_svg("svgz") && !is_svg("png"));
        assert!(is_wic("heic") && is_wic("cr3") && is_wic("dng") && !is_wic("jpg"));
        assert!(own_decoder("svg"));
        assert!(!own_decoder("png"));
        assert_eq!(own_decoder("heic"), cfg!(windows));
    }

    #[test]
    fn frames_follow_their_delays() {
        let ms = Duration::from_millis;
        assert_eq!(frame_delay(0), ms(100));
        assert_eq!(frame_delay(10), ms(100));
        assert_eq!(frame_delay(40), ms(40));
        let delays = [ms(100), ms(50), ms(200)];
        assert_eq!(frame_at(&delays, ms(0)), (0, ms(100)));
        assert_eq!(frame_at(&delays, ms(120)), (1, ms(30)));
        assert_eq!(frame_at(&delays, ms(150)), (2, ms(200)));
        // It loops.
        assert_eq!(frame_at(&delays, ms(360)), (0, ms(90)));
        assert_eq!(frame_at(&[], ms(5)).0, 0);
    }
}
