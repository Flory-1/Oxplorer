//! Reading pictures upright and encoding them with their EXIF data and
//! colour profile.

use std::borrow::Cow;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::Path;

use image::codecs::gif::GifDecoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{PngDecoder, PngEncoder};
use image::codecs::webp::{WebPDecoder, WebPEncoder};
use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader};

use crate::i18n::t;

/// A decoded picture, turned the way it is meant to be seen.
pub(super) struct Picture {
    pub img: DynamicImage,
    /// The format of the file's content (whatever its extension says).
    pub format: ImageFormat,
    /// The EXIF data (without the "Exif" header), orientation reset to normal.
    pub exif: Option<Vec<u8>>,
    pub icc: Option<Vec<u8>>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(super) fn read(path: &Path) -> Result<Picture, String> {
    let reader = ImageReader::open(path)
        .map_err(err)?
        .with_guessed_format()
        .map_err(err)?;
    let format = reader
        .format()
        .ok_or_else(|| t("This isn't a picture that can be read.").to_owned())?;
    let mut decoder = reader.into_decoder().map_err(err)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut exif = decoder.exif_metadata().ok().flatten();
    let icc = decoder.icc_profile().ok().flatten();
    let mut img = DynamicImage::from_decoder(decoder).map_err(err)?;
    img.apply_orientation(orientation);
    // The pixels are upright now; viewers mustn't turn them again.
    if let Some(e) = exif.as_mut() {
        let _ = Orientation::remove_from_exif_chunk(e);
    }
    Ok(Picture {
        img,
        format,
        exif: exif.filter(|e| !e.is_empty()),
        icc: icc.filter(|i| !i.is_empty()),
    })
}

/// The format of the picture and its size as it is seen, from its header.
pub(super) fn header(path: &Path) -> Option<(ImageFormat, (u32, u32))> {
    let reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let format = reader.format()?;
    let mut decoder = reader.into_decoder().ok()?;
    let (w, h) = decoder.dimensions();
    let turned = matches!(
        decoder.orientation().unwrap_or(Orientation::NoTransforms),
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    Some((format, if turned { (h, w) } else { (w, h) }))
}

/// Whether the picture has more than one frame (GIF, WebP, APNG).
pub(super) fn is_animated(path: &Path, format: ImageFormat) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let r = BufReader::new(file);
    match format {
        ImageFormat::Gif => GifDecoder::new(r).is_ok_and(|d| d.into_frames().take(2).count() > 1),
        ImageFormat::WebP => WebPDecoder::new(r).is_ok_and(|d| d.has_animation()),
        ImageFormat::Png => PngDecoder::new(r)
            .and_then(|d| d.is_apng())
            .unwrap_or(false),
        _ => false,
    }
}

/// `img` as a file in `format`. JPEGs get `quality`; PNG and WebP are
/// lossless. EXIF data and the colour profile go along where the format
/// has room for them (JPEG, PNG, WebP).
pub(super) fn encode(
    img: &DynamicImage,
    format: ImageFormat,
    quality: u8,
    exif: Option<&[u8]>,
    icc: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    match format {
        ImageFormat::Jpeg => {
            let mut e = JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100));
            add_metadata(&mut e, exif, icc);
            opaque(img).write_with_encoder(e)
        }
        ImageFormat::Png => {
            let mut e = PngEncoder::new(&mut out);
            add_metadata(&mut e, exif, icc);
            img.write_with_encoder(e)
        }
        ImageFormat::WebP => {
            let mut e = WebPEncoder::new_lossless(&mut out);
            add_metadata(&mut e, exif, icc);
            img.write_with_encoder(e)
        }
        other => img.write_to(Cursor::new(&mut out), other),
    }
    .map_err(err)?;
    Ok(out)
}

fn add_metadata(e: &mut impl ImageEncoder, exif: Option<&[u8]>, icc: Option<&[u8]>) {
    if let Some(x) = exif {
        let _ = e.set_exif_metadata(x.to_vec());
    }
    if let Some(p) = icc {
        let _ = e.set_icc_profile(p.to_vec());
    }
}

/// `img` on a white background, if it has transparency (JPEG has none:
/// the encoder would just drop it, which turns transparent areas black).
fn opaque(img: &DynamicImage) -> Cow<'_, DynamicImage> {
    if !img.color().has_alpha() {
        return Cow::Borrowed(img);
    }
    let rgba = img.to_rgba8();
    let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let a = a as u32;
        let over = |c: u8| ((c as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        image::Rgb([over(r), over(g), over(b)])
    });
    Cow::Owned(DynamicImage::ImageRgb8(rgb))
}
