//! How the file view looks: its columns and groups, and the view each
//! folder keeps (view mode, sort order, columns, grouping) when *Remember the view of each
//! folder* is on.

use super::*;
use crate::tabs::Tab;
use ox_core::columns::{ColumnId, ColumnLayout};
use ox_core::folder_views::{FolderViews, SavedView};

pub enum ViewAction {
    /// Change the columns of the details view.
    Columns(ColumnEdit),
    /// Forget the current folder's view; it gets the default view again.
    ResetFolder,
    /// The current view becomes the default for all folders without one of their own.
    MakeDefault,
    /// Forget the views of all folders.
    ForgetAll,
    /// Group the items of the active tab.
    GroupBy(ox_core::grouping::GroupBy),
    /// Collapse or expand a group (by its id).
    ToggleGroup(String),
    /// Collapse (`true`) or expand all groups.
    CollapseAll(bool),
}

pub enum ColumnEdit {
    /// Show a hidden column or hide a shown one.
    Toggle(ColumnId),
    /// Move a column in front of another one (to the end if `None`).
    MoveBefore(ColumnId, Option<ColumnId>),
    /// New width while an edge is dragged (saved with [`ColumnEdit::Save`]).
    Width(ColumnId, f32),
    /// The drag is over: keep the widths.
    Save,
    /// Back to the default columns.
    Reset,
}

impl ColumnEdit {
    fn apply(self, layout: &mut ColumnLayout) {
        match self {
            Self::Toggle(id) => layout.toggle(id),
            Self::MoveBefore(id, before) => layout.move_before(id, before),
            Self::Width(id, w) => layout.set_width(id, w),
            Self::Save => {}
            Self::Reset => *layout = ColumnLayout::default(),
        }
    }
}

/// The view `tab` shows now.
fn view_of(tab: &Tab) -> SavedView {
    SavedView {
        mode: tab.view_mode,
        sort: tab.sort_key,
        order: tab.sort_order,
        columns: tab.columns.clone(),
        group: tab.group_by,
    }
}

fn show_view(tab: &mut Tab, view: &SavedView) {
    tab.view_mode = view.mode;
    tab.sort_key = view.sort;
    tab.sort_order = view.order;
    tab.columns = view.columns.clone();
    if tab.group_by != view.group {
        tab.group_by = view.group;
        tab.collapsed.clear();
    }
}

/// Give `tab` its folder's view when it opened another folder, or remember
/// how its view changed. Returns whether the view changed (re-sort then).
fn sync_tab(tab: &mut Tab, views: &mut FolderViews, defaults: &SavedView, remember: bool) -> bool {
    let folder = match tab.location() {
        Location::Folder(p) if remember => p.clone(),
        // Elsewhere the tab keeps the view it has.
        _ => {
            tab.remembered = None;
            return false;
        }
    };
    let current = view_of(tab);
    match &tab.remembered {
        Some((f, applied)) if *f == folder => {
            if current != *applied {
                views.set(&folder, current.clone(), defaults);
                tab.remembered = Some((folder, current));
            }
            false
        }
        _ => {
            let view = views.get(&folder).cloned();
            let view = view.unwrap_or_else(|| defaults.clone());
            views.touch(&folder);
            let changed = view != current;
            if changed {
                show_view(tab, &view);
            }
            tab.remembered = Some((folder, view));
            changed
        }
    }
}

impl OxApp {
    /// The columns `tab` shows: its folder's own, or the default ones.
    pub fn columns_of<'a>(&'a self, tab: &'a Tab) -> &'a ColumnLayout {
        tab.columns
            .as_ref()
            .unwrap_or(&self.settings.browsing.columns)
    }

    /// The folder the active tab shows, if its view is remembered.
    pub fn remembered_folder(&self) -> Option<PathBuf> {
        match self.tabs.active().location() {
            Location::Folder(p) if self.settings.browsing.remember_folder_views => Some(p.clone()),
            _ => None,
        }
    }

    /// Once per frame, before the tabs load: a tab that opened a folder
    /// gets the folder's view (or the default one), and changes to the
    /// view of a folder are remembered.
    pub(super) fn sync_folder_views(&mut self) {
        let folders_first = self.settings.browsing.folders_first;
        let case = self.settings.search.case_sensitive;
        let defaults = SavedView::defaults(&self.settings.browsing);
        let remember = self.settings.browsing.remember_folder_views;
        for tab in &mut self.tabs.tabs {
            if sync_tab(tab, &mut self.folder_views, &defaults, remember) {
                tab.refresh_visible(folders_first, case);
            }
        }
        // Not on every change: a dragged column edge changes it each frame.
        if self.folder_views.dirty && self.folder_views_saved.elapsed() > Duration::from_secs(2) {
            self.save_folder_views();
        }
    }

    /// Group the tabs that need the tags for it (grouped by tag, or the
    /// tags changed). Once per frame, before keys are handled.
    pub(super) fn sync_groups(&mut self) {
        let tags = &self.tags;
        for tab in &mut self.tabs.tabs {
            let tags_changed = tab.group_by == ox_core::grouping::GroupBy::Tag
                && tab.view.grouped_tags != tags.version;
            if tab.view.needs_grouping || tags_changed {
                tab.regroup(tags);
            }
        }
    }

    pub(super) fn save_folder_views(&mut self) {
        self.folder_views_saved = Instant::now();
        if !self.folder_views.dirty {
            return;
        }
        if let Err(e) = self.folder_views.save() {
            log::warn!("could not save folder views: {e}");
            // Don't try again every frame.
            self.folder_views.dirty = false;
        }
    }

    pub(super) fn process_view(&mut self, action: ViewAction) {
        match action {
            ViewAction::Columns(edit) => self.edit_columns(edit),
            ViewAction::ResetFolder => {
                if let Some(folder) = self.remembered_folder() {
                    self.folder_views.remove(&folder);
                    // Shown with the default view from the next frame.
                    self.tabs.active_mut().remembered = None;
                    self.save_folder_views();
                }
            }
            ViewAction::MakeDefault => {
                let tab = self.tabs.active();
                let view = view_of(tab);
                let columns = self.columns_of(tab).clone();
                let b = &mut self.settings.browsing;
                b.default_view = view.mode;
                b.default_sort = view.sort;
                b.default_order = view.order;
                b.default_group = view.group;
                b.columns = columns;
                let defaults = SavedView::defaults(b);
                if let Some(folder) = self.remembered_folder() {
                    self.folder_views.remove(&folder);
                    self.save_folder_views();
                }
                let tab = self.tabs.active_mut();
                tab.columns = None;
                if let Some((_, applied)) = &mut tab.remembered {
                    *applied = defaults;
                }
                self.save_browsing_settings();
                self.toast(tr(
                    "Folders without a view of their own are now shown like this.",
                ));
            }
            ViewAction::GroupBy(by) => {
                let tab = self.tabs.active_mut();
                if tab.group_by != by {
                    tab.group_by = by;
                    tab.collapsed.clear();
                    tab.regroup(&self.tags);
                }
            }
            ViewAction::ToggleGroup(id) => {
                let tab = self.tabs.active_mut();
                tab.toggle_group(&id, &self.tags);
            }
            ViewAction::CollapseAll(collapse) => {
                let tab = self.tabs.active_mut();
                tab.collapse_all(collapse, &self.tags);
            }
            ViewAction::ForgetAll => {
                self.folder_views.clear();
                for tab in &mut self.tabs.tabs {
                    tab.remembered = None;
                }
                self.save_folder_views();
            }
        }
    }

    /// Column changes go to the folder when its view is remembered, and to
    /// the default columns otherwise.
    fn edit_columns(&mut self, edit: ColumnEdit) {
        let save = !matches!(edit, ColumnEdit::Width(..));
        if self.remembered_folder().is_some() {
            let defaults = self.settings.browsing.columns.clone();
            let tab = self.tabs.active_mut();
            if matches!(edit, ColumnEdit::Reset) {
                tab.columns = None;
                return;
            }
            let layout = tab.columns.get_or_insert_with(|| defaults.clone());
            edit.apply(layout);
            if *layout == defaults {
                tab.columns = None;
            }
            // `sync_folder_views` remembers it.
            return;
        }
        edit.apply(&mut self.settings.browsing.columns);
        if save {
            self.save_browsing_settings();
        }
    }

    /// Save the settings without reloading the tabs (as `SettingsChanged`
    /// does) and without applying the look again.
    fn save_browsing_settings(&mut self) {
        // Only browsing settings change here, and there is nothing to apply for them.
        if let Err(e) = self.save_settings() {
            self.toast(ox_core::trf!("Couldn't save settings: {e}", e = e));
        }
        // Saving may have taken over changes from another window.
        if let Some(applied) = &mut self.applied_settings {
            applied.browsing = self.settings.browsing.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ox_core::settings::ViewMode;

    #[test]
    fn folders_keep_their_view() {
        let settings = Settings::default();
        let defaults = SavedView::defaults(&settings.browsing);
        let mut views = FolderViews::default();
        let (pictures, docs) = (PathBuf::from("/pictures"), PathBuf::from("/docs"));
        let mut tab = Tab::new(2, Location::Folder(pictures.clone()), &settings);
        let mut sync = |tab: &mut Tab| sync_tab(tab, &mut views, &defaults, true);

        // The first time the folder has the default view; switching to the
        // grid is remembered for it.
        assert!(!sync(&mut tab));
        tab.view_mode = ViewMode::Grid;
        sync(&mut tab);
        // Another folder: the default view again.
        tab.navigate(Location::Folder(docs.clone()));
        assert!(sync(&mut tab));
        assert_eq!(tab.view_mode, ViewMode::Details);
        tab.sort_key = ox_core::entry::SortKey::Modified;
        sync(&mut tab);
        // Back: the grid, and the other folder keeps its sort order.
        tab.back();
        assert!(sync(&mut tab));
        assert_eq!(tab.view_mode, ViewMode::Grid);
        assert_eq!(tab.sort_key, ox_core::entry::SortKey::Name);
        tab.forward();
        sync(&mut tab);
        assert_eq!(tab.sort_key, ox_core::entry::SortKey::Modified);
        assert_eq!(tab.view_mode, ViewMode::Details);
        // Columns of its own too.
        let mut columns = ColumnLayout::default();
        columns.toggle(ColumnId::Dimensions);
        tab.columns = Some(columns.clone());
        sync(&mut tab);
        assert_eq!(views.get(&docs).unwrap().columns, Some(columns));
        // Going back to the default view forgets it.
        tab.sort_key = ox_core::entry::SortKey::Name;
        tab.columns = None;
        sync_tab(&mut tab, &mut views, &defaults, true);
        assert!(views.get(&docs).is_none());
        assert!(views.get(&pictures).is_some());
    }

    #[test]
    fn nothing_is_remembered_when_it_is_off() {
        let settings = Settings::default();
        let defaults = SavedView::defaults(&settings.browsing);
        let mut views = FolderViews::default();
        let mut tab = Tab::new(2, Location::Folder("/a".into()), &settings);
        tab.view_mode = ViewMode::Grid;
        assert!(!sync_tab(&mut tab, &mut views, &defaults, false));
        assert!(views.is_empty());
        // Search results keep the tab's view.
        tab.navigate(Location::Search {
            root: None,
            query: "x".into(),
        });
        assert!(!sync_tab(&mut tab, &mut views, &defaults, true));
        assert_eq!(tab.view_mode, ViewMode::Grid);
    }
}
