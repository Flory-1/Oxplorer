//! Tabs, tab groups and per-tab browsing state.

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

/// What a tab is showing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Location {
    /// Quick access / favorites start page.
    Home,
    Folder(PathBuf),
    /// Search results. `root: None` searches all drives.
    Search {
        root: Option<PathBuf>,
        query: String,
    },
    Settings,
    RecycleBin,
    /// Everything with this colour tag, wherever it is.
    Tagged(TagColor),
}

impl Location {
    pub fn title(&self) -> String {
        match self {
            Location::Home => "Home".into(),
            Location::Folder(p) => entry::display_name(p),
            Location::Search { query, .. } => format!("Search: {query}"),
            Location::Settings => "Settings".into(),
            Location::RecycleBin => "Recycle Bin".into(),
            Location::Tagged(c) => c.name().into(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Location::Home => "🏠",
            Location::Folder(p) if p.parent().is_none() => "🖴",
            Location::Folder(_) => "📁",
            Location::Search { .. } => "🔍",
            Location::Settings => "⚙",
            Location::RecycleBin => "🗑",
            Location::Tagged(_) => "🏷",
        }
    }

    /// The folder this location represents, if any (used for paste, new folder, …).
    pub fn folder(&self) -> Option<&Path> {
        match self {
            Location::Folder(p) => Some(p),
            _ => None,
        }
    }

    pub fn shows_files(&self) -> bool {
        matches!(
            self,
            Location::Folder(_)
                | Location::Search { .. }
                | Location::RecycleBin
                | Location::Tagged(_)
        )
    }

    pub fn is_bin(&self) -> bool {
        matches!(self, Location::RecycleBin)
    }
}

/// Result of loading a location in the background.
pub struct Listing {
    pub entries: Vec<FileEntry>,
    /// Trash items behind the entries, when showing the Recycle Bin.
    pub bin: BinListing,
}

impl Listing {
    pub fn folder(path: &Path, show_hidden: bool) -> io::Result<Self> {
        Ok(Self {
            entries: entry::list_dir(path, show_hidden)?,
            bin: BinListing::default(),
        })
    }

    /// The items in `paths` that still exist.
    pub fn items(paths: &[PathBuf]) -> Self {
        Self {
            entries: paths
                .iter()
                .filter_map(|p| FileEntry::from_path(p).ok())
                .collect(),
            bin: BinListing::default(),
        }
    }

    pub fn recycle_bin() -> Result<Self, String> {
        let items = ox_core::recycle::list()?;
        let mut bin = BinListing::default();
        let entries = items
            .into_iter()
            .map(|b| {
                bin.items.insert(b.entry.path.clone(), b.item);
                b.entry
            })
            .collect();
        Ok(Self { entries, bin })
    }
}

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
        Self {
            id,
            group: None,
            history: vec![location],
            pos: 0,
            view: FolderView::default(),
            search: None,
            filter: String::new(),
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
            Location::Folder(p) => p.parent().map(Path::to_path_buf),
            Location::Search { root, .. } => root.clone(),
            _ => None,
        };
        let from = self.location().clone();
        match parent {
            Some(p) => self.navigate(Location::Folder(p)),
            None => self.navigate(Location::Home),
        }
        self.select_after_return(&from);
    }

    /// When going back/up, highlight the folder we came from, like Explorer.
    fn select_after_return(&mut self, from: &Location) {
        if let Location::Folder(p) = from {
            self.view.select_after_load = Some(p.clone());
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
        let filtering =
            (self.location().folder().is_some() || self.location().is_bin()) && !query.is_empty();
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

pub struct Tabs {
    pub tabs: Vec<Tab>,
    pub groups: Vec<TabGroup>,
    pub active: TabId,
    next_id: u64,
}

impl Tabs {
    pub fn new(first: Location, settings: &Settings) -> Self {
        let tab = Tab::new(1, first, settings);
        Self {
            active: tab.id,
            tabs: vec![tab],
            groups: Vec::new(),
            next_id: 2,
        }
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

    /// Close a tab. The last tab is replaced by a Home tab instead.
    pub fn close(&mut self, id: TabId, settings: &Settings) {
        let Some(idx) = self.index_of(id) else { return };
        if self.tabs.len() == 1 {
            let new_id = self.alloc_id();
            self.tabs[0] = Tab::new(new_id, Location::Home, settings);
            self.active = new_id;
            self.groups.clear();
            return;
        }
        self.tabs.remove(idx);
        if self.active == id {
            let next = idx.min(self.tabs.len() - 1);
            self.active = self.tabs[next].id;
        }
        self.drop_empty_groups();
    }

    pub fn close_others(&mut self, keep: TabId) {
        self.tabs.retain(|t| t.id == keep);
        self.active = keep;
        self.drop_empty_groups();
    }

    pub fn duplicate(&mut self, id: TabId, settings: &Settings) {
        let Some(src) = self.tabs.iter().find(|t| t.id == id) else {
            return;
        };
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
        let Some(from) = self.index_of(id) else {
            return;
        };
        let tab = self.tabs.remove(from);
        let target = target.min(self.tabs.len());
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
        gid
    }

    /// Add a tab to a group, moving it next to the group's other tabs.
    pub fn add_to_group(&mut self, tab: TabId, gid: GroupId) {
        let Some(from) = self.index_of(tab) else {
            return;
        };
        let mut t = self.tabs.remove(from);
        t.group = Some(gid);
        let insert_at = self
            .tabs
            .iter()
            .rposition(|x| x.group == Some(gid))
            .map(|i| i + 1)
            .unwrap_or(from.min(self.tabs.len()));
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

    pub fn close_group(&mut self, gid: GroupId, settings: &Settings) {
        let ids: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|t| t.group == Some(gid))
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.close(id, settings);
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

    fn drop_empty_groups(&mut self) {
        let tabs = &self.tabs;
        self.groups
            .retain(|g| tabs.iter().any(|t| t.group == Some(g.id)));
    }

    /// Mark every tab showing `dir` (or every tab, if `None`) as needing a reload.
    /// Recycle Bin tabs are always refreshed, since any change may have deleted
    /// or restored something.
    pub fn mark_stale(&mut self, dir: Option<&Path>) {
        for t in &mut self.tabs {
            let hit = match (dir, t.location()) {
                (None, _) => true,
                (Some(d), Location::Folder(p)) => p == d,
                (_, Location::RecycleBin | Location::Tagged(_)) => true,
                _ => false,
            };
            if hit {
                t.stale = true;
            }
        }
    }

    // ---- session persistence ------------------------------------------------

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
        let mut tabs = Tabs {
            tabs: Vec::new(),
            groups: session.groups,
            active: 0,
            next_id: 1,
        };
        tabs.next_id = tabs.groups.iter().map(|g| g.id + 1).max().unwrap_or(1);
        for st in session.tabs {
            let loc = match st.location {
                Location::Folder(p) if !p.is_dir() => Location::Home,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs() -> (Tabs, Settings) {
        let s = Settings::default();
        let mut t = Tabs::new(Location::Home, &s);
        t.open(Location::Folder("/a".into()), None, false, &s);
        t.open(Location::Folder("/b".into()), None, false, &s);
        t.open(Location::Folder("/c".into()), None, false, &s);
        (t, s)
    }

    fn order(t: &Tabs) -> Vec<TabId> {
        t.tabs.iter().map(|t| t.id).collect()
    }

    #[test]
    fn grouping_keeps_tabs_together() {
        let (mut t, _) = tabs();
        assert_eq!(order(&t), vec![1, 2, 3, 4]);
        let g = t.new_group(4);
        t.add_to_group(1, g);
        // Tab 1 moved next to tab 4.
        assert_eq!(order(&t), vec![2, 3, 4, 1]);
        t.remove_from_group(4);
        assert_eq!(order(&t), vec![2, 3, 1, 4]);
        t.remove_from_group(1);
        assert!(t.groups.is_empty(), "empty group is removed");
    }

    #[test]
    fn closing_and_collapsing() {
        let (mut t, s) = tabs();
        t.activate(3);
        let g = t.new_group(3);
        t.toggle_group(g);
        assert_ne!(t.active, 3, "active tab leaves a collapsed group");
        t.activate(3);
        assert!(
            !t.group(g).unwrap().collapsed,
            "activating expands the group"
        );
        t.close_group(g, &s);
        assert_eq!(order(&t), vec![1, 2, 4]);
        for id in [1, 2, 4] {
            t.close(id, &s);
        }
        assert_eq!(t.tabs.len(), 1, "last tab is replaced, not removed");
        assert_eq!(*t.active().location(), Location::Home);
    }

    #[test]
    fn history() {
        let s = Settings::default();
        let mut tab = Tab::new(1, Location::Home, &s);
        tab.navigate(Location::Folder("/x".into()));
        tab.navigate(Location::Folder("/x/y".into()));
        tab.back();
        assert_eq!(*tab.location(), Location::Folder("/x".into()));
        assert_eq!(tab.view.select_after_load, Some(PathBuf::from("/x/y")));
        tab.forward();
        tab.up();
        assert_eq!(*tab.location(), Location::Folder("/x".into()));
        assert!(!tab.can_go_forward());
    }

    #[test]
    fn session_roundtrip() {
        let (mut t, s) = tabs();
        let g = t.new_group(2);
        t.group_mut(g).unwrap().name = "Work".into();
        let text = toml::to_string(&t.to_session()).unwrap();
        let session: Session = toml::from_str(&text).unwrap();
        let restored = Tabs::from_session(session, &s).unwrap();
        assert_eq!(restored.tabs.len(), 4);
        assert_eq!(restored.groups[0].name, "Work");
        assert_eq!(
            restored.tabs.iter().filter(|t| t.group.is_some()).count(),
            1
        );
    }
}
