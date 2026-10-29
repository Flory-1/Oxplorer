//! Pictures for the preview, Quick Look and the grid's thumbnails: the
//! `image` crate's formats turned as the camera says, SVG drawings and
//! Windows' codecs (see `ox_core::picture`), and animated GIF/WebP. Also
//! the Info tab's button showing where a photo was taken.

use std::io::{BufRead, Cursor, Seek};
use std::path::Path;

use egui::{ColorImage, Ui};
use image::{DynamicImage, ImageDecoder, ImageReader};
use ox_core::details::Details;
use ox_core::i18n::t as tr;
use ox_core::picture::{self, Source};

use super::Loaded;

/// The `image` crate's formats.
pub(crate) fn decodable(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "jfif" | "gif" | "bmp" | "webp" | "ico" | "tif" | "tiff"
    )
}

/// Pictures shown in the preview (and as thumbnails when the Shell has none).
pub(crate) fn is_picture(ext: &str) -> bool {
    decodable(ext) || picture::own_decoder(ext)
}

/// `src`, an `ext` picture, with its long side at most `side`, and its
/// own size (both as it is shown).
pub(crate) fn still(src: Source, ext: &str, side: u32) -> Result<(ColorImage, [u32; 2]), String> {
    if picture::own_decoder(ext) {
        let b = picture::decode(src, ext, side)?;
        let size = [b.width as usize, b.height as usize];
        return Ok((ColorImage::from_rgba_unmultiplied(size, &b.rgba), b.full));
    }
    let img = match src {
        Source::File(path) => ImageReader::open(path)
            .map_err(image::ImageError::IoError)
            .and_then(|r| with_image(r, side)),
        Source::Bytes(bytes) => with_image(ImageReader::new(Cursor::new(bytes)), side),
    };
    img.map_err(|e| ox_core::trf!("Can't read this image: {e}", e = e))
}

/// Decode with the `image` crate, scaled down, then turned as its EXIF
/// orientation says (JPEG, TIFF, WebP, PNG).
fn with_image<R: BufRead + Seek>(
    reader: ImageReader<R>,
    side: u32,
) -> image::ImageResult<(ColorImage, [u32; 2])> {
    let mut decoder = reader.with_guessed_format()?.into_decoder()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let img = DynamicImage::from_decoder(decoder)?;
    let (w, h) = (img.width(), img.height());
    let mut img = if w.max(h) > side {
        img.thumbnail(side, side)
    } else {
        img
    };
    img.apply_orientation(orientation);
    let full = match picture::swaps_sides(orientation.to_exif().into()) {
        true => [h, w],
        false => [w, h],
    };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok((
        ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        full,
    ))
}

/// The preview of a picture: playing if it is an animated GIF/WebP.
pub(super) fn load(src: Source, ext: &str, side: u32) -> Loaded {
    if let Some(frames) = super::animation::decode(src, ext, side) {
        return frames;
    }
    match still(src, ext, side) {
        Ok((img, full)) => Loaded::Image(img, full),
        Err(e) => Loaded::Error(e),
    }
}

/// The picture's size without decoding it (as it is shown).
pub(super) fn dimensions(path: &Path, ext: &str) -> Option<[u32; 2]> {
    if picture::own_decoder(ext) {
        return picture::dimensions(path, ext);
    }
    let (w, h) = image::image_dimensions(path).ok()?;
    let turned = ox_core::exif_info::may_have_exif(ext)
        && ox_core::exif_info::orientation(path).is_some_and(picture::swaps_sides);
    Some(if turned { [h, w] } else { [w, h] })
}

/// Below the Info tab's rows: where the photo was taken, on a map. Nothing
/// is sent anywhere until the button is clicked.
pub(super) fn map_button(ui: &mut Ui, details: Option<&Details>) {
    let Some((lat, lon)) = details.and_then(|d| d.location) else {
        return;
    };
    ui.add_space(6.0);
    let button = ui
        .button(tr("🌐 Open in map"))
        .on_hover_text(tr("Shows the place on OpenStreetMap in the browser"));
    if button.clicked() {
        if let Err(e) = ox_core::exif_info::open_map(lat, lon) {
            log::warn!("opening the map failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_are_turned_as_the_camera_says() {
        // A 4 × 2 JPEG with EXIF orientation 6 (turn clockwise): 2 × 4.
        let mut jpeg = Vec::new();
        image::RgbImage::from_pixel(4, 2, image::Rgb([200, 10, 10]))
            .write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        // APP1 "Exif" with a big-endian TIFF holding one field: Orientation = 6.
        let tiff: &[u8] = &[
            b'M', b'M', 0, 42, 0, 0, 0, 8, // header, IFD at 8
            0, 1, // one entry
            0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, // Orientation SHORT 1 = 6
            0, 0, 0, 0, // no next IFD
        ];
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(tiff);
        let len = (app1.len() + 2) as u16;
        let mut with_exif = vec![0xFF, 0xD8, 0xFF, 0xE1];
        with_exif.extend(len.to_be_bytes());
        with_exif.extend(app1);
        with_exif.extend(&jpeg[2..]);
        let (img, full) = still(Source::Bytes(&with_exif), "jpg", 100).unwrap();
        assert_eq!(img.size, [2, 4]);
        assert_eq!(full, [2, 4]);
        // Scaled down: the long side fits.
        let (img, full) = still(Source::Bytes(&with_exif), "jpg", 2).unwrap();
        assert_eq!((img.size, full), ([1, 2], [2, 4]));
        // Without EXIF it stays as it is.
        let (img, _) = still(Source::Bytes(&jpeg), "jpg", 100).unwrap();
        assert_eq!(img.size, [4, 2]);
    }

    #[test]
    fn drawings_are_pictures() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5"/>"#;
        let (img, full) = still(Source::Bytes(svg), "svg", 200).unwrap();
        assert_eq!((img.size, full), ([200, 100], [10, 5]));
        assert!(is_picture("svg") && is_picture("png") && !is_picture("txt"));
    }
}
