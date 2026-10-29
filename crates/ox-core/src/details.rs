//! More about a file or folder than its listing says, for the preview's Info
//! tab and the Properties dialog: when it was last opened, the space it
//! takes, where a link points, the program that opens it, a program's
//! version and a photo's camera details.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::i18n::t as tr;

#[derive(Debug, Clone, Default)]
pub struct Details {
    pub accessed: Option<SystemTime>,
    /// Space the file takes on the disk: whole clusters, less for
    /// compressed and sparse files (files only).
    pub size_on_disk: Option<u64>,
    /// Where a symbolic link or junction points.
    pub link_target: Option<PathBuf>,
    /// Windows attributes besides read-only and hidden (System, Compressed, …).
    pub attributes: Vec<&'static str>,
    /// The program that opens the file.
    pub opens_with: Option<String>,
    /// When a photo was taken.
    pub date_taken: Option<SystemTime>,
    /// A program's version information and a photo's camera details.
    pub facts: Vec<(&'static str, String)>,
}

/// Read everything there is to know about `path`. Can take a moment (reads
/// the file's headers), so call it off the UI thread.
pub fn read(path: &Path, is_dir: bool) -> Details {
    let mut d = Details {
        accessed: fs::metadata(path).and_then(|m| m.accessed()).ok(),
        link_target: fs::symlink_metadata(path)
            .is_ok_and(|m| m.file_type().is_symlink() || is_junction(&m))
            .then(|| fs::read_link(path).ok())
            .flatten(),
        attributes: attributes(path),
        ..Default::default()
    };
    if is_dir {
        return d;
    }
    d.size_on_disk = size_on_disk(path);
    d.opens_with = opens_with(path);
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if matches!(
        ext.as_str(),
        "exe" | "dll" | "sys" | "ocx" | "cpl" | "scr" | "drv" | "mui" | "efi"
    ) {
        d.facts.extend(version_info(path));
    }
    if matches!(
        ext.as_str(),
        "jpg"
            | "jpeg"
            | "jfif"
            | "tif"
            | "tiff"
            | "heic"
            | "heif"
            | "avif"
            | "png"
            | "webp"
            | "dng"
    ) {
        photo(path, &mut d);
    }
    d
}

// ---- Windows attributes ------------------------------------------------------

#[cfg(windows)]
fn is_junction(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    m.file_attributes() & REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_junction(_m: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn attributes(path: &Path) -> Vec<&'static str> {
    use std::os::windows::fs::MetadataExt;
    let Ok(m) = fs::symlink_metadata(path) else {
        return Vec::new();
    };
    let attrs = m.file_attributes();
    [
        (0x4, tr("System")),
        (0x800, tr("Compressed")),
        (0x4000, tr("Encrypted")),
        (0x200, tr("Sparse")),
        (0x1000, tr("Offline")),
        (0x40_0000, tr("Online only")),
        (0x8_0000, tr("Always kept on this device")),
    ]
    .into_iter()
    .filter(|(bit, _)| attrs & bit != 0)
    .map(|(_, label)| label)
    .collect()
}

#[cfg(not(windows))]
fn attributes(_path: &Path) -> Vec<&'static str> {
    Vec::new()
}

/// Make `path` read-only or not, and hidden or not (`None` leaves it as it is).
#[cfg(windows)]
pub fn set_attributes(
    path: &Path,
    readonly: Option<bool>,
    hidden: Option<bool>,
) -> std::io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::SetFileAttributesW;
    const READONLY: u32 = 0x1;
    const HIDDEN: u32 = 0x2;
    let mut attrs = fs::symlink_metadata(path)?.file_attributes();
    for (bit, on) in [(READONLY, readonly), (HIDDEN, hidden)] {
        match on {
            Some(true) => attrs |= bit,
            Some(false) => attrs &= !bit,
            None => {}
        }
    }
    let wide = wide(path);
    // SAFETY: `wide` is a NUL-terminated path that outlives the call.
    if unsafe { SetFileAttributesW(wide.as_ptr(), attrs) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Make `path` read-only or not (there is no hidden attribute here: hidden
/// files are the ones whose name starts with a dot).
#[cfg(not(windows))]
pub fn set_attributes(
    path: &Path,
    readonly: Option<bool>,
    _hidden: Option<bool>,
) -> std::io::Result<()> {
    if let Some(readonly) = readonly {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_readonly(readonly);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

// ---- size on disk ------------------------------------------------------------

#[cfg(windows)]
fn size_on_disk(path: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetCompressedFileSizeW, GetDiskFreeSpaceW, GetVolumePathNameW,
    };
    let file = wide(path);
    let mut volume = [0u16; 512];
    let (mut sectors, mut bytes, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
    let mut high = 0u32;
    // SAFETY: all buffers are NUL-terminated or as long as passed, and
    // outlive the calls.
    let (low, cluster) = unsafe {
        let low = GetCompressedFileSizeW(file.as_ptr(), &mut high);
        if low == u32::MAX && std::io::Error::last_os_error().raw_os_error() != Some(0) {
            return None;
        }
        let cluster = if GetVolumePathNameW(file.as_ptr(), volume.as_mut_ptr(), 512) != 0
            && GetDiskFreeSpaceW(
                volume.as_ptr(),
                &mut sectors,
                &mut bytes,
                &mut free,
                &mut total,
            ) != 0
        {
            u64::from(sectors) * u64::from(bytes)
        } else {
            0
        };
        (low, cluster)
    };
    let size = (u64::from(high) << 32) | u64::from(low);
    Some(if cluster > 0 {
        size.div_ceil(cluster) * cluster
    } else {
        size
    })
}

#[cfg(not(windows))]
fn size_on_disk(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| m.blocks() * 512)
}

// ---- the program that opens it -----------------------------------------------

#[cfg(windows)]
fn opens_with(path: &Path) -> Option<String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::UI::Shell::{
        AssocQueryStringW, ASSOCF_NOTRUNCATE, ASSOCSTR_FRIENDLYAPPNAME,
    };
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if matches!(ext.as_str(), "exe" | "com" | "scr") {
        return None; // programs run themselves
    }
    let assoc: Vec<u16> = format!(".{ext}").encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    // SAFETY: `assoc` is NUL-terminated and `buf` holds `len` characters.
    let hr = unsafe {
        AssocQueryStringW(
            ASSOCF_NOTRUNCATE,
            ASSOCSTR_FRIENDLYAPPNAME,
            PCWSTR(assoc.as_ptr()),
            PCWSTR::null(),
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
    };
    if hr.is_err() {
        return None;
    }
    let name = String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]);
    (!name.trim().is_empty()).then_some(name)
}

#[cfg(not(windows))]
fn opens_with(_path: &Path) -> Option<String> {
    None
}

// ---- version information of programs -----------------------------------------

#[cfg(windows)]
fn version_info(path: &Path) -> Vec<(&'static str, String)> {
    use std::ffi::c_void;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    let file = wide(path);
    let mut out = Vec::new();
    // SAFETY: `data` is as long as passed, and the pointers VerQueryValueW
    // returns point into it.
    unsafe {
        let size = GetFileVersionInfoSizeW(file.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return out;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(file.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return out;
        }
        let query = |sub: &str| -> Option<(*mut c_void, u32)> {
            let sub: Vec<u16> = sub.encode_utf16().chain(Some(0)).collect();
            let mut ptr = std::ptr::null_mut();
            let mut len = 0u32;
            (VerQueryValueW(data.as_ptr().cast(), sub.as_ptr(), &mut ptr, &mut len) != 0
                && !ptr.is_null()
                && len > 0)
                .then_some((ptr, len))
        };
        // The first language the file has its texts in (US English if none is listed).
        let lang = query("\\VarFileInfo\\Translation")
            .map(|(ptr, _)| {
                let pair = ptr.cast::<u16>();
                format!("{:04x}{:04x}", *pair, *pair.add(1))
            })
            .unwrap_or_else(|| "040904b0".to_owned());
        for (key, label) in [
            ("FileDescription", tr("Description")),
            ("FileVersion", tr("File version")),
            ("ProductName", tr("Product")),
            ("ProductVersion", tr("Product version")),
            ("CompanyName", tr("Company")),
            ("LegalCopyright", tr("Copyright")),
        ] {
            let Some((ptr, len)) = query(&format!("\\StringFileInfo\\{lang}\\{key}")) else {
                continue;
            };
            let text = std::slice::from_raw_parts(ptr.cast::<u16>(), len as usize);
            let text = String::from_utf16_lossy(text);
            let text = text.trim_end_matches('\0').trim();
            if !text.is_empty() {
                out.push((label, text.to_owned()));
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn version_info(_path: &Path) -> Vec<(&'static str, String)> {
    Vec::new()
}

// ---- photos --------------------------------------------------------------------

/// Camera, lens, exposure, date taken and location from a photo's EXIF data.
fn photo(path: &Path, d: &mut Details) {
    use exif::{In, Tag};
    let Ok(file) = fs::File::open(path) else {
        return;
    };
    let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file))
    else {
        return;
    };
    let text = |tag: Tag| {
        exif.get_field(tag, In::PRIMARY)
            .map(|f| f.display_value().with_unit(&exif).to_string())
            .map(|s| s.trim_matches('"').trim().to_owned())
            .filter(|s| !s.is_empty() && s != "unknown")
    };
    let make = text(Tag::Make).unwrap_or_default();
    let model = text(Tag::Model).unwrap_or_default();
    // Most cameras repeat the maker in the model ("Canon" "Canon EOS R6").
    let camera = if model.to_lowercase().starts_with(&make.to_lowercase()) {
        model
    } else {
        format!("{make} {model}").trim().to_owned()
    };
    if !camera.is_empty() {
        d.facts.push((tr("Camera"), camera));
    }
    if let Some(lens) = text(Tag::LensModel) {
        d.facts.push((tr("Lens"), lens));
    }
    let exposure: Vec<String> = [
        text(Tag::ExposureTime),
        text(Tag::FNumber),
        text(Tag::PhotographicSensitivity).map(|iso| format!("ISO {iso}")),
        text(Tag::FocalLength),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !exposure.is_empty() {
        d.facts.push((tr("Exposure"), exposure.join(" · ")));
    }
    d.date_taken = exif
        .get_field(Tag::DateTimeOriginal, In::PRIMARY)
        .and_then(|f| match &f.value {
            exif::Value::Ascii(v) => v.first().and_then(|b| exif::DateTime::from_ascii(b).ok()),
            _ => None,
        })
        .and_then(|t| {
            use chrono::TimeZone;
            let date =
                chrono::NaiveDate::from_ymd_opt(t.year.into(), t.month.into(), t.day.into())?;
            let time = date.and_hms_opt(t.hour.into(), t.minute.into(), t.second.into())?;
            // Cameras write the local time without a time zone.
            chrono::Local
                .from_local_datetime(&time)
                .earliest()
                .map(SystemTime::from)
        });
    let coordinate = |value: Tag, reference: Tag| {
        let f = exif.get_field(value, In::PRIMARY)?;
        let exif::Value::Rational(parts) = &f.value else {
            return None;
        };
        let degrees = parts
            .iter()
            .zip([1.0, 60.0, 3600.0])
            .map(|(r, div)| r.to_f64() / div)
            .sum::<f64>();
        Some(format!(
            "{degrees:.5}° {}",
            text(reference).unwrap_or_default()
        ))
    };
    if let (Some(lat), Some(lon)) = (
        coordinate(Tag::GPSLatitude, Tag::GPSLatitudeRef),
        coordinate(Tag::GPSLongitude, Tag::GPSLongitudeRef),
    ) {
        d.facts
            .push((tr("Location taken"), format!("{lat}, {lon}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_plain_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "hello").unwrap();
        let d = read(&file, false);
        assert!(d.accessed.is_some());
        assert!(d.size_on_disk.is_some_and(|s| s == 0 || s >= 5));
        assert!(d.link_target.is_none());
        assert!(d.date_taken.is_none());
    }

    #[test]
    fn sets_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "hello").unwrap();
        set_attributes(&file, Some(true), None).unwrap();
        assert!(fs::metadata(&file).unwrap().permissions().readonly());
        set_attributes(&file, Some(false), None).unwrap();
        assert!(!fs::metadata(&file).unwrap().permissions().readonly());
    }
}
