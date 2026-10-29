//! Saving and restoring the open tabs between runs.

use super::*;

impl Tabs {
    pub fn to_session(&self) -> Session {
        Session {
            tabs: self
                .tabs
                .iter()
                .map(|t| SessionTab {
                    location: match t.location() {
                        // Search results aren't worth restoring; reopen the folder instead.
                        Location::Search { root: Some(r), .. } => Location::search_root(r.clone()),
                        Location::Search { root: None, .. } => Location::Home,
                        other => other.clone(),
                    },
                    group: t.group,
                    pinned: t.pinned,
                })
                .collect(),
            groups: self.groups.clone(),
            active: self.index_of(self.active).unwrap_or(0),
        }
    }

    pub fn from_session(session: Session, settings: &Settings) -> Option<Self> {
        Self::from_session_at(session, settings, 1)
    }

    /// Like [`Self::from_session`], with ids from `first_id` on (so they
    /// differ from those of the tabs this replaces).
    fn from_session_at(session: Session, settings: &Settings, first_id: u64) -> Option<Self> {
        if session.tabs.is_empty() {
            return None;
        }
        // Sessions from before the Home tab was fixed may not start with it.
        let mut session = session;
        let starts_home = session
            .tabs
            .first()
            .is_some_and(|t| t.location == Location::Home && t.group.is_none());
        if !starts_home {
            session.tabs.insert(
                0,
                SessionTab {
                    location: Location::Home,
                    group: None,
                    pinned: false,
                },
            );
            session.active += 1;
        }
        let mut tabs = Tabs {
            tabs: Vec::new(),
            groups: session.groups,
            active: 0,
            panes: None,
            focus: 0,
            next_id: first_id,
        };
        // Group ids are kept, so tab ids come after them.
        let after_groups = tabs.groups.iter().map(|g| g.id + 1).max().unwrap_or(1);
        tabs.next_id = after_groups.max(first_id);
        for st in session.tabs {
            let loc = match st.location {
                Location::Folder(p) if !ox_core::network::is_dir(&p) => Location::Home,
                other => other,
            };
            let id = tabs.alloc_id();
            let mut tab = Tab::new(id, loc, settings);
            tab.group = st.group.filter(|g| tabs.groups.iter().any(|x| x.id == *g));
            tab.pinned = st.pinned;
            tabs.tabs.push(tab);
        }
        let active = session.active.min(tabs.tabs.len() - 1);
        tabs.active = tabs.tabs[active].id;
        tabs.sort_pinned();
        Some(tabs)
    }

    /// Show a saved session instead of the open tabs.
    pub fn replace_with(&mut self, session: Session, settings: &Settings) {
        if let Some(new) = Self::from_session_at(session, settings, self.next_id) {
            *self = new;
        }
    }

    /// Open the tabs of a saved session next to the open ones (without its
    /// Home tab), and switch to its active tab.
    pub fn add_session(&mut self, session: Session, settings: &Settings) {
        let Some(other) = Self::from_session_at(session, settings, self.next_id) else {
            return;
        };
        self.next_id = self.next_id.max(other.next_id);
        // Its groups get new ids, so they don't mix with ours.
        let mut ids = std::collections::HashMap::new();
        for g in other.groups {
            let id = self.alloc_id();
            ids.insert(g.id, id);
            self.groups.push(TabGroup { id, ..g });
        }
        let mut active = None;
        for mut t in other.tabs.into_iter().skip(1) {
            if t.id == other.active {
                active = Some(t.id);
            }
            t.group = t.group.and_then(|g| ids.get(&g).copied());
            if t.pinned {
                let at = self.pinned_end();
                self.tabs.insert(at, t);
            } else {
                self.tabs.push(t);
            }
        }
        self.drop_empty_groups();
        if let Some(id) = active {
            self.activate(id);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Session {
    pub tabs: Vec<SessionTab>,
    pub groups: Vec<TabGroup>,
    pub active: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTab {
    pub location: Location,
    pub group: Option<GroupId>,
    #[serde(default)]
    pub pinned: bool,
}

impl Session {
    pub fn path() -> PathBuf {
        ox_core::config_dir().join("session.toml")
    }

    pub fn load() -> Option<Self> {
        ox_core::settings::load_toml(&Self::path())
    }

    pub fn save(&self) -> io::Result<()> {
        ox_core::settings::save_toml(&Self::path(), self)
    }
}
