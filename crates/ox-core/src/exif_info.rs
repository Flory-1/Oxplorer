//! A photo's EXIF details for the Info tab: camera, lens, exposure, flash,
//! when and where it was taken, and how it is turned.

use std::path::Path;

use exif::{In, Tag, Value};

use crate::i18n::t as tr;

/// What a photo's EXIF data says (what it has of it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoInfo {
    /// Maker and model ("Canon EOS R6").
    pub camera: Option<String>,
    pub lens: Option<String>,
    /// Exposure time in seconds, as a fraction.
    pub exposure: Option<(u32, u32)>,
    pub f_number: Option<f64>,
    pub iso: Option<u32>,
    /// Focal length in mm, and what it equals on 35 mm film.
    pub focal_length: Option<f64>,
    pub focal_35mm: Option<u32>,
    /// The EXIF flash bits.
    pub flash: Option<u32>,
    /// Local time on the camera's clock (cameras don't write the zone).
    pub taken: Option<chrono::NaiveDateTime>,
    /// Latitude and longitude in degrees (south and west negative).
    pub location: Option<(f64, f64)>,
    /// Metres above sea level (below: negative).
    pub altitude: Option<f64>,
    /// EXIF orientation, 1–8.
    pub orientation: Option<u16>,
}

/// Files whose EXIF data can be read: JPEG, TIFF, PNG, WebP, HEIF/AVIF
/// and the RAW formats built on TIFF.
pub fn may_have_exif(ext: &str) -> bool {
    matches!(
        ext,
        "jpg" | "jpeg" | "jfif" | "tif" | "tiff" | "png" | "webp"
    ) || crate::picture::is_wic(ext)
}

/// The EXIF data of `path`, if it has any.
pub fn read(path: &Path) -> Option<PhotoInfo> {
    let file = std::fs::File::open(path).ok()?;
    let reader = exif::Reader::new();
    let exif = match reader.read_from_container(&mut std::io::BufReader::new(file)) {
        Ok(exif) => exif,
        // Some fields broken: the others are still good.
        Err(exif::Error::PartialResult(partial)) => partial.into_inner().0,
        Err(_) => return None,
    };
    Some(from_exif(&exif))
}

/// The EXIF orientation of `path` (1–8), if it says.
pub fn orientation(path: &Path) -> Option<u16> {
    read(path)?.orientation
}

/// The EXIF orientation of a picture file's bytes.
pub fn orientation_in(bytes: &[u8]) -> Option<u16> {
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(bytes))
        .ok()?;
    from_exif(&exif).orientation
}

/// The details in `exif`.
pub fn from_exif(exif: &exif::Exif) -> PhotoInfo {
    let field = |tag: Tag| exif.get_field(tag, In::PRIMARY).map(|f| &f.value);
    let text = |tag: Tag| match field(tag)? {
        Value::Ascii(parts) => {
            let s: String = parts
                .iter()
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
            (!s.is_empty() && !s.eq_ignore_ascii_case("unknown")).then(|| s.to_owned())
        }
        _ => None,
    };
    let rational = |tag: Tag| match field(tag)? {
        Value::Rational(r) => r.first().filter(|r| r.denom != 0).copied(),
        _ => None,
    };
    let uint = |tag: Tag| field(tag)?.get_uint(0);
    let camera = camera_name(
        &text(Tag::Make).unwrap_or_default(),
        &text(Tag::Model).unwrap_or_default(),
    );
    let coordinate = |value: Tag, reference: Tag| match field(value)? {
        Value::Rational(parts) if parts.iter().all(|r| r.denom != 0) => {
            let dms: Vec<f64> = parts.iter().map(|r| r.to_f64()).collect();
            Some(degrees(&dms, &text(reference).unwrap_or_default()))
        }
        _ => None,
    };
    let location = match (
        coordinate(Tag::GPSLatitude, Tag::GPSLatitudeRef),
        coordinate(Tag::GPSLongitude, Tag::GPSLongitudeRef),
    ) {
        // 0, 0 is what phones write without a fix.
        (Some(lat), Some(lon)) if lat != 0.0 || lon != 0.0 => Some((lat, lon)),
        _ => None,
    };
    let below_sea = uint(Tag::GPSAltitudeRef) == Some(1);
    PhotoInfo {
        camera,
        lens: text(Tag::LensModel),
        exposure: rational(Tag::ExposureTime)
            .filter(|r| r.num > 0)
            .map(|r| (r.num, r.denom)),
        f_number: rational(Tag::FNumber)
            .map(|r| r.to_f64())
            .filter(|f| *f > 0.0),
        iso: uint(Tag::PhotographicSensitivity).filter(|i| *i > 0),
        focal_length: rational(Tag::FocalLength)
            .map(|r| r.to_f64())
            .filter(|f| *f > 0.0),
        focal_35mm: uint(Tag::FocalLengthIn35mmFilm).filter(|f| *f > 0),
        flash: uint(Tag::Flash),
        taken: date_time(field(Tag::DateTimeOriginal)),
        location,
        altitude: rational(Tag::GPSAltitude).map(|r| {
            let m = r.to_f64();
            if below_sea {
                -m
            } else {
                m
            }
        }),
        orientation: uint(Tag::Orientation)
            .and_then(|o| u16::try_from(o).ok())
            .filter(|o| (1..=8).contains(o)),
    }
}

fn date_time(value: Option<&Value>) -> Option<chrono::NaiveDateTime> {
    let Some(Value::Ascii(parts)) = value else {
        return None;
    };
    let t = exif::DateTime::from_ascii(parts.first()?).ok()?;
    chrono::NaiveDate::from_ymd_opt(t.year.into(), t.month.into(), t.day.into())?.and_hms_opt(
        t.hour.into(),
        t.minute.into(),
        t.second.into(),
    )
}

impl PhotoInfo {
    /// The rows for the Info tab and the Properties dialog.
    pub fn facts(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if let Some(camera) = &self.camera {
            out.push((tr("Camera"), camera.clone()));
        }
        if let Some(lens) = &self.lens {
            out.push((tr("Lens"), lens.clone()));
        }
        if let Some((num, den)) = self.exposure {
            out.push((tr("Exposure time"), exposure_text(num, den)));
        }
        if let Some(f) = self.f_number {
            out.push((tr("Aperture"), f_number_text(f)));
        }
        if let Some(iso) = self.iso {
            out.push((tr("ISO"), iso.to_string()));
        }
        if let Some(mm) = self.focal_length {
            out.push((tr("Focal length"), focal_text(mm, self.focal_35mm)));
        }
        if let Some(flash) = self.flash {
            out.push((tr("Flash"), flash_text(flash).to_owned()));
        }
        if let Some((lat, lon)) = self.location {
            out.push((tr("Location taken"), coordinates_text(lat, lon)));
        }
        if let Some(m) = self.altitude {
            out.push((tr("Altitude"), format!("{} m", trim_number(m, 0))));
        }
        out
    }
}

/// Most cameras repeat the maker in the model ("Canon" "Canon EOS R6").
fn camera_name(make: &str, model: &str) -> Option<String> {
    // "NIKON CORPORATION" "NIKON Z 6" too.
    let maker = make.split_whitespace().next().unwrap_or_default();
    let name = if model.to_lowercase().starts_with(&maker.to_lowercase()) {
        model.to_owned()
    } else {
        format!("{make} {model}").trim().to_owned()
    };
    (!name.is_empty()).then_some(name)
}

/// `n` with at most `decimals` decimals and no trailing zeros.
fn trim_number(n: f64, decimals: usize) -> String {
    let s = format!("{n:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// An exposure time of `num`/`den` seconds: "1/250 s", "0.4 s", "2 s".
pub fn exposure_text(num: u32, den: u32) -> String {
    if den == 0 {
        return String::new();
    }
    let secs = f64::from(num) / f64::from(den);
    if secs < 1.0 && den.is_multiple_of(num) {
        format!("1/{} s", den / num)
    } else if secs >= 0.3 {
        format!("{} s", trim_number(secs, 1))
    } else {
        format!("1/{} s", (1.0 / secs).round())
    }
}

/// "f/2.8", "f/8".
pub fn f_number_text(f: f64) -> String {
    format!("f/{}", trim_number(f, 1))
}

/// "50 mm", or "4.3 mm (35 mm equivalent: 24 mm)".
pub fn focal_text(mm: f64, equiv: Option<u32>) -> String {
    let mm = trim_number(mm, 1);
    match equiv {
        Some(e) => crate::trf!("{v1} mm (35 mm equivalent: {v2} mm)", v1 = mm, v2 = e),
        None => format!("{mm} mm"),
    }
}

/// The EXIF flash bits in words.
pub fn flash_text(bits: u32) -> &'static str {
    if bits & 0x20 != 0 {
        tr("No flash")
    } else if bits & 0x1 != 0 {
        tr("Fired")
    } else {
        tr("Did not fire")
    }
}

/// Degrees, minutes and seconds as decimal degrees; `reference` "S" or
/// "W" makes them negative.
pub fn degrees(dms: &[f64], reference: &str) -> f64 {
    let d: f64 = dms
        .iter()
        .zip([1.0, 60.0, 3600.0])
        .map(|(v, div)| v / div)
        .sum();
    match reference.trim().to_ascii_uppercase().as_str() {
        "S" | "W" => -d,
        _ => d,
    }
}

/// "48.85837° N, 2.29448° E".
pub fn coordinates_text(lat: f64, lon: f64) -> String {
    let ns = if lat < 0.0 { "S" } else { "N" };
    let ew = if lon < 0.0 { "W" } else { "E" };
    format!("{:.5}° {ns}, {:.5}° {ew}", lat.abs(), lon.abs())
}

/// Where the photo was taken on OpenStreetMap.
pub fn map_url(lat: f64, lon: f64) -> String {
    format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=16/{lat:.6}/{lon:.6}")
}

/// Open the map in the browser (only when the user asks: it tells
/// OpenStreetMap the place).
pub fn open_map(lat: f64, lon: f64) -> std::io::Result<()> {
    open::that_detached(map_url(lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;
    use exif::{Field, Rational};

    #[test]
    fn exposure_and_lens_are_written_like_cameras_do() {
        assert_eq!(exposure_text(1, 250), "1/250 s");
        assert_eq!(exposure_text(10, 2500), "1/250 s");
        assert_eq!(exposure_text(1, 3), "1/3 s");
        assert_eq!(exposure_text(1, 2), "1/2 s");
        assert_eq!(exposure_text(4, 10), "0.4 s");
        assert_eq!(exposure_text(333, 100_000), "1/300 s");
        assert_eq!(exposure_text(2, 1), "2 s");
        assert_eq!(exposure_text(25, 10), "2.5 s");
        assert_eq!(f_number_text(2.8), "f/2.8");
        assert_eq!(f_number_text(8.0), "f/8");
        assert_eq!(f_number_text(1.4), "f/1.4");
        assert_eq!(focal_text(50.0, None), "50 mm");
        assert_eq!(
            focal_text(4.26, Some(26)),
            "4.3 mm (35 mm equivalent: 26 mm)"
        );
        assert_eq!(flash_text(0x0), "Did not fire");
        assert_eq!(flash_text(0x19), "Fired");
        assert_eq!(flash_text(0x20), "No flash");
        assert_eq!(
            camera_name("Canon", "Canon EOS R6").unwrap(),
            "Canon EOS R6"
        );
        assert_eq!(
            camera_name("Apple", "iPhone 15").unwrap(),
            "Apple iPhone 15"
        );
        assert_eq!(camera_name("", ""), None);
    }

    #[test]
    fn gps_degrees_minutes_seconds_become_decimal() {
        let lat = degrees(&[48.0, 51.0, 29.4], "N");
        assert!((lat - 48.858_166).abs() < 1e-5);
        assert!((degrees(&[33.0, 52.0, 4.0], "S") + 33.867_777).abs() < 1e-5);
        assert!((degrees(&[2.0, 17.0, 40.2], "E") - 2.294_5).abs() < 1e-5);
        assert!((degrees(&[74.0, 0.0, 21.5], "W") + 74.005_972).abs() < 1e-5);
        // Some cameras write the minutes with decimals and no seconds.
        assert!((degrees(&[10.0, 30.5], "N") - 10.508_333).abs() < 1e-5);
        assert_eq!(
            coordinates_text(48.858_166, -74.005_972),
            "48.85817° N, 74.00597° W"
        );
        assert_eq!(
            map_url(-33.5, 151.25),
            "https://www.openstreetmap.org/?mlat=-33.500000&mlon=151.250000#map=16/-33.500000/151.250000"
        );
    }

    /// EXIF data written and read back.
    #[test]
    fn fields_are_read_from_exif_data() {
        let r = |num, denom| Rational { num, denom };
        let ascii = |s: &str| Value::Ascii(vec![s.as_bytes().to_vec()]);
        let fields = [
            (Tag::Make, ascii("NIKON CORPORATION")),
            (Tag::Model, ascii("NIKON Z 6")),
            (Tag::LensModel, ascii("NIKKOR Z 24-70mm f/4 S")),
            (Tag::Orientation, Value::Short(vec![6])),
            (Tag::ExposureTime, Value::Rational(vec![r(1, 250)])),
            (Tag::FNumber, Value::Rational(vec![r(28, 10)])),
            (Tag::PhotographicSensitivity, Value::Short(vec![400])),
            (Tag::FocalLength, Value::Rational(vec![r(50, 1)])),
            (Tag::Flash, Value::Short(vec![0x10])),
            (Tag::DateTimeOriginal, ascii("2024:07:14 18:05:09")),
            (Tag::GPSLatitudeRef, ascii("S")),
            (
                Tag::GPSLatitude,
                Value::Rational(vec![r(33, 1), r(52, 1), r(4, 1)]),
            ),
            (Tag::GPSLongitudeRef, ascii("E")),
            (
                Tag::GPSLongitude,
                Value::Rational(vec![r(151, 1), r(12, 1), r(30, 1)]),
            ),
            (Tag::GPSAltitudeRef, Value::Byte(vec![0])),
            (Tag::GPSAltitude, Value::Rational(vec![r(58, 1)])),
        ]
        .map(|(tag, value)| Field {
            tag,
            ifd_num: In::PRIMARY,
            value,
        });
        let mut writer = exif::experimental::Writer::new();
        for f in &fields {
            writer.push_field(f);
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        writer.write(&mut buf, false).unwrap();
        let exif = exif::Reader::new().read_raw(buf.into_inner()).unwrap();
        let info = from_exif(&exif);
        assert_eq!(info.camera.as_deref(), Some("NIKON Z 6"));
        assert_eq!(info.lens.as_deref(), Some("NIKKOR Z 24-70mm f/4 S"));
        assert_eq!(info.orientation, Some(6));
        assert_eq!(info.exposure, Some((1, 250)));
        assert_eq!(info.iso, Some(400));
        assert_eq!(info.flash, Some(0x10));
        assert_eq!(
            info.taken.unwrap().to_string(),
            "2024-07-14 18:05:09".to_owned()
        );
        let (lat, lon) = info.location.unwrap();
        assert!((lat + 33.867_777).abs() < 1e-5 && (lon - 151.208_333).abs() < 1e-5);
        let facts = info.facts();
        let get = |label: &str| {
            facts
                .iter()
                .find(|(l, _)| *l == label)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("Exposure time"), Some("1/250 s"));
        assert_eq!(get("Aperture"), Some("f/2.8"));
        assert_eq!(get("Focal length"), Some("50 mm"));
        assert_eq!(get("Flash"), Some("Did not fire"));
        assert_eq!(get("Location taken"), Some("33.86778° S, 151.20833° E"));
        assert_eq!(get("Altitude"), Some("58 m"));
    }
}
