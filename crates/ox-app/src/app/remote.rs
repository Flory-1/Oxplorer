//! Remote locations (SFTP, FTP, WebDAV) in the app: connecting (with the
//! password and host key questions), listing in the background, the file
//! operations of remote folders and their transfers. The protocols live
//! in `ox_core::remote`, the dialogs in `ui/remote.rs`.

mod edits;
mod resume;
mod stream;

pub use edits::EditAnswer;

use super::*;
use ox_core::entry::FileEntry;
use ox_core::remote::{
    self, ConnectOptions, Connections, HostKeyQuestion, RemoteAddr, RemoteError, RemoteFs,
};
use ox_core::transfer::{JobState, RemoteSide};

/// Remote files up to this size are downloaded for the preview pane.
const MAX_PREVIEW: u64 = 16 * 1024 * 1024;

pub enum RemoteAction {
    /// Open an address (connecting first if needed); `true`: in a new tab.
    Open(RemoteAddr, bool),
    /// An address typed into the address bar (it may hold a password).
    OpenTyped(String),
    Disconnect(RemoteAddr),
    /// The dialog for a new place, filled in from this address if any.
    Add(Option<RemoteAddr>),
    Edit(usize),
    Remove(usize),
    ForgetPassword(RemoteAddr),
}

/// What to do once connected.
#[derive(Clone, Copy)]
pub enum After {
    /// Show the folder (`true`: in a new tab).
    Open(bool),
    /// List the server's tabs again.
    Reload,
}

enum Event {
    Connected(RemoteAddr, After),
    /// The connection needs the user: a password or a decision on the host key.
    Ask(RemoteAddr, RemoteError, After),
    Failed(RemoteAddr, String),
    /// A remote folder changed: list it again, show the error, select (and
    /// rename) an item.
    Changed {
        dir: RemoteAddr,
        error: Option<String>,
        select: Option<PathBuf>,
        rename: bool,
    },
    /// The downloaded copy of a remote file for the preview.
    Preview(PathBuf, Result<PathBuf, String>),
}

/// The open remote dialog (drawn by `ui/remote.rs`).
pub enum RemoteDialog {
    Place(Box<crate::ui::remote::PlaceForm>),
    Password {
        addr: RemoteAddr,
        text: String,
        remember: bool,
        rejected: bool,
        then: After,
        focus: bool,
    },
    HostKey {
        addr: RemoteAddr,
        question: Box<HostKeyQuestion>,
        then: After,
        /// A changed key: the user confirmed checking the new one.
        confirmed: bool,
    },
    Rename {
        path: PathBuf,
        text: String,
        focus: bool,
    },
}

enum PreviewCopy {
    Loading,
    Ready(PathBuf),
    Failed(String),
}

pub struct RemoteState {
    pub conns: Arc<Connections>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    pub dialog: Option<RemoteDialog>,
    /// Servers being connected to (server keys).
    pub connecting: HashSet<String>,
    /// Finished remote jobs already looked at.
    seen_jobs: HashSet<u64>,
    /// Downloads started to open a file: job id → the local copy.
    open_after: Vec<(u64, PathBuf)>,
    /// The remote file in the preview and its downloaded copy.
    preview: Option<(PathBuf, PreviewCopy)>,
    /// Opened remote files and uploading their changes.
    pub edits: edits::EditState,
    resume: resume::ResumeState,
}

impl Default for RemoteState {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            conns: Default::default(),
            tx,
            rx,
            dialog: None,
            connecting: HashSet::new(),
            seen_jobs: HashSet::new(),
            open_after: Vec::new(),
            preview: None,
            edits: Default::default(),
            resume: Default::default(),
        }
    }
}

/// What the preview pane shows for a selected item.
pub enum RemotePreview {
    /// Not remote: preview it as usual.
    Local,
    /// The downloaded copy, with the remote item's name and dates.
    Ready(FileEntry),
    /// Only the item's details, and this text (if any).
    Info(Option<String>),
}

impl RemoteState {
    /// Run `work` with the server's connection on another thread.
    fn with_fs(
        &self,
        ctx: &Context,
        addr: RemoteAddr,
        opts: ConnectOptions,
        work: impl FnOnce(&dyn RemoteFs, RemoteAddr) -> Event + Send + 'static,
    ) {
        let (conns, tx, ctx) = (self.conns.clone(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let event = match conns.get(&addr, &opts) {
                Ok(fs) => work(fs.as_ref(), addr),
                Err(e) if e.needs_user() => Event::Ask(addr, e, After::Reload),
                Err(e) => Event::Failed(addr, e.to_string()),
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }

    /// List a remote folder in the background (for the file view).
    pub fn list(
        &self,
        addr: RemoteAddr,
        opts: ConnectOptions,
        show_hidden: bool,
        ctx: &Context,
    ) -> Receiver<Result<Listing, String>> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (conns, events, ctx) = (self.conns.clone(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = conns.get(&addr, &opts).and_then(|fs| {
                let dir = match addr.path.as_str() {
                    "" => addr.with_path(&fs.home()?),
                    _ => addr.clone(),
                };
                remote::list_entries(fs.as_ref(), &dir, show_hidden)
            });
            if let Err(e) = &result {
                if e.needs_user() {
                    let _ = events.send(Event::Ask(addr.clone(), e.clone(), After::Reload));
                }
            }
            let _ = tx.send(result.map(Listing::remote).map_err(|e| e.to_string()));
            ctx.request_repaint();
        });
        rx
    }
}

fn is_remote(p: &Path) -> bool {
    remote::is_remote_path(p)
}

fn any_remote(paths: &[PathBuf]) -> bool {
    paths.iter().any(|p| is_remote(p))
}

/// The folder an item is in (`/` for items at the top).
fn parent_dir(addr: &RemoteAddr) -> RemoteAddr {
    addr.parent().unwrap_or_else(|| addr.with_path("/"))
}

impl OxApp {
    fn remote_opts(&self, addr: &RemoteAddr) -> ConnectOptions {
        ConnectOptions::for_addr(&self.settings.remote, addr)
    }

    /// Connect (in the background), then do `then`.
    fn remote_connect(&mut self, addr: RemoteAddr, then: After) {
        self.remote.connecting.insert(addr.server_key());
        let opts = self.remote_opts(&addr);
        self.remote.with_fs(&self.ctx, addr, opts, move |fs, addr| {
            match addr.path.as_str() {
                "" => match fs.home() {
                    Ok(home) => Event::Connected(addr.with_path(&home), then),
                    Err(e) => Event::Failed(addr, e.to_string()),
                },
                _ => Event::Connected(addr, then),
            }
        });
    }

    /// Actions on remote items are done here; the rest go on to `process`.
    pub(super) fn remote_filter(&mut self, action: Action) -> Option<Action> {
        let here_remote = self.tabs.active().location().is_remote();
        match action {
            Action::Navigate(Location::Remote(addr))
                if addr.path.is_empty() || !self.remote.conns.is_connected(&addr) =>
            {
                self.remote_connect(addr, After::Open(false));
            }
            Action::OpenInNewTab(Location::Remote(addr), _)
                if addr.path.is_empty() || !self.remote.conns.is_connected(&addr) =>
            {
                self.remote_connect(addr, After::Open(true));
            }
            Action::Open(path) if is_remote(&path) => self.remote_open(path),
            Action::Delete(paths, _) | Action::MoveToBin(paths) if any_remote(&paths) => {
                // Nothing on a server goes to the Recycle Bin.
                self.dialog = Some(Dialog::ConfirmDelete {
                    paths,
                    permanent: true,
                });
            }
            Action::DeleteConfirmed(paths, _) if any_remote(&paths) => self.remote_delete(paths),
            Action::Rename(path) if is_remote(&path) => {
                self.remote.dialog = Some(RemoteDialog::Rename {
                    text: entry::display_name(&path),
                    path,
                    focus: true,
                });
            }
            Action::NewFolder(dir) if is_remote(&dir) => self.remote_new_folder(&dir),
            // A remote folder is kept as a remote location, not a favorite.
            Action::Pin(path) if is_remote(&path) => {
                self.remote_action(RemoteAction::Add(RemoteAddr::from_path(&path)));
            }
            // The search box only filters a remote folder.
            Action::Search(..) | Action::SearchLive(..) if here_remote => {}
            Action::ToggleFlatView if here_remote => self.remote_unsupported(),
            Action::NewFile(p)
            | Action::NewFromItem(p, _)
            | Action::OpenTerminal(p)
            | Action::Reveal(p)
            | Action::OpenWith(p)
            | Action::NewLink(p, _)
            | Action::ShowLinkTarget(p)
                if is_remote(&p) =>
            {
                self.remote_unsupported()
            }
            Action::Properties(p)
            | Action::ShellMenu(p)
            | Action::BulkRename(p)
            | Action::Compress(p)
            | Action::OpenInEditor(p)
            | Action::SetAttributes(p, ..)
            | Action::KeepOnDevice(p, _)
                if any_remote(&p) =>
            {
                self.remote_unsupported()
            }
            other => return Some(other),
        }
        None
    }

    fn remote_unsupported(&mut self) {
        self.toast(tr("This isn't available in remote locations."));
    }

    pub(super) fn remote_action(&mut self, action: RemoteAction) {
        match action {
            RemoteAction::Open(addr, new_tab) => {
                let loc = Location::Remote(addr);
                self.process(if new_tab {
                    Action::OpenInNewTab(loc, true)
                } else {
                    Action::Navigate(loc)
                });
            }
            RemoteAction::OpenTyped(text) => {
                let Some(addr) = RemoteAddr::parse(&text) else {
                    self.toast(ox_core::trf!("Can't find \"{text}\".", text = text));
                    return;
                };
                if let Some(pw) = remote::url::password_in(&text) {
                    let _ = self.remote.conns.set_password(&addr, &pw, false);
                }
                self.process(Action::Navigate(Location::Remote(addr)));
            }
            RemoteAction::Disconnect(addr) => {
                self.remote.conns.disconnect(&addr);
                self.toast(ox_core::trf!(
                    "Disconnected from {server}.",
                    server = addr.server_url()
                ));
            }
            RemoteAction::Add(addr) => {
                let form = crate::ui::remote::PlaceForm::new(addr.as_ref());
                self.remote.dialog = Some(RemoteDialog::Place(Box::new(form)));
            }
            RemoteAction::Edit(i) => {
                if let Some(place) = self.settings.remote.0.get(i) {
                    let form = crate::ui::remote::PlaceForm::edit(i, place);
                    self.remote.dialog = Some(RemoteDialog::Place(Box::new(form)));
                }
            }
            RemoteAction::Remove(i) => {
                if i < self.settings.remote.0.len() {
                    let place = self.settings.remote.0.remove(i);
                    remote::credentials::delete(&place.addr);
                    self.process(Action::SettingsChanged);
                }
            }
            RemoteAction::ForgetPassword(addr) => {
                self.remote.conns.forget_password(&addr);
                self.toast(tr("The saved password was removed."));
            }
        }
    }

    /// Save a place from the dialog (`index`: the place it replaces).
    pub(crate) fn save_remote_place(
        &mut self,
        index: Option<usize>,
        place: remote::RemotePlace,
        password: &str,
    ) {
        if !password.is_empty() {
            if let Err(e) =
                self.remote
                    .conns
                    .set_password(&place.addr, password, place.remember_password)
            {
                self.toast(ox_core::trf!("Couldn't save the password: {e}", e = e));
            }
        }
        if !place.remember_password {
            remote::credentials::delete(&place.addr);
        }
        let places = &mut self.settings.remote.0;
        match index.filter(|&i| i < places.len()) {
            Some(i) => places[i] = place,
            None => places.push(place),
        }
        self.process(Action::SettingsChanged);
    }

    /// The password dialog was answered: remember it and try again.
    pub(crate) fn remote_password(
        &mut self,
        addr: RemoteAddr,
        password: &str,
        remember: bool,
        then: After,
    ) {
        if let Err(e) = self.remote.conns.set_password(&addr, password, remember) {
            self.toast(ox_core::trf!("Couldn't save the password: {e}", e = e));
        }
        if let Some(i) = self
            .settings
            .remote
            .0
            .iter()
            .position(|p| p.addr.same_server(&addr))
        {
            if self.settings.remote.0[i].remember_password != remember {
                self.settings.remote.0[i].remember_password = remember;
                self.process(Action::SettingsChanged);
            }
        }
        self.remote_retry(addr, then);
    }

    /// The host key was trusted: save it and try again.
    pub(crate) fn remote_trust(&mut self, addr: RemoteAddr, q: &HostKeyQuestion, then: After) {
        match q.trust() {
            Ok(()) => self.remote_retry(addr, then),
            Err(e) => self.toast(ox_core::trf!("Couldn't save the key: {e}", e = e)),
        }
    }

    fn remote_retry(&mut self, addr: RemoteAddr, then: After) {
        match then {
            After::Open(_) => self.remote_connect(addr, then),
            After::Reload => self.mark_remote_stale(&addr, true),
        }
    }

    /// List the tabs showing `dir` again (`server`: every folder of its server).
    fn mark_remote_stale(&mut self, dir: &RemoteAddr, server: bool) {
        for t in &mut self.tabs.tabs {
            if let Location::Remote(a) = t.location() {
                if a == dir || (server && a.same_server(dir)) {
                    t.stale = true;
                }
            }
        }
    }

    pub(super) fn poll_remote(&mut self) {
        while let Ok(event) = self.remote.rx.try_recv() {
            self.on_remote_event(event);
        }
        self.poll_remote_edits();
        self.poll_remote_resume();
        let folder = match self.tabs.active().location() {
            Location::Remote(a) if self.settings.appearance.remote_thumbnails => Some(a.clone()),
            _ => None,
        };
        self.thumbs.set_remote_folder(folder, &self.remote.conns);
        // After downloads and uploads: list the remote folders again, open
        // the files that were downloaded to be opened.
        for job in self.engine.jobs() {
            if !job.is_remote()
                || !job.state().is_finished()
                || !self.remote.seen_jobs.insert(job.id)
            {
                continue;
            }
            let paths = std::iter::once(job.destination.clone()).chain(
                job.sources
                    .iter()
                    .filter(|_| job.kind == TransferKind::Move)
                    .filter_map(|s| RemoteAddr::from_path(s).map(|a| parent_dir(&a).to_path())),
            );
            for p in paths.collect::<Vec<_>>() {
                if let Some(a) = RemoteAddr::from_path(&p) {
                    self.mark_remote_stale(&a, false);
                }
            }
            if let Some(i) = self
                .remote
                .open_after
                .iter()
                .position(|(id, _)| *id == job.id)
            {
                let (_, local) = self.remote.open_after.remove(i);
                if job.state() == JobState::Completed && job.error_count() == 0 {
                    match fsops::open_default(&local) {
                        Ok(()) => self.remote_edit_opened(job.id),
                        Err(e) => self.toast(ox_core::trf!(
                            "Couldn't open {v1}: {e}",
                            v1 = local.display(),
                            e = e
                        )),
                    }
                }
            }
        }
    }

    fn on_remote_event(&mut self, event: Event) {
        match event {
            Event::Connected(addr, then) => {
                self.remote.connecting.remove(&addr.server_key());
                match then {
                    After::Open(true) => {
                        let after = Some(self.tabs.active);
                        self.tabs
                            .open(Location::Remote(addr), after, true, &self.settings);
                    }
                    After::Open(false) => self.navigate(Location::Remote(addr)),
                    After::Reload => self.mark_remote_stale(&addr, true),
                }
            }
            Event::Ask(addr, error, then) => {
                self.remote.connecting.remove(&addr.server_key());
                if self.remote.dialog.is_some() {
                    return;
                }
                self.remote.dialog = Some(match error {
                    RemoteError::HostKey(question) => RemoteDialog::HostKey {
                        addr,
                        question,
                        then,
                        confirmed: false,
                    },
                    RemoteError::NeedPassword { rejected } => RemoteDialog::Password {
                        remember: self
                            .settings
                            .remote
                            .for_addr(&addr)
                            .is_some_and(|p| p.remember_password),
                        addr,
                        text: String::new(),
                        rejected,
                        then,
                        focus: true,
                    },
                    _ => return,
                });
            }
            Event::Failed(addr, message) => {
                self.remote.connecting.remove(&addr.server_key());
                self.toast(format!("{}: {message}", addr.server_url()));
            }
            Event::Changed {
                dir,
                error,
                select,
                rename,
            } => {
                self.mark_remote_stale(&dir, false);
                if let Some(e) = error {
                    self.toast(e);
                }
                if let Some(path) = select {
                    let tab = self.tabs.active_mut();
                    if tab.location() == &Location::Remote(dir) {
                        tab.view.select_after_load = Some(path.clone());
                    }
                    if rename {
                        self.process(Action::Rename(path));
                    }
                }
            }
            Event::Preview(path, result) => {
                if let Some((p, copy)) = &mut self.remote.preview {
                    if *p == path {
                        *copy = match result {
                            Ok(local) => PreviewCopy::Ready(local),
                            Err(e) => PreviewCopy::Failed(e),
                        };
                    }
                }
            }
        }
    }

    /// Open a remote item: a folder in this tab, a file with its app (after
    /// downloading it through the transfer engine, with progress).
    fn remote_open(&mut self, path: PathBuf) {
        let Some(addr) = RemoteAddr::from_path(&path) else {
            return;
        };
        let known = self
            .tabs
            .active()
            .view
            .entries
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.is_dir);
        if known != Some(false) {
            // A folder, or (when unknown) the listing tells.
            self.process(Action::Navigate(Location::Remote(addr)));
            return;
        }
        // A copy of its own, followed to upload its changes.
        let local = remote::edits::copy_path(&addr);
        if self.remote_edit_unsent(&local) {
            // Opened before and changed: not downloaded over.
            if let Err(e) = fsops::open_default(&local) {
                self.toast(e.to_string());
            }
            return;
        }
        let Some(fs) = self.remote.conns.cached(&addr) else {
            self.toast(tr(
                "Not connected to this server any more. Refresh to connect again.",
            ));
            return;
        };
        let dir = local.parent().map(Path::to_path_buf).unwrap_or_default();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.toast(e.to_string());
            return;
        }
        let entry = self
            .tabs
            .active()
            .view
            .entries
            .iter()
            .find(|e| e.path == path);
        let stamp = remote::edits::Stamp {
            size: entry.map_or(0, |e| e.size),
            modified: entry.and_then(|e| e.modified),
        };
        let req = TransferRequest {
            remote: Some(RemoteSide::new(fs, &addr)),
            pairs: vec![(path, local.clone())],
            ..TransferRequest::new(
                TransferKind::Copy,
                Vec::new(),
                dir,
                ox_core::transfer::ConflictPolicy::Overwrite,
            )
        };
        let job = self.engine.submit(req);
        self.remote.open_after.push((job.id, local.clone()));
        self.remote_edit_download(job.id, local, addr, stamp);
        self.ui.show_transfers = true;
    }

    fn remote_delete(&mut self, paths: Vec<PathBuf>) {
        let entries = &self.tabs.active().view.entries;
        let items: Vec<(RemoteAddr, bool)> = paths
            .iter()
            .filter_map(|p| {
                let is_dir = entries
                    .iter()
                    .find(|e| e.path == *p)
                    .is_some_and(|e| e.is_dir);
                Some((RemoteAddr::from_path(p)?, is_dir))
            })
            .collect();
        let Some((first, _)) = items.first().cloned() else {
            return;
        };
        let opts = self.remote_opts(&first);
        self.remote
            .with_fs(&self.ctx, first, opts, move |fs, first| {
                let mut errors = Vec::new();
                for (addr, is_dir) in &items {
                    if let Err(e) = remote::remove_all(fs, &addr.path, *is_dir) {
                        errors.push(format!("{}: {e}", addr.name()));
                    }
                }
                Event::Changed {
                    dir: parent_dir(&first),
                    error: (!errors.is_empty()).then(|| errors.join("\n")),
                    select: None,
                    rename: false,
                }
            });
        self.tabs.active_mut().view.clear_selection();
    }

    fn remote_new_folder(&mut self, dir: &Path) {
        let Some(dir) = RemoteAddr::from_path(dir) else {
            return;
        };
        let base = tr("New folder").to_owned();
        let opts = self.remote_opts(&dir);
        self.remote.with_fs(&self.ctx, dir, opts, move |fs, dir| {
            let names: Vec<String> = fs
                .list(&dir.path)
                .map(|l| l.into_iter().map(|e| e.name).collect())
                .unwrap_or_default();
            let name = (1..)
                .map(|i| match i {
                    1 => base.clone(),
                    i => format!("{base} ({i})"),
                })
                .find(|n| !names.contains(n))
                .unwrap_or(base);
            let created = dir.join(&name);
            match fs.mkdir(&created.path) {
                Ok(()) => Event::Changed {
                    dir,
                    error: None,
                    select: Some(created.to_path()),
                    rename: true,
                },
                Err(e) => Event::Changed {
                    dir,
                    error: Some(ox_core::trf!("Couldn't create folder: {e}", e = e)),
                    select: None,
                    rename: false,
                },
            }
        });
    }

    /// Rename a remote item (from the rename dialog).
    pub(crate) fn remote_rename(&mut self, path: &Path, name: &str) {
        let Some(addr) = RemoteAddr::from_path(path) else {
            return;
        };
        let target = parent_dir(&addr).join(name);
        let opts = self.remote_opts(&addr);
        self.remote.with_fs(&self.ctx, addr, opts, move |fs, addr| {
            let result = fs.rename(&addr.path, &target.path);
            Event::Changed {
                dir: parent_dir(&addr),
                error: result.err().map(|e| e.to_string()),
                select: Some(target.to_path()),
                rename: false,
            }
        });
    }

    /// Copies and moves with a remote side become downloads and uploads.
    /// Returns the local sources still to be copied as usual.
    pub(super) fn remote_transfer(
        &mut self,
        kind: TransferKind,
        sources: Vec<PathBuf>,
        dest: &Path,
    ) -> Option<Vec<PathBuf>> {
        let (remote_src, local_src): (Vec<PathBuf>, Vec<PathBuf>) =
            sources.into_iter().partition(|p| is_remote(p));
        let dest_remote = RemoteAddr::from_path(dest);
        if remote_src.is_empty() && dest_remote.is_none() {
            return Some(local_src);
        }
        let conflict = self.settings.transfer.default_conflict;
        let mut requests = Vec::new();
        let mut rest = Vec::new();
        match &dest_remote {
            Some(dest_addr) => {
                if !local_src.is_empty() {
                    requests.push((dest_addr.clone(), local_src));
                }
                if !remote_src.is_empty() {
                    self.remote_to_remote(kind, remote_src, dest_addr);
                }
            }
            None => {
                rest = local_src;
                // One download per server.
                let mut by_server: Vec<(RemoteAddr, Vec<PathBuf>)> = Vec::new();
                for p in remote_src {
                    let Some(a) = RemoteAddr::from_path(&p) else {
                        continue;
                    };
                    match by_server.iter_mut().find(|(s, _)| s.same_server(&a)) {
                        Some((_, list)) => list.push(p),
                        None => by_server.push((a, vec![p])),
                    }
                }
                requests.extend(by_server);
            }
        }
        for (server, items) in requests {
            let Some(fs) = self.remote.conns.cached(&server) else {
                self.toast(ox_core::trf!(
                    "Connect to {server} first.",
                    server = server.server_url()
                ));
                continue;
            };
            let req = TransferRequest {
                remote: Some(RemoteSide::new(fs, &server)),
                ..TransferRequest::new(kind, items, dest.to_path_buf(), conflict)
            };
            if std::mem::take(&mut self.transfer_watch.run_after_next) {
                self.engine.submit_after_current(req);
            } else {
                self.engine.submit(req);
            }
            self.ui.show_transfers = true;
        }
        (!rest.is_empty()).then_some(rest)
    }

    /// Between remote folders: a move on one server is a rename there;
    /// everything else streams from server to server (one job per server).
    fn remote_to_remote(&mut self, kind: TransferKind, sources: Vec<PathBuf>, dest: &RemoteAddr) {
        let same = sources
            .iter()
            .all(|s| RemoteAddr::from_path(s).is_some_and(|a| a.same_server(dest)));
        if kind != TransferKind::Move || !same {
            return self.remote_stream(kind, sources, dest);
        }
        let items: Vec<RemoteAddr> = sources
            .iter()
            .filter_map(|s| RemoteAddr::from_path(s))
            .filter(|a| parent_dir(a) != *dest && a != dest)
            .collect();
        if items.is_empty() {
            return;
        }
        let opts = self.remote_opts(dest);
        self.remote
            .with_fs(&self.ctx, dest.clone(), opts, move |fs, dest| {
                let mut errors = Vec::new();
                for a in &items {
                    if let Err(e) = fs.rename(&a.path, &dest.join(&a.name()).path) {
                        errors.push(format!("{}: {e}", a.name()));
                    }
                }
                Event::Changed {
                    dir: dest,
                    error: (!errors.is_empty()).then(|| errors.join("\n")),
                    select: None,
                    rename: false,
                }
            });
        // The folders the items came from change too.
        for a in sources.iter().filter_map(|s| RemoteAddr::from_path(s)) {
            self.mark_remote_stale(&parent_dir(&a), false);
        }
    }

    /// The preview of a remote file: a downloaded copy (small files only).
    pub fn remote_preview(&mut self, entry: &FileEntry) -> RemotePreview {
        let Some(addr) = RemoteAddr::from_path(&entry.path) else {
            return RemotePreview::Local;
        };
        if entry.is_dir {
            return RemotePreview::Info(None);
        }
        if entry.size > MAX_PREVIEW {
            return RemotePreview::Info(Some(
                tr("This file is too large to preview from the server.").to_owned(),
            ));
        }
        match &self.remote.preview {
            Some((p, copy)) if *p == entry.path => match copy {
                PreviewCopy::Loading => {
                    RemotePreview::Info(Some(tr("Downloading the preview…").to_owned()))
                }
                PreviewCopy::Ready(local) => RemotePreview::Ready(FileEntry {
                    path: local.clone(),
                    ..entry.clone()
                }),
                PreviewCopy::Failed(e) => RemotePreview::Info(Some(e.clone())),
            },
            _ => {
                // A new file: the last one's copy isn't needed any more.
                if let Some((_, PreviewCopy::Ready(old))) = self.remote.preview.take() {
                    if let Some(dir) = old.parent() {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                }
                self.remote.preview = Some((entry.path.clone(), PreviewCopy::Loading));
                let path = entry.path.clone();
                let opts = self.remote_opts(&addr);
                self.remote.with_fs(&self.ctx, addr, opts, move |fs, addr| {
                    let copy = download_preview(fs, &addr).map_err(|e| e.to_string());
                    Event::Preview(path, copy)
                });
                RemotePreview::Info(Some(tr("Downloading the preview…").to_owned()))
            }
        }
    }
}

/// Download a file for the preview, into a folder of its own.
fn download_preview(fs: &dyn RemoteFs, addr: &RemoteAddr) -> Result<PathBuf, RemoteError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir()
        .join("Oxplorer remote")
        .join("preview")
        .join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&dir)?;
    let local = dir.join(remote::sanitize(&addr.name()));
    let data = remote::read_head(fs, &addr.path, MAX_PREVIEW as usize)?;
    std::fs::write(&local, data)?;
    Ok(local)
}
