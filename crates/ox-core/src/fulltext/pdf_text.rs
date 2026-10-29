//! Text of PDF files, with the pure-Rust `pdf-extract` crate (Windows'
//! own PDF reader in [`crate::pdf`] only draws pages, it has no text).

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// The text of a PDF of at most `limit` bytes; `None` if it can't be read
/// (damaged, encrypted, scanned pages without text, too large).
pub(super) fn text(path: &Path, limit: u64) -> Option<String> {
    let mut data = Vec::new();
    File::open(path)
        .ok()?
        .take(limit.saturating_add(1))
        .read_to_end(&mut data)
        .ok()?;
    if data.len() as u64 > limit {
        return None;
    }
    // The parser may panic on odd files: that costs this file, not the app.
    let text = crate::logfile::catch_panic(|| pdf_extract::extract_text_from_mem(&data))?.ok()?;
    (!text.trim().is_empty()).then_some(text)
}
