//! Equal names share one place in the names buffer. A drive has many files
//! with the same name (`index.js`, `README.md`, the parts of every
//! installed update), typically half of them: on a whole system drive this
//! keeps a few dozen MB free. Readers don't notice: an entry's name is
//! still the bytes at its offset and length.
//!
//! Done after a build, when compacting, and when an index saved without it
//! is loaded. Names added by live updates are appended as they come and
//! share again at the next compaction.

use std::hash::BuildHasher;

use hashbrown::{DefaultHashBuilder, HashTable};

use super::FileIndex;

impl FileIndex {
    /// Does any name share its place with another (done before)?
    pub(super) fn names_shared(&self) -> bool {
        let used: u64 = self.entries.iter().map(|e| u64::from(e.name_len)).sum();
        used > self.names.len() as u64
    }

    /// Let equal names share one place, in place: entries are visited in
    /// the order of their names in the buffer, and each new name is moved
    /// to the front part that is already done (it never overtakes the part
    /// still to read).
    pub(super) fn share_names(&mut self) {
        let mut order: Vec<u32> = (0..self.entries.len() as u32).collect();
        order.sort_unstable_by_key(|&i| self.entries[i as usize].name_off);
        // In place only works if names are apart or the same place (always
        // so, unless the file was damaged): else leave them as they are.
        let mut end = 0usize;
        let mut last = None;
        for &i in &order {
            let e = &self.entries[i as usize];
            let range = (e.name_off as usize, e.name_len as usize);
            if last != Some(range) && range.0 < end {
                return;
            }
            end = end.max(range.0 + range.1);
            last = Some(range);
        }
        let mut bytes = std::mem::take(&mut self.names).into_bytes();
        let hasher = DefaultHashBuilder::default();
        // (offset, length) of each different name in the part already done.
        let mut table: HashTable<(u32, u16)> = HashTable::new();
        let mut done = 0usize;
        for i in order {
            let e = &mut self.entries[i as usize];
            let (off, len) = (e.name_off as usize, e.name_len as usize);
            let hash = hasher.hash_one(&bytes[off..off + len]);
            let found = table
                .find(hash, |&(o, l)| {
                    bytes[o as usize..o as usize + l as usize] == bytes[off..off + len]
                })
                .copied();
            e.name_off = match found {
                Some((o, _)) => o,
                None => {
                    bytes.copy_within(off..off + len, done);
                    let o = done as u32;
                    done += len;
                    table.insert_unique(hash, (o, len as u16), |&(o, l)| {
                        hasher.hash_one(&bytes[o as usize..o as usize + l as usize])
                    });
                    o
                }
            };
        }
        drop(table);
        bytes.truncate(done);
        bytes.shrink_to_fit();
        // Whole names (each valid UTF-8) put one after another.
        self.names = String::from_utf8(bytes).expect("names stay valid UTF-8");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64};

    use super::super::IndexConfig;
    use super::*;

    #[test]
    fn records_are_20_bytes_and_keep_sizes() {
        use super::super::Entry;
        assert_eq!(std::mem::size_of::<Entry>(), 20);
        let mut e = Entry::default();
        for size in [0, 5000, u32::MAX as u64, 7 << 32 | 12345, Entry::MAX_SIZE] {
            e.set_size(size);
            assert_eq!(e.size(), size);
        }
        e.set_size(u64::MAX);
        assert_eq!(e.size(), Entry::MAX_SIZE, "larger than 1 TiB: capped");
    }

    #[test]
    fn equal_names_share_and_paths_stay() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("r");
        for d in ["a", "b", "c/deeper"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
            std::fs::write(root.join(d).join("README.md"), "x").unwrap();
            std::fs::write(root.join(d).join("index.js"), "x").unwrap();
        }
        std::fs::write(root.join("ünïcode-only-once.txt"), "x").unwrap();
        let cfg = IndexConfig {
            roots: vec![root.clone()],
            exclude: Vec::new(),
            threads: 1,
        };
        let mut idx = FileIndex::build(&cfg, &AtomicBool::new(false), &AtomicU64::new(0));
        assert!(idx.names_shared(), "a build shares names");
        let paths = |idx: &FileIndex| {
            let mut v: Vec<_> = (0..idx.entries.len() as u32)
                .map(|i| idx.path_of(i))
                .collect();
            v.sort();
            v
        };
        let before = paths(&idx);
        let used: usize = idx.entries.iter().map(|e| e.name_len as usize).sum();
        assert!(idx.names.len() < used);

        // Sharing again changes nothing; a saved and loaded index keeps it.
        let len = idx.names.len();
        idx.share_names();
        assert_eq!(idx.names.len(), len);
        assert_eq!(paths(&idx), before);
        let file = dir.path().join("index.bin");
        idx.save(&file).unwrap();
        let loaded = FileIndex::load(&file).unwrap();
        assert_eq!(loaded.names.len(), len);
        assert_eq!(paths(&loaded), before);
    }
}
