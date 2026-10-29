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
        }],
        groups: Vec::new(),
        active: 0,
    };
    let restored = Tabs::from_session(old, &s).unwrap();
    assert_eq!(*restored.tabs[0].location(), Location::Home);
    assert_eq!(*restored.active().location(), Location::Help);
}
