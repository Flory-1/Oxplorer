//! Catching up on what changed while Oxplorer was closed. The index was
//! saved with the change journal position of each drive (`checkpoint`); on
//! start, if each journal is still the same and still holds that position,
//! the records since are read and applied like live changes (`patch`)
//! instead of rebuilding. Otherwise the index is rebuilt as before.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use super::checkpoint::{Checkpoint, JournalState};
use super::helper::proto::JournalInfo;
use super::patch::Batch;
use super::service::Shared;
use super::usn::{signals_for, Dirs, UsnRecord};

/// Why reading the journal failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadError {
    /// The journal was recreated or the position was overwritten: changes are lost.
    Gone,
    /// Couldn't be read this time.
    Failed,
}

/// A drive's change journal (read directly or through the index service).
pub(super) trait JournalSource {
    fn info(&mut self) -> Option<JournalInfo>;
    /// Records from `start` on, and where to continue.
    fn read(&mut self, journal: u64, start: i64) -> Result<(i64, Vec<UsnRecord>), ReadError>;
    /// The current path of a file reference, if it still exists.
    fn path_of_id(&mut self, frn: u64) -> Option<PathBuf>;
}

/// Can reading resume at `cp`: same journal, position still in it?
pub(super) fn resumes(info: &JournalInfo, cp: &Checkpoint) -> bool {
    cp.journal == info.journal && info.first <= cp.usn && cp.usn <= info.next
}

/// Where to start following a journal: at the checkpoint when it is still
/// good, else at the end (nothing to catch up on, or it can't be).
pub(super) fn start_at(info: &JournalInfo, cp: Option<&Checkpoint>) -> i64 {
    match cp {
        Some(cp) if resumes(info, cp) => cp.usn,
        _ => info.next,
    }
}

/// Read the changes since `cp` into `batch`. Returns the new checkpoint, or
/// `None` if the changes can't be caught up on (journal gone, unreadable, or
/// so much changed that rebuilding is cheaper).
pub(super) fn catch_up_drive(
    src: &mut dyn JournalSource,
    cp: Checkpoint,
    prefixes: &[(String, PathBuf)],
    batch: &mut Batch,
) -> Option<Checkpoint> {
    let info = src.info()?;
    if !resumes(&info, &cp) {
        return None;
    }
    let mut dirs = Dirs::default();
    let mut usn = cp.usn;
    // Up to where the journal was when catching up started; what comes
    // after is read by live updates (they start from the new checkpoint).
    while usn < info.next {
        let (next, records) = src.read(info.journal, usn).ok()?;
        if next <= usn {
            break;
        }
        let signals = signals_for(
            &records,
            &mut dirs,
            &mut |frn| src.path_of_id(frn),
            prefixes,
        );
        for s in signals {
            batch.push(s);
        }
        if batch.needs_rescan() {
            return None;
        }
        usn = next;
    }
    Some(Checkpoint {
        drive: cp.drive,
        journal: info.journal,
        usn,
    })
}

#[cfg(windows)]
fn read_all(roots: &[PathBuf], state: &JournalState) -> Option<(Batch, Vec<Checkpoint>)> {
    let (by_drive, rest) = super::journal::group_by_drive(roots);
    // Every root must be covered (a USB stick or network drive isn't): the
    // index is rebuilt anyway otherwise.
    if !rest.is_empty() || by_drive.iter().any(|(l, _)| state.get(*l).is_none()) {
        return None;
    }
    let mut batch = Batch::default();
    let mut done = Vec::new();
    for (letter, roots) in by_drive {
        let cp = state.get(letter)?;
        let mut journal = super::journal::open(letter)?;
        let prefixes = super::usn::prefixes(&roots);
        done.push(catch_up_drive(&mut journal, cp, &prefixes, &mut batch)?);
    }
    Some((batch, done))
}

#[cfg(not(windows))]
fn read_all(_roots: &[PathBuf], _state: &JournalState) -> Option<(Batch, Vec<Checkpoint>)> {
    None
}

/// Bring the loaded index up to date from the change journals. True when
/// every root was caught up, so the index is as good as a fresh build.
pub(super) fn run(shared: &Shared, state: &JournalState) -> bool {
    let Some(roots) = shared
        .index
        .read()
        .unwrap()
        .as_ref()
        .map(|i| i.roots.clone())
    else {
        return false;
    };
    let started = Instant::now();
    let Some((batch, done)) = read_all(&roots, state) else {
        log::info!("index: can't catch up on changes from the change journal");
        return false;
    };
    let exclude: Vec<String> = state.exclude.iter().map(|e| e.to_lowercase()).collect();
    let changed = super::live::apply(shared, &batch, &exclude);
    {
        let mut journal = shared.journal.lock().unwrap();
        for cp in done {
            journal.set(cp);
        }
    }
    if let Some(idx) = shared.index.write().unwrap().as_mut() {
        Arc::make_mut(idx).built = SystemTime::now();
    }
    log::info!(
        "index: caught up on {changed} changes from the change journal in {:.1?}",
        started.elapsed()
    );
    true
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicU64};

    use super::super::usn::{REASON_FILE_DELETE, REASON_RENAME_NEW_NAME, REASON_RENAME_OLD_NAME};
    use super::super::{FileIndex, IndexConfig, F_DELETED};
    use super::*;

    const JOURNAL: u64 = 0xABCD;
    const CREATE: u32 = 0x100;
    const DIR: u32 = 0x10;

    /// A journal of synthetic records, `per_read` at a time.
    struct Fake {
        info: JournalInfo,
        records: Vec<UsnRecord>,
        paths: HashMap<u64, PathBuf>,
        per_read: usize,
        reads: usize,
    }

    impl JournalSource for Fake {
        fn info(&mut self) -> Option<JournalInfo> {
            Some(self.info)
        }

        fn read(&mut self, journal: u64, start: i64) -> Result<(i64, Vec<UsnRecord>), ReadError> {
            self.reads += 1;
            if journal != self.info.journal || start < self.info.first {
                return Err(ReadError::Gone);
            }
            let out: Vec<_> = self
                .records
                .iter()
                .filter(|r| r.usn >= start)
                .take(self.per_read)
                .cloned()
                .collect();
            let next = out.last().map_or(self.info.next, |r| r.usn + 8);
            Ok((next, out))
        }

        fn path_of_id(&mut self, frn: u64) -> Option<PathBuf> {
            self.paths.get(&frn).cloned()
        }
    }

    fn rec(usn: i64, frn: u64, parent: u64, reason: u32, attrs: u32, name: &str) -> UsnRecord {
        UsnRecord {
            frn,
            parent,
            usn,
            reason,
            attrs,
            name: name.into(),
        }
    }

    fn names(idx: &FileIndex) -> Vec<String> {
        let mut v: Vec<String> = (0..idx.entries.len() as u32)
            .filter(|&i| idx.entries[i as usize].flags & F_DELETED == 0)
            .map(|i| idx.path_of(i).to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn build(root: &Path) -> FileIndex {
        let cfg = IndexConfig {
            roots: vec![root.to_path_buf()],
            exclude: vec![],
            threads: 2,
        };
        FileIndex::build(&cfg, &AtomicBool::new(false), &AtomicU64::new(0))
    }

    #[test]
    fn resumes_only_in_the_same_journal_and_range() {
        let info = JournalInfo {
            journal: JOURNAL,
            first: 1000,
            next: 5000,
        };
        let cp = |journal, usn| Checkpoint {
            drive: 'C',
            journal,
            usn,
        };
        assert!(resumes(&info, &cp(JOURNAL, 1000)));
        assert!(resumes(&info, &cp(JOURNAL, 5000)));
        assert!(!resumes(&info, &cp(JOURNAL, 999)), "overwritten");
        assert!(!resumes(&info, &cp(JOURNAL, 5001)), "from the future");
        assert!(!resumes(&info, &cp(JOURNAL + 1, 2000)), "recreated");
        assert_eq!(start_at(&info, Some(&cp(JOURNAL, 2000))), 2000);
        assert_eq!(start_at(&info, Some(&cp(1, 2000))), 5000);
        assert_eq!(start_at(&info, None), 5000);
    }

    /// Changes made "while closed", described by synthetic journal records,
    /// patch the old index into the same as a fresh build.
    #[test]
    fn catching_up_matches_a_fresh_build() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("r");
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("old-folder/inner")).unwrap();
        fs::write(root.join("docs/a.txt"), "a").unwrap();
        fs::write(root.join("old-folder/inner/b.txt"), "b").unwrap();
        fs::write(root.join("gone.txt"), "x").unwrap();
        let mut idx = build(&root);

        // While closed: a new file, a deleted file, a renamed folder with a
        // new file inside, and a file created and deleted again.
        fs::write(root.join("docs/new.txt"), "new").unwrap();
        fs::remove_file(root.join("gone.txt")).unwrap();
        fs::rename(root.join("old-folder"), root.join("new-folder")).unwrap();
        fs::write(root.join("new-folder/inner/c.txt"), "c").unwrap();

        // The journal spells paths in full (long names), as the prefixes do.
        let prefixes = super::super::usn::prefixes(std::slice::from_ref(&root));
        let long = PathBuf::from(prefixes[0].0.trim_end_matches(std::path::MAIN_SEPARATOR));
        // File references: 1 = root, 2 = docs, 3 = the renamed folder, 4 = inner.
        let paths = HashMap::from([
            (1, long.clone()),
            (2, long.join("docs")),
            (3, long.join("new-folder")),
            (4, long.join("new-folder").join("inner")),
        ]);
        let records = vec![
            rec(100, 10, 2, CREATE, 0, "new.txt"),
            rec(108, 11, 1, REASON_FILE_DELETE, 0, "gone.txt"),
            rec(116, 3, 1, REASON_RENAME_OLD_NAME, DIR, "old-folder"),
            rec(124, 3, 1, REASON_RENAME_NEW_NAME, DIR, "new-folder"),
            rec(132, 12, 4, CREATE, 0, "c.txt"),
            rec(140, 13, 2, CREATE, 0, "temp.tmp"),
            rec(148, 13, 2, REASON_FILE_DELETE, 0, "temp.tmp"),
            // Outside the root and in a folder that no longer exists.
            rec(156, 14, 99, CREATE, 0, "elsewhere.txt"),
        ];
        let mut src = Fake {
            info: JournalInfo {
                journal: JOURNAL,
                first: 50,
                next: 164,
            },
            records,
            paths,
            per_read: 3,
            reads: 0,
        };
        let cp = Checkpoint {
            drive: 'C',
            journal: JOURNAL,
            usn: 100,
        };
        let mut batch = Batch::default();
        let done = catch_up_drive(&mut src, cp, &prefixes, &mut batch).unwrap();
        assert_eq!(done.usn, 164);
        assert_eq!(src.reads, 3, "read in pieces up to the end");
        assert!(!batch.needs_rescan());

        idx.update(&batch, &[]);
        assert_eq!(names(&idx), names(&build(&root)));
        assert!(names(&idx).iter().any(|n| n.ends_with("c.txt")));
    }

    #[test]
    fn gives_up_when_the_journal_moved_on() {
        let mut src = Fake {
            info: JournalInfo {
                journal: JOURNAL,
                first: 500,
                next: 900,
            },
            records: Vec::new(),
            paths: HashMap::new(),
            per_read: 10,
            reads: 0,
        };
        let cp = |journal, usn| Checkpoint {
            drive: 'D',
            journal,
            usn,
        };
        let mut batch = Batch::default();
        // Overwritten, recreated: rebuild instead.
        assert!(catch_up_drive(&mut src, cp(JOURNAL, 100), &[], &mut batch).is_none());
        assert!(catch_up_drive(&mut src, cp(7, 600), &[], &mut batch).is_none());
        assert_eq!(src.reads, 0);
        // Nothing changed: the checkpoint stays.
        let same = catch_up_drive(&mut src, cp(JOURNAL, 900), &[], &mut batch).unwrap();
        assert_eq!(same.usn, 900);
        assert!(batch.is_empty());
    }

    #[test]
    fn gives_up_when_too_much_changed() {
        let root = std::env::temp_dir().join("ox-catchup-flood");
        let records: Vec<_> = (0..100_002)
            .map(|i| rec(i * 8, 1000 + i as u64, 1, CREATE, 0, &format!("f{i}")))
            .collect();
        let next = records.len() as i64 * 8;
        let mut src = Fake {
            info: JournalInfo {
                journal: JOURNAL,
                first: 0,
                next,
            },
            records,
            paths: HashMap::from([(1, root.clone())]),
            per_read: 50_000,
            reads: 0,
        };
        let prefixes = vec![(
            format!("{}{}", root.display(), std::path::MAIN_SEPARATOR),
            root,
        )];
        let cp = Checkpoint {
            drive: 'C',
            journal: JOURNAL,
            usn: 0,
        };
        let mut batch = Batch::default();
        assert!(catch_up_drive(&mut src, cp, &prefixes, &mut batch).is_none());
    }
}
