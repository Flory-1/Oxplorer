//! Where the NTFS change journals were when the index was saved, so the
//! next start can catch up on what changed while Oxplorer was closed
//! instead of rebuilding (see `catchup`).
//!
//! Saved next to the index cache (`index.journal` beside `index.bin`) with
//! the size and modification time of the index file it belongs to: if the
//! index file was written without it (or replaced), the checkpoints are
//! ignored. The index is always saved first, so a checkpoint is never newer
//! than the index it describes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAGIC: &[u8; 8] = b"OXJRN\0\0\x01";

/// Changes up to `usn` in the journal `journal` of drive `drive` are in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Checkpoint {
    pub drive: char,
    pub journal: u64,
    pub usn: i64,
}

/// The checkpoints of an index, and the excluded names it was built with
/// (catching up applies them to new items).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct JournalState {
    pub checkpoints: Vec<Checkpoint>,
    pub exclude: Vec<String>,
}

impl JournalState {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn get(&self, drive: char) -> Option<Checkpoint> {
        self.checkpoints.iter().copied().find(|c| c.drive == drive)
    }

    /// Add or move a drive's checkpoint.
    pub fn set(&mut self, cp: Checkpoint) {
        match self.checkpoints.iter_mut().find(|c| c.drive == cp.drive) {
            Some(c) => *c = cp,
            None => self.checkpoints.push(cp),
        }
    }
}

pub(super) fn path_for(cache: &Path) -> PathBuf {
    cache.with_extension("journal")
}

/// Identifies one version of the index file.
fn stamp(cache: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(cache).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos() as u64))
}

fn encode(state: &JournalState, stamp: (u64, u64)) -> Vec<u8> {
    let mut b = MAGIC.to_vec();
    b.extend_from_slice(&stamp.0.to_le_bytes());
    b.extend_from_slice(&stamp.1.to_le_bytes());
    b.extend_from_slice(&(state.exclude.len() as u32).to_le_bytes());
    for e in &state.exclude {
        b.extend_from_slice(&(e.len() as u32).to_le_bytes());
        b.extend_from_slice(e.as_bytes());
    }
    b.extend_from_slice(&(state.checkpoints.len() as u32).to_le_bytes());
    for c in &state.checkpoints {
        b.push(c.drive as u8);
        b.extend_from_slice(&c.journal.to_le_bytes());
        b.extend_from_slice(&c.usn.to_le_bytes());
    }
    b
}

fn decode(b: &[u8]) -> Option<((u64, u64), JournalState)> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = b.get(at..at.checked_add(n)?)?;
        at += n;
        Some(s)
    };
    if take(8)? != MAGIC {
        return None;
    }
    let u64_ = |s: &[u8]| u64::from_le_bytes(s.try_into().unwrap());
    let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
    let stamp = (u64_(take(8)?), u64_(take(8)?));
    let n = u32_(take(4)?) as usize;
    if n > 1024 {
        return None;
    }
    let mut exclude = Vec::with_capacity(n);
    for _ in 0..n {
        let len = u32_(take(4)?) as usize;
        if len > 4096 {
            return None;
        }
        exclude.push(String::from_utf8(take(len)?.to_vec()).ok()?);
    }
    let n = u32_(take(4)?) as usize;
    if n > 26 {
        return None;
    }
    let mut state = JournalState {
        checkpoints: Vec::with_capacity(n),
        exclude,
    };
    for _ in 0..n {
        let drive = take(1)?[0];
        if !drive.is_ascii_uppercase() || state.get(drive as char).is_some() {
            return None;
        }
        state.checkpoints.push(Checkpoint {
            drive: drive as char,
            journal: u64_(take(8)?),
            usn: u64_(take(8)?) as i64,
        });
    }
    (at == b.len()).then_some((stamp, state))
}

/// Save the checkpoints of the index just saved at `cache` (or remove
/// them when there are none).
pub(super) fn save(cache: &Path, state: &JournalState) -> io::Result<()> {
    let path = path_for(cache);
    let Some(stamp) = stamp(cache).filter(|_| !state.checkpoints.is_empty()) else {
        remove(cache);
        return Ok(());
    };
    let tmp = path.with_extension("journal-tmp");
    fs::write(&tmp, encode(state, stamp))?;
    fs::rename(&tmp, &path)
}

/// The checkpoints saved with the index at `cache`, if they belong to it.
pub(super) fn load(cache: &Path) -> Option<JournalState> {
    let bytes = fs::read(path_for(cache)).ok()?;
    let (saved, state) = decode(&bytes)?;
    (Some(saved) == stamp(cache)).then_some(state)
}

pub(super) fn remove(cache: &Path) {
    let _ = fs::remove_file(path_for(cache));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> JournalState {
        let mut s = JournalState {
            checkpoints: Vec::new(),
            exclude: vec!["node_modules".into(), "größe".into()],
        };
        s.set(Checkpoint {
            drive: 'C',
            journal: 0x01DA_0000_1234,
            usn: 5_000_000,
        });
        s.set(Checkpoint {
            drive: 'D',
            journal: 7,
            usn: 0,
        });
        s.set(Checkpoint {
            drive: 'C',
            journal: 0x01DA_0000_1234,
            usn: 6_000_000,
        });
        s
    }

    #[test]
    fn set_replaces_a_drives_checkpoint() {
        let s = state();
        assert_eq!(s.checkpoints.len(), 2);
        assert_eq!(s.get('C').unwrap().usn, 6_000_000);
        assert!(s.get('E').is_none());
    }

    #[test]
    fn saved_with_the_index_it_belongs_to() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("index.bin");
        // No index file: nothing to belong to.
        save(&cache, &state()).unwrap();
        assert!(load(&cache).is_none());

        fs::write(&cache, b"index v1").unwrap();
        save(&cache, &state()).unwrap();
        assert_eq!(load(&cache), Some(state()));

        // The index was written again without its checkpoints.
        fs::write(&cache, b"index v2, longer").unwrap();
        assert!(load(&cache).is_none());

        // No checkpoints: the file goes away.
        save(&cache, &state()).unwrap();
        assert!(load(&cache).is_some());
        save(&cache, &JournalState::default()).unwrap();
        assert!(!path_for(&cache).exists());
    }

    #[test]
    fn rejects_damaged_files() {
        let good = encode(&state(), (1, 2));
        assert_eq!(decode(&good), Some(((1, 2), state())));
        for cut in 0..good.len() {
            assert!(decode(&good[..cut]).is_none(), "cut at {cut}");
        }
        let mut extra = good.clone();
        extra.push(0);
        assert!(decode(&extra).is_none());
        let mut bad = good.clone();
        bad[0] = b'X';
        assert!(decode(&bad).is_none());
        // A lowercase or repeated drive letter.
        let drive_at = good.len() - 2 * 17;
        let mut bad = good.clone();
        bad[drive_at] = b'c';
        assert!(decode(&bad).is_none());
        let mut bad = good.clone();
        bad[drive_at + 17] = b'C';
        assert!(decode(&bad).is_none());
        // Absurd counts.
        let mut bad = MAGIC.to_vec();
        bad.extend([0; 16]);
        bad.extend(u32::MAX.to_le_bytes());
        assert!(decode(&bad).is_none());
    }
}
