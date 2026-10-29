//! The preview's Hex tab: a file read page by page as it is scrolled
//! (big files are never loaded whole), and the formatting of its rows.
//! Items inside archives are shown from their bytes, read into memory up to
//! [`MAX_IN_MEMORY`].

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Bytes per row.
pub const ROW: usize = 16;
/// Bytes read at a time.
const PAGE: u64 = 16 * 1024;
/// Pages kept in memory.
const PAGES_KEPT: usize = 32;
/// At most this much is copied at once.
pub const MAX_COPY: u64 = 1024 * 1024;
/// At most this much of an item inside an archive is shown.
pub const MAX_IN_MEMORY: u64 = 32 * 1024 * 1024;

enum Source {
    File(File),
    /// The bytes themselves (an item inside an archive).
    Memory(Vec<u8>),
}

/// A file, read on demand.
pub struct HexFile {
    source: Source,
    len: u64,
    /// How big the whole file is (more than `len` if only its start is
    /// shown).
    full_len: u64,
    /// Recently read pages (by number), the latest last.
    pages: Vec<(u64, Vec<u8>)>,
}

impl HexFile {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            source: Source::File(file),
            len,
            full_len: len,
            pages: Vec::new(),
        })
    }

    /// The bytes of a file whose size is `full_len`: all of it, or its
    /// start (cut to [`MAX_IN_MEMORY`]).
    pub fn from_bytes(mut bytes: Vec<u8>, full_len: u64) -> Self {
        bytes.truncate(MAX_IN_MEMORY as usize);
        let len = bytes.len() as u64;
        Self {
            source: Source::Memory(bytes),
            len,
            full_len: full_len.max(len),
            pages: Vec::new(),
        }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The size of the whole file: more than [`Self::len`] if only its
    /// start is shown.
    pub fn full_len(&self) -> u64 {
        self.full_len
    }

    /// Number of rows.
    pub fn rows(&self) -> u64 {
        self.len.div_ceil(ROW as u64)
    }

    fn page(&mut self, n: u64) -> io::Result<&[u8]> {
        let file = match &mut self.source {
            Source::Memory(bytes) => {
                let from = (n * PAGE).min(bytes.len() as u64) as usize;
                let to = (from + PAGE as usize).min(bytes.len());
                return Ok(&bytes[from..to]);
            }
            Source::File(f) => f,
        };
        if let Some(at) = self.pages.iter().position(|(p, _)| *p == n) {
            let page = self.pages.remove(at);
            self.pages.push(page);
        } else {
            let mut buf = Vec::with_capacity(PAGE as usize);
            file.seek(SeekFrom::Start(n * PAGE))?;
            file.take(PAGE).read_to_end(&mut buf)?;
            if self.pages.len() >= PAGES_KEPT {
                self.pages.remove(0);
            }
            self.pages.push((n, buf));
        }
        Ok(&self.pages.last().expect("just added").1)
    }

    /// Up to `len` bytes from `offset` (fewer at the end of the file).
    pub fn read(&mut self, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        let end = offset.saturating_add(len).min(self.len);
        let mut out = Vec::with_capacity(end.saturating_sub(offset) as usize);
        let mut at = offset;
        while at < end {
            let n = at / PAGE;
            let page = self.page(n)?;
            let from = (at - n * PAGE) as usize;
            let to = ((end - n * PAGE) as usize).min(page.len());
            if from >= to {
                break; // The file got shorter.
            }
            out.extend_from_slice(&page[from..to]);
            at = n * PAGE + to as u64;
        }
        Ok(out)
    }

    /// The bytes of row `row`.
    pub fn row(&mut self, row: u64) -> io::Result<Vec<u8>> {
        self.read(row * ROW as u64, ROW as u64)
    }
}

/// Hex digits the offsets of a file of `len` bytes are shown with (8 at least).
pub fn offset_digits(len: u64) -> usize {
    let digits = (64 - len.saturating_sub(1).leading_zeros() as usize).div_ceil(4);
    digits.max(8)
}

pub fn offset_label(offset: u64, digits: usize) -> String {
    format!("{offset:0digits$X}")
}

/// Character column where byte `i` of a row starts in [`hex_row`]: two
/// digits and a space each, with an extra space after the eighth byte.
pub fn hex_column(i: usize) -> usize {
    i * 3 + usize::from(i >= ROW / 2)
}

/// Width of a full [`hex_row`] in characters.
pub const HEX_WIDTH: usize = ROW * 3;

/// The byte of a row under character column `col` of [`hex_row`].
pub fn byte_at_hex_column(col: usize) -> Option<usize> {
    let col = if col >= hex_column(ROW / 2) - 1 {
        col.checked_sub(1)?
    } else {
        col
    };
    let i = col / 3;
    (i < ROW).then_some(i)
}

/// `bytes` as hex: `4D 5A 90 00 …`, split in two halves of eight.
pub fn hex_row(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(HEX_WIDTH);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
            if i == ROW / 2 {
                out.push(' ');
            }
        }
        out.push_str(&format!("{b:02X}"));
    }
    out
}

/// `bytes` as text: printable ASCII as is, everything else as `.`.
pub fn ascii_row(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

/// `bytes` as hex for the clipboard: `4D 5A 90`.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{b:02X}"));
    }
    out
}

/// An offset typed in: hex with `0x`, `$` or a trailing `h` (or with
/// letters A–F), otherwise decimal.
pub fn parse_offset(s: &str) -> Option<u64> {
    let s = s.trim().replace(['_', ' '], "");
    let lower = s.to_ascii_lowercase();
    let hex = lower
        .strip_prefix("0x")
        .or_else(|| lower.strip_prefix('$'))
        .or_else(|| lower.strip_suffix('h'));
    if let Some(h) = hex {
        return u64::from_str_radix(h, 16).ok();
    }
    if lower.bytes().any(|b| (b'a'..=b'f').contains(&b)) {
        return u64::from_str_radix(&lower, 16).ok();
    }
    lower.parse().ok()
}

/// The bytes from `a` to `b` (either order, both included), as a range.
pub fn selection(a: u64, b: u64) -> std::ops::RangeInclusive<u64> {
    a.min(b)..=a.max(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_formatted() {
        let bytes: Vec<u8> = (0..16).collect();
        let hex = hex_row(&bytes);
        assert_eq!(hex, "00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F");
        assert_eq!(hex.len(), HEX_WIDTH);
        for i in 0..ROW {
            assert_eq!(&hex[hex_column(i)..hex_column(i) + 2], format!("{i:02X}"));
            assert_eq!(byte_at_hex_column(hex_column(i)), Some(i));
            assert_eq!(byte_at_hex_column(hex_column(i) + 1), Some(i));
        }
        // The gap between the halves belongs to the byte before it.
        assert_eq!(byte_at_hex_column(hex_column(8) - 1), Some(7));
        assert_eq!(byte_at_hex_column(HEX_WIDTH + 5), None);
        assert_eq!(hex_row(&[0xAB]), "AB");
        assert_eq!(ascii_row(b"Hi\0\x7f ~\xff"), "Hi.. ~.");
        assert_eq!(to_hex(&[0x4d, 0x5a, 0x90]), "4D 5A 90");
    }

    #[test]
    fn offsets() {
        assert_eq!(offset_digits(0), 8);
        assert_eq!(offset_digits(0x1_0000_0000), 8);
        assert_eq!(offset_digits(0x1_0000_0001), 9);
        assert_eq!(offset_label(0x1f, 8), "0000001F");
        assert_eq!(parse_offset("0x1F"), Some(31));
        assert_eq!(parse_offset("1fh"), Some(31));
        assert_eq!(parse_offset("$10"), Some(16));
        assert_eq!(parse_offset("ff"), Some(255));
        assert_eq!(parse_offset("100"), Some(100));
        assert_eq!(parse_offset(" 1_000 "), Some(1000));
        assert_eq!(parse_offset("zz"), None);
        assert_eq!(parse_offset(""), None);
        assert_eq!(selection(5, 2), 2..=5);
    }

    #[test]
    fn reads_pages_on_demand() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.bin");
        let data: Vec<u8> = (0..PAGE * 3 + 100).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let mut f = HexFile::open(&path).unwrap();
        assert_eq!(f.len(), data.len() as u64);
        assert_eq!(f.rows(), (data.len() as u64).div_ceil(16));
        assert_eq!(f.row(0).unwrap(), &data[..16]);
        // Across a page boundary.
        let at = PAGE - 5;
        assert_eq!(
            f.read(at, 10).unwrap(),
            &data[at as usize..at as usize + 10]
        );
        // The last, short row and past the end.
        let last = f.rows() - 1;
        assert_eq!(f.row(last).unwrap(), &data[(last * 16) as usize..]);
        assert!(f.read(data.len() as u64 + 10, 4).unwrap().is_empty());
        // Everything, through the page cache.
        assert_eq!(f.read(0, u64::MAX).unwrap(), data);
        assert!(f.pages.len() <= PAGES_KEPT);
        assert_eq!(f.full_len(), f.len());
    }

    #[test]
    fn pages_through_bytes_in_memory() {
        let data: Vec<u8> = (0..PAGE * 2 + 7).map(|i| (i % 253) as u8).collect();
        let mut f = HexFile::from_bytes(data.clone(), data.len() as u64);
        assert_eq!(f.len(), data.len() as u64);
        assert_eq!(f.full_len(), f.len());
        assert_eq!(f.row(0).unwrap(), &data[..16]);
        let at = PAGE * 2 - 3;
        assert_eq!(f.read(at, 8).unwrap(), &data[at as usize..at as usize + 8]);
        let last = f.rows() - 1;
        assert_eq!(f.row(last).unwrap(), &data[(last * 16) as usize..]);
        assert!(f.read(data.len() as u64, 4).unwrap().is_empty());
        assert_eq!(f.read(0, u64::MAX).unwrap(), data);
        // Nothing is copied into the page cache.
        assert!(f.pages.is_empty());
        // Only the start of a bigger file.
        let f = HexFile::from_bytes(vec![1; 40], 1000);
        assert_eq!((f.len(), f.full_len(), f.rows()), (40, 1000, 3));
        let f = HexFile::from_bytes(Vec::new(), 0);
        assert!(f.is_empty());
        assert_eq!(f.rows(), 0);
    }
}
