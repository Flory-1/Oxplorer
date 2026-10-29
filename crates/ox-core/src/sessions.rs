//! Named sessions ("workspaces"): sets of tabs saved under a name, to open
//! again later. They live in `sessions.toml` in the config folder.
//!
//! What a session holds (tabs, groups, …) is up to the app, so the store is
//! generic over it. Several Oxplorer windows may change the file, so every
//! change reads it again first ([`SessionStore::update`]) instead of writing
//! a copy that may be stale.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedSession<T> {
    pub name: String,
    pub session: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStore<T> {
    #[serde(default = "Vec::new")]
    pub sessions: Vec<NamedSession<T>>,
}

impl<T> Default for SessionStore<T> {
    fn default() -> Self {
        Self {
            sessions: Vec::new(),
        }
    }
}

/// Why a name can't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    /// Another session already has this name.
    Taken,
    /// There's no session with the old name (any more).
    Missing,
}

impl NameError {
    /// What to tell the user (English; the caller translates it).
    pub fn message(self) -> &'static str {
        match self {
            NameError::Empty => "Enter a name.",
            NameError::Taken => "A session with this name already exists.",
            NameError::Missing => "This session doesn't exist any more.",
        }
    }
}

/// Names are compared without surrounding spaces and case ("Work" = "work ").
fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

impl<T> SessionStore<T> {
    pub fn get(&self, name: &str) -> Option<&NamedSession<T>> {
        self.sessions.iter().find(|s| same(&s.name, name))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Save `session` under `name`, replacing a session of the same name
    /// (in its place in the list), or adding it at the end.
    pub fn put(&mut self, name: &str, session: T) -> Result<(), NameError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NameError::Empty);
        }
        match self.sessions.iter_mut().find(|s| same(&s.name, name)) {
            Some(s) => {
                s.name = name.to_owned();
                s.session = session;
            }
            None => self.sessions.push(NamedSession {
                name: name.to_owned(),
                session,
            }),
        }
        Ok(())
    }

    /// Rename a session. Changing only the case of its name is fine.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), NameError> {
        let new = new.trim();
        if new.is_empty() {
            return Err(NameError::Empty);
        }
        let i = self
            .sessions
            .iter()
            .position(|s| same(&s.name, old))
            .ok_or(NameError::Missing)?;
        if self
            .sessions
            .iter()
            .enumerate()
            .any(|(j, s)| j != i && same(&s.name, new))
        {
            return Err(NameError::Taken);
        }
        self.sessions[i].name = new.to_owned();
        Ok(())
    }

    /// Remove a session; `false` if there was none of that name.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.sessions.len();
        self.sessions.retain(|s| !same(&s.name, name));
        self.sessions.len() != before
    }

    /// A name for a new session that isn't taken yet: `base`, else
    /// "`base` 2", "`base` 3", …
    pub fn free_name(&self, base: &str) -> String {
        let base = base.trim();
        if !self.contains(base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|n| !self.contains(n))
            .expect("some number is free")
    }
}

impl<T: Serialize + DeserializeOwned> SessionStore<T> {
    pub fn path() -> PathBuf {
        crate::config_dir().join("sessions.toml")
    }

    /// The saved sessions; none if the file is missing or unreadable.
    pub fn load(path: &Path) -> Self {
        crate::settings::load_toml(path).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        crate::settings::save_toml(path, self)
    }

    /// Read the file again, change it and write it back; returns the store
    /// as written, or the reason it couldn't be changed.
    pub fn update<R>(
        path: &Path,
        change: impl FnOnce(&mut Self) -> Result<R, NameError>,
    ) -> Result<(Self, R), UpdateError> {
        let mut store = Self::load(path);
        let r = change(&mut store).map_err(UpdateError::Name)?;
        store.save(path).map_err(UpdateError::Io)?;
        Ok((store, r))
    }
}

/// Why [`SessionStore::update`] failed.
#[derive(Debug)]
pub enum UpdateError {
    Name(NameError),
    Io(io::Error),
}

/// When a file was last changed (to notice that another window wrote it).
pub fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
    struct Fake {
        tabs: Vec<String>,
        active: usize,
    }

    fn fake(tabs: &[&str]) -> Fake {
        Fake {
            tabs: tabs.iter().map(|s| s.to_string()).collect(),
            active: 0,
        }
    }

    #[test]
    fn put_replaces_by_name_and_keeps_order() {
        let mut s = SessionStore::default();
        s.put("Work", fake(&["a"])).unwrap();
        s.put("Photos", fake(&["b"])).unwrap();
        assert_eq!(s.put("  ", fake(&[])), Err(NameError::Empty));
        // Same name, other case and spaces: replaced where it was.
        s.put(" work ", fake(&["c"])).unwrap();
        let names: Vec<_> = s.sessions.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["work", "Photos"]);
        assert_eq!(s.get("WORK").unwrap().session, fake(&["c"]));
    }

    #[test]
    fn rename_and_remove() {
        let mut s = SessionStore::default();
        s.put("Work", fake(&["a"])).unwrap();
        s.put("Photos", fake(&["b"])).unwrap();
        assert_eq!(s.rename("Work", "photos"), Err(NameError::Taken));
        assert_eq!(s.rename("Work", ""), Err(NameError::Empty));
        assert_eq!(s.rename("Nope", "X"), Err(NameError::Missing));
        s.rename("Work", "WORK").unwrap();
        s.rename("WORK", "Project X").unwrap();
        assert!(s.contains("project x"));
        assert!(!s.contains("Work"));
        assert!(s.remove("PHOTOS"));
        assert!(!s.remove("Photos"));
        assert_eq!(s.sessions.len(), 1);
    }

    #[test]
    fn free_names() {
        let mut s = SessionStore::default();
        assert_eq!(s.free_name("Session"), "Session");
        s.put("Session", fake(&[])).unwrap();
        s.put("Session 2", fake(&[])).unwrap();
        assert_eq!(s.free_name("Session"), "Session 3");
    }

    #[test]
    fn update_reads_what_others_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.toml");
        assert!(SessionStore::<Fake>::load(&path).sessions.is_empty());
        let (mine, ()) = SessionStore::update(&path, |s| s.put("A", fake(&["x"]))).unwrap();
        assert_eq!(mine.sessions.len(), 1);
        // Another window adds one; ours must not drop it when it saves.
        let (_, ()) = SessionStore::update(&path, |s| s.put("B", fake(&["y"]))).unwrap();
        let (after, ()) = SessionStore::update(&path, |s| s.put("C", fake(&["z"]))).unwrap();
        let names: Vec<_> = after.sessions.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "C"]);
        // A failed change doesn't write.
        let err = SessionStore::<Fake>::update(&path, |s| s.rename("A", "b"));
        assert!(matches!(err, Err(UpdateError::Name(NameError::Taken))));
        let (removed, gone) = SessionStore::<Fake>::update(&path, |s| Ok(s.remove("B"))).unwrap();
        assert!(gone);
        assert_eq!(SessionStore::<Fake>::load(&path).sessions.len(), 2);
        assert_eq!(removed.sessions.len(), 2);
    }
}
