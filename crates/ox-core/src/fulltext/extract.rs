//! The text of a file: plain text and code (in its encoding), PDF, and
//! Word, Excel, PowerPoint and OpenDocument files.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::entry::FileKind;
use crate::textinfo::{self, Encoding};

/// How the text of a file is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Plain text or code; files of an unknown type are looked at and are
    /// text if their start has no NUL byte.
    Text,
    /// docx, xlsx, pptx (zipped XML).
    Office,
    /// odt, ods, odp (zipped XML).
    OpenDocument,
    Pdf,
    /// Pictures, videos, programs, archives, …: no text.
    Binary,
}

impl Format {
    /// Is the text read from the bytes as they are (like `content:` without
    /// the index does), not taken out of a document format?
    pub fn is_plain(self) -> bool {
        self == Format::Text
    }
}

/// How the text of `path` is read, by its extension.
pub fn format_of(path: &Path) -> Format {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "docx" | "docm" | "dotx" | "dotm" | "xlsx" | "xlsm" | "xltx" | "xltm" | "pptx" | "pptm"
        | "potx" | "potm" | "ppsx" => Format::Office,
        "odt" | "ott" | "ods" | "ots" | "odp" | "otp" => Format::OpenDocument,
        "pdf" => Format::Pdf,
        // Pictures that are text.
        "svg" | "eps" | "ps" => Format::Text,
        // Old Office files (compound files, no plain text).
        "doc" | "xls" | "ppt" | "msg" | "pst" => Format::Binary,
        _ => match FileKind::from_extension(&ext) {
            FileKind::Image
            | FileKind::Video
            | FileKind::Audio
            | FileKind::Archive
            | FileKind::Executable
            | FileKind::Font => Format::Binary,
            _ => Format::Text,
        },
    }
}

/// The text of `path` (read up to `limit` bytes), `None` if it has none.
/// Doesn't look at the file's size or cloud state: the caller does.
pub fn extract(path: &Path, limit: u64) -> io::Result<Option<String>> {
    match format_of(path) {
        Format::Text => plain(path, limit),
        Format::Office => super::office::ooxml(path, limit).map(Some),
        Format::OpenDocument => super::office::odf(path, limit).map(Some),
        Format::Pdf => Ok(super::pdf_text::text(path, limit)),
        Format::Binary => Ok(None),
    }
}

/// Bytes looked at for a NUL that marks a binary file (as `content:` does).
const SNIFF: usize = 8192;

/// A text file in its encoding (UTF-8, UTF-16 with a byte order mark, or a
/// legacy code page). `None` for a binary file.
fn plain(path: &Path, limit: u64) -> io::Result<Option<String>> {
    let mut f = File::open(path)?.take(limit);
    let mut data = vec![0u8; SNIFF];
    let mut got = 0;
    while got < SNIFF {
        match f.read(&mut data[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    data.truncate(got);
    let utf16 = matches!(
        textinfo::encoding(&data),
        Encoding::Utf16Le | Encoding::Utf16Be
    );
    if !utf16 && data.contains(&0) {
        return Ok(None);
    }
    f.read_to_end(&mut data)?;
    Ok(Some(decode(&data)))
}

/// `data` as text. Bytes that aren't UTF-8 are read as Windows-1252 too (an
/// old German text), next to the UTF-8 reading.
pub(super) fn decode(data: &[u8]) -> String {
    let enc = textinfo::encoding(data);
    let mut text = textinfo::decode(data, enc);
    if enc == Encoding::Other {
        text.push('\n');
        text.extend(data.iter().map(|&b| cp1252(b)));
    }
    text
}

/// A Windows-1252 byte as a character (Latin-1 plus the € and quotes block).
fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH[usize::from(b - 0x80)],
        _ => char::from(b),
    }
}
