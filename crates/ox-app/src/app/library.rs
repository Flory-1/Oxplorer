//! Libraries and the Gallery: reading the libraries for the sidebar, and
//! stopping Gallery walks nobody waits for any more.

use super::*;
use std::sync::atomic::Ordering;

/// How often the libraries are read again (the user may change them in Explorer).
const LIBRARIES_REFRESH: Duration = Duration::from_secs(60);

impl OxApp {
    pub(super) fn poll_libraries(&mut self) {
        if let Some(rx) = &self.libraries_rx {
            match rx.try_recv() {
                Ok(libs) => {
                    self.libraries = libs;
                    self.libraries_rx = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(_) => self.libraries_rx = None,
            }
        }
        let due = self
            .libraries_at
            .is_none_or(|at| at.elapsed() >= LIBRARIES_REFRESH);
        if due && self.libraries_rx.is_none() && cfg!(windows) {
            self.libraries_at = Some(Instant::now());
            let (tx, rx) = crossbeam_channel::bounded(1);
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(ox_core::library::list());
                ctx.request_repaint();
            });
            self.libraries_rx = Some(rx);
        }
        // A Gallery walk stops when its tab went elsewhere or got its result.
        let tabs = &self.tabs;
        self.gallery_walks.retain(|(id, cancel)| {
            let waiting = tabs
                .get(*id)
                .is_some_and(|t| *t.location() == Location::Gallery && t.view.loading.is_some());
            if !waiting {
                cancel.store(true, Ordering::Relaxed);
            }
            waiting
        });
    }

    /// Where files dropped or pasted into the library `file` go.
    pub fn library_save_folder(&self, file: &Path) -> Option<PathBuf> {
        let lib = self.libraries.iter().find(|l| l.file == file)?;
        lib.save_folder.clone()
    }

    /// The folders of the library `file` (read again if it isn't in the
    /// sidebar's list).
    fn library_folders(&self, file: &Path) -> Vec<PathBuf> {
        match self.libraries.iter().find(|l| l.file == file) {
            Some(lib) => lib.folders.clone(),
            None => ox_core::library::load(file).map_or_else(Vec::new, |l| l.folders),
        }
    }

    /// What a search in `root` walks: a library's folders (its
    /// `.library-ms` file is the root of a search from a library).
    pub(super) fn library_roots(&self, root: &Path) -> Option<Vec<PathBuf>> {
        ox_core::library::is_library_file(root).then(|| self.library_folders(root))
    }

    /// The name of a search's root: the folder's, or the library's.
    pub fn search_place(&self, root: &Path) -> String {
        if ox_core::library::is_library_file(root) {
            if let Some(lib) = self.libraries.iter().find(|l| l.file == root) {
                return lib.name.clone();
            }
            return Location::search_root(root.to_path_buf()).title();
        }
        ox_core::entry::display_name(root)
    }

    /// The library `here` shows and its folders, to watch for changes
    /// (with *Refresh automatically* on). Only those already known: the
    /// list is read in the background.
    pub(super) fn watched_library(&self, here: &Location) -> Option<(PathBuf, Vec<PathBuf>)> {
        let Location::Library { file, .. } = here else {
            return None;
        };
        let lib = self.libraries.iter().find(|l| l.file == *file)?;
        self.settings
            .browsing
            .auto_refresh
            .then(|| (file.clone(), lib.folders.clone()))
    }

    /// Files changed in a folder of the library `file`: reload the tabs
    /// that show it.
    pub(super) fn library_changed(&mut self, file: &Path) {
        for t in &mut self.tabs.tabs {
            if matches!(t.location(), Location::Library { file: f, .. } if f == file) {
                t.stale = true;
            }
        }
    }
}
