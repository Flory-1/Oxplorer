//! Values for the extra columns of the details view: a picture's or video's
//! dimensions, the length of audio and video, authors, a program's version,
//! the date a photo was taken and the attribute letters.
//!
//! On Windows they come from the Shell's property system, which knows every
//! format that has a property handler. Files that are only in the cloud
//! (OneDrive "online only") are asked for the properties Windows already
//! has, so that reading a column never downloads them.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// What the extra columns show for one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Props {
    /// Width and height in pixels.
    pub dimensions: Option<(u32, u32)>,
    pub duration: Option<Duration>,
    /// Several are joined with "; ".
    pub authors: Option<String>,
    pub version: Option<String>,
    pub date_taken: Option<SystemTime>,
    /// Windows file attributes (`FILE_ATTRIBUTE_*`).
    pub attributes: Option<u32>,
}

/// Read the values of `path`. Blocks (opens the file), so call it off the UI
/// thread. Folders only have attributes.
pub fn read(path: &Path, is_dir: bool) -> Props {
    let mut p = Props {
        attributes: attributes(path),
        ..Default::default()
    };
    if !is_dir {
        read_file(path, &mut p);
    }
    p
}

const READONLY: u32 = 0x1;
const HIDDEN: u32 = 0x2;
const SYSTEM: u32 = 0x4;
const ARCHIVE: u32 = 0x20;
const SPARSE: u32 = 0x200;
const REPARSE_POINT: u32 = 0x400;
const COMPRESSED: u32 = 0x800;
const OFFLINE: u32 = 0x1000;
const NOT_INDEXED: u32 = 0x2000;
const ENCRYPTED: u32 = 0x4000;
const RECALL_ON_OPEN: u32 = 0x4_0000;
const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

/// The file's content isn't on this computer (a cloud placeholder or an
/// archived file): reading it would download or recall it.
pub fn is_remote(attributes: u32) -> bool {
    attributes & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
}

/// Attribute letters as Explorer shows them: R(ead-only) H(idden) S(ystem)
/// A(rchive) C(ompressed) E(ncrypted), and O(ffline), P (sparse),
/// L (link/reparse point), I (not content indexed).
pub fn attribute_letters(attributes: u32) -> String {
    [
        (READONLY, 'R'),
        (HIDDEN, 'H'),
        (SYSTEM, 'S'),
        (ARCHIVE, 'A'),
        (COMPRESSED, 'C'),
        (ENCRYPTED, 'E'),
        (OFFLINE, 'O'),
        (SPARSE, 'P'),
        (REPARSE_POINT, 'L'),
        (NOT_INDEXED, 'I'),
    ]
    .into_iter()
    .filter(|(bit, _)| attributes & bit != 0)
    .map(|(_, letter)| letter)
    .collect()
}

/// `3:05` or `1:02:03`, like Explorer's Length column.
pub fn duration_text(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `1920 × 1080`.
pub fn dimensions_text((w, h): (u32, u32)) -> String {
    format!("{w} × {h}")
}

#[cfg(windows)]
fn attributes(path: &Path) -> Option<u32> {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .ok()
        .map(|m| m.file_attributes())
}

#[cfg(not(windows))]
fn attributes(_path: &Path) -> Option<u32> {
    None
}

#[cfg(windows)]
fn read_file(path: &Path, p: &mut Props) {
    let remote = p.attributes.is_some_and(is_remote);
    // SAFETY: COM is set up for the calling thread inside.
    unsafe { imp::read(path, remote, p) }
}

/// Elsewhere only the date a photo was taken (from its EXIF data).
#[cfg(not(windows))]
fn read_file(path: &Path, p: &mut Props) {
    p.date_taken = crate::details::read(path, false).date_taken;
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::StructuredStorage::{
        PropVariantClear, PropVariantToFileTime, PropVariantToStringAlloc, PropVariantToUInt32,
        PropVariantToUInt64, PROPVARIANT,
    };
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::Variant::{PSTF_UTC, VT_EMPTY};
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, SHGetPropertyStoreFromParsingName, GPS_DEFAULT, GPS_FASTPROPERTIESONLY,
    };

    use super::Props;

    const fn key(fmtid: u128, pid: u32) -> PROPERTYKEY {
        PROPERTYKEY {
            fmtid: GUID::from_u128(fmtid),
            pid,
        }
    }

    const IMAGE_WIDTH: PROPERTYKEY = key(0x6444048f_4c8b_11d1_8b70_080036b11a03, 3);
    const IMAGE_HEIGHT: PROPERTYKEY = key(0x6444048f_4c8b_11d1_8b70_080036b11a03, 4);
    const VIDEO_WIDTH: PROPERTYKEY = key(0x64440491_4c8b_11d1_8b70_080036b11a03, 3);
    const VIDEO_HEIGHT: PROPERTYKEY = key(0x64440491_4c8b_11d1_8b70_080036b11a03, 4);
    const DURATION: PROPERTYKEY = key(0x64440490_4c8b_11d1_8b70_080036b11a03, 3);
    const AUTHOR: PROPERTYKEY = key(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9, 4);
    const FILE_VERSION: PROPERTYKEY = key(0x0cef7d53_fa64_11d1_a203_0000f81fedee, 4);
    const DATE_TAKEN: PROPERTYKEY = key(0x14b81da1_0135_4d31_96d9_6cbfc9671a99, 36867);

    /// A property value, cleared when dropped.
    struct Value(PROPVARIANT);

    impl Drop for Value {
        fn drop(&mut self) {
            // SAFETY: the value came from `GetValue` and is cleared once.
            let _ = unsafe { PropVariantClear(&mut self.0) };
        }
    }

    unsafe fn get(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<Value> {
        // SAFETY: `key` lives through the call.
        let v = Value(unsafe { store.GetValue(key) }.ok()?);
        // SAFETY: every PROPVARIANT starts with its type.
        let empty = unsafe { v.0.Anonymous.Anonymous.vt } == VT_EMPTY;
        (!empty).then_some(v)
    }

    unsafe fn text(v: &Value) -> Option<String> {
        // SAFETY: the returned string is ours to free.
        unsafe {
            let s = PropVariantToStringAlloc(&v.0).ok()?;
            let text = s.to_string().ok();
            CoTaskMemFree(Some(s.0 as *const std::ffi::c_void));
            text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
        }
    }

    pub(super) unsafe fn read(path: &Path, remote: bool, p: &mut Props) {
        // SAFETY: plain COM calls on values that outlive them.
        unsafe {
            // Once per thread; later calls just return S_FALSE.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let Ok(abs) = std::path::absolute(path) else {
                return;
            };
            let name: Vec<u16> = abs.as_os_str().encode_wide().chain(Some(0)).collect();
            // Online-only files: only what Windows knows without the content.
            let flags = if remote {
                GPS_FASTPROPERTIESONLY
            } else {
                GPS_DEFAULT
            };
            let Ok(store) = SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(
                PCWSTR(name.as_ptr()),
                None,
                flags,
            ) else {
                return;
            };
            let number =
                |k: &PROPERTYKEY| get(&store, k).and_then(|v| PropVariantToUInt32(&v.0).ok());
            let size = |w, h| match (number(w), number(h)) {
                (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
                _ => None,
            };
            p.dimensions =
                size(&IMAGE_WIDTH, &IMAGE_HEIGHT).or_else(|| size(&VIDEO_WIDTH, &VIDEO_HEIGHT));
            // In units of 100 ns.
            p.duration = get(&store, &DURATION)
                .and_then(|v| PropVariantToUInt64(&v.0).ok())
                .filter(|&d| d > 0)
                .map(|d| Duration::from_nanos(d.saturating_mul(100)));
            p.authors = get(&store, &AUTHOR).and_then(|v| text(&v));
            p.version = get(&store, &FILE_VERSION).and_then(|v| text(&v));
            p.date_taken = get(&store, &DATE_TAKEN)
                .and_then(|v| PropVariantToFileTime(&v.0, PSTF_UTC).ok())
                .and_then(|ft| {
                    // 100 ns since 1601.
                    let t = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
                    let unix = t.checked_sub(116_444_736_000_000_000)?;
                    Some(UNIX_EPOCH + Duration::from_nanos(unix.saturating_mul(100)))
                })
                .filter(|&t: &SystemTime| t > UNIX_EPOCH);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_letters_like_explorer() {
        assert_eq!(attribute_letters(0), "");
        assert_eq!(attribute_letters(ARCHIVE), "A");
        assert_eq!(
            attribute_letters(READONLY | HIDDEN | SYSTEM | ARCHIVE),
            "RHSA"
        );
        assert_eq!(attribute_letters(COMPRESSED | ENCRYPTED), "CE");
        assert!(is_remote(RECALL_ON_DATA_ACCESS | ARCHIVE));
        assert!(!is_remote(ARCHIVE | HIDDEN));
    }

    #[test]
    fn durations_and_dimensions() {
        assert_eq!(duration_text(Duration::from_secs(185)), "3:05");
        assert_eq!(duration_text(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(duration_text(Duration::from_millis(900)), "0:00");
        assert_eq!(dimensions_text((1920, 1080)), "1920 × 1080");
    }

    #[test]
    fn reads_a_plain_file_and_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "hello").unwrap();
        let p = read(&file, false);
        assert!(p.dimensions.is_none() && p.duration.is_none());
        if cfg!(windows) {
            assert!(p.attributes.is_some());
            assert!(read(dir.path(), true).attributes.is_some());
        }
        // A missing file has nothing, and doesn't fail.
        assert_eq!(read(&dir.path().join("missing"), false), Props::default());
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_version_of_a_program() {
        let notepad = Path::new("C:\\Windows\\System32\\notepad.exe");
        if !notepad.exists() {
            return;
        }
        let p = read(notepad, false);
        assert!(p.version.as_ref().is_some_and(|v| v.contains('.')), "{p:?}");
    }

    /// A 3 × 2 pixel BMP file.
    #[cfg(windows)]
    fn bmp() -> Vec<u8> {
        let (w, h) = (3u32, 2u32);
        let row = (w * 3).div_ceil(4) * 4;
        let data = row * h;
        let mut b = Vec::new();
        b.extend(b"BM");
        b.extend((54 + data).to_le_bytes());
        b.extend([0u8; 4]);
        b.extend(54u32.to_le_bytes());
        b.extend(40u32.to_le_bytes());
        b.extend(w.to_le_bytes());
        b.extend(h.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(24u16.to_le_bytes());
        b.extend([0u8; 24]);
        b.extend(vec![0x80u8; data as usize]);
        b
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_dimensions_of_a_picture() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.bmp");
        std::fs::write(&file, bmp()).unwrap();
        assert_eq!(read(&file, false).dimensions, Some((3, 2)));
    }
}
