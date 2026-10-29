//! Selection helpers of the file view: by pattern, inverted, same type.

use std::collections::HashSet;

use ox_core::select::{Patterns, TypeKey};

use super::FolderView;

impl FolderView {
    /// Add the visible items matching `patterns` to the selection, or
    /// (`select` false) take them out of it. Returns how many matched.
    pub fn select_matching(&mut self, patterns: &Patterns, select: bool) -> usize {
        let matching: Vec<_> = self
            .visible
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|e| patterns.matches(&e.name))
            .map(|e| e.path.clone())
            .collect();
        let n = matching.len();
        for p in matching {
            if select {
                self.selection.insert(p);
            } else {
                self.selection.remove(&p);
            }
        }
        n
    }

    /// Select what isn't selected and the other way round.
    pub fn invert_selection(&mut self) {
        self.selection = self
            .visible
            .iter()
            .map(|&i| &self.entries[i].path)
            .filter(|p| !self.selection.contains(*p))
            .cloned()
            .collect();
        self.anchor = None;
    }

    /// Add every visible item of the same type as the selected ones (or as
    /// the one at the keyboard cursor when nothing is selected): the same
    /// extension, or folders. Returns whether there was anything to go by.
    pub fn select_same_type(&mut self) -> bool {
        let mut types: HashSet<TypeKey> = self
            .selected_entries()
            .into_iter()
            .map(TypeKey::of)
            .collect();
        if types.is_empty() {
            types.extend(
                self.cursor
                    .and_then(|c| self.visible_entry(c))
                    .map(TypeKey::of),
            );
        }
        if types.is_empty() {
            return false;
        }
        let same: Vec<_> = self
            .visible
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|e| types.contains(&TypeKey::of(e)))
            .map(|e| e.path.clone())
            .collect();
        self.selection.extend(same);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ox_core::entry::FileEntry;

    fn view(names: &[&str]) -> (tempfile::TempDir, FolderView) {
        let dir = tempfile::tempdir().unwrap();
        let entries: Vec<FileEntry> = names
            .iter()
            .map(|n| {
                let p = dir.path().join(n);
                if n.ends_with('/') {
                    std::fs::create_dir(&p).unwrap();
                } else {
                    std::fs::write(&p, b"").unwrap();
                }
                FileEntry::from_path(&p).unwrap()
            })
            .collect();
        let view = FolderView {
            visible: (0..entries.len()).collect(),
            entries,
            ..Default::default()
        };
        (dir, view)
    }

    fn selected(v: &FolderView) -> Vec<String> {
        let mut names: Vec<String> = v
            .selected_entries()
            .into_iter()
            .map(|e| e.name.clone())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn patterns_add_to_and_take_from_the_selection() {
        let (_d, mut v) = view(&["a.jpg", "b.PNG", "c.txt", "report.doc"]);
        v.select_only(2);
        assert_eq!(v.select_matching(&Patterns::parse("*.jpg;*.png"), true), 2);
        assert_eq!(selected(&v), ["a.jpg", "b.PNG", "c.txt"]);
        assert_eq!(v.select_matching(&Patterns::parse("*.png"), false), 1);
        assert_eq!(selected(&v), ["a.jpg", "c.txt"]);
        assert_eq!(v.select_matching(&Patterns::parse("*.zip"), true), 0);
    }

    #[test]
    fn hidden_by_the_filter_is_left_alone() {
        let (_d, mut v) = view(&["a.jpg", "b.jpg"]);
        v.visible = vec![1];
        v.select_matching(&Patterns::parse("*.jpg"), true);
        assert_eq!(selected(&v), ["b.jpg"]);
        v.invert_selection();
        assert!(v.selection.is_empty());
    }

    #[test]
    fn inverting_swaps_selected_and_unselected() {
        let (_d, mut v) = view(&["a", "b", "c"]);
        v.select_only(1);
        v.invert_selection();
        assert_eq!(selected(&v), ["a", "c"]);
        v.invert_selection();
        assert_eq!(selected(&v), ["b"]);
    }

    #[test]
    fn same_type_follows_the_selection_or_the_cursor() {
        let (_d, mut v) = view(&["a.jpg", "B.JPG", "c.png", "d/", "e/", "f.png"]);
        assert!(!v.select_same_type());
        v.select_only(0);
        assert!(v.select_same_type());
        assert_eq!(selected(&v), ["B.JPG", "a.jpg"]);
        // Several types at once, folders included.
        v.toggle(3);
        v.select_same_type();
        assert_eq!(selected(&v), ["B.JPG", "a.jpg", "d", "e"]);
        // Nothing selected: the item at the cursor.
        v.selection.clear();
        v.cursor = Some(2);
        v.select_same_type();
        assert_eq!(selected(&v), ["c.png", "f.png"]);
    }
}
