//! Pinned tabs: drawn small, always first (right after the Home tab), not
//! in a group and not closed by "Close other tabs" or by closing a group.

use super::*;

impl Tabs {
    /// Index of the first tab that isn't pinned. The pinned tabs are the
    /// ones from index 1 up to here.
    pub fn pinned_end(&self) -> usize {
        1 + self.tabs.iter().skip(1).take_while(|t| t.pinned).count()
    }

    pub fn is_pinned(&self, id: TabId) -> bool {
        self.get(id).is_some_and(|t| t.pinned)
    }

    /// Pin a tab: it leaves its group and goes after the other pinned tabs.
    pub fn pin(&mut self, id: TabId) {
        let Some(from) = self.index_of(id).filter(|&i| i > 0) else {
            return;
        };
        if self.tabs[from].pinned {
            return;
        }
        let mut tab = self.tabs.remove(from);
        tab.pinned = true;
        tab.group = None;
        let at = self.pinned_end();
        self.tabs.insert(at, tab);
        self.drop_empty_groups();
    }

    /// Unpin a tab: it becomes the first of the other tabs.
    pub fn unpin(&mut self, id: TabId) {
        let Some(from) = self.index_of(id).filter(|&i| i > 0) else {
            return;
        };
        if !self.tabs[from].pinned {
            return;
        }
        let mut tab = self.tabs.remove(from);
        tab.pinned = false;
        let at = self.pinned_end();
        self.tabs.insert(at, tab);
    }

    /// Where a tab moved to `target` may go: pinned tabs stay among the
    /// pinned ones, the others after them (`tab` is already taken out).
    pub(super) fn clamp_target(&self, pinned: bool, target: usize) -> usize {
        let end = self.pinned_end();
        if pinned {
            target.clamp(1, end)
        } else {
            target.clamp(end, self.tabs.len())
        }
    }

    /// Put the Home tab first, then the pinned tabs, then the others
    /// (keeping their order), and take pinned tabs out of groups.
    pub(super) fn sort_pinned(&mut self) {
        let Some((home, rest)) = self.tabs.split_first_mut() else {
            return;
        };
        home.pinned = false;
        rest.sort_by_key(|t| !t.pinned);
        for t in rest.iter_mut().filter(|t| t.pinned) {
            t.group = None;
        }
        self.drop_empty_groups();
    }
}
