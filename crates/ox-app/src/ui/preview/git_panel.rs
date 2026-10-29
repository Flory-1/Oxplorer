//! State of the Git tab's Changes and Branches views and of the Diff tab:
//! what was read from git, the command running and the commit message.
//! Git runs on threads of its own; only reads start by themselves.

use super::*;
use ox_core::git::diff::{self as gitdiff, Diff, DiffOf};
use ox_core::git::{Branch, Change, Changes, Group, RepoStatus};

use crate::app::GitAction;

/// The views of the Git tab.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum GitView {
    #[default]
    Changes,
    Branches,
    History,
}

/// What the Changes and Branches views show, read together.
pub(super) struct Snapshot {
    pub(super) changes: Changes,
    pub(super) branches: Vec<Branch>,
    /// The last commit's message (to amend it).
    pub(super) last_message: Option<String>,
    /// The history, when it was read again (after a commit or a switch).
    pub(super) history: Option<Arc<GitInfo>>,
}

impl Snapshot {
    pub(super) fn has_remote(&self) -> bool {
        self.changes.head.upstream.is_some() || self.branches.iter().any(|b| b.remote)
    }
}

/// What a finished command means for the views.
enum Then {
    Nothing,
    /// The message was committed: empty the boxes.
    Committed,
    /// A switch to this branch; git may refuse it for local changes.
    Switched(Branch),
    /// The new branch was created: empty its box.
    Created,
}

struct Running {
    what: String,
    then: Then,
    rx: Receiver<Result<String, String>>,
}

/// What the last command printed.
pub(super) struct Outcome {
    pub(super) ok: bool,
    pub(super) text: String,
}

/// A diff to show: of which file and side, and when it was read.
#[derive(Clone, PartialEq, Eq)]
struct DiffKey {
    root: PathBuf,
    paths: Vec<String>,
    of: DiffOf,
    /// Changes when the files may have changed.
    generation: usize,
}

/// A diff on screen, and the one loading.
#[derive(Default)]
pub(super) struct DiffPane {
    key: Option<DiffKey>,
    rx: Option<Receiver<Result<Diff, String>>>,
    pub(super) shown: Option<Result<Arc<Diff>, String>>,
}

impl DiffPane {
    /// Show the diff of `paths` (the first is the file) as `of`.
    fn want(
        &mut self,
        ctx: &egui::Context,
        root: &Path,
        paths: Vec<String>,
        of: DiffOf,
        generation: usize,
    ) {
        let key = DiffKey {
            root: root.to_path_buf(),
            paths,
            of,
            generation,
        };
        if let Some(rx) = &self.rx {
            if let Ok(result) = rx.try_recv() {
                self.shown = Some(result.map(Arc::new));
                self.rx = None;
            }
        }
        if self.key.as_ref() == Some(&key) {
            return;
        }
        // The same file read again keeps showing until the new one is in.
        let same_file = self
            .key
            .as_ref()
            .is_some_and(|k| k.root == key.root && k.paths == key.paths && k.of == key.of);
        if !same_file {
            self.shown = None;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (k, repaint) = (key.clone(), ctx.clone());
        std::thread::spawn(move || {
            let paths: Vec<&str> = k.paths.iter().map(String::as_str).collect();
            let _ = tx.send(gitdiff::diff(&k.root, &paths, k.of).map_err(|e| e.to_string()));
            repaint.request_repaint();
        });
        self.key = Some(key);
        self.rx = Some(rx);
    }

    pub(super) fn loading(&self) -> bool {
        self.rx.is_some()
    }
}

/// The Git tab's state (one per preview).
#[derive(Default)]
pub struct GitPanel {
    /// The Git status of the folder the file list shows (handed in by the
    /// pane): it offers the Diff tab, and a new one means files changed.
    pub(super) status: Option<Arc<RepoStatus>>,
    seen_status: Option<Arc<RepoStatus>>,
    pub(super) view: GitView,
    root: Option<PathBuf>,
    pub(super) snapshot: Option<Result<Arc<Snapshot>, String>>,
    loading: Option<Receiver<Result<Snapshot, String>>>,
    loaded_at: Option<Instant>,
    /// Read again as soon as possible (or at this time).
    stale: bool,
    refresh_at: Option<Instant>,
    /// Read the history again with the next read.
    history_stale: bool,
    /// The history as last read again (newer than the folder preview's).
    pub(super) history: Option<Arc<GitInfo>>,
    /// Counts the reads; diffs are read again with each.
    generation: usize,
    running: Option<Running>,
    pub(super) outcome: Option<Outcome>,
    pub(super) summary: String,
    pub(super) description: String,
    pub(super) amend: bool,
    /// The file whose diff the Changes view shows, and from which list.
    pub(super) selected: Option<(String, Group)>,
    pub(super) diff: DiffPane,
    /// The Diff tab of a file selected in the file list.
    pub(super) file_diff: DiffPane,
    pub(super) side_by_side: bool,
    /// Discard (or move to the Recycle Bin) this file? Asked first.
    pub(super) confirm: Option<(Change, Group)>,
    pub(super) new_branch: String,
    pub(super) filter: String,
    /// Git refused to switch to this branch because of local changes.
    pub(super) blocked: Option<Branch>,
}

/// Without news from the file list, the views are read again this often
/// while they are shown (files can change outside the shown folder).
const REREAD: Duration = Duration::from_secs(10);

impl GitPanel {
    /// Keep up with the repository at `root`: read it when it's new, when
    /// files changed, after a command and every few seconds; collect what
    /// finished. Call each frame the Git tab is shown.
    pub(super) fn update(&mut self, ctx: &egui::Context, root: &Path, actions: &mut Vec<Action>) {
        if self.root.as_deref() != Some(root) {
            // Another repository: start over, but keep what the user set.
            let (view, side_by_side, status) = (self.view, self.side_by_side, self.status.take());
            *self = Self {
                view,
                side_by_side,
                status,
                root: Some(root.to_path_buf()),
                stale: true,
                ..Default::default()
            };
        }
        self.notice_status();
        if self.refresh_at.is_some_and(|t| Instant::now() >= t)
            || self.loaded_at.is_some_and(|t| t.elapsed() >= REREAD)
        {
            self.refresh_at = None;
            self.stale = true;
        }
        if let Some(rx) = &self.loading {
            if let Ok(result) = rx.try_recv() {
                if let Some(h) = result.as_ref().ok().and_then(|s| s.history.clone()) {
                    self.history = Some(h);
                }
                self.snapshot = Some(result.map(Arc::new));
                self.loading = None;
                self.loaded_at = Some(Instant::now());
                self.generation += 1;
            }
        }
        self.finish_command(actions);
        if self.stale && self.loading.is_none() && self.running.is_none() {
            self.read(ctx, root);
        }
        ctx.request_repaint_after(REREAD);
    }

    /// A new status from the file list: files changed.
    fn notice_status(&mut self) {
        let same = match (&self.status, &self.seen_status) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.seen_status = self.status.clone();
            if self.loaded_at.is_some() {
                self.stale = true;
            }
        }
    }

    fn read(&mut self, ctx: &egui::Context, root: &Path) {
        self.stale = false;
        let history = std::mem::take(&mut self.history_stale);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (root, repaint) = (root.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let read = || -> std::io::Result<Snapshot> {
                let changes = ox_core::git::changes(&root)?;
                let branches = ox_core::git::branches(&root).unwrap_or_default();
                let last_message = match changes.head.commit {
                    Some(_) => ox_core::git::last_message(&root).ok(),
                    None => None,
                };
                Ok(Snapshot {
                    changes,
                    branches,
                    last_message,
                    history: history.then(|| fresh_git_info(&root)).flatten(),
                })
            };
            let _ = tx.send(read().map_err(|e| e.to_string()));
            repaint.request_repaint();
        });
        self.loading = Some(rx);
    }

    /// Read again now (the refresh button).
    pub(super) fn refresh(&mut self) {
        self.stale = true;
        self.history_stale = true;
    }

    /// Read again in a moment (after something the app does for the view,
    /// like moving a file to the Recycle Bin).
    pub(super) fn refresh_soon(&mut self) {
        self.refresh_at = Some(Instant::now() + Duration::from_millis(1500));
    }

    /// The Snapshot, if it was read.
    pub(super) fn snapshot(&self) -> Option<Arc<Snapshot>> {
        self.snapshot.as_ref()?.as_ref().ok().cloned()
    }

    pub(super) fn reading(&self) -> bool {
        self.loading.is_some()
    }

    /// What is running ("Committing…"), if a command is.
    pub(super) fn busy(&self) -> Option<&str> {
        self.running.as_ref().map(|r| r.what.as_str())
    }

    /// Run a command that changes the repository; the views are read again
    /// when it's done.
    fn start(
        &mut self,
        ctx: &egui::Context,
        what: String,
        then: Then,
        work: impl FnOnce(&Path) -> std::io::Result<String> + Send + 'static,
    ) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work(&root).map_err(|e| e.to_string()));
            repaint.request_repaint();
        });
        self.outcome = None;
        self.running = Some(Running { what, then, rx });
    }

    fn finish_command(&mut self, actions: &mut Vec<Action>) {
        let Some(running) = &self.running else {
            return;
        };
        let Ok(result) = running.rx.try_recv() else {
            return;
        };
        let then = self.running.take().map_or(Then::Nothing, |r| r.then);
        let ok = result.is_ok();
        match (&then, &result) {
            (Then::Committed, Ok(_)) => {
                self.summary.clear();
                self.description.clear();
                self.amend = false;
            }
            (Then::Created, Ok(_)) => self.new_branch.clear(),
            (Then::Switched(b), Err(e)) if ox_core::git::would_overwrite(e) => {
                self.blocked = Some(b.clone());
            }
            (Then::Switched(_), Ok(_)) => self.blocked = None,
            _ => {}
        }
        let text = result.unwrap_or_else(|e| e);
        self.outcome = (!text.is_empty() || !ok).then_some(Outcome { ok, text });
        self.stale = true;
        self.history_stale = true;
        // The Git column and the status bar show the new state too.
        actions.push(Action::Git(GitAction::StatusChanged));
    }

    pub(super) fn stage(&mut self, ctx: &egui::Context, paths: Vec<String>) {
        self.start(ctx, tr("Staging…").into(), Then::Nothing, move |root| {
            let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
            ox_core::git::stage(root, &paths)
        });
    }

    pub(super) fn unstage(&mut self, ctx: &egui::Context, paths: Vec<String>) {
        self.start(ctx, tr("Unstaging…").into(), Then::Nothing, move |root| {
            let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
            ox_core::git::unstage(root, &paths)
        });
    }

    pub(super) fn unstage_all(&mut self, ctx: &egui::Context) {
        self.start(ctx, tr("Unstaging…").into(), Then::Nothing, |root| {
            ox_core::git::unstage_all(root)
        });
    }

    /// Discard the changes of `change` that aren't staged (asked first).
    pub(super) fn discard(&mut self, ctx: &egui::Context, change: &Change) {
        let paths: Vec<String> = change.paths().into_iter().map(str::to_owned).collect();
        self.start(
            ctx,
            tr("Discarding changes…").into(),
            Then::Nothing,
            move |root| {
                let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
                ox_core::git::discard(root, &paths)
            },
        );
    }

    pub(super) fn commit(&mut self, ctx: &egui::Context) {
        let (summary, description, amend) =
            (self.summary.clone(), self.description.clone(), self.amend);
        self.start(
            ctx,
            tr("Committing…").into(),
            Then::Committed,
            move |root| ox_core::git::commit(root, &summary, &description, amend),
        );
    }

    /// Switch to `branch`; with `stash`, local changes are stashed first.
    pub(super) fn switch(&mut self, ctx: &egui::Context, branch: Branch, stash: bool) {
        let all = self
            .snapshot()
            .map(|s| s.branches.clone())
            .unwrap_or_default();
        self.blocked = None;
        let what = ox_core::trf!("Switching to {branch}…", branch = branch.name);
        let b = branch.clone();
        self.start(ctx, what, Then::Switched(branch), move |root| match stash {
            true => ox_core::git::stash_and_switch(root, &b, &all),
            false => ox_core::git::switch(root, &b, &all),
        });
    }

    pub(super) fn create_branch(&mut self, ctx: &egui::Context) {
        let name = self.new_branch.trim().to_owned();
        let what = ox_core::trf!("Creating {branch}…", branch = name);
        self.start(ctx, what, Then::Created, move |root| {
            ox_core::git::create_branch(root, &name)
        });
    }

    pub(super) fn fetch(&mut self, ctx: &egui::Context) {
        self.start(ctx, tr("Fetching…").into(), Then::Nothing, |root| {
            ox_core::git::fetch(root)
        });
    }

    pub(super) fn pull(&mut self, ctx: &egui::Context) {
        self.start(ctx, tr("Pulling…").into(), Then::Nothing, |root| {
            ox_core::git::pull(root)
        });
    }

    pub(super) fn push(&mut self, ctx: &egui::Context) {
        let head = self
            .snapshot()
            .map(|s| s.changes.head.clone())
            .unwrap_or_default();
        self.start(ctx, tr("Pushing…").into(), Then::Nothing, move |root| {
            ox_core::git::push(root, &head)
        });
    }

    /// The Changes view's diff: of the selected file, if it's still changed.
    pub(super) fn update_diff(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let selected = self.selected.as_ref().and_then(|(path, g)| {
            snap.changes
                .get(path)
                .filter(|c| c.in_group(*g))
                .map(|c| (c.clone(), *g))
        });
        let Some((change, group)) = selected else {
            self.selected = None;
            return;
        };
        let of = match group {
            Group::Staged => DiffOf::Staged,
            Group::Unstaged => DiffOf::Unstaged,
            Group::Untracked => DiffOf::Untracked,
            // `git diff` shows conflicts in another form: the file with its
            // conflict markers against HEAD instead.
            Group::Conflicts => DiffOf::Head,
        };
        let paths = change.paths().into_iter().map(str::to_owned).collect();
        self.diff.want(ctx, &root, paths, of, self.generation);
    }

    /// The Diff tab is offered for changed files of the file list.
    pub(super) fn offer(&self, e: &FileEntry, tabs: &mut Vec<PreviewTab>) {
        if self.file_change(e).is_some() {
            tabs.push(PreviewTab::Diff);
        }
    }

    /// The repository, path (relative, with `/`) and diff of `e` if Git
    /// says it changed.
    fn file_change(&self, e: &FileEntry) -> Option<(PathBuf, String, DiffOf)> {
        if e.is_dir {
            return None;
        }
        let status = self.status.as_ref()?;
        let of = match status.state(&e.path)? {
            ox_core::git::GitState::Ignored => return None,
            ox_core::git::GitState::Untracked => DiffOf::Untracked,
            _ => DiffOf::Head,
        };
        let rel = e.path.strip_prefix(&status.root).ok()?;
        let rel: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        Some((status.root.clone(), rel.join("/"), of))
    }

    /// The Diff tab's diff of `e` (read again when the file list's status
    /// is).
    pub(super) fn update_file_diff(&mut self, ctx: &egui::Context, e: &FileEntry) -> bool {
        let Some((root, rel, of)) = self.file_change(e) else {
            return false;
        };
        let generation = self.status.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize);
        self.file_diff.want(ctx, &root, vec![rel], of, generation);
        true
    }
}
