//! A temporary copy of some bytes on disk, for things that need a path
//! (an SQLite database inside an archive): deleted when dropped.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Names of the copies start with this (in the temp folder).
const PREFIX: &str = "oxplorer-preview-";

/// A file in the temp folder, deleted when this is dropped.
#[derive(Debug)]
pub struct TempCopy {
    path: PathBuf,
}

impl TempCopy {
    /// Write `bytes` to a new file in the temp folder, named like `name`
    /// (its extension is kept).
    pub fn write(bytes: &[u8], name: &str) -> io::Result<Self> {
        Self::write_in(&std::env::temp_dir(), bytes, name)
    }

    pub fn write_in(dir: &Path, bytes: &[u8], name: &str) -> io::Result<Self> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let ext = Path::new(name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        for _ in 0..100 {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos());
            let path = dir.join(format!("{PREFIX}{}-{n}-{nanos:x}{ext}", std::process::id()));
            let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            };
            let copy = Self { path };
            file.write_all(bytes)?;
            return Ok(copy);
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no free temporary name",
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_and_deleted_when_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let a = TempCopy::write_in(dir.path(), b"hello", "data.sqlite").unwrap();
        let b = TempCopy::write_in(dir.path(), b"other", "data.sqlite").unwrap();
        assert_ne!(a.path(), b.path());
        assert_eq!(fs::read(a.path()).unwrap(), b"hello");
        assert_eq!(a.path().extension().unwrap(), "sqlite");
        let path = a.path().to_path_buf();
        drop(a);
        assert!(!path.exists());
        assert!(b.path().exists());
    }
}
