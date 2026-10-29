//! Flat view: a folder tab that lists every file in its sub-folders too.
//!
//! It's a mode of the tab, not a place of its own: the location stays the
//! folder (so paste, new folder, the address bar and history work as usual),
//! and going somewhere else turns it off.

use std::time::{Duration, Instant};

use ox_core::flat::{FlatEvent, FlatHandle, FlatOptions};

use super::*;

/// How a flat view's listing is going.
#[derive(Default)]
pub struct FlatState {
    /// The walk in progress.
    pub handle: Option<FlatHandle>,
    /// A refresh collects the files here and replaces the list when it's
    /// done, so the list doesn't empty and fill up again.
    pending: Option<Vec<FileEntry>>,
    /// The last walk stopped at the limit.
    pub truncated: bool,
    /// Folders the last walk couldn't read.
    pub unreadable: u64,
    /// New files arrived that aren't sorted into the list yet.
    unsorted: bool,
    last_sort: Option<Instant>,
    /// How long the last sort took (sorting is throttled on big lists).
    sort_time: Duration,
}

impl FlatState {
    pub fn walking(&self) -> bool {
        self.handle.is_some()
    }
}

impl Tab {
    pub fn is_flat(&self) -> bool {
        self.flat.is_some()
    }

    /// Flat view can be turned on in folders only.
    pub fn can_flat(&self) -> bool {
        self.location().folder().is_some()
    }

    /// Turn the flat view on or off; either way the list is loaded again.
    pub fn set_flat(&mut self, on: bool) {
        if on == self.is_flat() || (on && !self.can_flat()) {
            return;
        }
        self.flat = on.then(FlatState::default);
        self.view.loading = None;
        if on {
            // Streams into an empty list rather than below the folder's items.
            self.view.entries.clear();
            self.view.visible.clear();
            self.view.cursor = None;
            self.view.anchor = None;
        }
        self.stale = true;
    }

    /// Start (or restart) listing the files under `root`.
    pub(crate) fn start_flat(
        &mut self,
        root: PathBuf,
        opts: FlatOptions,
        notify: impl Fn() + Send + Sync + 'static,
    ) {
        let first = self.view.entries.is_empty();
        let Some(flat) = &mut self.flat else { return };
        flat.pending = (!first).then(Vec::new);
        flat.handle = Some(ox_core::flat::start(root, opts, notify));
        self.view.loading = None;
        self.view.error = None;
    }

    /// Take in the files that arrived. Returns when to look again, if new
    /// files are waiting to be sorted into the list.
    pub fn poll_flat(&mut self, folders_first: bool, case: bool) -> Option<Duration> {
        let flat = self.flat.as_mut()?;
        let handle = flat.handle.as_ref()?;
        let mut done = false;
        for ev in handle.events.try_iter() {
            match ev {
                FlatEvent::Files(mut files) => match &mut flat.pending {
                    Some(pending) => pending.append(&mut files),
                    None => {
                        self.view.entries.append(&mut files);
                        flat.unsorted = true;
                    }
                },
                FlatEvent::Done {
                    truncated,
                    unreadable,
                    ..
                } => {
                    flat.truncated = truncated;
                    flat.unreadable = unreadable;
                    done = true;
                }
            }
        }
        if done {
            flat.handle = None;
            if let Some(pending) = flat.pending.take() {
                let existing: HashSet<&PathBuf> = pending.iter().map(|e| &e.path).collect();
                self.view.selection.retain(|p| existing.contains(p));
                self.view.entries = pending;
            }
            flat.unsorted = false;
            self.view.version += 1;
            self.refresh_visible(folders_first, case);
            return None;
        }
        if !flat.unsorted {
            return None;
        }
        // Sorting a big list on every batch would make the window sluggish:
        // sort at most every 300 ms, and less often when sorting is slow.
        let every = Duration::from_millis(300).max(flat.sort_time * 5);
        let since = flat.last_sort.map_or(every, |t| t.elapsed());
        if since < every {
            return Some(every - since);
        }
        flat.unsorted = false;
        let started = Instant::now();
        self.refresh_visible(folders_first, case);
        if let Some(flat) = &mut self.flat {
            flat.sort_time = started.elapsed();
            flat.last_sort = Some(Instant::now());
        }
        None
    }
}
