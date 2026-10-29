//! Picture tools: rotate, flip, convert and resize pictures, one or many.
//!
//! [`start`] runs a [`Task`] on a few threads (see [`job`]). Every result is
//! encoded in memory first and only then written:
//!
//! - New files (converted or resized copies) are created next to the
//!   originals or in a folder, under a name that isn't taken yet.
//! - Files that replace their originals (rotating, flipping, "replace the
//!   originals") are written under a temporary name in the same folder;
//!   then the original goes to the Recycle Bin and the new file is renamed
//!   into its place, with the original's created date. Undo
//!   ([`crate::undo::Op::Replaced`]) brings the original back.
//!
//! Rotating or flipping a JPEG is lossless: only its EXIF orientation is
//! rewritten ([`exif_turn`]); the pixels and every other byte stay as they
//! are, and viewers (Oxplorer's preview and thumbnails too) turn the
//! picture as the tag says. With [`JpegTurn::Reencode`], and when the EXIF
//! data is too odd to rewrite, the JPEG is re-encoded instead.
//!
//! Re-encoding decodes pictures upright: the EXIF orientation is applied to
//! the pixels and reset to "normal" in the EXIF data that is written back.
//! So a re-encoded JPEG (at quality [`ROTATE_QUALITY`]) keeps its EXIF data
//! (date taken, camera, …) and colour profile, and looks right in every
//! viewer, also in ones that ignore the orientation tag. PNG, WebP, BMP,
//! GIF and TIFF are re-encoded losslessly (WebP always lossless: the
//! encoder can't do lossy). GIFs give their first frame; animated pictures
//! aren't rotated in place.

mod codec;
mod exif_turn;
mod job;

use std::path::Path;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};

use crate::fsops;
use crate::i18n::t;

pub use job::{start, Job, Summary};

/// JPEG quality for rotated and flipped JPEGs.
pub const ROTATE_QUALITY: u8 = 95;

/// Extensions of the pictures the tools work on.
pub const EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "webp", "bmp", "gif", "tif", "tiff",
];

/// Whether the picture tools work on `path` (by its extension).
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Turning or mirroring a picture in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    RotateRight,
    RotateLeft,
    Rotate180,
    FlipHorizontal,
    FlipVertical,
}

impl Transform {
    pub const ALL: [Self; 5] = [
        Self::RotateRight,
        Self::RotateLeft,
        Self::Rotate180,
        Self::FlipHorizontal,
        Self::FlipVertical,
    ];

    pub fn label(self) -> &'static str {
        t(match self {
            Self::RotateRight => "Rotate right",
            Self::RotateLeft => "Rotate left",
            Self::Rotate180 => "Rotate 180°",
            Self::FlipHorizontal => "Flip horizontally",
            Self::FlipVertical => "Flip vertically",
        })
    }

    pub fn is_flip(self) -> bool {
        matches!(self, Self::FlipHorizontal | Self::FlipVertical)
    }

    fn apply(self, img: &DynamicImage) -> DynamicImage {
        match self {
            Self::RotateRight => img.rotate90(),
            Self::RotateLeft => img.rotate270(),
            Self::Rotate180 => img.rotate180(),
            Self::FlipHorizontal => img.fliph(),
            Self::FlipVertical => img.flipv(),
        }
    }
}

/// How rotating and flipping treats JPEGs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegTurn {
    /// Only the EXIF orientation is changed (re-encoded if that can't be
    /// done).
    Lossless,
    /// Re-encoded upright, for programs that ignore the orientation.
    Reencode,
}

/// The format converted pictures are saved in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// The format each picture has.
    Original,
    Png,
    Jpeg,
    WebP,
}

impl OutputFormat {
    pub const ALL: [Self; 4] = [Self::Original, Self::Png, Self::Jpeg, Self::WebP];

    pub fn label(self) -> &'static str {
        match self {
            Self::Original => t("Same as the original"),
            Self::Png => "PNG",
            Self::Jpeg => "JPG",
            Self::WebP => "WebP",
        }
    }

    fn image_format(self) -> Option<ImageFormat> {
        match self {
            Self::Original => None,
            Self::Png => Some(ImageFormat::Png),
            Self::Jpeg => Some(ImageFormat::Jpeg),
            Self::WebP => Some(ImageFormat::WebP),
        }
    }
}

/// How converted pictures are resized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resize {
    /// To this many percent of the width and height.
    Percent(u32),
    /// To fit within a box; with `keep_aspect` off, to exactly its size.
    Fit {
        width: u32,
        height: u32,
        keep_aspect: bool,
    },
    /// So the longer side has this many pixels.
    LongestSide(u32),
}

impl Resize {
    /// Whether the result has the same proportions as the original.
    fn keeps_aspect(self) -> bool {
        !matches!(
            self,
            Self::Fit {
                keep_aspect: false,
                ..
            }
        )
    }

    /// What goes in brackets after the name of a picture resized to `w`×`h`:
    /// "50%", "800px" or "800x600".
    fn suffix(self, w: u32, h: u32) -> String {
        match self {
            Self::Percent(p) => format!("{p}%"),
            Self::LongestSide(_) => format!("{}px", w.max(h)),
            Self::Fit { .. } => format!("{w}x{h}"),
        }
    }
}

/// The size a `w`×`h` picture is resized to. With `enlarge` off, pictures
/// never get bigger: those that would keep their size (or, when stretched
/// to a box, get no wider or taller than they are).
pub fn target_size(w: u32, h: u32, resize: Resize, enlarge: bool) -> (u32, u32) {
    let scaled = |s: f64| {
        (
            ((w as f64 * s).round() as u32).max(1),
            ((h as f64 * s).round() as u32).max(1),
        )
    };
    let (nw, nh) = match resize {
        Resize::Percent(p) => scaled(p as f64 / 100.0),
        Resize::Fit {
            width,
            height,
            keep_aspect: true,
        } => scaled((width as f64 / w as f64).min(height as f64 / h as f64)),
        Resize::Fit { width, height, .. } => (width.max(1), height.max(1)),
        Resize::LongestSide(n) => scaled(n as f64 / w.max(h) as f64),
    };
    if enlarge {
        (nw, nh)
    } else if resize.keeps_aspect() {
        if nw > w || nh > h {
            (w, h)
        } else {
            (nw, nh)
        }
    } else {
        (nw.min(w), nh.min(h))
    }
}

/// Where converted pictures go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Next to the originals, e.g. `photo (800px).jpg`.
    Beside,
    /// Into this folder (made if it doesn't exist).
    Folder(std::path::PathBuf),
    /// In place of the originals (which go to the Recycle Bin).
    Replace,
}

/// Convert and/or resize pictures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Convert {
    pub format: OutputFormat,
    /// JPEG quality, 1–100.
    pub quality: u8,
    pub resize: Option<Resize>,
    /// Let pictures get bigger than they are.
    pub enlarge: bool,
    pub destination: Destination,
}

impl Default for Convert {
    fn default() -> Self {
        Self {
            format: OutputFormat::Jpeg,
            quality: 85,
            resize: None,
            enlarge: false,
            destination: Destination::Beside,
        }
    }
}

/// What to do with each picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    Transform(Transform, JpegTurn),
    Convert(Convert),
}

/// The file name of the result for the picture `name`, in `format`: with
/// the format's extension (the picture's own if that is one of the
/// format's), and `suffix` in brackets after the name.
pub fn output_name(name: &str, format: ImageFormat, suffix: Option<&str>) -> String {
    let (stem, ext) = fsops::split_name(name);
    let own = ext.trim_start_matches('.');
    let ext = if ImageFormat::from_extension(own) == Some(format) {
        own
    } else {
        format.extensions_str().first().copied().unwrap_or(own)
    };
    match suffix {
        Some(s) => format!("{stem} ({s}).{ext}"),
        None => format!("{stem}.{ext}"),
    }
}

/// What converting a picture gives, worked out without decoding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// Its size, as it is seen (with the EXIF orientation applied).
    pub size: (u32, u32),
    pub new_size: (u32, u32),
    /// The name of the result (if that's free).
    pub new_name: String,
}

pub fn preview(path: &Path, c: &Convert) -> Option<Preview> {
    let (format, (w, h)) = codec::header(path)?;
    let format = c.format.image_format().unwrap_or(format);
    let (nw, nh) = match c.resize {
        Some(r) => target_size(w, h, r, c.enlarge),
        None => (w, h),
    };
    let suffix = c
        .resize
        .filter(|_| (nw, nh) != (w, h) && c.destination != Destination::Replace)
        .map(|r| r.suffix(nw, nh));
    let name = path.file_name()?.to_string_lossy();
    Some(Preview {
        size: (w, h),
        new_size: (nw, nh),
        new_name: output_name(&name, format, suffix.as_deref()),
    })
}

/// The picture at `path`, converted: its bytes and the name for them.
fn convert(path: &Path, c: &Convert) -> Result<(Vec<u8>, String, String), String> {
    let pic = codec::read(path)?;
    let format = c.format.image_format().unwrap_or(pic.format);
    let mut img = pic.img;
    let mut suffix = None;
    if let Some(resize) = c.resize {
        let (w, h) = (img.width(), img.height());
        let (nw, nh) = target_size(w, h, resize, c.enlarge);
        if (nw, nh) != (w, h) {
            img = img.resize_exact(nw, nh, FilterType::Lanczos3);
            suffix = Some(resize.suffix(nw, nh));
        }
    }
    let bytes = codec::encode(
        &img,
        format,
        c.quality,
        pic.exif.as_deref(),
        pic.icc.as_deref(),
    )?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let plain = output_name(&name, format, None);
    let with_suffix = output_name(&name, format, suffix.as_deref());
    Ok((bytes, plain, with_suffix))
}

/// The picture at `path`, turned: its bytes in its own format.
fn transform(path: &Path, tf: Transform, jpeg: JpegTurn) -> Result<Vec<u8>, String> {
    let is_jpeg = || codec::header(path).is_some_and(|(f, _)| f == ImageFormat::Jpeg);
    if jpeg == JpegTurn::Lossless && is_jpeg() {
        if let Some(bytes) = std::fs::read(path).ok().and_then(|b| lossless(&b, tf)) {
            return Ok(bytes);
        }
    }
    let pic = codec::read(path)?;
    if codec::is_animated(path, pic.format) {
        return Err(t("Animated pictures can't be rotated or flipped.").to_owned());
    }
    let img = tf.apply(&pic.img);
    codec::encode(
        &img,
        pic.format,
        ROTATE_QUALITY,
        pic.exif.as_deref(),
        pic.icc.as_deref(),
    )
}

/// The JPEG `bytes` turned by rewriting its EXIF orientation, checked by
/// reading the orientation back the way the decoder does.
fn lossless(bytes: &[u8], tf: Transform) -> Option<Vec<u8>> {
    if !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return None;
    }
    let (out, o) = exif_turn::turn_jpeg(bytes, tf)?;
    let read = image::metadata::Orientation::from_exif_chunk(exif_turn::exif_chunk(&out)?)?;
    (u16::from(read.to_exif()) == o).then_some(out)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod turn_tests;
