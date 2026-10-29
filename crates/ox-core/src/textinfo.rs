//! Details of text files for developers: encoding, line endings and
//! indentation, and checksums of any file.

use std::io::{self, Read};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Ascii,
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// Not valid UTF-8: probably a legacy code page such as Windows-1252.
    Other,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Ascii => "ASCII",
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => crate::i18n::t("UTF-8 with BOM"),
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Other => crate::i18n::t("Not UTF-8 (e.g. Windows-1252)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEndings {
    /// Windows (`\r\n`).
    Crlf,
    /// Unix (`\n`).
    Lf,
    /// Classic Mac (`\r`).
    Cr,
    Mixed,
    /// A single line.
    None,
}

impl LineEndings {
    pub fn label(self) -> &'static str {
        match self {
            LineEndings::Crlf => crate::i18n::t("CRLF (Windows)"),
            LineEndings::Lf => crate::i18n::t("LF (Unix)"),
            LineEndings::Cr => crate::i18n::t("CR (classic Mac)"),
            LineEndings::Mixed => crate::i18n::t("Mixed"),
            LineEndings::None => crate::i18n::t("None (one line)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indent {
    Tabs,
    Spaces(usize),
    Mixed,
    /// No indented lines.
    None,
}

impl Indent {
    pub fn label(self) -> String {
        match self {
            Indent::Tabs => crate::i18n::t("Tabs").into(),
            Indent::Spaces(n) => crate::trf!("{n} spaces", n = n),
            Indent::Mixed => crate::i18n::t("Tabs and spaces").into(),
            Indent::None => crate::i18n::t("None").into(),
        }
    }
}

/// The encoding of `bytes` (the start of a file) by its byte order mark,
/// or by whether it is valid UTF-8.
pub fn encoding(bytes: &[u8]) -> Encoding {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Encoding::Utf8Bom
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Encoding::Utf16Le
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Encoding::Utf16Be
    } else if bytes.is_ascii() {
        Encoding::Ascii
    } else {
        match std::str::from_utf8(bytes) {
            Ok(_) => Encoding::Utf8,
            // A character cut off at the end of what was read is fine.
            Err(e) if e.error_len().is_none() => Encoding::Utf8,
            Err(_) => Encoding::Other,
        }
    }
}

/// Text of a file in `enc` (UTF-16 decoded, a BOM dropped).
pub fn decode(bytes: &[u8], enc: Encoding) -> String {
    let utf16 = |be: bool| {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| {
                if be {
                    u16::from_be_bytes(c)
                } else {
                    u16::from_le_bytes(c)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match enc {
        Encoding::Utf8Bom => String::from_utf8_lossy(&bytes[3..]).into_owned(),
        Encoding::Utf16Le => utf16(false),
        Encoding::Utf16Be => utf16(true),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

pub fn line_endings(text: &str) -> LineEndings {
    let (mut crlf, mut lf, mut cr) = (0, 0, 0);
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    match (crlf > 0, lf > 0, cr > 0) {
        (false, false, false) => LineEndings::None,
        (true, false, false) => LineEndings::Crlf,
        (false, true, false) => LineEndings::Lf,
        (false, false, true) => LineEndings::Cr,
        _ => LineEndings::Mixed,
    }
}

/// How the lines of `text` are indented. For spaces, the step most lines
/// go in or out by (2, 4, …).
pub fn indent(text: &str) -> Indent {
    let (mut tabs, mut spaces) = (0usize, 0usize);
    // How often each change in indentation between lines occurs.
    let mut steps = [0usize; 9];
    let mut prev = 0usize;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if line.starts_with('\t') {
            tabs += 1;
            continue;
        }
        let n = line.len() - line.trim_start_matches(' ').len();
        if n > 0 {
            spaces += 1;
        }
        let step = n.abs_diff(prev);
        if (1..steps.len()).contains(&step) {
            steps[step] += 1;
        }
        prev = n;
    }
    if tabs == 0 && spaces == 0 {
        return Indent::None;
    }
    // A few lines of the other kind (e.g. a pasted block) don't count.
    if spaces * 10 < tabs {
        return Indent::Tabs;
    }
    if tabs * 10 < spaces {
        let width = (1..steps.len())
            .rev()
            .max_by_key(|&w| steps[w])
            .filter(|&w| steps[w] > 0)
            .unwrap_or(4);
        return Indent::Spaces(width);
    }
    Indent::Mixed
}

/// SHA-256 and MD5 of a file, as lowercase hex, read once.
pub fn checksums(path: &Path) -> io::Result<(String, String)> {
    use md5::Digest as _;
    let mut file = std::fs::File::open(path)?;
    let mut sha = sha2::Sha256::new();
    let mut md5 = md5::Md5::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
        md5.update(&buf[..n]);
    }
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok((hex(&sha.finalize()), hex(&md5.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings() {
        assert_eq!(encoding(b"plain"), Encoding::Ascii);
        assert_eq!(encoding("Grüße".as_bytes()), Encoding::Utf8);
        assert_eq!(encoding(b"\xEF\xBB\xBFhi"), Encoding::Utf8Bom);
        assert_eq!(encoding(b"Gr\xfc\xdfe"), Encoding::Other);
        // Cut off in the middle of "ü".
        assert_eq!(encoding(&"Grü".as_bytes()[..3]), Encoding::Utf8);
        let utf16 = b"\xFF\xFEh\0i\0";
        assert_eq!(encoding(utf16), Encoding::Utf16Le);
        assert_eq!(decode(utf16, Encoding::Utf16Le), "hi");
        assert_eq!(decode(b"\xEF\xBB\xBFhi", Encoding::Utf8Bom), "hi");
    }

    #[test]
    fn line_ending_styles() {
        assert_eq!(line_endings("a\r\nb\r\n"), LineEndings::Crlf);
        assert_eq!(line_endings("a\nb\n"), LineEndings::Lf);
        assert_eq!(line_endings("a\rb"), LineEndings::Cr);
        assert_eq!(line_endings("a\r\nb\n"), LineEndings::Mixed);
        assert_eq!(line_endings("one line"), LineEndings::None);
    }

    #[test]
    fn indentation() {
        assert_eq!(
            indent("fn a() {\n    x();\n    if y {\n        z();\n    }\n}\n"),
            Indent::Spaces(4)
        );
        assert_eq!(indent("a:\n  b:\n    c: 1\n  d: 2\n"), Indent::Spaces(2));
        assert_eq!(indent("a {\n\tb;\n\t\tc;\n}\n"), Indent::Tabs);
        assert_eq!(indent("a\n\tb\n  c\n"), Indent::Mixed);
        assert_eq!(indent("a\nb\n"), Indent::None);
    }

    #[test]
    fn file_checksums() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.txt");
        std::fs::write(&path, "abc").unwrap();
        let (sha, md5) = checksums(&path).unwrap();
        assert_eq!(
            sha,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(md5, "900150983cd24fb0d6963f7d28e17f72");
    }
}
