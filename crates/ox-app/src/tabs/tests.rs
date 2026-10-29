//! Unit tests for tabs, groups and sessions.

use super::session::SessionTab;
use super::*;

fn tabs() -> (Tabs, Settings) {
    let s = Settings::default();
    let mut t = Tabs::new(Location::Home, &s);
    t.open(Location::Folder("/a".into()), None, false, &s);
    t.open(Location::Folder("/b".into()), None, false, &s);
    t.open(Location::Folder("/c".into()), None, false, &s);
    (t, s)
}

#[test]
fn split_view_follows_the_focused_pane() {
    let (mut t, s) = tabs();
    t.activate(2);
    t.toggle_split(&s);
    let [left, right] = t.panes.unwrap();
    assert_eq!(left, 2);
    assert_eq!(t.active, 2);
    assert_eq!(t.split_partner(), Some(right));
    assert_eq!(
        t.get(right).unwrap().location(),
        &Location::Folder("/a".into())
    );

    // Activating another tab replaces the focused pane only.
    t.switch_pane();
    t.fix_panes();
    assert_eq!(t.active, right);
    t.activate(4);
    t.fix_panes();
    assert_eq!(t.panes, Some([2, 4]));

    // Closing a pane's tab ends the split; toggling twice turns it off.
    t.close(2);
    t.fix_panes();
    assert_eq!(t.panes, None);
    t.toggle_split(&s);
    t.toggle_split(&s);
    assert_eq!(t.panes, None);
}

fn order(t: &Tabs) -> Vec<TabId> {
    t.tabs.iter().map(|t| t.id).collect()
}

#[test]
fn grouping_keeps_tabs_together() {
    let (mut t, _) = tabs();
    assert_eq!(order(&t), vec![1, 2, 3, 4]);
    let g = t.new_group(4);
    t.add_to_group(2, g);
    // Tab 2 moved next to tab 4.
    assert_eq!(order(&t), vec![1, 3, 4, 2]);
    t.remove_from_group(4);
    assert_eq!(order(&t), vec![1, 3, 2, 4]);
    t.remove_from_group(2);
    assert!(t.groups.is_empty(), "empty group is removed");
}

#[test]
fn closing_and_collapsing() {
    let (mut t, _) = tabs();
    t.activate(3);
    let g = t.new_group(3);
    t.toggle_group(g);
    assert_ne!(t.active, 3, "active tab leaves a collapsed group");
    t.activate(3);
    assert!(
        !t.group(g).unwrap().collapsed,
        "activating expands the group"
    );
    t.close_group(g);
    assert_eq!(order(&t), vec![1, 2, 4]);
    for id in [1, 2, 4] {
        t.close(id);
    }
    assert_eq!(order(&t), vec![1], "the Home tab stays");
    assert_eq!(*t.active().location(), Location::Home);
}

#[test]
fn the_home_tab_stays_first() {
    let (mut t, s) = tabs();
    assert_eq!(t.home(), 1);
    // It can't be moved, grouped, duplicated or closed, and nothing
    // moves in front of it.
    t.move_tab(1, 3);
    t.move_tab(4, 0);
    assert_eq!(order(&t), vec![1, 4, 2, 3]);
    let g = t.new_group(1);
    assert!(t.group(g).is_none() && t.tabs[0].group.is_none());
    t.duplicate(1, &s);
    t.close(1);
    assert_eq!(t.tabs.len(), 4);
    // Tabs opened from it go after it; "close others" keeps it.
    let id = t.open(Location::Folder("/d".into()), Some(1), true, &s);
    assert_eq!(t.index_of(id), Some(1));
    t.close_others(id);
    assert_eq!(order(&t), vec![1, id]);
    // Starting in a folder gives the Home tab and the folder's tab.
    let t = Tabs::new(Location::Folder("/x".into()), &s);
    assert_eq!(t.tabs.len(), 2);
    assert_eq!(*t.tabs[0].location(), Location::Home);
    assert_eq!(t.active, t.tabs[1].id);
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

    // An older session without the Home tab in front gets one.
    let old = Session {
        tabs: vec![SessionTab {
            location: Location::Help,
            group: None,
            pinned: false,
        }],
        groups: Vec::new(),
        active: 0,
    };
    let restored = Tabs::from_session(old, &s).unwrap();
    assert_eq!(*restored.tabs[0].location(), Location::Home);
    assert_eq!(*restored.active().location(), Location::Help);
}

#[test]
fn flat_view_is_a_mode_of_a_folder_tab() {
    let (mut t, s) = tabs();
    let id = t.open(Location::Folder("/a".into()), None, true, &s);
    let tab = t.get_mut(id).unwrap();
    tab.stale = false;
    tab.set_flat(true);
    assert!(tab.is_flat() && tab.stale);

    // Changes in a sub-folder reload a flat view, not a normal one.
    tab.stale = false;
    t.get_mut(2).unwrap().stale = false;
    t.mark_stale(Some(Path::new("/a/sub")));
    assert!(t.get(id).unwrap().stale);
    assert!(!t.get(2).unwrap().stale);

    // Refreshing keeps it; going somewhere else turns it off.
    let tab = t.get_mut(id).unwrap();
    tab.navigate(Location::Folder("/a".into()));
    assert!(tab.is_flat());
    tab.navigate(Location::Folder("/a/sub".into()));
    assert!(!tab.is_flat());
    tab.back();
    assert!(!tab.is_flat());

    // Only folders have a flat view.
    tab.navigate(Location::RecycleBin);
    tab.set_flat(true);
    assert!(!tab.is_flat());
}

fn pinned(t: &Tabs) -> Vec<TabId> {
    t.tabs.iter().filter(|t| t.pinned).map(|t| t.id).collect()
}

#[test]
fn pinned_tabs_come_first() {
    let (mut t, s) = tabs();
    let g = t.new_group(3);
    t.pin(3);
    // It left its group (now empty, so gone) and went after the Home tab.
    assert_eq!(order(&t), vec![1, 3, 2, 4]);
    assert!(t.group(g).is_none());
    t.pin(4);
    assert_eq!(order(&t), vec![1, 3, 4, 2]);
    assert_eq!(t.pinned_end(), 3);
    // Dragging keeps pinned and other tabs apart.
    t.move_tab(2, 1);
    assert_eq!(order(&t), vec![1, 3, 4, 2]);
    t.move_tab(3, 3);
    assert_eq!(order(&t), vec![1, 4, 3, 2]);
    t.move_tab(4, 0);
    assert_eq!(order(&t), vec![1, 4, 3, 2]);
    // New tabs, also from a pinned tab, go after the pinned ones.
    let n = t.open(Location::Home, Some(4), true, &s);
    assert_eq!(t.index_of(n), Some(3));
    assert!(!t.is_pinned(n));
    // A copy of a pinned tab is pinned, right after it.
    t.duplicate(4, &s);
    let copy = t.active;
    assert_eq!(order(&t), vec![1, 4, copy, 3, n, 2]);
    assert!(t.is_pinned(copy));
    // Joining a group unpins.
    let g = t.new_group(copy);
    assert!(!t.is_pinned(copy));
    assert_eq!(t.get(copy).unwrap().group, Some(g));
    assert_eq!(pinned(&t), vec![4, 3]);
    // "Close other tabs" and closing a group keep pinned tabs.
    t.close_others(2);
    assert_eq!(order(&t), vec![1, 4, 3, 2]);
    // Unpinning makes it the first of the other tabs.
    t.unpin(4);
    assert_eq!(order(&t), vec![1, 3, 4, 2]);
    assert_eq!(pinned(&t), vec![3]);
    // Closing a pinned tab directly works.
    t.close(3);
    assert_eq!(order(&t), vec![1, 4, 2]);
}

#[test]
fn pinned_tabs_in_sessions() {
    let (mut t, s) = tabs();
    t.pin(4);
    let g = t.new_group(2);
    t.activate(3);
    let session = t.to_session();
    let text = toml::to_string(&session).unwrap();
    let restored = Tabs::from_session(toml::from_str(&text).unwrap(), &s).unwrap();
    assert_eq!(pinned(&restored).len(), 1);
    assert!(restored.tabs[1].pinned);
    assert_eq!(restored.index_of(restored.active), Some(3));

    // Sessions saved before pinned tabs load as unpinned; a pinned tab
    // that is out of place (hand-edited file) is moved to the front.
    let old =
        "active = 0\ngroups = []\n[[tabs]]\nlocation = \"Home\"\n[[tabs]]\nlocation = \"Help\"\n";
    let old: Session = toml::from_str(old).unwrap();
    assert!(old.tabs.iter().all(|t| !t.pinned));
    let mut odd = session.clone();
    odd.tabs[3].pinned = true;
    odd.tabs[3].group = Some(g);
    let odd = Tabs::from_session(odd, &s).unwrap();
    assert_eq!(pinned(&odd).len(), 2);
    assert!(odd.tabs[1].pinned && odd.tabs[2].pinned && !odd.tabs[3].pinned);
    assert!(odd
        .tabs
        .iter()
        .filter(|t| t.pinned)
        .all(|t| t.group.is_none()));

    // Replacing gives new ids; adding keeps what's open.
    let mut other = Tabs::new(Location::Settings, &s);
    let before = other.tabs.len();
    other.add_session(session.clone(), &s);
    assert_eq!(other.tabs.len(), before + 3);
    assert!(other.tabs[1].pinned, "its pinned tab is pinned here too");
    assert_eq!(*other.tabs[2].location(), Location::Settings);
    assert_eq!(other.groups.len(), 1);
    let ids: HashSet<TabId> = other.tabs.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), other.tabs.len(), "tab ids stay unique");
    assert!(other.groups.iter().all(|g| !ids.contains(&g.id)));
    let old_ids = ids;
    other.replace_with(session, &s);
    assert_eq!(other.tabs.len(), 4);
    assert!(other.tabs.iter().all(|t| !old_ids.contains(&t.id)));
}
