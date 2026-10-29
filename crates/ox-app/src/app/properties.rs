//! The Properties dialog's data: what it shows of the selected items and
//! changing their attributes.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use ox_core::entry::{self, FileEntry};
use ox_core::fsops;
use ox_core::i18n::t as tr;

use super::{Dialog, OxApp};
use crate::ui;

pub struct Properties {
    pub paths: Vec<PathBuf>,
    pub entries: Vec<FileEntry>,
    /// (bytes, files, folders), computed in the background.
    pub totals: Option<(u64, u64, u64)>,
    pub rx: Option<Receiver<(u64, u64, u64)>>,
    pub cancel: Arc<AtomicBool>,
    /// More about a single item, found out in the background.
    pub more: Option<MoreProperties>,
    pub more_rx: Option<Receiver<MoreProperties>>,
    /// The item's state in its Git repository.
    pub git: Option<&'static str>,
    /// The Read-only and Hidden boxes: `None` while the items differ.
    pub readonly: Option<bool>,
    pub hidden: Option<bool>,
}

/// What the Properties dialog shows of a single item besides its listing.
pub struct MoreProperties {
    pub details: ox_core::details::Details,
    /// The file type's own details (dimensions, length, artist, …).
    pub facts: Vec<(&'static str, String)>,
    pub last_commit: Option<ox_core::git::Commit>,
}

impl OxApp {
    pub(super) fn open_properties(&mut self, paths: Vec<PathBuf>) {
        let entries: Vec<FileEntry> = paths
            .iter()
            .filter_map(|p| FileEntry::from_path(p).ok())
            .collect();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let worker_paths = paths.clone();
        let worker_cancel = cancel.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut totals = (0, 0, 0);
            for p in &worker_paths {
                if p.is_dir() {
                    let (b, f, d) = fsops::folder_size(p, &worker_cancel);
                    totals.0 += b;
                    totals.1 += f;
                    totals.2 += d + 1;
                } else if let Ok(m) = std::fs::metadata(p) {
                    totals.0 += m.len();
                    totals.1 += 1;
                }
            }
            let _ = tx.send(totals);
            ctx.request_repaint();
        });
        let mut more_rx = None;
        if let [e] = entries.as_slice() {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let (e, ctx) = (e.clone(), self.ctx.clone());
            std::thread::spawn(move || {
                let more = MoreProperties {
                    details: ox_core::details::read(&e.path, e.is_dir),
                    facts: ui::preview::type_facts(&e),
                    last_commit: ox_core::git::repo_root(&e.path)
                        .and_then(|root| ox_core::git::last_commit(&root, &e.path).ok())
                        .flatten(),
                };
                let _ = tx.send(more);
                ctx.request_repaint();
            });
            more_rx = Some(rx);
        }
        let git = match (self.tabs.active().view.git.as_ref(), entries.as_slice()) {
            (Some(git), [e]) if e.path.starts_with(&git.root) => Some(
                git.state(&e.path)
                    .map_or(tr("Unchanged"), |s| tr(s.label())),
            ),
            _ => None,
        };
        // Checked when all items are, empty when none is, mixed otherwise.
        let all_or_none = |on: fn(&FileEntry) -> bool| {
            let n = entries.iter().filter(|e| on(e)).count();
            (n == 0 || n == entries.len()).then_some(n > 0)
        };
        self.dialog = Some(Dialog::Properties(Box::new(Properties {
            readonly: all_or_none(|e| e.readonly),
            hidden: all_or_none(|e| e.hidden),
            paths,
            entries,
            totals: None,
            rx: Some(rx),
            cancel,
            more: None,
            more_rx,
            git,
        })));
    }

    /// Make the items read-only or not, hidden or not (`None` leaves it).
    pub(super) fn set_attributes(
        &mut self,
        paths: &[PathBuf],
        readonly: Option<bool>,
        hidden: Option<bool>,
    ) {
        for p in paths {
            if let Err(e) = ox_core::details::set_attributes(p, readonly, hidden) {
                self.toast(ox_core::trf!(
                    "Couldn't change {v1}: {e}",
                    v1 = entry::display_name(p),
                    e = e
                ));
            }
        }
        self.tabs.active_mut().stale = true;
    }
}
