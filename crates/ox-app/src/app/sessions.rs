//! Named sessions: save the open tabs under a name and open them again later
//! (stored by `ox_core::sessions` in `sessions.toml`).

use super::*;
use ox_core::sessions::{NamedSession, SessionStore, UpdateError};
use std::time::SystemTime;

pub type Store = SessionStore<Session>;

pub enum SessionAction {
    /// Ask for a name and save the open tabs under it.
    SaveAs,
    /// Save the open tabs under this name (replacing a session of that name).
    Save(String),
    /// Open a session: instead of the open tabs, or (`true`) next to them.
    Open(String, bool),
    /// Ask for a new name for this session.
    Rename(String),
    RenameTo(String, String),
    Delete(String),
}

/// The saved sessions as last read, read again when another window changed them.
#[derive(Default)]
pub struct SavedSessions {
    store: Store,
    /// When the file was changed when it was read (`None`: not read yet).
    read: Option<Option<SystemTime>>,
}

impl SavedSessions {
    pub fn list(&self) -> &[NamedSession<Session>] {
        &self.store.sessions
    }

    pub fn contains(&self, name: &str) -> bool {
        self.store.contains(name)
    }

    /// Read the file if it changed since (a cheap check, fine every frame
    /// a menu with the sessions is open).
    pub fn refresh(&mut self) {
        let path = Store::path();
        let stamp = ox_core::sessions::modified(&path);
        if self.read != Some(stamp) {
            self.store = Store::load(&path);
            self.read = Some(stamp);
        }
    }

    fn update<R>(
        &mut self,
        change: impl FnOnce(&mut Store) -> Result<R, ox_core::sessions::NameError>,
    ) -> Result<R, String> {
        let path = Store::path();
        match Store::update(&path, change) {
            Ok((store, r)) => {
                self.store = store;
                self.read = Some(ox_core::sessions::modified(&path));
                Ok(r)
            }
            Err(UpdateError::Name(e)) => Err(tr(e.message()).to_owned()),
            Err(UpdateError::Io(e)) => Err(ox_core::trf!(
                "Couldn't save the sessions: {e}",
                e = e.to_string()
            )),
        }
    }
}

impl OxApp {
    pub(super) fn process_session(&mut self, action: SessionAction) {
        match action {
            SessionAction::SaveAs => {
                self.sessions.refresh();
                self.dialog = Some(Dialog::SessionName {
                    old: None,
                    text: String::new(),
                    focus: true,
                });
            }
            SessionAction::Save(name) => {
                let session = self.tabs.to_session();
                let name = name.trim().to_owned();
                match self.sessions.update(|s| s.put(&name, session)) {
                    Ok(()) => {
                        self.toast(ox_core::trf!("Saved the tabs as \"{name}\".", name = name))
                    }
                    Err(e) => self.toast(e),
                }
            }
            SessionAction::Open(name, add) => {
                self.sessions.refresh();
                let Some(session) = self.sessions.store.get(&name).map(|s| s.session.clone())
                else {
                    self.toast(tr("This session doesn't exist any more."));
                    return;
                };
                if add {
                    self.tabs.add_session(session, &self.settings);
                } else {
                    self.compare = None;
                    self.tabs.replace_with(session, &self.settings);
                }
            }
            SessionAction::Rename(name) => {
                self.sessions.refresh();
                self.dialog = Some(Dialog::SessionName {
                    text: name.clone(),
                    old: Some(name),
                    focus: true,
                });
            }
            SessionAction::RenameTo(old, new) => {
                if let Err(e) = self.sessions.update(|s| s.rename(&old, &new)) {
                    self.toast(e);
                }
            }
            SessionAction::Delete(name) => match self.sessions.update(|s| Ok(s.remove(&name))) {
                Ok(_) => self.toast(ox_core::trf!(
                    "Deleted the session \"{name}\".",
                    name = name
                )),
                Err(e) => self.toast(e),
            },
        }
    }
}
