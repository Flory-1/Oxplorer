//! Turning a JPEG without touching its pixels: only the EXIF Orientation
//! tag is rewritten (the viewer turns the picture as it says).
//!
//! - The tag is in IFD0: its value is replaced in place; every other byte
//!   of the file stays as it was.
//! - There is EXIF data but no tag: IFD0 is copied, with the tag added, to
//!   the end of the EXIF data and the header points to the copy. Nothing
//!   else moves, so every offset in the EXIF data (maker notes too) stays
//!   valid.
//! - There is no EXIF data: an APP1 segment holding just the tag is added
//!   after the SOI marker (and after a JFIF APP0 segment).
//!
//! Anything unusual (a tag that isn't one SHORT, a broken IFD, EXIF data
//! too big to grow) gives `None`: the caller re-encodes instead.

use super::Transform;

/// EXIF Orientation.
const TAG: u16 = 0x0112;
/// TIFF type SHORT.
const SHORT: u16 = 3;

/// How orientation `o` (1–8) moves the stored pixels to where they are
/// shown: the matrix `[a, b, c, d]` maps a point (x, y), measured from the
/// middle of the picture with y going down, to (ax + by, cx + dy).
fn matrix(o: u16) -> [i8; 4] {
    match o {
        2 => [-1, 0, 0, 1],
        3 => [-1, 0, 0, -1],
        4 => [1, 0, 0, -1],
        5 => [0, 1, 1, 0],
        6 => [0, -1, 1, 0],
        7 => [0, -1, -1, 0],
        8 => [0, 1, -1, 0],
        _ => [1, 0, 0, 1],
    }
}

/// The orientation that does what `tf` does.
fn orientation_of(tf: Transform) -> u16 {
    match tf {
        Transform::RotateRight => 6,
        Transform::RotateLeft => 8,
        Transform::Rotate180 => 3,
        Transform::FlipHorizontal => 2,
        Transform::FlipVertical => 4,
    }
}

/// The orientation of a picture shown as `o` says and then turned by
/// `tf`. Values outside 1–8 count as 1 (that's how viewers show them).
pub fn compose(o: u16, tf: Transform) -> u16 {
    let [a, b, c, d] = matrix(o);
    let [e, f, g, h] = matrix(orientation_of(tf));
    let m = [e * a + f * c, e * b + f * d, g * a + h * c, g * b + h * d];
    (1..=8).find(|&n| matrix(n) == m).unwrap_or(1)
}

#[derive(Clone, Copy)]
struct Endian(bool);

impl Endian {
    fn u16(self, b: &[u8], at: usize) -> Option<u16> {
        let s: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
        Some(if self.0 {
            u16::from_be_bytes(s)
        } else {
            u16::from_le_bytes(s)
        })
    }

    fn u32(self, b: &[u8], at: usize) -> Option<u32> {
        let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        Some(if self.0 {
            u32::from_be_bytes(s)
        } else {
            u32::from_le_bytes(s)
        })
    }

    fn put_u16(self, v: u16) -> [u8; 2] {
        if self.0 {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    }

    fn put_u32(self, v: u32) -> [u8; 4] {
        if self.0 {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    }
}

/// Where things are in a JPEG's header.
struct Layout {
    /// The first APP1 "Exif" segment: where its marker starts and its
    /// length field's value.
    exif: Option<(usize, usize)>,
    /// Where a new APP1 segment goes.
    insert_at: usize,
}

/// Walk the segments before the first scan.
fn layout(jpeg: &[u8]) -> Option<Layout> {
    if !jpeg.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut pos = 2;
    let mut insert_at = 2;
    let mut leading_app0 = true;
    loop {
        if *jpeg.get(pos)? != 0xFF {
            return None;
        }
        let mut m = pos + 1;
        while *jpeg.get(m)? == 0xFF {
            m += 1;
        }
        let marker = jpeg[m];
        // Start of scan or end of picture: the header is over.
        if marker == 0xDA || marker == 0xD9 {
            return Some(Layout {
                exif: None,
                insert_at,
            });
        }
        // Markers without a length.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos = m + 1;
            continue;
        }
        let len = u16::from_be_bytes([*jpeg.get(m + 1)?, *jpeg.get(m + 2)?]) as usize;
        let end = m + 1 + len;
        if len < 2 || end > jpeg.len() {
            return None;
        }
        if marker == 0xE1 && jpeg[m + 3..end].starts_with(b"Exif\0\0") {
            return Some(Layout {
                exif: Some((m - 1, len)),
                insert_at,
            });
        }
        if marker == 0xE0 && leading_app0 {
            insert_at = end;
        } else {
            leading_app0 = false;
        }
        pos = end;
    }
}

/// The JPEG `jpeg` turned by `tf` through its EXIF orientation, and the
/// new orientation; `None` if that can't be done safely.
pub fn turn_jpeg(jpeg: &[u8], tf: Transform) -> Option<(Vec<u8>, u16)> {
    let l = layout(jpeg)?;
    let Some((seg, len)) = l.exif else {
        let o = compose(1, tf);
        let mut out = Vec::with_capacity(jpeg.len() + 40);
        out.extend_from_slice(&jpeg[..l.insert_at]);
        out.extend_from_slice(&new_segment(o));
        out.extend_from_slice(&jpeg[l.insert_at..]);
        return Some((out, o));
    };
    // The TIFF data: after the marker, length and "Exif\0\0".
    let tiff_at = seg + 10;
    let tiff = &jpeg[tiff_at..seg + 2 + len];
    let e = match tiff.get(..4)? {
        [b'I', b'I', 42, 0] => Endian(false),
        [b'M', b'M', 0, 42] => Endian(true),
        _ => return None,
    };
    let ifd = e.u32(tiff, 4)? as usize;
    let n = e.u16(tiff, ifd)? as usize;
    let entries = ifd + 2;
    tiff.get(entries..entries + n * 12 + 4)?;
    let found = (0..n)
        .map(|i| entries + i * 12)
        .find(|&at| e.u16(tiff, at) == Some(TAG));
    if let Some(at) = found {
        if e.u16(tiff, at + 2)? != SHORT || e.u32(tiff, at + 4)? != 1 {
            return None;
        }
        let o = compose(e.u16(tiff, at + 8)?, tf);
        let mut out = jpeg.to_vec();
        let value = tiff_at + at + 8;
        out[value..value + 2].copy_from_slice(&e.put_u16(o));
        return Some((out, o));
    }
    // No tag: a copy of IFD0 with it, at the end of the TIFF data.
    let o = compose(1, tf);
    let mut grown = tiff.to_vec();
    if grown.len() % 2 == 1 {
        grown.push(0);
    }
    let new_ifd = grown.len();
    grown.extend_from_slice(&e.put_u16(n as u16 + 1));
    let mut ours = Vec::with_capacity(12);
    ours.extend_from_slice(&e.put_u16(TAG));
    ours.extend_from_slice(&e.put_u16(SHORT));
    ours.extend_from_slice(&e.put_u32(1));
    ours.extend_from_slice(&e.put_u16(o));
    ours.extend_from_slice(&[0, 0]);
    let mut placed = false;
    for i in 0..n {
        let at = entries + i * 12;
        if !placed && e.u16(tiff, at)? > TAG {
            grown.extend_from_slice(&ours);
            placed = true;
        }
        grown.extend_from_slice(&tiff[at..at + 12]);
    }
    if !placed {
        grown.extend_from_slice(&ours);
    }
    // The next IFD (the thumbnail's) as before.
    grown.extend_from_slice(&tiff[entries + n * 12..entries + n * 12 + 4]);
    grown[4..8].copy_from_slice(&e.put_u32(u32::try_from(new_ifd).ok()?));
    let new_len = u16::try_from(2 + 6 + grown.len()).ok()?;
    let mut out = Vec::with_capacity(jpeg.len() + grown.len() - tiff.len());
    out.extend_from_slice(&jpeg[..seg + 2]);
    out.extend_from_slice(&new_len.to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&grown);
    out.extend_from_slice(&jpeg[seg + 2 + len..]);
    Some((out, o))
}

/// An APP1 segment with EXIF data holding only the orientation `o`.
fn new_segment(o: u16) -> Vec<u8> {
    let mut s = vec![0xFF, 0xE1, 0, 34];
    s.extend_from_slice(b"Exif\0\0");
    s.extend_from_slice(b"MM\0\x2a\0\0\0\x08");
    s.extend_from_slice(&[0, 1]);
    s.extend_from_slice(&TAG.to_be_bytes());
    s.extend_from_slice(&SHORT.to_be_bytes());
    s.extend_from_slice(&1u32.to_be_bytes());
    s.extend_from_slice(&o.to_be_bytes());
    s.extend_from_slice(&[0, 0]);
    s.extend_from_slice(&[0, 0, 0, 0]);
    debug_assert_eq!(s.len(), 2 + 34);
    s
}

/// The EXIF data of a JPEG as the `image` crate reads it (without the
/// "Exif" header), for checking the result.
pub(super) fn exif_chunk(jpeg: &[u8]) -> Option<&[u8]> {
    let (seg, len) = layout(jpeg)?.exif?;
    jpeg.get(seg + 10..seg + 2 + len)
}
