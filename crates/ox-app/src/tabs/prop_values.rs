//! The values read from the files (dimensions, date taken, …) that a tab
//! sorts or groups by, kept with its listing. The app fills them in as they
//! are read (`app/view/prop_values.rs`) and sorts again.

use super::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use ox_core::grouping::GroupBy;
use ox_core::props::Props;

#[derive(Default)]
pub struct PropValues {
    /// By path, with the modification time they were read for.
    values: HashMap<PathBuf, (Option<SystemTime>, Arc<Props>)>,
    /// Listing version and `ColumnValues::arrived` at the last look.
    pub seen: Option<(u64, u64)>,
    /// Items still without values at the last look.
    pub missing: usize,
    /// Values arrived that aren't sorted in yet.
    pub unsorted: bool,
    /// When they were last sorted in.
    pub sorted_at: Option<Instant>,
}

impl PropValues {
    /// The values of `e`, if read (for its current version).
    pub fn get(&self, e: &FileEntry) -> Option<&Props> {
        self.values
            .get(&e.path)
            .filter(|(modified, _)| *modified == e.modified)
            .map(|(_, p)| &**p)
    }

    pub fn insert(&mut self, e: &FileEntry, props: Arc<Props>) {
        self.values.insert(e.path.clone(), (e.modified, props));
    }

    pub fn date_taken(&self, e: &FileEntry) -> Option<SystemTime> {
        self.get(e).and_then(|p| p.date_taken)
    }
}

impl Tab {
    /// Sorted or grouped by values read from the files.
    pub fn needs_props(&self) -> bool {
        self.sort_key.reads_props()
            || (self.group_by == GroupBy::Month && self.location().shows_files())
    }

    /// Sort the entries (by the values read so far, for those keys).
    pub(super) fn sort_view(&mut self, folders_first: bool) {
        let view = &mut self.view;
        let values = &view.props;
        entry::sort_entries_with(
            &mut view.entries,
            self.sort_key,
            self.sort_order,
            folders_first,
            |e| values.get(e),
        );
    }

    /// Sort and group again for values that arrived, keeping the selection
    /// and the cursor on their items; the cursor item stays in view.
    pub fn resort_keeping_cursor(&mut self, folders_first: bool, case_sensitive: bool) {
        let view = &self.view;
        let path_at = |i: Option<usize>| {
            i.and_then(|i| view.visible_entry(i))
                .map(|e| e.path.clone())
        };
        let (cursor, anchor) = (path_at(view.cursor), path_at(view.anchor));
        let before = view.cursor;
        self.refresh_visible(folders_first, case_sensitive);
        let view = &mut self.view;
        let find = |p: Option<PathBuf>| {
            p.and_then(|p| view.visible.iter().position(|&i| view.entries[i].path == p))
        };
        let (cursor, anchor) = (find(cursor), find(anchor));
        if cursor.is_some() {
            view.cursor = cursor;
            if cursor != before {
                view.scroll_to = cursor;
            }
        }
        if anchor.is_some() {
            view.anchor = anchor;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(format!("/a/{name}")),
            name: name.into(),
            is_dir: false,
            is_symlink: false,
            size: 1,
            modified: None,
            created: None,
            hidden: false,
            readonly: false,
            cloud: None,
        }
    }

    fn names(t: &Tab) -> Vec<String> {
        t.view
            .visible
            .iter()
            .map(|&i| t.view.entries[i].name.clone())
            .collect()
    }

    #[test]
    fn values_that_arrive_move_items_and_the_cursor_follows() {
        let s = Settings::default();
        let mut tab = Tab::new(2, Location::Folder("/a".into()), &s);
        tab.view.entries = vec![entry("a.jpg"), entry("b.jpg"), entry("c.jpg")];
        tab.sort_key = SortKey::Dimensions;
        assert!(tab.needs_props());
        tab.refresh_visible(true, false);
        // Nothing read yet: by name.
        assert_eq!(names(&tab), ["a.jpg", "b.jpg", "c.jpg"]);
        tab.view.select_only(0);

        let dims = |w, h| {
            Arc::new(Props {
                dimensions: Some((w, h)),
                ..Default::default()
            })
        };
        let c = tab.view.entries[2].clone();
        let a = tab.view.entries[0].clone();
        tab.view.props.insert(&c, dims(10, 10));
        tab.view.props.insert(&a, dims(50, 50));
        tab.resort_keeping_cursor(true, false);
        // Read ones first, the one not read yet last.
        assert_eq!(names(&tab), ["c.jpg", "a.jpg", "b.jpg"]);
        assert_eq!(tab.view.cursor, Some(1));
        assert_eq!(tab.view.scroll_to, Some(1));
        assert!(tab.view.selection.contains(&PathBuf::from("/a/a.jpg")));
    }

    #[test]
    fn the_gallery_is_grouped_by_month() {
        let s = Settings::default();
        let mut tab = Tab::new(2, Location::Folder("/a".into()), &s);
        tab.navigate(Location::Gallery);
        assert_eq!(tab.group_by, GroupBy::Month);
        assert!(tab.needs_props());
    }
}
