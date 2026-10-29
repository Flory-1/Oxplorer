//! Remote files opened with their app: when the downloaded copy is saved,
//! offer to upload it to the server (Upload / Ignore / Always for this
//! file), and ask first if the server's file changed meanwhile. The bar is
//! drawn by `ui/remote/edits.rs`; the logic is `ox_core::remote::edits`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use ox_core::i18n::t as tr;
use ox_core::remote::edits::{self, EditedFiles, Stamp, UploadError};
use ox_core::remote::{RemoteAddr, RemoteEntry};
use ox_core::watcher::DirWatcher;

use crate::app::OxApp;

/// How often the copies are looked at while some are open.
const CHECK_EVERY: Duration = Duration::from_millis(700);

enum Done {
    Uploaded {
        local: PathBuf,
        sent: Option<Stamp>,
        remote: Stamp,
    },
    Conflict(PathBuf, Option<RemoteEntry>),
    Failed(PathBuf, String),
}

/// The answers of the bar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditAnswer {
    Upload,
    Ignore,
    /// Upload this file's changes without asking (this session).
    Always,
    /// Replace the server's newer file anyway.
    Replace,
    /// Keep the server's newer file.
    Keep,
}

pub struct EditState {
    pub files: EditedFiles,
    /// Downloads started to open a file: job id, copy, address, the
    /// server's file.
    opening: Vec<(u64, PathBuf, RemoteAddr, Stamp)>,
    watcher: Option<DirWatcher>,
    checked: Option<Instant>,
    /// The copy whose upload the bar offers.
    pub offer: Option<PathBuf>,
    /// The server's file changed too: the bar asks before replacing it.
    pub conflict: Option<(PathBuf, Option<RemoteEntry>)>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    swept: bool,
}

impl Default for EditState {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            files: EditedFiles::default(),
            opening: Vec::new(),
            watcher: None,
            checked: None,
            offer: None,
            conflict: None,
            tx,
            rx,
            swept: false,
        }
    }
}

impl Drop for EditState {
    /// The app closes: remove the copies (those with changes that weren't
    /// uploaded stay a day) and this session's preview copies.
    fn drop(&mut self) {
        self.files.clean_up();
        let prefix = format!("{}-", std::process::id());
        if let Ok(dirs) = std::fs::read_dir(edits::temp_root().join("preview")) {
            for d in dirs.flatten() {
                if d.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_dir_all(d.path());
                }
            }
        }
    }
}

impl OxApp {
    /// A remote file is downloaded to `local` (job `job`) to be opened.
    pub(super) fn remote_edit_download(
        &mut self,
        job: u64,
        local: PathBuf,
        addr: RemoteAddr,
        remote: Stamp,
    ) {
        self.remote.edits.opening.push((job, local, addr, remote));
    }

    /// An opened copy that still has changes the server doesn't have (it
    /// is opened again instead of being downloaded over).
    pub(super) fn remote_edit_unsent(&self, local: &Path) -> bool {
        self.remote
            .edits
            .files
            .get(local)
            .is_some_and(|f| f.has_changes())
    }

    /// The download of job `job` was opened with its app: follow the copy.
    pub(super) fn remote_edit_opened(&mut self, job: u64) {
        let edits = &mut self.remote.edits;
        let Some(i) = edits.opening.iter().position(|o| o.0 == job) else {
            return;
        };
        let (_, local, addr, remote) = edits.opening.remove(i);
        edits.files.track(local, addr, remote);
        if edits.watcher.is_none() {
            let ctx = self.ctx.clone();
            let mut w = DirWatcher::new(move || ctx.request_repaint());
            w.watch_tree(Some(&edits::temp_root().join("edit")), true);
            edits.watcher = Some(w);
        }
        self.toast(tr(
            "Opened a downloaded copy. When you save it, Oxplorer offers to upload the changes.",
        ));
    }

    pub(super) fn poll_remote_edits(&mut self) {
        if !self.remote.edits.swept {
            self.remote.edits.swept = true;
            std::thread::spawn(|| edits::sweep(&edits::temp_root(), edits::MAX_AGE));
        }
        while let Ok(done) = self.remote.edits.rx.try_recv() {
            self.on_edit_done(done);
        }
        let state = &mut self.remote.edits;
        if state.files.is_empty() {
            return;
        }
        let changed = state.watcher.as_ref().is_some_and(|w| w.take_dirty());
        let due = state.checked.is_none_or(|t| t.elapsed() >= CHECK_EVERY);
        if !changed && !due {
            return;
        }
        state.checked = Some(Instant::now());
        let saved = state.files.saved(Instant::now());
        if state.files.settling() {
            // Saved copies settle before they are offered: look again soon.
            self.ctx.request_repaint_after(CHECK_EVERY);
        }
        for local in saved {
            let state = &mut self.remote.edits;
            let always = state.files.get(&local).is_some_and(|f| f.always);
            if always {
                self.remote_edit_upload(&local, false);
            } else if state.offer.is_none() && state.conflict.is_none() {
                state.offer = Some(local);
            }
        }
    }

    fn on_edit_done(&mut self, done: Done) {
        match done {
            Done::Uploaded {
                local,
                sent,
                remote,
            } => {
                let state = &mut self.remote.edits;
                state.files.uploaded(&local, sent, remote);
                if let Some(f) = state.files.get(&local) {
                    let text = ox_core::trf!(
                        "Uploaded \"{name}\" to {server}.",
                        name = f.addr.name(),
                        server = f.addr.server_url()
                    );
                    self.toast(text);
                }
            }
            Done::Conflict(local, now) => {
                if let Some(f) = self.remote.edits.files.get_mut(&local) {
                    f.busy = false;
                }
                self.remote.edits.offer = None;
                self.remote.edits.conflict = Some((local, now));
            }
            Done::Failed(local, e) => {
                if let Some(f) = self.remote.edits.files.get_mut(&local) {
                    f.busy = false;
                }
                // Not offered again until it changes (the error says why).
                self.remote.edits.files.mark_seen(&local);
                self.toast(ox_core::trf!(
                    "Couldn't upload \"{name}\": {e}",
                    name = ox_core::entry::display_name(&local),
                    e = e
                ));
            }
        }
    }

    /// An answer of the bar about `local`.
    pub(crate) fn remote_edit_answer(&mut self, local: &Path, answer: EditAnswer) {
        let state = &mut self.remote.edits;
        state.offer = None;
        state.conflict = None;
        match answer {
            EditAnswer::Upload => self.remote_edit_upload(local, false),
            EditAnswer::Always => {
                if let Some(f) = self.remote.edits.files.get_mut(local) {
                    f.always = true;
                }
                self.remote_edit_upload(local, false);
            }
            EditAnswer::Replace => self.remote_edit_upload(local, true),
            EditAnswer::Ignore | EditAnswer::Keep => self.remote.edits.files.mark_seen(local),
        }
    }

    /// Upload the copy in the background (`force`: even if the server's
    /// file changed since it was downloaded).
    fn remote_edit_upload(&mut self, local: &Path, force: bool) {
        let Some(file) = self.remote.edits.files.get(local).cloned() else {
            return;
        };
        let Some(fs) = self.remote.conns.cached(&file.addr) else {
            self.remote.edits.files.mark_seen(local);
            self.toast(ox_core::trf!(
                "Connect to {server} first.",
                server = file.addr.server_url()
            ));
            return;
        };
        if let Some(f) = self.remote.edits.files.get_mut(local) {
            f.busy = true;
        }
        let tx = self.remote.edits.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let done = match edits::upload(fs.as_ref(), &file.local, &file.addr, file.remote, force)
            {
                Ok((sent, remote)) => Done::Uploaded {
                    local: file.local,
                    sent,
                    remote,
                },
                Err(UploadError::Conflict(now)) => Done::Conflict(file.local, now),
                Err(UploadError::Failed(e)) => Done::Failed(file.local, e),
            };
            let _ = tx.send(done);
            ctx.request_repaint();
        });
    }
}
