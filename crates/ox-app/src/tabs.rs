//! Tabs, tab groups and per-tab browsing state.

use ox_core::i18n::t as tr;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use ox_core::entry::{self, FileEntry, SortKey, SortOrder};
use ox_core::recycle::BinListing;
use ox_core::search::{Query, SearchHandle};
use ox_core::settings::{Settings, ViewMode};
use ox_core::tags::TagColor;
use serde::{Deserialize, Serialize};

pub type TabId = u64;
pub type GroupId = u64;

mod groups;
mod location;
mod session;
#[cfg(test)]
mod tests;

pub use location::{Listing, Location};
pub use session::Session;

/// Listing + selection state of the file view in one tab.
#[derive(Default)]
pub struct FolderView {
    /// All entries, sorted.
    pub entries: Vec<FileEntry>,
    /// Indices into `entries` that pass the quick filter, in display order.
    pub visible: Vec<usize>,
    pub loading: Option<Receiver<Result<Listing, String>>>,
    /// Trash items of the Recycle Bin view (empty for normal folders).
    pub bin: BinListing,
    pub error: Option<String>,
    pub selection: HashSet<PathBuf>,
    /// Position (in `visible`) of the keyboard cursor and the shift-select anchor.
    pub cursor: Option<usize>,
    pub anchor: Option<usize>,
    /// Scroll so that this visible row is on screen during the next frame.
    pub scroll_to: Option<usize>,
    /// After a reload, select this path (e.g. a freshly created folder).
    pub select_after_load: Option<PathBuf>,
    /// Bumped whenever a new listing arrives.
    pub version: u64,
    /// Git status of the repository this folder is in.
    pub git: Option<std::sync::Arc<ox_core::git::RepoStatus>>,
}

impl FolderView {
    pub fn visible_entry(&self, i: usize) -> Option<&FileEntry> {
        self.visible.get(i).and_then(|&e| self.entries.get(e))
    }

    pub fn selected_entries(&self) -> Vec<&FileEntry> {
        self.visible
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|e| self.selection.contains(&e.path))
            .collect()
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_entries()
            .into_iter()
            .map(|e| e.path.clone())
            .collect()
    }

    pub fn select_only(&mut self, i: usize) {
        self.selection.clear();
        if let Some(e) = self.visible_entry(i) {
            self.selection.insert(e.path.clone());
        }
        self.cursor = Some(i);
        self.anchor = Some(i);
    }

    pub fn toggle(&mut self, i: usize) {
        if let Some(p) = self.visible_entry(i).map(|e| e.path.clone()) {
            if !self.selection.remove(&p) {
                self.selection.insert(p);
            }
        }
        self.cursor = Some(i);
        self.anchor = Some(i);
    }

    pub fn select_range(&mut self, to: usize, additive: bool) {
        let from = self.anchor.unwrap_or(0);
        if !additive {
            self.selection.clear();
        }
        let (a, b) = if from <= to { (from, to) } else { (to, from) };
        for i in a..=b {
            if let Some(e) = self.visible_entry(i) {
                self.selection.insert(e.path.clone());
            }
        }
        self.cursor = Some(to);
    }

    pub fn select_all(&mut self) {
        self.selection = self
            .visible
            .iter()
            .map(|&i| self.entries[i].path.clone())
            .collect();
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.anchor = None;
    }
}

pub struct SearchState {
    pub handle: Option<SearchHandle>,
    pub done: Option<String>,
}

pub struct Tab {
    pub id: TabId,
    pub group: Option<GroupId>,
    history: Vec<Location>,
    pos: usize,
    pub view: FolderView,
    pub search: Option<SearchState>,
    /// Text in the search box; filters the current folder instantly.
    pub filter: String,
    pub view_mode: ViewMode,
    pub sort_key: SortKey,
    pub sort_order: SortOrder,
    /// Listing must be reloaded before the tab is shown again.
    pub stale: bool,
}

impl Tab {
    pub fn new(id: TabId, location: Location, settings: &Settings) -> Self {
        // The search box shows the query of a search tab.
        let filter = match &location {
            Location::Search { query, .. } => query.clone(),
            _ => String::new(),
        };
        Self {
            id,
            group: None,
            history: vec![location],
            pos: 0,
            view: FolderView::default(),
            search: None,
            filter,
            view_mode: settings.browsing.default_view,
            sort_key: settings.browsing.default_sort,
            sort_order: settings.browsing.default_order,
            stale: true,
        }
    }

    pub fn location(&self) -> &Location {
        &self.history[self.pos]
    }

    pub fn can_go_back(&self) -> bool {
        self.pos > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.pos + 1 < self.history.len()
    }

    pub fn navigate(&mut self, loc: Location) {
        if *self.location() == loc {
            self.stale = true;
            return;
        }
        self.history.truncate(self.pos + 1);
        self.history.push(loc);
        self.pos = self.history.len() - 1;
        self.on_location_changed();
    }

    pub fn back(&mut self) {
        if self.can_go_back() {
            let from = self.location().clone();
            self.pos -= 1;
            self.on_location_changed();
            self.select_after_return(&from);
        }
    }

    pub fn forward(&mut self) {
        if self.can_go_forward() {
            self.pos += 1;
            self.on_location_changed();
        }
    }

    pub fn up(&mut self) {
        let parent = match self.location() {
            // Up from a share's root goes to its server.
            Location::Folder(p) => ox_core::network::parent(p).map(Location::Folder),
            Location::Search { root, .. } => root.clone().map(Location::Folder),
            // Up from an archive's top level leaves the archive.
            Location::Archive { archive, inner } => Some(match inner.parent() {
                Some(up) => Location::Archive {
                    archive: archive.clone(),
                    inner: up.to_path_buf(),
                },
                None => Location::Folder(archive.parent().unwrap_or(archive).to_path_buf()),
            }),
            _ => None,
        };
        let from = self.location().clone();
        self.navigate(parent.unwrap_or(Location::Home));
        self.select_after_return(&from);
    }

    /// When going back/up, highlight the folder we came from, like Explorer.
    fn select_after_return(&mut self, from: &Location) {
        match from {
            Location::Folder(p) => self.view.select_after_load = Some(p.clone()),
            Location::Archive { archive, inner } => {
                self.view.select_after_load = Some(archive.join(inner));
            }
            _ => {}
        }
    }

    /// Replace the current location without adding a history entry
    /// (used while typing a search).
    pub fn replace(&mut self, loc: Location) {
        if *self.location() != loc {
            self.history[self.pos] = loc;
            self.on_location_changed();
        }
    }

    fn on_location_changed(&mut self) {
        self.view = FolderView::default();
        self.search = None;
        self.filter.clear();
        // The search box shows the query of a search tab.
        if let Location::Search { query, .. } = self.location() {
            self.filter = query.clone();
        }
        if *self.location() == Location::Gallery {
            // Like Windows' Gallery: large pictures, newest first.
            self.view_mode = ViewMode::Grid;
            self.sort_key = SortKey::Modified;
            self.sort_order = SortOrder::Descending;
        }
        self.stale = true;
    }

    pub fn title(&self) -> String {
        self.location().title()
    }

    /// Re-sort entries and re-apply the quick filter.
    pub fn refresh_visible(&mut self, folders_first: bool, case_sensitive: bool) {
        entry::sort_entries(
            &mut self.view.entries,
            self.sort_key,
            self.sort_order,
            folders_first,
        );
        let query = Query::parse(&self.filter, case_sensitive);
        let here = self.location();
        let filtering =
            (here.folder().is_some() || here.is_bin() || here.is_collection()) && !query.is_empty();
        self.view.visible = self
            .view
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !filtering || query.matches_name(&e.name))
            .map(|(i, _)| i)
            .collect();
        let max = self.view.visible.len();
        if self.view.cursor.is_some_and(|c| c >= max) {
            self.view.cursor = max.checked_sub(1);
        }
        if let Some(path) = self.view.select_after_load.take() {
            if let Some(i) = self
                .view
                .visible
                .iter()
                .position(|&e| self.view.entries[e].path == path)
            {
                self.view.select_only(i);
                self.view.scroll_to = Some(i);
            }
        }
    }
}

/// Group colours (Chrome-like palette).
pub const GROUP_COLORS: [(&str, [u8; 3]); 8] = [
    ("Grey", [0x8a, 0x8f, 0x98]),
    ("Blue", [0x4c, 0x8d, 0xf6]),
    ("Red", [0xe5, 0x48, 0x4d]),
    ("Yellow", [0xf0, 0xb4, 0x29]),
    ("Green", [0x3b, 0xb2, 0x73]),
    ("Pink", [0xe0, 0x5c, 0xb0]),
    ("Purple", [0x9b, 0x6b, 0xf2]),
    ("Cyan", [0x2c, 0xb5, 0xc8]),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabGroup {
    pub id: GroupId,
    pub name: String,
    pub color: usize,
    pub collapsed: bool,
}

/// The tabs. The first one is always the Home tab: it can't be closed,
/// moved or grouped, and what is opened from it opens in a new tab.
pub struct Tabs {
    pub tabs: Vec<Tab>,
    pub groups: Vec<TabGroup>,
    /// The tab in the focused pane; every command acts on it.
    pub active: TabId,
    /// Split view: the tabs in the left and right pane. One of them is `active`.
    pub panes: Option<[TabId; 2]>,
    /// Pane (0 = left, 1 = right) that holds the active tab.
    focus: usize,
    next_id: u64,
}

impl Tabs {
    /// The Home tab, and `first` in a tab of its own unless it's Home.
    pub fn new(first: Location, settings: &Settings) -> Self {
        let home = Tab::new(1, Location::Home, settings);
        let mut tabs = Self {
            active: home.id,
            tabs: vec![home],
            groups: Vec::new(),
            panes: None,
            focus: 0,
            next_id: 2,
        };
        if first != Location::Home {
            tabs.open(first, None, true, settings);
        }
        tabs
    }

    /// The fixed Home tab (always the first).
    pub fn home(&self) -> TabId {
        self.tabs[0].id
    }

    pub fn is_home(&self, id: TabId) -> bool {
        self.home() == id
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn active(&self) -> &Tab {
        self.tabs
            .iter()
            .find(|t| t.id == self.active)
            .unwrap_or(&self.tabs[0])
    }

    pub fn active_mut(&mut self) -> &mut Tab {
        let id = self.active;
        let idx = self.tabs.iter().position(|t| t.id == id).unwrap_or(0);
        &mut self.tabs[idx]
    }

    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn group(&self, id: GroupId) -> Option<&TabGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut TabGroup> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// Open a new tab right after `after` (or at the end) and return its id.
    pub fn open(
        &mut self,
        loc: Location,
        after: Option<TabId>,
        activate: bool,
        settings: &Settings,
    ) -> TabId {
        let id = self.alloc_id();
        let mut tab = Tab::new(id, loc, settings);
        let idx = match after.and_then(|a| self.index_of(a)) {
            // Nothing goes before the Home tab.
            Some(i) => {
                // New tabs opened from a grouped tab join that group.
                tab.group = self.tabs[i].group;
                i + 1
            }
            None => self.tabs.len(),
        };
        self.tabs.insert(idx, tab);
        if activate {
            self.activate(id);
        }
        id
    }

    pub fn activate(&mut self, id: TabId) {
        if let Some(gid) = self.tabs.iter().find(|t| t.id == id).and_then(|t| t.group) {
            if let Some(g) = self.group_mut(gid) {
                g.collapsed = false;
            }
        }
        if self.index_of(id).is_some() {
            self.active = id;
        }
    }

    /// Close a tab (not the Home tab).
    pub fn close(&mut self, id: TabId) {
        let Some(idx) = self.index_of(id).filter(|&i| i > 0) else {
            return;
        };
        self.tabs.remove(idx);
        if self.active == id {
            let next = idx.min(self.tabs.len() - 1);
            self.active = self.tabs[next].id;
        }
        self.drop_empty_groups();
    }

    /// The tab shown in the other (unfocused) pane of the split view.
    pub fn split_partner(&self) -> Option<TabId> {
        self.panes
            .and_then(|p| p.into_iter().find(|&id| id != self.active))
    }

    pub fn get(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Turn the split view on (the second pane opens the same place in a
    /// new tab) or off.
    pub fn toggle_split(&mut self, settings: &Settings) {
        if self.panes.take().is_some() {
            return;
        }
        let src = self.active();
        let (loc, mode, key, order) = (
            src.location().clone(),
            src.view_mode,
            src.sort_key,
            src.sort_order,
        );
        let active = self.active;
        let id = self.open(loc, Some(active), false, settings);
        if let Some(t) = self.get_mut(id) {
            t.view_mode = mode;
            t.sort_key = key;
            t.sort_order = order;
        }
        self.panes = Some([active, id]);
        self.focus = 0;
    }

    /// Move the focus to the other pane.
    pub fn switch_pane(&mut self) {
        if let Some(other) = self.split_partner() {
            self.activate(other);
        }
    }

    /// Keep the split view consistent after tabs were activated or closed:
    /// the focused pane shows whatever tab is active now, and closing a
    /// pane's tab ends the split.
    pub fn fix_panes(&mut self) {
        let Some(mut p) = self.panes else { return };
        if !p.iter().all(|&id| self.index_of(id).is_some()) {
            self.panes = None;
            return;
        }
        match p.iter().position(|&id| id == self.active) {
            Some(i) => self.focus = i,
            None => p[self.focus] = self.active,
        }
        self.panes = (p[0] != p[1]).then_some(p);
    }

    /// Close every tab but `keep` (and the Home tab).
    pub fn close_others(&mut self, keep: TabId) {
        let home = self.home();
        self.tabs.retain(|t| t.id == keep || t.id == home);
        self.active = keep;
        self.drop_empty_groups();
    }

    pub fn duplicate(&mut self, id: TabId, settings: &Settings) {
        let Some(src) = self.tabs.iter().find(|t| t.id == id) else {
            return;
        };
        if self.is_home(id) {
            return;
        }
        let (loc, mode, key, order) = (
            src.location().clone(),
            src.view_mode,
            src.sort_key,
            src.sort_order,
        );
        let new_id = self.open(loc, Some(id), true, settings);
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == new_id) {
            t.view_mode = mode;
            t.sort_key = key;
            t.sort_order = order;
        }
    }

    pub fn cycle(&mut self, delta: isize) {
        let visible: Vec<TabId> = self.tabs.iter().map(|t| t.id).collect();
        if let Some(i) = visible.iter().position(|&t| t == self.active) {
            let n = visible.len() as isize;
            let j = ((i as isize + delta) % n + n) % n;
            self.activate(visible[j as usize]);
        }
    }

    /// Move tab `id` so that it sits at `target` index.
    pub fn move_tab(&mut self, id: TabId, target: usize) {
        let Some(from) = self.index_of(id).filter(|&i| i > 0) else {
            return;
        };
        let tab = self.tabs.remove(from);
        // The Home tab stays first.
        let target = target.clamp(1, self.tabs.len());
        // Dropping a tab between tabs of a group makes it join that group.
        let neighbour_group = self
            .tabs
            .get(target)
            .and_then(|t| t.group)
            .filter(|g| target > 0 && self.tabs[target - 1].group == Some(*g));
        self.tabs.insert(target, tab);
        if neighbour_group.is_some() {
            self.tabs[target].group = neighbour_group;
        }
        self.drop_empty_groups();
    }

    /// Mark every tab showing `dir` (or every tab, if `None`) as needing a reload.
    /// Recycle Bin tabs are always refreshed, since any change may have deleted
    /// or restored something.
    pub fn mark_stale(&mut self, dir: Option<&Path>) {
        for t in &mut self.tabs {
            let hit = match (dir, t.location()) {
                (None, _) => true,
                (Some(d), Location::Folder(p)) => p == d,
                (_, Location::RecycleBin | Location::Tagged(_) | Location::Library { .. }) => true,
                _ => false,
            };
            if hit {
                t.stale = true;
            }
        }
    }
}
