//! Loading what the active tab shows: folder listings, searches and the
//! other places, in the background.

use super::*;

impl OxApp {
    pub(super) fn load_active_tab(&mut self) {
        let show_hidden = self.settings.browsing.show_hidden;
        let ctx = self.ctx.clone();
        let search_opts = SearchOptions {
            include_hidden: self.settings.search.include_hidden,
            case_sensitive: self.settings.search.case_sensitive,
            max_results: self.settings.search.max_results,
            threads: self.settings.search.threads,
        };
        let drives = self.index_drives();
        let index = self
            .settings
            .search
            .index_enabled
            .then(|| self.index.index())
            .flatten();
        // A search from a library walks all its folders.
        let library_roots = match self.tabs.active().location() {
            Location::Search { root: Some(r), .. } if self.tabs.active().stale => {
                self.library_roots(r)
            }
            _ => None,
        };
        let tab = self.tabs.active_mut();
        if !tab.stale {
            return;
        }
        tab.stale = false;
        match tab.location().clone() {
            Location::Folder(path) if tab.is_flat() => {
                let opts = ox_core::flat::FlatOptions {
                    show_hidden,
                    ..Default::default()
                };
                tab.start_flat(path, opts, move || ctx.request_repaint());
            }
            Location::Folder(path) => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Listing::folder(&path, show_hidden).map_err(|e| e.to_string()));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Search { root, query } => {
                let mut q = Query::parse(&query, search_opts.case_sensitive);
                q.resolve_tags(&self.tags);
                if let Some(text_index) = self.fulltext.index() {
                    q.use_text_index(text_index);
                }
                tab.view = Default::default();
                let repaint = move || ctx.request_repaint();
                // Answer from the index when it covers the place we search in
                // (it only knows file system names, not what's in archives).
                let handle = match index {
                    _ if library_roots.is_some() => {
                        let roots = library_roots.unwrap_or_default();
                        search::start(roots, q, search_opts, repaint)
                    }
                    Some(idx)
                        if !q.in_archives() && root.as_deref().is_none_or(|r| idx.covers(r)) =>
                    {
                        search::start_indexed(idx, root, q, search_opts, repaint)
                    }
                    _ => {
                        let roots = root.map(|r| vec![r]).unwrap_or(drives);
                        search::start(roots, q, search_opts, repaint)
                    }
                };
                tab.search = Some(SearchState {
                    handle: Some(handle),
                    done: None,
                });
            }
            Location::RecycleBin => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Listing::recycle_bin());
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Archive { archive, inner } => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Listing::archive(&archive, &inner).map_err(|e| e.to_string()));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Tagged(color) => {
                let paths = self.tags.with_tag(color);
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Ok(Listing::items(&paths)));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Library { file, .. } => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send(Listing::library(&file, show_hidden));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
            }
            Location::Gallery => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                let cancel = Arc::new(AtomicBool::new(false));
                let flag = cancel.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Ok(Listing::gallery(show_hidden, &flag)));
                    ctx.request_repaint();
                });
                tab.view.loading = Some(rx);
                // A walk started earlier for this tab isn't needed any more.
                self.gallery_walks.retain(|(id, old)| {
                    let same = *id == tab.id;
                    if same {
                        old.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    !same
                });
                self.gallery_walks.push((tab.id, cancel));
            }
            Location::Cleanup { root, mode } => {
                self.cleanups.start(tab.id, root, mode, show_hidden, ctx);
            }
            Location::Remote(addr) => {
                let opts = ox_core::remote::ConnectOptions::for_addr(&self.settings.remote, &addr);
                tab.view.loading = Some(self.remote.list(addr, opts, show_hidden, &ctx));
            }
            // Disk usage views get their scan in `poll_disk_usage`.
            Location::Home | Location::Settings | Location::Help | Location::DiskUsage(_) => {}
        }
    }
}
