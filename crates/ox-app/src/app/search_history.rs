//! Search history: the searches that were run, suggested again under the
//! search box (stored by `ox_core::search_history` in `search_history.toml`).

use super::*;
use ox_core::search_history::SearchHistory;
use std::time::SystemTime;

pub enum HistoryAction {
    /// A search was run: remember it (if the settings allow).
    Remember(String),
    Remove(String),
    Clear,
}

/// The history as last read, read again when another window changed it.
#[derive(Default)]
pub struct RecentSearches {
    history: SearchHistory,
    /// When the file was changed when it was read (`None`: not read yet).
    read: Option<Option<SystemTime>>,
}

impl RecentSearches {
    pub fn entries(&self) -> &[String] {
        &self.history.entries
    }

    /// Read the file if it changed since (a cheap check, fine every frame
    /// the search box has focus).
    pub fn refresh(&mut self) {
        let path = SearchHistory::path();
        let stamp = ox_core::sessions::modified(&path);
        if self.read != Some(stamp) {
            self.history = SearchHistory::load(&path);
            self.read = Some(stamp);
        }
    }

    fn update(&mut self, change: impl FnOnce(&mut SearchHistory) -> bool) {
        let path = SearchHistory::path();
        match SearchHistory::update(&path, change) {
            Ok(h) => {
                self.history = h;
                self.read = Some(ox_core::sessions::modified(&path));
            }
            Err(e) => log::warn!("could not save the search history: {e}"),
        }
    }
}

impl OxApp {
    pub(super) fn process_history(&mut self, action: HistoryAction) {
        match action {
            HistoryAction::Remember(query) => {
                if self.settings.search.remember_searches {
                    self.search_history.update(|h| h.add(&query));
                }
            }
            HistoryAction::Remove(query) => self.search_history.update(|h| h.remove(&query)),
            HistoryAction::Clear => {
                self.search_history.update(SearchHistory::clear);
                self.toast(tr("Search history cleared"));
            }
        }
    }
}
