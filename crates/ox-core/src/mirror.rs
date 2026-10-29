//! Mirror or update one folder to another, one way (source → target).
//!
//! [`plan`] walks both folders and lists what would change: items that are
//! copied (new), files that are updated (newer or different), items removed
//! from the target (mirror only) and what stays as it is. Nothing is done
//! until the plan is carried out: [`Plan::steps`] turns the ticked rows into
//! transfer jobs for the copy engine and a list of items for the Recycle Bin.
//!
//! Items are matched by name ignoring case, like the folder compare. Files
//! count as the same when size and date modified match (dates within two
//! seconds are equal, for FAT and network drives) or, when comparing by
//! contents, when they are equal byte by byte.
//!
//! Safety: a folder can't be mirrored to itself, into itself or into a folder
//! that holds it ([`check`]); extras are only ever moved to the Recycle Bin,
//! and not at all on network drives (they have none); folders that couldn't
//! be read are left alone, so nothing in the target is taken for an extra
//! just because the source couldn't be listed.

mod recent;
mod scan;
#[cfg(test)]
mod tests;

pub use recent::{RecentPair, RecentPairs};

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, TryRecvError};
use serde::{Deserialize, Serialize};

use crate::i18n::t;
use crate::transfer::{ConflictPolicy, TransferKind, TransferRequest};

/// Share of the target's files above which removing them is warned about.
pub const MANY_REMOVED: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Mode {
    /// Make the target exactly like the source: extras are removed.
    #[default]
    Mirror,
    /// Copy what's new and changed; never remove anything.
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CompareBy {
    /// Size and date modified (two seconds of tolerance).
    #[default]
    SizeAndDate,
    /// Files of the same size byte by byte.
    Contents,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub mode: Mode,
    pub by: CompareBy,
    /// Names or paths to leave out, separated by `;` (see [`Exclude`]).
    pub exclude: String,
}

/// Items left out of a mirror, on both sides: `*.tmp; node_modules; .git`.
/// A pattern matches a name (`*` any run of characters, `?` one, case
/// ignored); one with a `\` or `/` matches the path below the folders
/// (`build/cache`, `docs\*.pdf`). Excluded items in the target are kept.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Exclude {
    names: Vec<Vec<char>>,
    paths: Vec<Vec<char>>,
}

impl Exclude {
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        for p in text.split([';', ',', '\n', '|']) {
            let p = p.trim().trim_matches(['/', '\\']).to_lowercase();
            if p.is_empty() {
                continue;
            }
            if p.contains(['/', '\\']) {
                out.paths.push(p.replace('\\', "/").chars().collect());
            } else {
                out.names.push(p.chars().collect());
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.paths.is_empty()
    }

    /// Is the item at `rel` (below the mirrored folders) left out?
    pub fn matches(&self, rel: &Path) -> bool {
        let name: Vec<char> = rel
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
            .chars()
            .collect();
        if self.names.iter().any(|p| crate::search::wildcard(p, &name)) {
            return true;
        }
        if self.paths.is_empty() {
            return false;
        }
        let path: Vec<char> = rel_text(rel).to_lowercase().chars().collect();
        self.paths.iter().any(|p| crate::search::wildcard(p, &path))
    }
}

/// `a/b/c` for a relative path.
fn rel_text(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The groups of the preview, in the order they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// Copied: the target doesn't have it.
    New,
    /// Copied over the target's file.
    Update,
    /// Moved from the target to the Recycle Bin.
    Remove,
    Same,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::New, Group::Update, Group::Remove, Group::Same];
}

/// Why a row is in its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Why {
    /// Only the source has it.
    New,
    /// The source's file is newer.
    Newer,
    /// The target's file is newer (copied over it only when mirroring).
    TargetNewer,
    /// Same date, other size or contents.
    Different,
    /// A file on one side and a folder on the other: the target's goes, the
    /// source's comes.
    Kind,
    /// Only the target has it.
    Extra,
    Same,
}

impl Why {
    pub fn label(self) -> &'static str {
        t(match self {
            Why::New => "new",
            Why::Newer => "newer",
            Why::TargetNewer => "newer in the target",
            Why::Different => "different",
            Why::Kind => "file instead of folder or the other way",
            Why::Extra => "only in the target",
            Why::Same => "same",
        })
    }
}

/// One item of the preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Path below the mirrored folders.
    pub rel: PathBuf,
    pub group: Group,
    pub why: Why,
    pub is_dir: bool,
    /// Files in it (1 for a file) and their bytes.
    pub files: u64,
    pub bytes: u64,
    /// Where it comes from (`None` for removals).
    pub source: Option<PathBuf>,
    /// Where it goes, or what is removed.
    pub target: PathBuf,
    /// Ticked: part of the mirror.
    pub include: bool,
}

/// Rows, files and bytes of a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    pub items: usize,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub source: PathBuf,
    pub target: PathBuf,
    pub options: Options,
    pub rows: Vec<Row>,
    /// Files in the target (excluded ones not counted).
    pub target_files: u64,
    /// Folders that couldn't be read (left alone) and files that couldn't
    /// be compared.
    pub errors: usize,
    /// Items only the target has, kept since the mode is update.
    pub kept_extras: usize,
    /// A file on one side and a folder on the other, left alone (update).
    pub conflicts: usize,
    /// The target doesn't exist yet (it is created).
    pub new_target: bool,
    /// Extras can go to the Recycle Bin (`false` on network drives, which
    /// have none: nothing is removed there).
    pub can_remove: bool,
}

/// What carrying out a plan takes: first the items that go to the Recycle
/// Bin, then the copies.
#[derive(Debug, Clone, Default)]
pub struct Steps {
    pub trash: Vec<PathBuf>,
    pub copies: Vec<TransferRequest>,
}

impl Plan {
    pub fn totals(&self, group: Group, ticked_only: bool) -> Totals {
        let mut out = Totals::default();
        for r in self
            .rows
            .iter()
            .filter(|r| r.group == group && (r.include || !ticked_only))
        {
            out.items += 1;
            out.files += r.files;
            out.bytes += r.bytes;
        }
        out
    }

    /// Nothing is ticked to be done.
    pub fn is_empty(&self) -> bool {
        !self
            .rows
            .iter()
            .any(|r| r.include && r.group != Group::Same)
    }

    /// Files the ticked removals take from the target.
    pub fn removed_files(&self) -> u64 {
        self.totals(Group::Remove, true).files
    }

    /// The mirror would remove more than [`MANY_REMOVED`] of the target's files.
    pub fn removes_most(&self) -> bool {
        let removed = self.removed_files();
        removed > 0 && removed as f64 > self.target_files as f64 * MANY_REMOVED
    }

    /// Tick or untick every row of a group.
    pub fn tick_group(&mut self, group: Group, on: bool) {
        if group == Group::Same || (group == Group::Remove && !self.can_remove) {
            return;
        }
        for r in self.rows.iter_mut().filter(|r| r.group == group) {
            r.include = on;
        }
    }

    /// The ticked rows as removals and transfer jobs. New items are copied
    /// with `conflict` (something may have appeared since the preview),
    /// updated files overwrite the target's.
    pub fn steps(&self, conflict: ConflictPolicy) -> Steps {
        let mut steps = Steps::default();
        if check(&self.source, &self.target).is_err() {
            return steps;
        }
        let ticked = |g: Group| self.rows.iter().filter(move |r| r.include && r.group == g);
        if self.can_remove && self.options.mode == Mode::Mirror {
            steps.trash = ticked(Group::Remove)
                // Never anything outside the target, nor the target itself.
                .filter(|r| r.target != self.target && r.target.starts_with(&self.target))
                .map(|r| r.target.clone())
                .collect();
        }
        for (group, policy) in [
            (Group::New, conflict),
            (Group::Update, ConflictPolicy::Overwrite),
        ] {
            let mut req =
                TransferRequest::new(TransferKind::Copy, Vec::new(), self.target.clone(), policy);
            req.pairs = ticked(group)
                .filter_map(|r| Some((r.source.clone()?, r.target.clone())))
                .collect();
            if !req.pairs.is_empty() {
                steps.copies.push(req);
            }
        }
        steps
    }
}

/// A path for comparing: absolute, `..` resolved through what exists, lower
/// case on Windows, without the `\\?\` prefix.
fn normalized(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut base = path.to_path_buf();
    let resolved = loop {
        if let Ok(c) = std::fs::canonicalize(&base) {
            break crate::links::plain(&c);
        }
        match (base.file_name().map(|n| n.to_owned()), base.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                base = parent.to_path_buf();
            }
            _ => break std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
        }
    };
    let mut out = resolved;
    for name in rest.into_iter().rev() {
        out.push(name);
    }
    if cfg!(windows) {
        PathBuf::from(out.to_string_lossy().to_lowercase())
    } else {
        out
    }
}

/// Can `source` be mirrored to `target`? The error says why not: the same
/// folder, one inside the other, a source that isn't a folder, a target
/// that is a file, or a remote location.
pub fn check(source: &Path, target: &Path) -> Result<(), String> {
    if crate::remote::url::is_remote_path(source) || crate::remote::url::is_remote_path(target) {
        return Err(t("Only folders on this computer or the network can be mirrored.").into());
    }
    if source.as_os_str().is_empty() || target.as_os_str().is_empty() {
        return Err(t("Choose the folder to mirror to.").into());
    }
    if !source.is_dir() {
        return Err(crate::trf!(
            "\"{name}\" is not a folder.",
            name = source.display()
        ));
    }
    if crate::fsops::exists(target) && !target.is_dir() {
        return Err(crate::trf!(
            "\"{name}\" is not a folder.",
            name = target.display()
        ));
    }
    let (s, d) = (normalized(source), normalized(target));
    if s == d {
        return Err(t("The target is the source folder itself.").into());
    }
    if d.starts_with(&s) {
        return Err(t("The target is inside the source folder.").into());
    }
    if s.starts_with(&d) {
        return Err(t("The source folder is inside the target.").into());
    }
    Ok(())
}

/// The plan for mirroring `source` to `target`. `None` if `cancel` was set;
/// `scanned` counts the items looked at so far.
pub fn plan(
    source: &Path,
    target: &Path,
    options: &Options,
    cancel: &AtomicBool,
    scanned: &AtomicU64,
) -> Result<Option<Plan>, String> {
    check(source, target)?;
    std::fs::read_dir(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let new_target = !crate::fsops::exists(target);
    let exclude = Exclude::parse(&options.exclude);
    let mut s = scan::Scanner::new(options, &exclude, cancel, scanned);
    if s.pair(
        source,
        (!new_target).then_some(target),
        target,
        Path::new(""),
    )
    .is_err()
    {
        return Ok(None);
    }
    let can_remove = !crate::links::is_network(target);
    let mut rows = s.rows;
    if !can_remove {
        for r in rows.iter_mut().filter(|r| r.group == Group::Remove) {
            r.include = false;
        }
    }
    Ok(Some(Plan {
        source: source.to_path_buf(),
        target: target.to_path_buf(),
        options: options.clone(),
        rows,
        target_files: s.target_files,
        errors: s.errors,
        kept_extras: s.kept_extras,
        conflicts: s.conflicts,
        new_target,
        can_remove,
    }))
}

/// A plan made on its own thread. Dropping it cancels it.
pub struct PlanJob {
    rx: Receiver<Result<Plan, String>>,
    cancel: Arc<AtomicBool>,
    scanned: Arc<AtomicU64>,
}

impl PlanJob {
    /// Start planning; `notify` is called when the result is ready.
    pub fn start(
        source: PathBuf,
        target: PathBuf,
        options: Options,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let scanned = Arc::new(AtomicU64::new(0));
        let (c, n) = (cancel.clone(), scanned.clone());
        std::thread::Builder::new()
            .name("mirror plan".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                if let Some(result) = plan(&source, &target, &options, &c, &n).transpose() {
                    let _ = tx.send(result);
                    notify();
                }
            })
            .expect("spawn mirror plan thread");
        Self {
            rx,
            cancel,
            scanned,
        }
    }

    /// The result once it's there (`Some` only once).
    pub fn poll(&self) -> Option<Result<Plan, String>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("cancelled".into())),
        }
    }

    /// Items looked at so far.
    pub fn scanned(&self) -> u64 {
        self.scanned.load(Ordering::Relaxed)
    }
}

impl Drop for PlanJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
