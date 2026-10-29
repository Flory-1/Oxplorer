//! Undo and redo for file operations: renames, moves, copies, new folders,
//! files and links, deletes to the Recycle Bin, and files replaced by new
//! versions (rotated pictures, …).
//!
//! Each operation is recorded as an [`Op`] that describes what was done.
//! [`reverse`] first checks that the files are still as the operation left
//! them (and refuses otherwise), then applies the inverse and returns an
//! [`Op`] describing *that*, which is what a redo reverses again. Undoing a
//! copy or a new file moves it to the Recycle Bin, so nothing is lost; the
//! redo restores it from there. Permanent deletes are never recorded.
//!
//! The [`History`] is kept across restarts by a [`Store`], which merges what
//! several windows write.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::entry::display_name;
use crate::i18n::t;
use crate::links::{self, LinkKind};
use crate::recycle::{self, TrashItem};
use crate::{fsops, rename};

/// How many operations can be undone (and are kept across restarts).
pub const LIMIT: usize = 100;

/// What kind of operation an entry is, for messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Rename,
    Move,
    Copy,
    NewFolder,
    NewFile,
    NewLink,
    Delete,
    Rotate,
    Flip,
    Convert,
    Resize,
}

impl Kind {
    pub fn label(self) -> &'static str {
        t(match self {
            Self::Rename => "Rename",
            Self::Move => "Move",
            Self::Copy => "Copy",
            Self::NewFolder => "New folder",
            Self::NewFile => "New file",
            Self::NewLink => "New link",
            Self::Delete => "Move to Recycle Bin",
            Self::Rotate => "Rotate pictures",
            Self::Flip => "Flip pictures",
            Self::Convert => "Convert pictures",
            Self::Resize => "Resize pictures",
        })
    }
}

/// Enough about an item to tell whether it changed: for a folder the total
/// size, number of entries and newest modification time of everything in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    is_dir: bool,
    size: u64,
    entries: u64,
    modified: Option<SystemTime>,
}

/// The [`Stamp`] of `path` as it is now (links are not followed).
pub fn stamp(path: &Path) -> io::Result<Stamp> {
    let meta = fs::symlink_metadata(path)?;
    let mut s = Stamp {
        is_dir: meta.is_dir(),
        size: if meta.is_dir() { 0 } else { meta.len() },
        entries: 0,
        modified: meta.modified().ok(),
    };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        add_dir(path, &mut s)?;
    }
    Ok(s)
}

fn add_dir(dir: &Path, s: &mut Stamp) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        s.entries += 1;
        s.modified = s.modified.max(meta.modified().ok());
        if meta.is_dir() && !meta.file_type().is_symlink() {
            add_dir(&entry.path(), s)?;
        } else if !meta.is_dir() {
            s.size += meta.len();
        }
    }
    Ok(())
}

/// An item that was put somewhere, as it was right after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    pub path: PathBuf,
    pub stamp: Stamp,
}

impl Placed {
    /// `path` with its current stamp.
    pub fn now(path: PathBuf) -> io::Result<Self> {
        let stamp = stamp(&path)?;
        Ok(Self { path, stamp })
    }
}

/// What an operation did (and so how to reverse it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Op {
    /// Items renamed in their folder, (from, to). Undone by renaming back.
    Renamed(Vec<(PathBuf, PathBuf)>),
    /// Items moved, (from, to). Undone by moving them back.
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Items that were put in place (copies, new files, items restored from
    /// the Recycle Bin). Undone by moving them to the Recycle Bin, as long as
    /// they haven't changed.
    Placed(Vec<Placed>),
    /// A new folder. Undone by removing it while it's still empty.
    NewFolder(PathBuf),
    /// An empty folder that was removed. Undone by creating it again.
    RemovedFolder(PathBuf),
    /// Items moved to the Recycle Bin at or after `since` (Unix seconds).
    /// Undone by restoring them.
    Trashed { paths: Vec<PathBuf>, since: i64 },
    /// New links. Undone by removing the links (never their targets).
    Linked(Vec<Link>),
    /// Links that were removed. Undone by making them again.
    Unlinked(Vec<Link>),
    /// Files that took the place of others (a rotated picture, a converted
    /// one replacing its original); the originals went to the Recycle Bin
    /// at or after `since`. Undone by putting the originals back and the new
    /// files in the Recycle Bin, as long as they haven't changed.
    Replaced { items: Vec<Replacement>, since: i64 },
    /// Originals that were put back; the files that had replaced them went
    /// to the Recycle Bin (named `aside`) at or after `since`. Undone by
    /// swapping them again.
    Unreplaced { items: Vec<Restored>, since: i64 },
}

/// A file at `new` that took the place of the file at `original` (often the
/// same path).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Replacement {
    pub original: PathBuf,
    pub new: Placed,
}

/// An original put back at `original`. The file that had replaced it was at
/// `new` and went to the Recycle Bin as `aside` (renamed when it had the
/// original's name, so the original could come back).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Restored {
    pub original: PathBuf,
    pub new: PathBuf,
    pub aside: PathBuf,
}

/// A symbolic link, junction or hard link at `path` to `target`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub path: PathBuf,
    pub target: PathBuf,
    pub kind: LinkKind,
}

impl Op {
    /// The items the operation is about (where they were before, for renames and moves).
    pub fn paths(&self) -> Vec<&Path> {
        match self {
            Self::Renamed(pairs) | Self::Moved(pairs) => {
                pairs.iter().map(|(from, _)| from.as_path()).collect()
            }
            Self::Placed(items) => items.iter().map(|p| p.path.as_path()).collect(),
            Self::NewFolder(p) | Self::RemovedFolder(p) => vec![p.as_path()],
            Self::Trashed { paths, .. } => paths.iter().map(PathBuf::as_path).collect(),
            Self::Linked(links) | Self::Unlinked(links) => {
                links.iter().map(|l| l.path.as_path()).collect()
            }
            Self::Replaced { items, .. } => items.iter().map(|r| r.new.path.as_path()).collect(),
            Self::Unreplaced { items, .. } => items.iter().map(|r| r.original.as_path()).collect(),
        }
    }

    /// Folders whose contents the operation changed.
    pub fn folders(&self) -> Vec<PathBuf> {
        let mut paths: Vec<&Path> = self.paths();
        if let Self::Renamed(pairs) | Self::Moved(pairs) = self {
            paths.extend(pairs.iter().map(|(_, to)| to.as_path()));
        }
        let mut dirs: Vec<PathBuf> = paths
            .into_iter()
            .filter_map(|p| p.parent().map(Path::to_path_buf))
            .collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }

    /// Items that went from one place to another (old, new), so tags can follow.
    pub fn moved_pairs(&self) -> Vec<(PathBuf, PathBuf)> {
        match self {
            Self::Renamed(pairs) | Self::Moved(pairs) => pairs.clone(),
            _ => Vec::new(),
        }
    }
}

/// The result of reversing an operation.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Done; this describes what was done.
    Done(Op),
    /// Some items have to be moved by the transfer engine (to another drive,
    /// or into a folder that exists again): `pending` (from, to). `done` were
    /// moved back already. Once the job has finished, what was done is
    /// `Op::Moved(done + job.created())`.
    Transfer {
        done: Vec<(PathBuf, PathBuf)>,
        pending: Vec<(PathBuf, PathBuf)>,
    },
}

/// Where undone copies go and deleted items come back from.
pub trait Bin {
    fn delete(&self, paths: &[PathBuf]) -> Result<(), String>;
    /// Put back the items that were deleted from `paths` at or after `since`.
    fn restore(&self, paths: &[PathBuf], since: i64) -> Result<(), String>;
}

/// The system's Recycle Bin.
pub struct RecycleBin;

impl Bin for RecycleBin {
    fn delete(&self, paths: &[PathBuf]) -> Result<(), String> {
        fsops::delete(paths, true)
    }

    fn restore(&self, paths: &[PathBuf], since: i64) -> Result<(), String> {
        let items = find_deleted(&recycle::items()?, paths, since)?;
        recycle::restore(items)
    }
}

/// The current time in Unix seconds (taken before deleting, for [`Op::Trashed`]).
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The Recycle Bin items that were deleted from `paths` at or after `since`
/// (the newest one for each path).
pub fn find_deleted(
    items: &[TrashItem],
    paths: &[PathBuf],
    since: i64,
) -> Result<Vec<TrashItem>, String> {
    paths
        .iter()
        .map(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let parent = path.parent().unwrap_or(Path::new(""));
            let recent = items.iter().filter(|i| {
                // A second of slack: deletion times are whole seconds.
                i.time_deleted >= since - 1 && same_path(&i.original_parent, parent)
            });
            let exact = recent
                .clone()
                .filter(|i| same_name(&i.name.to_string_lossy(), &name))
                .max_by_key(|i| i.time_deleted);
            // The shell may give names without a hidden extension.
            let stem = fsops::split_name(&name).0;
            exact
                .or_else(|| {
                    recent
                        .filter(|i| same_name(&i.name.to_string_lossy(), stem))
                        .max_by_key(|i| i.time_deleted)
                })
                .cloned()
                .ok_or_else(|| {
                    crate::trf!(
                        "\"{name}\" is no longer in the Recycle Bin.",
                        name = display_name(path)
                    )
                })
        })
        .collect()
}

fn same_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy();
        s.trim_end_matches(['/', '\\']).to_owned()
    };
    same_name(&norm(a), &norm(b))
}

fn missing(path: &Path) -> String {
    crate::trf!(
        "\"{name}\" is no longer where it was.",
        name = display_name(path)
    )
}

fn taken(path: &Path) -> String {
    crate::trf!(
        "\"{new_name}\" already exists.",
        new_name = display_name(path)
    )
}

/// Fails if `item` is gone or has changed since it was put there.
fn unchanged(item: &Placed) -> Result<(), String> {
    match stamp(&item.path) {
        Ok(s) if s == item.stamp => Ok(()),
        Ok(_) => Err(crate::trf!(
            "\"{name}\" has changed since then.",
            name = display_name(&item.path)
        )),
        Err(_) => Err(missing(&item.path)),
    }
}

/// `photo (edited).jpg` next to `photo.jpg` (or `photo (edited) (2).jpg`, …).
fn edited_name(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let (stem, ext) = fsops::split_name(&name);
    fsops::unique_path(
        path.parent().unwrap_or(Path::new("")),
        &format!("{stem} (edited){ext}"),
    )
}

fn is_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// Check that `op` can still be reversed, then reverse it.
pub fn reverse(op: &Op, bin: &dyn Bin) -> Result<Outcome, String> {
    match op {
        Op::Renamed(pairs) => {
            for (from, to) in pairs {
                if !fsops::exists(to) {
                    return Err(missing(to));
                }
                // Free, the same item (a case-only rename), or another item
                // of this rename that moves away (a swap).
                let free = !fsops::exists(from)
                    || same_path(from, to)
                    || pairs.iter().any(|(_, other)| same_path(other, from));
                if !free {
                    return Err(taken(from));
                }
            }
            let plan: Vec<rename::Planned> = pairs
                .iter()
                .map(|(from, to)| rename::Planned {
                    from: to.clone(),
                    to: from.clone(),
                    error: None,
                })
                .collect();
            rename::apply(&plan)?;
            Ok(Outcome::Done(Op::Renamed(swap(pairs))))
        }
        Op::Moved(pairs) => {
            for (from, to) in pairs {
                if !fsops::exists(to) {
                    return Err(missing(to));
                }
                // A folder that is there again (e.g. with files that couldn't
                // be moved) is merged into; anything else is in the way.
                if fsops::exists(from) && !(is_dir(from) && is_dir(to)) {
                    return Err(taken(from));
                }
            }
            let mut done = Vec::new();
            let mut pending = Vec::new();
            for (from, to) in swap(pairs) {
                let renamed = !fsops::exists(&to)
                    && to
                        .parent()
                        .is_none_or(|p| p.as_os_str().is_empty() || fs::create_dir_all(p).is_ok())
                    && fs::rename(&from, &to).is_ok();
                if renamed {
                    done.push((from, to));
                } else {
                    pending.push((from, to));
                }
            }
            Ok(if pending.is_empty() {
                Outcome::Done(Op::Moved(done))
            } else {
                Outcome::Transfer { done, pending }
            })
        }
        Op::Placed(items) => {
            for item in items {
                unchanged(item)?;
            }
            let paths: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();
            let since = now();
            bin.delete(&paths)?;
            Ok(Outcome::Done(Op::Trashed { paths, since }))
        }
        Op::NewFolder(path) => {
            if !is_dir(path) {
                return Err(missing(path));
            }
            let empty = fs::read_dir(path)
                .map_err(|e| e.to_string())?
                .next()
                .is_none();
            if !empty {
                return Err(crate::trf!(
                    "The folder \"{name}\" isn't empty anymore.",
                    name = display_name(path)
                ));
            }
            fs::remove_dir(path).map_err(|e| e.to_string())?;
            Ok(Outcome::Done(Op::RemovedFolder(path.clone())))
        }
        Op::RemovedFolder(path) => {
            if fsops::exists(path) {
                return Err(taken(path));
            }
            fs::create_dir(path).map_err(|e| e.to_string())?;
            Ok(Outcome::Done(Op::NewFolder(path.clone())))
        }
        Op::Trashed { paths, since } => {
            if let Some(p) = paths.iter().find(|p| fsops::exists(p)) {
                return Err(taken(p));
            }
            bin.restore(paths, *since)?;
            let items = paths
                .iter()
                .map(|p| Placed::now(p.clone()).map_err(|_| missing(p)))
                .collect::<Result<_, _>>()?;
            Ok(Outcome::Done(Op::Placed(items)))
        }
        Op::Linked(items) => {
            for l in items {
                if !fsops::exists(&l.path) {
                    return Err(missing(&l.path));
                }
                // Only what is still the same link is removed.
                if !links::is_link(&l.path, l.kind, &l.target) {
                    return Err(crate::trf!(
                        "\"{name}\" has changed since then.",
                        name = display_name(&l.path)
                    ));
                }
            }
            for l in items {
                links::remove(&l.path, l.kind).map_err(|e| e.to_string())?;
            }
            Ok(Outcome::Done(Op::Unlinked(items.clone())))
        }
        Op::Unlinked(items) => {
            if let Some(l) = items.iter().find(|l| fsops::exists(&l.path)) {
                return Err(taken(&l.path));
            }
            for l in items {
                links::create(l.kind, &l.path, &l.target)?;
            }
            Ok(Outcome::Done(Op::Linked(items.clone())))
        }
        Op::Replaced { items, since } => unreplace(items, *since, bin),
        Op::Unreplaced { items, since } => replace_again(items, *since, bin),
    }
}

/// Undo [`Op::Replaced`]: the new files step aside (when they have the
/// original's name), the originals come back from the Recycle Bin and the
/// new files go there.
fn unreplace(items: &[Replacement], since: i64, bin: &dyn Bin) -> Result<Outcome, String> {
    for r in items {
        unchanged(&r.new)?;
        if !same_path(&r.original, &r.new.path) && fsops::exists(&r.original) {
            return Err(taken(&r.original));
        }
    }
    let step_back = |moved: &[Restored]| {
        for m in moved.iter().rev().filter(|m| m.aside != m.new) {
            let _ = fs::rename(&m.aside, &m.new);
        }
    };
    let mut moved = Vec::new();
    for r in items {
        let aside = if same_path(&r.original, &r.new.path) {
            edited_name(&r.new.path)
        } else {
            r.new.path.clone()
        };
        if aside != r.new.path {
            if let Err(e) = fs::rename(&r.new.path, &aside) {
                step_back(&moved);
                return Err(e.to_string());
            }
        }
        moved.push(Restored {
            original: r.original.clone(),
            new: r.new.path.clone(),
            aside,
        });
    }
    let originals: Vec<PathBuf> = items.iter().map(|r| r.original.clone()).collect();
    if let Err(e) = bin.restore(&originals, since) {
        step_back(&moved);
        return Err(e);
    }
    let asides: Vec<PathBuf> = moved.iter().map(|m| m.aside.clone()).collect();
    let since = now();
    bin.delete(&asides)?;
    Ok(Outcome::Done(Op::Unreplaced {
        items: moved,
        since,
    }))
}

/// Redo [`Op::Replaced`] (undo [`Op::Unreplaced`]): the originals go to the
/// Recycle Bin again and the new files come back to where they were.
fn replace_again(items: &[Restored], since: i64, bin: &dyn Bin) -> Result<Outcome, String> {
    for r in items {
        if !fsops::exists(&r.original) {
            return Err(missing(&r.original));
        }
        if !same_path(&r.original, &r.new) && fsops::exists(&r.new) {
            return Err(taken(&r.new));
        }
        if fsops::exists(&r.aside) {
            return Err(taken(&r.aside));
        }
    }
    let originals: Vec<PathBuf> = items.iter().map(|r| r.original.clone()).collect();
    let deleted = now();
    bin.delete(&originals)?;
    let asides: Vec<PathBuf> = items.iter().map(|r| r.aside.clone()).collect();
    if let Err(e) = bin.restore(&asides, since) {
        let _ = bin.restore(&originals, deleted);
        return Err(e);
    }
    let mut back = Vec::new();
    for r in items {
        if r.aside != r.new {
            fs::rename(&r.aside, &r.new).map_err(|e| e.to_string())?;
        }
        let new = Placed::now(r.new.clone()).map_err(|_| missing(&r.new))?;
        back.push(Replacement {
            original: r.original.clone(),
            new,
        });
    }
    Ok(Outcome::Done(Op::Replaced {
        items: back,
        since: deleted,
    }))
}

fn swap(pairs: &[(PathBuf, PathBuf)]) -> Vec<(PathBuf, PathBuf)> {
    pairs.iter().map(|(a, b)| (b.clone(), a.clone())).collect()
}

/// One operation in the history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: Kind,
    /// What it was done to: a name or "3 items" (fixed when recorded).
    pub subject: String,
    pub op: Op,
}

impl Entry {
    pub fn new(kind: Kind, op: Op) -> Self {
        let paths = op.paths();
        let subject = match paths.as_slice() {
            [one] => display_name(one),
            many => crate::i18n::count(many.len() as u64, "item"),
        };
        Self { kind, subject, op }
    }

    /// "Rename (notes.txt)".
    pub fn describe(&self) -> String {
        format!("{} ({})", self.kind.label(), self.subject)
    }
}

mod history;
mod store;
#[cfg(test)]
mod tests;

pub use history::{History, Record};
pub use store::Store;
