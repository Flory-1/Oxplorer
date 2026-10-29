//! Files that several Oxplorer windows write: settings, favorites and
//! recent places, tags.
//!
//! Each window keeps what it last read or wrote (the *base*). When the file
//! changed on disk since, a three-way merge keeps the other window's changes
//! and lays this window's own changes (what differs from the base) over
//! them, value by value. Two windows that change the very same value: the
//! one that saves last wins. Lists are single values (two windows adding to
//! the recent folders at the same moment: one of them wins), which is rare
//! since the windows look for changes every few seconds.

use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::de::DeserializeOwned;
use serde::Serialize;
use toml::{Table, Value};

pub struct Shared<T> {
    path: PathBuf,
    /// What this window last read or wrote.
    base: Option<Table>,
    /// When the file was changed when that was.
    stamp: Option<SystemTime>,
    _t: PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned> Shared<T> {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            base: None,
            stamp: None,
            _t: PhantomData,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the file (`None` if it's missing or unreadable).
    pub fn load(&mut self) -> Option<T> {
        self.stamp = crate::sessions::modified(&self.path);
        let table = read_table(&self.path);
        self.base = table.clone();
        table.and_then(|t| t.try_into().ok())
    }

    /// Write `ours`, merged with what another window wrote since we last
    /// looked. Returns the merged value when it differs from `ours`.
    pub fn save(&mut self, ours: &T) -> io::Result<Option<T>> {
        let mine = to_table(ours)?;
        let (table, merged) = match self.disk_changes() {
            Some(disk) => {
                let merged = merge(self.base.as_ref(), &mine, &disk);
                let changed = merged != mine;
                (merged, changed)
            }
            None => (mine, false),
        };
        // Written from the value itself when possible: keeps its order of keys.
        if merged {
            crate::settings::save_toml(&self.path, &table)?;
        } else {
            crate::settings::save_toml(&self.path, ours)?;
        }
        self.stamp = crate::sessions::modified(&self.path);
        self.base = Some(table.clone());
        Ok(if merged { table.try_into().ok() } else { None })
    }

    /// If another window changed the file: `ours` with those changes (our
    /// own changes since the last save are kept).
    pub fn poll(&mut self, ours: &T) -> Option<T> {
        let disk = self.disk_changes()?;
        let mine = to_table(ours).ok()?;
        let merged = merge(self.base.as_ref(), &mine, &disk);
        self.stamp = crate::sessions::modified(&self.path);
        self.base = Some(disk);
        (merged != mine).then(|| merged.try_into().ok()).flatten()
    }

    /// The file as on disk, if it was changed since we last read or wrote it.
    fn disk_changes(&self) -> Option<Table> {
        let stamp = crate::sessions::modified(&self.path);
        if stamp.is_none() || stamp == self.stamp {
            return None;
        }
        read_table(&self.path)
    }
}

fn read_table(path: &Path) -> Option<Table> {
    let text = std::fs::read_to_string(path).ok()?;
    text.parse::<Table>().ok()
}

fn to_table<T: Serialize>(value: &T) -> io::Result<Table> {
    Table::try_from(value).map_err(io::Error::other)
}

/// Three-way merge: `disk`, with every value where `ours` differs from
/// `base` taken from `ours` (tables are merged key by key). Without a base,
/// everything counts as ours.
pub fn merge(base: Option<&Table>, ours: &Table, disk: &Table) -> Table {
    let Some(base) = base else {
        return ours.clone();
    };
    let mut out = disk.clone();
    for (key, mine) in ours {
        match (base.get(key), disk.get(key)) {
            (Some(b), _) if b == mine => {} // unchanged here: keep the disk's
            (Some(Value::Table(b)), Some(Value::Table(d))) => {
                if let Value::Table(m) = mine {
                    out.insert(key.clone(), Value::Table(merge(Some(b), m, d)));
                } else {
                    out.insert(key.clone(), mine.clone());
                }
            }
            _ => {
                out.insert(key.clone(), mine.clone());
            }
        }
    }
    // Removed here (and not changed there): remove.
    for (key, b) in base {
        if !ours.contains_key(key) && disk.get(key) == Some(b) {
            out.remove(key);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn table(text: &str) -> Table {
        text.parse().unwrap()
    }

    #[test]
    fn merging_keeps_both_sides() {
        let base = table("a = 1\nb = 1\nlist = [1]\n[t]\nx = 1\ny = 1\ngone = 1\n");
        let ours = table("a = 2\nb = 1\nlist = [1]\n[t]\nx = 2\ny = 1\n");
        let disk = table("a = 1\nb = 3\nlist = [1, 2]\nnew = 1\n[t]\nx = 1\ny = 3\ngone = 1\n");
        let m = merge(Some(&base), &ours, &disk);
        assert_eq!(
            m,
            table("a = 2\nb = 3\nlist = [1, 2]\nnew = 1\n[t]\nx = 2\ny = 3\n")
        );
        // Both changed the same value: ours.
        let disk = table("a = 5\n");
        let ours = table("a = 6\n");
        assert_eq!(merge(Some(&table("a = 1\n")), &ours, &disk), ours);
        // No base: ours.
        assert_eq!(merge(None, &ours, &disk), ours);
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
    #[serde(default)]
    struct Prefs {
        theme: String,
        hidden: bool,
        favorites: Vec<String>,
    }

    #[test]
    fn two_windows_change_different_things() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.toml");
        crate::settings::save_toml(&path, &Prefs::default()).unwrap();
        let mut a = Shared::<Prefs>::new(path.clone());
        let mut b = Shared::<Prefs>::new(path.clone());
        let mut pa = a.load().unwrap();
        let mut pb = b.load().unwrap();

        pa.theme = "dark".into();
        assert_eq!(a.save(&pa).unwrap(), None, "nothing to merge");
        // Make sure the file's time differs (coarse clocks).
        let t = std::fs::File::options().append(true).open(&path).unwrap();
        t.set_modified(SystemTime::now() + std::time::Duration::from_secs(2))
            .unwrap();

        // B hasn't seen it, changes something else and saves: A's theme stays.
        pb.hidden = true;
        let merged = b.save(&pb).unwrap().expect("merged with A's change");
        assert_eq!(merged.theme, "dark");
        assert!(merged.hidden);
        pb = merged;
        let on_disk: Prefs = crate::settings::load_toml(&path).unwrap();
        assert_eq!(on_disk, pb);

        // A picks B's change up, keeping its own unsaved one.
        let t = std::fs::File::options().append(true).open(&path).unwrap();
        t.set_modified(SystemTime::now() + std::time::Duration::from_secs(4))
            .unwrap();
        pa.favorites.push("C:\\".into());
        let polled = a.poll(&pa).expect("B's change");
        assert!(polled.hidden);
        assert_eq!(polled.favorites, ["C:\\"]);
        assert_eq!(a.poll(&polled), None, "nothing new");
        // Saving it doesn't lose the favorite (it's still a change of ours).
        assert_eq!(a.save(&polled).unwrap(), None);
        let on_disk: Prefs = crate::settings::load_toml(&path).unwrap();
        assert_eq!(on_disk, polled);
    }
}
