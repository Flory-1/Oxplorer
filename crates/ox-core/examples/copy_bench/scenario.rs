//! Test data: folders of files with incompressible (pseudo-random) content.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Many small files in nested folders.
    Small,
    /// One large file.
    Large,
    /// Small, medium and one big file.
    Mixed,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Small, Kind::Large, Kind::Mixed];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Small => "small",
            Kind::Large => "large",
            Kind::Mixed => "mixed",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }
}

/// How big the generated scenarios are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizes {
    pub small_count: u64,
    pub small_size: u64,
    pub large_size: u64,
}

impl Default for Sizes {
    fn default() -> Self {
        Self {
            small_count: 10_000,
            small_size: 4 * 1024,
            large_size: 1024 * 1024 * 1024,
        }
    }
}

/// A generated folder `root` (named after the scenario) with its totals.
#[derive(Debug, Clone)]
pub struct Scenario {
    pub kind: Kind,
    pub root: PathBuf,
    pub files: u64,
    pub bytes: u64,
}

/// Fast xorshift generator; the data only has to defeat compression and dedup.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }

    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

const CHUNK: usize = 4 * 1024 * 1024;

fn write_file(path: &Path, size: u64, rng: &mut Rng, buf: &mut Vec<u8>) -> io::Result<()> {
    let mut f = fs::File::create(path)?;
    let mut left = size;
    while left > 0 {
        let n = left.min(CHUNK as u64) as usize;
        buf.resize(n, 0);
        rng.fill(buf);
        f.write_all(buf)?;
        left -= n as u64;
    }
    Ok(())
}

/// Path of the `i`-th small file: 100 files per folder, two folder levels.
fn small_path(root: &Path, i: u64) -> PathBuf {
    let folder = i / 100;
    root.join(format!("{:03}", folder / 10))
        .join(format!("{:03}", folder % 10))
        .join(format!("f{i:06}.bin"))
}

/// Write `count` files with sizes from `size` into nested folders under `root`.
fn small_tree(
    root: &Path,
    count: u64,
    mut size: impl FnMut(&mut Rng) -> u64,
    rng: &mut Rng,
    buf: &mut Vec<u8>,
) -> io::Result<u64> {
    let mut bytes = 0;
    for i in 0..count {
        let path = small_path(root, i);
        if i % 100 == 0 {
            fs::create_dir_all(path.parent().unwrap())?;
        }
        let n = size(rng);
        write_file(&path, n, rng, buf)?;
        bytes += n;
    }
    Ok(bytes)
}

/// Generate the scenario as `parent/<name>`.
pub fn generate(kind: Kind, parent: &Path, sizes: &Sizes) -> io::Result<Scenario> {
    let root = parent.join(kind.name());
    fs::create_dir_all(&root)?;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ kind as u64);
    let mut buf = Vec::new();
    let (files, bytes) = match kind {
        Kind::Small => {
            let bytes = small_tree(
                &root,
                sizes.small_count,
                |_| sizes.small_size,
                &mut rng,
                &mut buf,
            )?;
            (sizes.small_count, bytes)
        }
        Kind::Large => {
            write_file(
                &root.join("large.bin"),
                sizes.large_size,
                &mut rng,
                &mut buf,
            )?;
            (1, sizes.large_size)
        }
        Kind::Mixed => {
            // A fifth of the small files (1 KB to 256 KB), 16 medium files
            // (1 to 16 MB for a 1 GB large size) and one file a quarter of the large size.
            let small = (sizes.small_count / 5).max(1);
            let mut bytes = small_tree(
                &root.join("small"),
                small,
                |r| r.range(1024, 256 * 1024),
                &mut rng,
                &mut buf,
            )?;
            let medium = root.join("medium");
            fs::create_dir_all(&medium)?;
            for i in 0..16u64 {
                let n = (i + 1) * (sizes.large_size / 1024).max(1);
                write_file(&medium.join(format!("m{i:02}.bin")), n, &mut rng, &mut buf)?;
                bytes += n;
            }
            let big = sizes.large_size / 4;
            write_file(&root.join("big.bin"), big, &mut rng, &mut buf)?;
            bytes += big;
            (small + 17, bytes)
        }
    };
    Ok(Scenario {
        kind,
        root,
        files,
        bytes,
    })
}

/// Count files and bytes below `root` (to check that a copy is complete).
pub fn totals(root: &Path) -> io::Result<(u64, u64)> {
    let mut files = 0;
    let mut bytes = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    Ok((files, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_totals_match() {
        let tmp = tempfile::tempdir().unwrap();
        let sizes = Sizes {
            small_count: 150,
            small_size: 100,
            large_size: 64 * 1024,
        };
        for kind in Kind::ALL {
            let s = generate(kind, tmp.path(), &sizes).unwrap();
            assert_eq!(totals(&s.root).unwrap(), (s.files, s.bytes), "{kind:?}");
        }
    }
}
