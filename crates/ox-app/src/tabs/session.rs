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
                        Location::Search { root: Some(r), .. } => Location::Folder(r.clone()),
                        Location::Search { root: None, .. } => Location::Home,
                        other => other.clone(),
                    },
                    group: t.group,
                })
                .collect(),
            groups: self.groups.clone(),
            active: self.index_of(self.active).unwrap_or(0),
        }
    }

    pub fn from_session(session: Session, settings: &Settings) -> Option<Self> {
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
            next_id: 1,
        };
        tabs.next_id = tabs.groups.iter().map(|g| g.id + 1).max().unwrap_or(1);
        for st in session.tabs {
            let loc = match st.location {
                Location::Folder(p) if !ox_core::network::is_dir(&p) => Location::Home,
                other => other,
            };
            let id = tabs.alloc_id();
            let mut tab = Tab::new(id, loc, settings);
            tab.group = st.group.filter(|g| tabs.groups.iter().any(|x| x.id == *g));
            tabs.tabs.push(tab);
        }
        tabs.drop_empty_groups();
        let active = session.active.min(tabs.tabs.len() - 1);
        tabs.active = tabs.tabs[active].id;
        Some(tabs)
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
