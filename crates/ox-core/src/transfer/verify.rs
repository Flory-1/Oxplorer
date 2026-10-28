//! Optional verification after copying: compares xxh3-128 hashes of the
//! source and the copy.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use xxhash_rust::xxh3::Xxh3;

use super::copy::CopyError;

/// Hash a file in `buf`-sized chunks. `checkpoint` returns `false` to cancel.
fn hash_file(
    path: &Path,
    buf: &mut [u8],
    checkpoint: &dyn Fn() -> bool,
) -> Result<u128, CopyError> {
    let mut file = File::open(path)?;
    let mut hasher = Xxh3::new();
    loop {
        let n = match file.read(buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        hasher.update(&buf[..n]);
        if !checkpoint() {
            return Err(CopyError::Cancelled);
        }
    }
    Ok(hasher.digest128())
}

/// `Ok(true)` if both files have identical contents.
pub(crate) fn same_contents(
    a: &Path,
    b: &Path,
    buffer_size: usize,
    checkpoint: &dyn Fn() -> bool,
) -> Result<bool, CopyError> {
    let (ma, mb) = (std::fs::metadata(a)?, std::fs::metadata(b)?);
    if ma.len() != mb.len() {
        return Ok(false);
    }
    let mut buf = vec![0u8; buffer_size.clamp(64 * 1024, 16 * 1024 * 1024)];
    Ok(hash_file(a, &mut buf, checkpoint)? == hash_file(b, &mut buf, checkpoint)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_equal_and_different_files() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (
            dir.path().join("a"),
            dir.path().join("b"),
            dir.path().join("c"),
        );
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&a, &data).unwrap();
        std::fs::write(&b, &data).unwrap();
        let mut other = data.clone();
        other[123_456] ^= 1;
        std::fs::write(&c, &other).unwrap();
        let go = || true;
        assert!(same_contents(&a, &b, 64 * 1024, &go).unwrap());
        assert!(!same_contents(&a, &c, 64 * 1024, &go).unwrap());
        assert!(matches!(
            same_contents(&a, &b, 64 * 1024, &|| false),
            Err(CopyError::Cancelled)
        ));
    }
}
