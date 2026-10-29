//! Tab group operations.

use super::*;

impl Tabs {
    pub fn new_group(&mut self, tab: TabId) -> GroupId {
        let gid = self.alloc_id();
        let color = (self.groups.len() + 1) % GROUP_COLORS.len();
        self.groups.push(TabGroup {
            id: gid,
            name: String::new(),
            color,
            collapsed: false,
        });
        self.add_to_group(tab, gid);
        // (The Home tab can't join one.)
        self.drop_empty_groups();
        gid
    }

    /// Add a tab to a group, moving it next to the group's other tabs.
    pub fn add_to_group(&mut self, tab: TabId, gid: GroupId) {
        let Some(from) = self.index_of(tab).filter(|&i| i > 0) else {
            return;
        };
        let mut t = self.tabs.remove(from);
        t.group = Some(gid);
        let insert_at = self
            .tabs
            .iter()
            .rposition(|x| x.group == Some(gid))
            .map(|i| i + 1)
            .unwrap_or(from.min(self.tabs.len()))
            .max(1);
        self.tabs.insert(insert_at, t);
        self.drop_empty_groups();
    }

    pub fn remove_from_group(&mut self, tab: TabId) {
        let Some(from) = self.index_of(tab) else {
            return;
        };
        let Some(gid) = self.tabs[from].group else {
            return;
        };
        let mut t = self.tabs.remove(from);
        t.group = None;
        // Place it right after the group.
        let insert_at = self
            .tabs
            .iter()
            .rposition(|x| x.group == Some(gid))
            .map(|i| i + 1)
            .unwrap_or(from.min(self.tabs.len()));
        self.tabs.insert(insert_at, t);
        self.drop_empty_groups();
    }

    pub fn ungroup(&mut self, gid: GroupId) {
        for t in &mut self.tabs {
            if t.group == Some(gid) {
                t.group = None;
            }
        }
        self.groups.retain(|g| g.id != gid);
    }

    pub fn close_group(&mut self, gid: GroupId) {
        let ids: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|t| t.group == Some(gid))
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.close(id);
        }
        self.groups.retain(|g| g.id != gid);
    }

    pub fn toggle_group(&mut self, gid: GroupId) {
        let collapsed = match self.group_mut(gid) {
            Some(g) => {
                g.collapsed = !g.collapsed;
                g.collapsed
            }
            None => return,
        };
        // If the active tab disappears into the collapsed group, switch to a visible tab.
        if collapsed && self.active().group == Some(gid) {
            if let Some(t) = self.tabs.iter().find(|t| t.group != Some(gid)) {
                self.active = t.id;
            } else if let Some(g) = self.group_mut(gid) {
                g.collapsed = false;
            }
        }
    }

    pub(super) fn drop_empty_groups(&mut self) {
        let tabs = &self.tabs;
        self.groups
            .retain(|g| tabs.iter().any(|t| t.group == Some(g.id)));
    }
}
