//! Group by: the visible items of a tab in groups, collapsed ones left out.

use super::*;
use ox_core::grouping::{self, GroupBy, GroupContext};
use ox_core::tags::TagStore;

impl Tab {
    /// Put `view.ungrouped` into groups as `view.visible`. Grouping by tag
    /// needs the tags: without them it waits for [`Tab::regroup`].
    pub(super) fn group_visible(&mut self, tags: Option<&TagStore>) {
        let by = if self.location().shows_files() {
            self.group_by
        } else {
            GroupBy::None
        };
        let view = &mut self.view;
        let tag_of = |e: &FileEntry| tags.map_or_else(Vec::new, |t| t.tags(&e.path).to_vec());
        let tag_label = |c: TagColor| tags.map_or_else(String::new, |t| t.label(c).to_owned());
        let date_taken = |e: &FileEntry| view.props.date_taken(e);
        let cx = GroupContext {
            today: GroupContext::today(),
            tags: &tag_of,
            tag_label: &tag_label,
            date_taken: &date_taken,
        };
        if by == GroupBy::Tag && tags.is_none() {
            // Shown ungrouped until the next frame groups them.
            view.visible = view.ungrouped.clone();
            view.groups.clear();
            view.needs_grouping = true;
            return;
        }
        let grouped = grouping::group(&view.entries, &view.ungrouped, by, &cx, &self.collapsed);
        view.visible = grouped.visible;
        view.groups = grouped.groups;
        view.needs_grouping = false;
        if let Some(t) = tags {
            view.grouped_tags = t.version;
        }
    }

    /// Group the items again (the grouping, the tags or a collapsed group
    /// changed), keeping the cursor on the same item.
    pub fn regroup(&mut self, tags: &TagStore) {
        let view = &self.view;
        let item = |i: Option<usize>| i.and_then(|i| view.visible.get(i).copied());
        let (cursor, anchor, scroll) = (item(view.cursor), item(view.anchor), item(view.scroll_to));
        self.group_visible(Some(tags));
        let view = &mut self.view;
        let find = |e: Option<usize>| e.and_then(|e| view.visible.iter().position(|&v| v == e));
        let max = view.visible.len();
        view.cursor = find(cursor).or(view.cursor.filter(|&c| c < max));
        view.anchor = find(anchor).or(view.anchor.filter(|&c| c < max));
        view.scroll_to = find(scroll);
    }

    /// Collapse or expand the group `id`.
    pub fn toggle_group(&mut self, id: &str, tags: &TagStore) {
        if !self.collapsed.remove(id) {
            self.collapsed.insert(id.to_owned());
        }
        self.regroup(tags);
    }

    /// Collapse (`true`) or expand every group.
    pub fn collapse_all(&mut self, collapse: bool, tags: &TagStore) {
        self.collapsed.clear();
        if collapse {
            self.collapsed
                .extend(self.view.groups.iter().map(|g| g.id.clone()));
        }
        self.regroup(tags);
    }
}

impl FolderView {
    /// Items there are to show, also those in collapsed groups.
    pub fn shown_count(&self) -> usize {
        self.visible.len()
            + self
                .groups
                .iter()
                .filter(|g| g.collapsed)
                .map(|g| g.count)
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            path: PathBuf::from(format!("/a/{name}")),
            name: name.into(),
            is_dir,
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
    fn collapsed_groups_are_left_out_of_selection_and_navigation() {
        let s = Settings::default();
        let tags = TagStore::default();
        let mut tab = Tab::new(2, Location::Folder("/a".into()), &s);
        tab.view.entries = vec![
            entry("b.txt", false),
            entry("docs", true),
            entry("a.pdf", false),
            entry("c.txt", false),
        ];
        tab.group_by = GroupBy::Type;
        tab.refresh_visible(true, false);
        // Folders, PDF, text: sorted within each group.
        assert_eq!(names(&tab), ["docs", "a.pdf", "b.txt", "c.txt"]);
        assert_eq!(tab.view.groups.len(), 3);

        // The cursor stays on its item when a group above it collapses.
        tab.view.select_only(3);
        tab.toggle_group("folders", &tags);
        assert_eq!(names(&tab), ["a.pdf", "b.txt", "c.txt"]);
        assert_eq!(tab.view.cursor, Some(2));
        assert_eq!(tab.view.shown_count(), 4);
        tab.view.select_all();
        assert!(!tab.view.selection.contains(&PathBuf::from("/a/docs")));
        assert_eq!(tab.view.selection.len(), 3);

        // Re-sorting keeps the group collapsed; going elsewhere opens it again.
        tab.refresh_visible(true, false);
        assert_eq!(tab.view.visible.len(), 3);
        tab.collapse_all(false, &tags);
        assert_eq!(tab.view.visible.len(), 4);
        tab.collapse_all(true, &tags);
        assert!(tab.view.visible.is_empty());
        tab.navigate(Location::Folder("/b".into()));
        assert!(tab.collapsed.is_empty());
    }

    #[test]
    fn grouping_by_tag_waits_for_the_tags() {
        let s = Settings::default();
        let mut tags = TagStore::default();
        tags.toggle(&[PathBuf::from("/a/c.txt")], TagColor::Green);
        let mut tab = Tab::new(2, Location::Folder("/a".into()), &s);
        tab.view.entries = vec![entry("b.txt", false), entry("c.txt", false)];
        tab.group_by = GroupBy::Tag;
        tab.refresh_visible(true, false);
        assert!(tab.view.needs_grouping && tab.view.groups.is_empty());
        tab.regroup(&tags);
        assert!(!tab.view.needs_grouping);
        assert_eq!(tab.view.groups[0].id, "tag:3");
        assert_eq!(tab.view.visible, [1, 0]);
        assert_eq!(tab.view.grouped_tags, tags.version);
    }
}
