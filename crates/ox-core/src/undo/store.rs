//! Keeping the undo history across restarts, in `undo_history.json` in the
//! config folder: at most [`LIMIT`] operations on each stack, none older
//! than [`MAX_AGE_DAYS`].
//!
//! Every window has its own history; only the window that owns the session
//! loads the saved one when it starts. Several windows write the same file,
//! so a save merges: each record has an id, and the window remembers which
//! ids were its own when it last loaded or saved (the *base*). Records of
//! other windows are kept, records this window took off its stacks (undone,
//! redone) are left out, and records of this window's base that another
//! window took off (it undid them too) are dropped here as well, so they
//! can't come back. Whatever is loaded is still checked before it is undone
//! (see [`super::reverse`]): what changed since can't be undone.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::history::now_ms;
use super::{History, Record, LIMIT};

/// Operations older than this are forgotten.
pub const MAX_AGE_DAYS: i64 = 30;

/// The file: both stacks, oldest first. Records are read one by one, so a
/// record that can't be read (written by a newer version) doesn't cost the
/// others.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    undo: Vec<Value>,
    #[serde(default)]
    redo: Vec<Value>,
}

/// The stacks as read from the file.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Stacks {
    pub undo: Vec<Record>,
    pub redo: Vec<Record>,
}

pub struct Store {
    path: PathBuf,
    /// Ids of this window's records when it last loaded or saved.
    base: HashSet<u64>,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            base: HashSet::new(),
        }
    }

    /// `undo_history.json` in the config folder.
    pub fn default_path() -> PathBuf {
        crate::config_dir().join("undo_history.json")
    }

    /// The saved history (empty if there is none); its records count as
    /// this window's from now on.
    pub fn load(&mut self) -> History {
        let stacks = read(&self.path).unwrap_or_default();
        let stacks = merge(&HashSet::new(), &History::default(), Some(stacks), now_ms());
        let history = History::from_records(stacks.undo, stacks.redo);
        self.base = ids(&history);
        history
    }

    /// Write `history`, merged with what other windows wrote. Records that
    /// another window took off its stacks since are dropped from `history`.
    pub fn save(&mut self, history: &mut History) -> io::Result<()> {
        let disk = read(&self.path);
        if let Some(disk) = &disk {
            let on_disk: HashSet<u64> = disk.undo.iter().chain(&disk.redo).map(|r| r.id).collect();
            let base = &self.base;
            history.retain(|r| !base.contains(&r.id) || on_disk.contains(&r.id));
        }
        let stacks = merge(&self.base, history, disk, now_ms());
        write(&self.path, &stacks)?;
        self.base = ids(history);
        Ok(())
    }

    /// Forget the saved history (the setting was turned off).
    pub fn remove(&mut self) -> io::Result<()> {
        self.base.clear();
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn ids(history: &History) -> HashSet<u64> {
    let (undo, redo) = history.records();
    undo.chain(redo).map(|r| r.id).collect()
}

/// The stacks in the file; `None` if it is missing or unreadable.
pub(super) fn read(path: &Path) -> Option<Stacks> {
    let text = fs::read_to_string(path).ok()?;
    let saved: Saved = serde_json::from_str(&text).ok()?;
    let records = |values: Vec<Value>| -> Vec<Record> {
        values
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect()
    };
    Some(Stacks {
        undo: records(saved.undo),
        redo: records(saved.redo),
    })
}

fn write(path: &Path, stacks: &Stacks) -> io::Result<()> {
    // Record by record: one that can't be written (a path that isn't valid
    // Unicode) is left out instead of failing the whole file.
    let values = |records: &[Record]| -> Vec<Value> {
        records
            .iter()
            .filter_map(|r| serde_json::to_value(r).ok())
            .collect()
    };
    let saved = Saved {
        undo: values(&stacks.undo),
        redo: values(&stacks.redo),
    };
    let text = serde_json::to_string(&saved).map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

/// This window's records (`ours`) with those of other windows on `disk`
/// (the ones not in `base`: those this window had and took off are left
/// out), oldest first, without records older than [`MAX_AGE_DAYS`] and at
/// most [`LIMIT`] on each stack. Records of the same millisecond keep the
/// order they were saved in: the other windows' first, they are on disk.
pub(super) fn merge(base: &HashSet<u64>, ours: &History, disk: Option<Stacks>, now: i64) -> Stacks {
    let mine = ids(ours);
    let disk = disk.unwrap_or_default();
    let oldest = now - MAX_AGE_DAYS * 24 * 60 * 60 * 1000;
    let stack = |own: Vec<Record>, other: Vec<Record>| -> Vec<Record> {
        let mut all: Vec<Record> = other
            .into_iter()
            .filter(|r| !base.contains(&r.id) && !mine.contains(&r.id))
            .collect();
        all.extend(own);
        all.retain(|r| r.at >= oldest);
        all.sort_by_key(|r| r.at);
        if all.len() > LIMIT {
            all.drain(..all.len() - LIMIT);
        }
        all
    };
    let (undo, redo) = ours.records();
    Stacks {
        undo: stack(undo.cloned().collect(), disk.undo),
        redo: stack(redo.cloned().collect(), disk.redo),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{reverse, Bin, Entry, Kind, Op, Placed};
    use super::*;

    fn folder(name: &str) -> Entry {
        Entry::new(Kind::NewFolder, Op::NewFolder(PathBuf::from(name)))
    }

    fn subjects(h: &History) -> (Vec<String>, Vec<String>) {
        let (undo, redo) = h.records();
        (
            undo.map(|r| r.entry.subject.clone()).collect(),
            redo.map(|r| r.entry.subject.clone()).collect(),
        )
    }

    fn on_disk(path: &Path) -> (Vec<String>, Vec<String>) {
        let s = read(path).unwrap();
        let names = |r: Vec<Record>| r.into_iter().map(|r| r.entry.subject).collect();
        (names(s.undo), names(s.redo))
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    struct NoBin;
    impl Bin for NoBin {
        fn delete(&self, _: &[PathBuf]) -> Result<(), String> {
            Err("no bin".into())
        }
        fn restore(&self, _: &[PathBuf], _: i64) -> Result<(), String> {
            Err("no bin".into())
        }
    }

    #[test]
    fn round_trip_keeps_both_stacks_and_their_checks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("undo.json");
        let file = dir.path().join("copy.txt");
        fs::write(&file, "copied").unwrap();
        let mut h = History::default();
        h.record(Entry::new(
            Kind::Copy,
            Op::Placed(vec![Placed::now(file.clone()).unwrap()]),
        ));
        h.record(Entry::new(
            Kind::Delete,
            Op::Trashed {
                paths: vec![dir.path().join("gone.txt")],
                since: 1_700_000_000,
            },
        ));
        h.record(Entry::new(
            Kind::Rename,
            Op::Renamed(vec![("a".into(), "b".into())]),
        ));
        let e = h.take(false).unwrap();
        h.reversed(false, e);
        assert!(h.take_changed());
        let mut store = Store::new(path.clone());
        store.save(&mut h).unwrap();

        let loaded = Store::new(path.clone()).load();
        let (a, b) = (h.records(), loaded.records());
        assert_eq!(a.0.collect::<Vec<_>>(), b.0.collect::<Vec<_>>());
        assert_eq!(a.1.collect::<Vec<_>>(), b.1.collect::<Vec<_>>());
        assert_eq!(loaded.peek(true).unwrap().kind, Kind::Rename);
        let Some(Entry {
            op: Op::Trashed { since, .. },
            ..
        }) = loaded.peek(false)
        else {
            panic!("the delete is next");
        };
        assert_eq!(*since, 1_700_000_000);

        // Undo is still refused when things changed since.
        fs::write(&file, "changed meanwhile").unwrap();
        let mut loaded = Store::new(path).load();
        loaded.take(false);
        let copy = loaded.take(false).unwrap();
        let err = reverse(&copy.op, &NoBin).unwrap_err();
        assert!(err.contains("changed"), "{err}");
    }

    #[test]
    fn keeps_the_newest_and_forgets_old_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("undo.json");
        let now = now_ms();
        let day = 24 * 60 * 60 * 1000;
        let mut records: Vec<Record> = (0..LIMIT + 30)
            .map(|i| Record {
                at: now - 1000 + i as i64,
                ..Record::new(folder(&i.to_string()))
            })
            .collect();
        records.insert(
            0,
            Record {
                at: now - (MAX_AGE_DAYS + 1) * day,
                ..Record::new(folder("ancient"))
            },
        );
        let stacks = Stacks {
            undo: records,
            redo: Vec::new(),
        };
        write(&path, &stacks).unwrap();
        let mut store = Store::new(path.clone());
        let mut h = store.load();
        let (undo, _) = subjects(&h);
        assert_eq!(undo.len(), LIMIT);
        assert_eq!(undo[0], "30");
        assert!(!undo.contains(&"ancient".to_owned()));
        h.record(folder("new"));
        store.save(&mut h).unwrap();
        let (undo, _) = on_disk(&path);
        assert_eq!(undo.len(), LIMIT);
        assert_eq!(undo.last().unwrap(), "new");
        // A file that isn't one: an empty history.
        fs::write(&path, "not json").unwrap();
        assert!(!Store::new(path).load().can_undo());
    }

    #[test]
    fn windows_keep_each_others_operations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("undo.json");
        let (mut a, mut b) = (Store::new(path.clone()), Store::new(path.clone()));
        let mut ha = a.load();
        let mut hb = History::default(); // a window that didn't load it

        ha.record(folder("a1"));
        a.save(&mut ha).unwrap();
        hb.record(folder("b1"));
        b.save(&mut hb).unwrap();
        // B's window still only undoes its own.
        assert_eq!(subjects(&hb).0, ["b1"]);
        assert_eq!(on_disk(&path).0, ["a1", "b1"]);

        // A undoes a1: it moves to A's redo stack; B's b1 stays.
        let e = ha.take(false).unwrap();
        ha.reversed(false, e);
        a.save(&mut ha).unwrap();
        assert_eq!(on_disk(&path), (names(&["b1"]), names(&["a1"])));
        // B saves again without having seen that: a1 doesn't come back.
        hb.record(folder("b2"));
        b.save(&mut hb).unwrap();
        assert_eq!(on_disk(&path), (names(&["b1", "b2"]), names(&["a1"])));

        // After a restart, the main window has all of it.
        let h = Store::new(path).load();
        assert_eq!(subjects(&h), (names(&["b1", "b2"]), names(&["a1"])));
    }

    #[test]
    fn records_of_the_same_millisecond_keep_the_order_they_were_saved_in() {
        let now = now_ms();
        let at = |name| Record {
            at: now,
            ..Record::new(folder(name))
        };
        let disk = Stacks {
            undo: vec![at("other")],
            redo: Vec::new(),
        };
        let ours = History::from_records(vec![at("mine")], Vec::new());
        let stacks = merge(&HashSet::new(), &ours, Some(disk), now);
        let undo: Vec<_> = stacks.undo.into_iter().map(|r| r.entry.subject).collect();
        assert_eq!(undo, ["other", "mine"]);
    }

    #[test]
    fn an_operation_undone_in_another_window_is_gone_here_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("undo.json");
        let mut first = Store::new(path.clone());
        let mut h = History::default();
        h.record(folder("x"));
        h.record(folder("y"));
        first.save(&mut h).unwrap();
        // Two windows that both loaded it.
        let (mut a, mut b) = (Store::new(path.clone()), Store::new(path.clone()));
        let (mut ha, mut hb) = (a.load(), b.load());
        ha.take(false); // A undid y
        a.save(&mut ha).unwrap();
        hb.record(folder("z"));
        b.save(&mut hb).unwrap();
        assert_eq!(subjects(&hb).0, ["x", "z"], "y left B's history too");
        assert_eq!(subjects(&Store::new(path.clone()).load()).0, ["x", "z"]);

        // The file was removed (setting off): nothing is dropped, all written again.
        b.remove().unwrap();
        assert!(!path.exists());
        b.save(&mut hb).unwrap();
        assert_eq!(subjects(&Store::new(path).load()).0, ["x", "z"]);
    }
}
