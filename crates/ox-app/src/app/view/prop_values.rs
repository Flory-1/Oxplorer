//! Sorting and grouping by the values read from the files (dimensions,
//! date taken, …): ask for them for all items of the panes that need them
//! (after the rows on screen), and sort them in as they arrive, a few times
//! a second at most.

use super::*;

/// Values that arrive are sorted in at most this often.
const RESORT_EVERY: Duration = Duration::from_millis(300);

/// `tab` sorts or groups by values that can be read where it is.
fn wants(tab: &Tab) -> bool {
    let here = tab.location();
    tab.needs_props()
        && tab.view.loading.is_none()
        && !here.is_bin()
        && !here.is_read_only()
        && !here.is_remote()
}

impl OxApp {
    pub(in crate::app) fn poll_prop_values(&mut self) {
        let folders_first = self.settings.browsing.folders_first;
        let case = self.settings.search.case_sensitive;
        let panes: Vec<TabId> = [Some(self.tabs.active), self.tabs.split_partner()]
            .into_iter()
            .flatten()
            .filter(|&id| self.tabs.get(id).is_some_and(wants))
            .collect();
        let owners = panes
            .iter()
            .filter_map(|&id| self.tabs.get(id).map(|t| (id, t.view.version)))
            .collect();
        // Requests for folders no longer shown are dropped.
        self.column_values.later_for(owners);
        let arrived = self.column_values.arrived();
        for id in panes {
            let Some(tab) = self.tabs.get_mut(id) else {
                continue;
            };
            let view = &mut tab.view;
            // Search results come in without a new listing version.
            let seen = Some((view.version + view.entries.len() as u64, arrived));
            if view.props.seen != seen {
                view.props.seen = seen;
                let mut missing = 0;
                for e in &view.entries {
                    if view.props.get(e).is_some() {
                        continue;
                    }
                    match self.column_values.get_later(e) {
                        Some(p) => {
                            view.props.insert(e, p);
                            view.props.unsorted = true;
                        }
                        None => missing += 1,
                    }
                }
                view.props.missing = missing;
            }
            if !view.props.unsorted {
                continue;
            }
            let since = view.props.sorted_at.map(|at| at.elapsed());
            if view.props.missing == 0 || since.is_none_or(|s| s >= RESORT_EVERY) {
                view.props.unsorted = false;
                view.props.sorted_at = Some(Instant::now());
                tab.resort_keeping_cursor(folders_first, case);
            } else if let Some(since) = since {
                self.ctx
                    .request_repaint_after(RESORT_EVERY.saturating_sub(since));
            }
        }
    }
}
