//! Per-frame polling of background work: tab loads, watcher, jobs, sizes, git, dropped files.

use super::*;

impl OxApp {
    pub(super) fn poll_tabs(&mut self) {
        let folders_first = self.settings.browsing.folders_first;
        let case = self.settings.search.case_sensitive;
        for tab in &mut self.tabs.tabs {
            if let Some(rx) = &tab.view.loading {
                if let Ok(result) = rx.try_recv() {
                    tab.view.loading = None;
                    match result {
                        Ok(listing) => {
                            let existing: HashSet<&PathBuf> =
                                listing.entries.iter().map(|e| &e.path).collect();
                            tab.view.selection.retain(|p| existing.contains(p));
                            tab.view.entries = listing.entries;
                            tab.view.bin = listing.bin;
                            tab.view.version += 1;
                            tab.view.error = None;
                        }
                        Err(e) => {
                            tab.view.entries.clear();
                            tab.view.error = Some(e);
                        }
                    }
                    tab.refresh_visible(folders_first, case);
                }
            }
            let mut changed = false;
            if let Some(search) = &mut tab.search {
                if let Some(handle) = &search.handle {
                    for ev in handle.events.try_iter() {
                        match ev {
                            SearchEvent::Results(mut r) => {
                                tab.view.entries.append(&mut r);
                                changed = true;
                            }
                            SearchEvent::Done {
                                elapsed,
                                scanned,
                                truncated,
                                from_index,
                            } => {
                                let mut limit = String::new();
                                if handle.from_text_index() {
                                    limit.push_str(tr(" · from the text index"));
                                }
                                if truncated {
                                    limit.push_str(tr(" · limit reached"));
                                }
                                let results =
                                    ox_core::i18n::count(tab.view.entries.len() as u64, "result");
                                search.done = Some(if from_index {
                                    ox_core::trf!("{results} in {v1} ms · ⚡ from the index ({scanned} items){limit}", results = results, v1 = elapsed.as_millis(), scanned = scanned, limit = limit)
                                } else {
                                    ox_core::trf!(
                                        "{results} · {scanned} items scanned in {secs} s{limit}",
                                        results = results,
                                        scanned = scanned,
                                        secs = ox_core::format::decimal(elapsed.as_secs_f64(), 2),
                                        limit = limit
                                    )
                                });
                                changed = true;
                            }
                        }
                    }
                }
                if search.done.is_some() {
                    search.handle = None;
                }
            }
            if changed {
                tab.refresh_visible(folders_first, case);
            }
            if let Some(again) = tab.poll_flat(folders_first, case) {
                self.ctx.request_repaint_after(again);
            }
        }
        self.poll_prop_values();
    }

    pub(super) fn poll_notices(&mut self) {
        while let Ok(notice) = self.notices.1.try_recv() {
            match notice {
                Notice::Nothing => {}
                Notice::Toast(text) => self.toast(text),
                Notice::OpenUrl(url) => self.ctx.open_url(egui::OpenUrl::new_tab(url)),
                Notice::Copy(text, toast) => {
                    self.ctx.copy_text(text);
                    self.toast(toast);
                }
                Notice::DeleteFailed(failed, dir) => {
                    self.mark_changed(dir.as_deref());
                    self.dialog = Some(Dialog::DeleteFailed(failed));
                }
                Notice::Changed(folders, toast) => {
                    for f in &folders {
                        self.mark_changed(Some(f));
                    }
                    if let Some(text) = toast {
                        self.toast(text);
                    }
                }
            }
        }
    }

    /// Run `work` on another thread; what it returns is shown or done next frame.
    pub(super) fn in_background(&self, work: impl FnOnce() -> Notice + Send + 'static) {
        let tx = self.notices.0.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_watcher(&mut self) {
        let panes = [Some(self.tabs.active), self.tabs.split_partner()];
        for (i, id) in panes.into_iter().enumerate() {
            let tab = id.and_then(|id| self.tabs.get(id));
            // A flat view watches the sub-folders too, unless it hit the
            // limit (a huge tree would keep re-listing itself).
            let flat = tab.and_then(|t| t.flat.as_ref());
            let walking = flat.is_some_and(|f| f.walking());
            let folder = tab
                .and_then(|t| t.location().folder().map(Path::to_path_buf))
                .filter(|_| self.settings.browsing.auto_refresh)
                .filter(|_| !flat.is_some_and(|f| f.truncated));
            // A library: all its folders.
            let library = tab.and_then(|t| self.watched_library(t.location()));
            let dirs = library
                .as_ref()
                .map_or_else(|| folder.clone().into_iter().collect(), |l| l.1.clone());
            self.watchers[i].watch_all(&dirs, flat.is_some());
            if self.watchers[i].take_dirty() && self.refresh_at[i].is_none() {
                // Debounce bursts of file system events.
                let wait = if flat.is_some() { 1000 } else { 250 };
                self.refresh_at[i] = Some(Instant::now() + Duration::from_millis(wait));
            }
            if let Some(at) = self.refresh_at[i] {
                if walking && Instant::now() >= at {
                    // Let the flat view finish listing before it starts over.
                    self.refresh_at[i] = Some(Instant::now() + Duration::from_millis(500));
                    self.ctx.request_repaint_after(Duration::from_millis(500));
                } else if Instant::now() >= at {
                    self.refresh_at[i] = None;
                    if let Some(folder) = &folder {
                        self.folder_sizes.invalidate(folder);
                        self.tabs.mark_stale(Some(folder));
                    }
                    if let Some((file, _)) = &library {
                        self.library_changed(file);
                    }
                } else {
                    self.ctx.request_repaint_after(at - Instant::now());
                }
            }
        }
    }

    pub(super) fn poll_jobs(&mut self) {
        for job in self.engine.jobs() {
            if !job.state().is_finished() || !self.seen_jobs.insert(job.id) {
                continue;
            }
            if let Err(e) = ox_core::transfer::history::append(&job) {
                log::warn!("could not write transfer log: {e}");
            }
            self.record_job(&job);
            self.mark_changed(Some(&job.destination));
            if job.kind == TransferKind::Move && !self.tags.is_empty() {
                // Tags follow moved items (those that left their old place).
                let moved = job
                    .sources
                    .iter()
                    .filter_map(|src| {
                        let to = job.destination.join(src.file_name()?);
                        (!src.exists() && to.exists()).then(|| (src.clone(), to))
                    })
                    .collect();
                self.push(Action::PathsMoved(moved));
            }
            for src in &job.sources {
                if let Some(parent) = src.parent() {
                    self.mark_changed(Some(parent));
                }
            }
            let errors = job.error_count();
            if errors > 0 {
                self.toast(ox_core::trf!(
                    "Transfer finished with {v1}. See the transfer panel.",
                    v1 = ox_core::i18n::count(errors as u64, "error")
                ));
                self.ui.show_transfers = true;
            }
        }
    }

    pub(super) fn poll_background(&mut self) {
        self.manage_fulltext();
        let mut finished = Vec::new();
        self.background
            .retain(|(label, rx, dir)| match rx.try_recv() {
                Ok(result) => {
                    finished.push((label.clone(), result, dir.clone()));
                    false
                }
                Err(crossbeam_channel::TryRecvError::Empty) => true,
                Err(_) => false,
            });
        for (label, result, dir) in finished {
            if let Err(e) = result {
                self.toast(ox_core::trf!("{label} failed: {e}", label = label, e = e));
            }
            self.mark_changed(dir.as_deref());
        }
    }

    /// Save the tag store and reload tag views.
    pub fn tags_changed(&mut self) {
        if let Err(e) = self.save_tags() {
            self.toast(ox_core::trf!("Could not save tags: {e}", e = e));
        }
        for t in &mut self.tabs.tabs {
            if matches!(t.location(), Location::Tagged(_)) {
                t.stale = true;
            }
        }
    }

    /// Files in `dir` changed: reload tabs showing it and forget folder sizes
    /// that included it.
    pub fn mark_changed(&mut self, dir: Option<&Path>) {
        if let Some(d) = dir {
            self.folder_sizes.invalidate(d);
            self.disk_usage.changed(d);
        }
        self.tabs.mark_stale(dir);
    }

    /// Ask for the sizes of the folders listed in the active tab, and copy
    /// finished sizes into the listing (so sorting by size works).
    pub(super) fn poll_folder_sizes(&mut self) {
        // The focused pane first, so its sizes come first.
        let panes: Vec<TabId> = [Some(self.tabs.active), self.tabs.split_partner()]
            .into_iter()
            .flatten()
            .filter(|&id| {
                self.tabs.get(id).is_some_and(|t| {
                    matches!(t.location(), Location::Folder(_)) && t.view.loading.is_none()
                })
            })
            .collect();
        if !self.settings.browsing.folder_sizes || panes.is_empty() {
            if self.sizes_key.take().is_some() {
                self.folder_sizes.clear_requests();
            }
            return;
        }
        let versions: Vec<(TabId, u64)> = panes
            .iter()
            .filter_map(|&id| self.tabs.get(id).map(|t| (id, t.view.version)))
            .collect();
        let key = (versions, self.folder_sizes.epoch());
        if self.sizes_key.as_ref() != Some(&key) {
            self.sizes_key = Some(key);
            let dirs = panes
                .iter()
                .filter_map(|&id| self.tabs.get(id))
                .flat_map(|t| t.view.entries.iter())
                .filter(|e| e.is_dir && !e.is_symlink)
                .map(|e| e.path.clone())
                .collect();
            self.folder_sizes.request(dirs);
            self.sizes_applied = u64::MAX;
        }
        let version = self.folder_sizes.version();
        if version == self.sizes_applied {
            return;
        }
        self.sizes_applied = version;
        let (folders_first, case) = (
            self.settings.browsing.folders_first,
            self.settings.search.case_sensitive,
        );
        for id in panes {
            let Some(tab) = self.tabs.get_mut(id) else {
                continue;
            };
            let mut changed = false;
            for e in tab.view.entries.iter_mut().filter(|e| e.is_dir) {
                if let Some(s) = self.folder_sizes.get(&e.path) {
                    changed |= e.size != s.bytes;
                    e.size = s.bytes;
                }
            }
            if changed && tab.sort_key == entry::SortKey::Size {
                tab.refresh_visible(folders_first, case);
            }
        }
    }

    /// Ask Git for the status of the repositories the panes show, whenever
    /// their listings reload, and hand the results to the tabs.
    pub(super) fn poll_git(&mut self) {
        let panes: Vec<TabId> = [Some(self.tabs.active), self.tabs.split_partner()]
            .into_iter()
            .flatten()
            .collect();
        let key: Vec<(TabId, u64)> = panes
            .iter()
            .filter_map(|&id| self.tabs.get(id).map(|t| (id, t.view.version)))
            .collect();
        if key != self.git_key {
            self.git_key = key;
            for &id in &panes {
                let Some(tab) = self.tabs.get_mut(id) else {
                    continue;
                };
                let root = tab
                    .location()
                    .folder()
                    .filter(|_| self.settings.browsing.git_status)
                    .and_then(ox_core::git::repo_root);
                let Some(root) = root else {
                    tab.view.git = None;
                    continue;
                };
                if tab.view.git.as_ref().is_some_and(|g| g.root != root) {
                    tab.view.git = None;
                }
                if self
                    .git_pending
                    .iter()
                    .any(|(t, r, _)| *t == id && *r == root)
                {
                    continue;
                }
                let (tx, rx) = crossbeam_channel::bounded(1);
                let ctx = self.ctx.clone();
                let dir = root.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(ox_core::git::status(&dir));
                    ctx.request_repaint();
                });
                self.git_pending.push((id, root, rx));
            }
        }
        let mut done = Vec::new();
        self.git_pending
            .retain(|(id, root, rx)| match rx.try_recv() {
                Ok(result) => {
                    done.push((*id, root.clone(), result));
                    false
                }
                Err(crossbeam_channel::TryRecvError::Empty) => true,
                Err(_) => false,
            });
        for (id, root, result) in done {
            let Some(tab) = self.tabs.get_mut(id) else {
                continue;
            };
            // Only if the tab is still in that repository.
            let same = tab
                .location()
                .folder()
                .is_some_and(|f| f.starts_with(&root));
            tab.view.git = match result {
                Ok(status) if same => Some(Arc::new(status)),
                _ => None,
            };
        }
    }

    /// Files dropped onto the window from other applications.
    pub(super) fn poll_dropped_files(&mut self, ctx: &Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        match self.target_folder() {
            Some(dest) => self.push(Action::Drop(dropped, dest)),
            None => self.toast(tr("Open a folder to drop files into it.")),
        }
    }
}
