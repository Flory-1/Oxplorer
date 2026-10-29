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
}
