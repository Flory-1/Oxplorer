//! Applying file system changes to a built index without rebuilding it.
//!
//! Changes arrive as paths ("something happened here"). They are coalesced
//! into a [`Batch`], and each path is then reconciled with the disk: items
//! that are gone are flagged deleted (with everything below them), new items
//! are appended (new folders are walked), changed ones get their new size and
//! date. Renames inside the index are patched in place, so renaming or moving
//! a big folder doesn't re-read it. Deleted records and old names stay in
//! memory until the index is compacted (or rebuilt).
//!
//! Appended records always come after their parent, and in-place moves only
//! go to a parent with a lower index, so "parents before children" holds and
//! a single forward pass finds everything below a deleted folder.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::checkpoint::Checkpoint;
use super::{
    is_excluded, push_entry, read_children, Child, Entry, FileIndex, F_DELETED, F_DIR, F_HIDDEN,
    NO_PARENT,
};

/// More changed paths than this in one batch: rebuilding is cheaper.
const MAX_PATHS: usize = 100_000;

/// One change reported by a file watcher or the change journal.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Signal {
    /// Something at this path was created, deleted or modified.
    Changed(PathBuf),
    /// The old name of a rename; the new name usually follows right after.
    RenamedFrom(PathBuf),
    RenamedTo(PathBuf),
    /// Changes were lost (e.g. the watcher overflowed): rebuild.
    Rescan,
    /// The change journal was read up to here (sent after the changes before it).
    #[cfg_attr(not(windows), allow(dead_code))]
    Checkpoint(Checkpoint),
}

/// Changes collected over a short time, with duplicates removed.
#[derive(Debug, Default)]
pub(crate) struct Batch {
    paths: Vec<PathBuf>,
    seen: HashSet<PathBuf>,
    renames: Vec<(PathBuf, PathBuf)>,
    /// A rename's old name waiting for its new name.
    from: Option<PathBuf>,
    rescan: bool,
    /// The latest journal position per drive.
    checkpoints: Vec<Checkpoint>,
}

/// Paths to reconcile, plus renames to try in place first.
pub(super) struct Step<'a> {
    pub renames: &'a [(PathBuf, PathBuf)],
    pub paths: Vec<&'a Path>,
}

impl Batch {
    pub fn push(&mut self, signal: Signal) {
        match signal {
            Signal::Changed(p) => {
                self.flush();
                self.add(p);
            }
            Signal::RenamedFrom(p) => {
                self.flush();
                self.from = Some(p);
            }
            Signal::RenamedTo(p) => match self.from.take() {
                Some(from) if !self.rescan => self.renames.push((from, p)),
                _ => self.add(p),
            },
            Signal::Rescan => self.set_rescan(),
            Signal::Checkpoint(c) => self.checkpoint(c),
        }
    }

    fn checkpoint(&mut self, c: Checkpoint) {
        match self.checkpoints.iter_mut().find(|k| k.drive == c.drive) {
            Some(k) => *k = c,
            None => self.checkpoints.push(c),
        }
    }

    /// Where the change journals were read up to when the last change in
    /// this batch was sent.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    fn add(&mut self, p: PathBuf) {
        if self.rescan {
            return;
        }
        if self.seen.insert(p.clone()) {
            self.paths.push(p);
        }
        if self.paths.len() + self.renames.len() > MAX_PATHS {
            self.set_rescan();
        }
    }

    fn set_rescan(&mut self) {
        self.rescan = true;
        self.paths = Vec::new();
        self.seen = HashSet::new();
        self.renames = Vec::new();
        self.from = None;
    }

    /// A rename's old name without a new name is a plain change (moved away).
    fn flush(&mut self) {
        if let Some(from) = self.from.take() {
            self.add(from);
        }
    }

    /// Add everything from `other`.
    pub fn merge(&mut self, mut other: Batch) {
        other.flush();
        self.flush();
        for c in std::mem::take(&mut other.checkpoints) {
            self.checkpoint(c);
        }
        if other.rescan {
            self.set_rescan();
            return;
        }
        for p in other.paths {
            self.add(p);
        }
        if !self.rescan {
            self.renames.append(&mut other.renames);
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.rescan
            && self.paths.is_empty()
            && self.renames.is_empty()
            && self.from.is_none()
            && self.checkpoints.is_empty()
    }

    /// Were changes lost, so the index must be rebuilt?
    pub fn needs_rescan(&self) -> bool {
        self.rescan
    }

    /// The renames on their own, then every path (including both ends of
    /// each rename, which are no-ops if the rename was patched in place).
    pub(super) fn steps(&self) -> Vec<Step<'_>> {
        let mut paths: Vec<&Path> = self.paths.iter().map(PathBuf::as_path).collect();
        for (from, to) in &self.renames {
            paths.push(from);
            paths.push(to);
        }
        if let Some(from) = &self.from {
            paths.push(from);
        }
        let mut steps = Vec::new();
        if !self.renames.is_empty() {
            steps.push(Step {
                renames: &self.renames,
                paths: Vec::new(),
            });
        }
        steps.push(Step {
            renames: &[],
            paths,
        });
        steps
    }
}

/// One change to the index, planned against a snapshot of it.
#[derive(Debug, PartialEq)]
pub(super) enum Op {
    /// Flag the entry and everything below it deleted.
    Remove(u32),
    Update {
        idx: u32,
        size: u64,
        mtime: u32,
    },
    /// Append `items` below `parent`.
    Add {
        parent: u32,
        items: Vec<NewItem>,
    },
    /// Rename or move an entry in place.
    Rename {
        idx: u32,
        parent: u32,
        name: String,
        size: u64,
        mtime: u32,
    },
}

#[derive(Debug, PartialEq)]
pub(super) struct NewItem {
    child: Child,
    /// Index into the same `items`, or `NO_PARENT` for the op's parent.
    parent: u32,
}

/// Compare paths the way the file system does (case-insensitively on Windows).
/// Components are joined with NUL, which can't occur in names, so a key is a
/// prefix of another exactly when the paths are nested.
fn key(path: &Path) -> String {
    let mut s = String::new();
    for c in path.components() {
        if !s.is_empty() {
            s.push('\0');
        }
        s.push_str(&c.as_os_str().to_string_lossy());
    }
    norm(s)
}

fn norm(s: String) -> String {
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

fn is_below(key: &str, ancestor: &str) -> bool {
    key.len() > ancestor.len() && key.starts_with(ancestor) && key.as_bytes()[ancestor.len()] == 0
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// For the name prefilter: length and first byte (case-folded).
fn name_slot(name: &str) -> Option<usize> {
    let first = *name.as_bytes().first()?;
    (name.len() < 256).then(|| name.len() << 8 | first.to_ascii_lowercase() as usize)
}

impl FileIndex {
    /// The live entry for each path, in one pass over the index.
    fn lookup(&self, paths: &[&Path]) -> Vec<Option<u32>> {
        let keys: Vec<String> = paths.iter().map(|p| key(p)).collect();
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        let mut slots = vec![false; 256 * 256];
        let mut long_names = false;
        for (j, p) in paths.iter().enumerate() {
            let Some(name) = file_name(p) else { continue };
            match name_slot(&name) {
                Some(s) => slots[s] = true,
                None => long_names = true,
            }
            by_name.entry(norm(name)).or_default().push(j);
        }
        let mut found = vec![None; paths.len()];
        for (i, e) in self.entries.iter().enumerate() {
            if e.flags & F_DELETED != 0 {
                continue;
            }
            let name = self.name(e);
            let candidates: Vec<usize> = if e.parent == NO_PARENT {
                // Roots are stored under their full path.
                let k = key(Path::new(name));
                (0..keys.len()).filter(|&j| keys[j] == k).collect()
            } else {
                let maybe = match name_slot(name) {
                    Some(s) => slots[s],
                    None => long_names,
                };
                if !maybe {
                    continue;
                }
                match by_name.get(&norm(name.to_owned())) {
                    Some(js) => js.clone(),
                    None => continue,
                }
            };
            if candidates.is_empty() {
                continue;
            }
            let k = key(&self.path_of(i as u32));
            for j in candidates {
                if found[j].is_none() && keys[j] == k {
                    found[j] = Some(i as u32);
                }
            }
        }
        found
    }

    /// Is `path` below an indexed root, and not inside an excluded folder?
    fn relevant(&self, path: &Path, exclude: &[String]) -> bool {
        let k = key(path);
        self.roots.iter().any(|root| {
            let rk = key(root);
            is_below(&k, &rk)
                && path
                    .components()
                    .skip(root.components().count())
                    .all(|c| match c {
                        Component::Normal(n) => !is_excluded(&n.to_string_lossy(), exclude),
                        _ => false,
                    })
        })
    }

    fn hidden(&self, i: u32) -> bool {
        self.entries[i as usize].flags & F_HIDDEN != 0
    }

    /// Work out what `step` changes, reading the disk but not changing the
    /// index (so it can run on a snapshot while queries continue).
    pub(super) fn plan(&self, step: &Step, exclude: &[String]) -> Vec<Op> {
        if !step.renames.is_empty() {
            return self.plan_renames(step.renames, exclude);
        }
        let mut paths: Vec<&Path> = step
            .paths
            .iter()
            .copied()
            .filter(|p| self.relevant(p, exclude))
            .collect();
        if paths.is_empty() {
            return Vec::new();
        }
        paths.sort_by_key(|p| p.components().count());
        paths.dedup();

        // Look up every path and all its folders up to the root at once.
        let roots: Vec<String> = self.roots.iter().map(|r| key(r)).collect();
        let mut want: Vec<&Path> = Vec::new();
        let mut pos: HashMap<String, usize> = HashMap::new();
        for p in &paths {
            for a in p.ancestors() {
                let k = key(a);
                if pos.contains_key(&k) {
                    break;
                }
                let is_root = roots.contains(&k);
                pos.insert(k, want.len());
                want.push(a);
                if is_root {
                    break;
                }
            }
        }
        let found = self.lookup(&want);
        let resolve = |p: &Path| pos.get(&key(p)).and_then(|&j| found[j]);

        let mut ops = Vec::new();
        // Folders that are being added with everything in them.
        let mut walked: HashSet<String> = HashSet::new();
        let is_walked =
            |walked: &HashSet<String>, p: &Path| p.ancestors().any(|a| walked.contains(&key(a)));
        for p in paths {
            if is_walked(&walked, p) {
                continue;
            }
            let meta = fs::symlink_metadata(p).ok();
            match (meta, resolve(p)) {
                (None, Some(i)) => ops.push(Op::Remove(i)),
                (None, None) => {}
                (Some(meta), Some(i)) => {
                    let e = &self.entries[i as usize];
                    if e.parent == NO_PARENT {
                        continue; // roots stay
                    }
                    let Some(name) = file_name(p) else { continue };
                    let Some(c) = Child::new(name, &meta, self.hidden(e.parent), exclude) else {
                        continue;
                    };
                    if (c.flags ^ e.flags) & (F_DIR | F_HIDDEN) == 0 {
                        if c.size.min(Entry::MAX_SIZE) != e.size() || c.mtime != e.mtime {
                            ops.push(Op::Update {
                                idx: i,
                                size: c.size,
                                mtime: c.mtime,
                            });
                        }
                    } else {
                        // Became a folder or file, or hidden: read it again.
                        ops.push(Op::Remove(i));
                        if let Some(items) = walk_new(p, self.hidden(e.parent), exclude) {
                            ops.push(Op::Add {
                                parent: e.parent,
                                items,
                            });
                            walked.insert(key(p));
                        }
                    }
                }
                (Some(_), None) => {
                    // Add the topmost folder that isn't indexed yet (with all of it).
                    let mut top = p;
                    let parent = loop {
                        let Some(up) = top.parent() else { break None };
                        if let Some(i) = resolve(up) {
                            break Some(i);
                        }
                        if !self.relevant(up, exclude) {
                            break None;
                        }
                        top = up;
                    };
                    let Some(parent) = parent else { continue };
                    if is_walked(&walked, top) {
                        continue;
                    }
                    if let Some(items) = walk_new(top, self.hidden(parent), exclude) {
                        ops.push(Op::Add { parent, items });
                        walked.insert(key(top));
                    }
                }
            }
        }
        ops
    }

    /// Renames that can be patched in place: the old entry is indexed and gone
    /// from disk, the new path isn't indexed yet, its folder is, and nothing
    /// else changes (kind, hidden state).
    fn plan_renames(&self, renames: &[(PathBuf, PathBuf)], exclude: &[String]) -> Vec<Op> {
        let mut want: Vec<&Path> = Vec::new();
        for (from, to) in renames {
            want.push(from);
            want.push(to);
            want.push(to.parent().unwrap_or(to));
        }
        let found = self.lookup(&want);
        let mut ops = Vec::new();
        let mut moved = HashSet::new();
        for (k, (from, to)) in renames.iter().enumerate() {
            let (Some(idx), None, Some(parent)) =
                (found[3 * k], found[3 * k + 1], found[3 * k + 2])
            else {
                continue;
            };
            let e = &self.entries[idx as usize];
            let p = &self.entries[parent as usize];
            if e.parent == NO_PARENT
                || parent >= idx
                || p.flags & F_DIR == 0
                || !self.relevant(to, exclude)
                || fs::symlink_metadata(from).is_ok()
                || !moved.insert(idx)
            {
                continue;
            }
            let (Ok(meta), Some(name)) = (fs::symlink_metadata(to), file_name(to)) else {
                continue;
            };
            let Some(c) = Child::new(name, &meta, self.hidden(parent), exclude) else {
                continue;
            };
            if (c.flags ^ e.flags) & (F_DIR | F_HIDDEN) != 0 {
                continue;
            }
            ops.push(Op::Rename {
                idx,
                parent,
                name: c.name,
                size: c.size,
                mtime: c.mtime,
            });
        }
        ops
    }

    /// Apply planned changes. Returns how many items changed.
    pub(super) fn apply(&mut self, ops: Vec<Op>) -> usize {
        let (mut entries, mut name_bytes) = (0, 0);
        for op in &ops {
            match op {
                Op::Rename { name, .. } => name_bytes += name.len(),
                Op::Add { items, .. } => {
                    entries += items.len();
                    name_bytes += items.iter().map(|it| it.child.name.len()).sum::<usize>();
                }
                _ => {}
            }
        }
        self.make_room(entries, name_bytes);
        let mut changed = 0;
        let mut removed = Vec::new();
        for op in ops {
            match op {
                Op::Remove(i) => removed.push(i),
                Op::Update { idx, size, mtime } => {
                    let e = &mut self.entries[idx as usize];
                    e.set_size(size);
                    e.mtime = mtime;
                    changed += 1;
                }
                Op::Rename {
                    idx,
                    parent,
                    name,
                    size,
                    mtime,
                } => {
                    let mut scratch = Vec::with_capacity(1);
                    push_entry(&mut self.names, &mut scratch, &name, parent, 0, size, mtime);
                    let new = scratch[0];
                    let e = &mut self.entries[idx as usize];
                    *e = Entry {
                        flags: e.flags,
                        ..new
                    };
                    self.garbage += 1;
                    changed += 1;
                }
                Op::Add { parent, items } => {
                    let base = self.entries.len() as u32;
                    for it in &items {
                        let c = &it.child;
                        let p = if it.parent == NO_PARENT {
                            parent
                        } else {
                            base + it.parent
                        };
                        push_entry(
                            &mut self.names,
                            &mut self.entries,
                            &c.name,
                            p,
                            c.flags,
                            c.size,
                            c.mtime,
                        );
                    }
                    changed += items.len();
                }
            }
        }
        // Removals last, so items added below a folder that is gone go too.
        let Some(&first) = removed.iter().min() else {
            return changed;
        };
        for i in removed {
            changed += self.mark_deleted(i as usize);
        }
        for i in first as usize + 1..self.entries.len() {
            let p = self.entries[i].parent;
            if p != NO_PARENT && self.entries[p as usize].flags & F_DELETED != 0 {
                changed += self.mark_deleted(i);
            }
        }
        changed
    }

    fn mark_deleted(&mut self, i: usize) -> usize {
        let e = &mut self.entries[i];
        if e.flags & F_DELETED != 0 {
            return 0;
        }
        e.flags |= F_DELETED;
        self.deleted += 1;
        self.garbage += 1;
        1
    }

    /// Reconcile all changes in `batch` with the disk (in one go).
    #[cfg(test)]
    pub(super) fn update(&mut self, batch: &Batch, exclude: &[String]) -> usize {
        batch
            .steps()
            .iter()
            .map(|step| {
                let ops = self.plan(step, exclude);
                self.apply(ops)
            })
            .sum()
    }

    /// Enough dead records and names piled up that compacting pays off.
    /// Room for `entries` more records and `name_bytes` more name bytes.
    /// The buffers are large and live updates add a few items at a time, so
    /// they grow by a little headroom: doubling would keep up to the size of
    /// the whole index reserved and unused.
    fn make_room(&mut self, entries: usize, name_bytes: usize) {
        if self.entries.capacity() - self.entries.len() < entries {
            let more = entries.max(self.entries.len() / 32);
            self.entries.reserve_exact(more);
        }
        if self.names.capacity() - self.names.len() < name_bytes {
            let more = name_bytes.max(self.names.len() / 32);
            self.names.reserve_exact(more);
        }
    }

    pub(super) fn needs_compaction(&self) -> bool {
        self.garbage > 200_000 || (self.garbage > 1000 && self.garbage * 4 > self.entries.len())
    }

    /// A copy without deleted records and unused names.
    pub(super) fn compact(&self) -> FileIndex {
        let mut names = String::with_capacity(self.names.len());
        let mut entries = Vec::with_capacity(self.len());
        let mut map = vec![NO_PARENT; self.entries.len()];
        for (i, e) in self.entries.iter().enumerate() {
            if e.flags & F_DELETED != 0 {
                continue;
            }
            let parent = if e.parent == NO_PARENT {
                NO_PARENT
            } else {
                map[e.parent as usize]
            };
            // A live entry never has a deleted parent; skip it defensively.
            if e.parent != NO_PARENT && parent == NO_PARENT {
                continue;
            }
            map[i] = push_entry(
                &mut names,
                &mut entries,
                self.name(e),
                parent,
                e.flags,
                e.size(),
                e.mtime,
            );
        }
        let mut idx = FileIndex {
            roots: self.roots.clone(),
            names,
            entries,
            built: self.built,
            deleted: 0,
            garbage: 0,
            mft_roots: self.mft_roots,
        };
        idx.share_names();
        idx
    }
}

/// `path` and, if it is a folder, everything in it (parents first).
fn walk_new(path: &Path, parent_hidden: bool, exclude: &[String]) -> Option<Vec<NewItem>> {
    let meta = fs::symlink_metadata(path).ok()?;
    let child = Child::new(file_name(path)?, &meta, parent_hidden, exclude)?;
    let mut stack = Vec::new();
    if child.flags & F_DIR != 0 {
        stack.push((path.to_path_buf(), 0, child.flags & F_HIDDEN != 0));
    }
    let mut items = vec![NewItem {
        child,
        parent: NO_PARENT,
    }];
    while let Some((dir, idx, hidden)) = stack.pop() {
        for c in read_children(&dir, hidden, exclude) {
            let i = items.len() as u32;
            if c.flags & F_DIR != 0 {
                stack.push((dir.join(&c.name), i, c.flags & F_HIDDEN != 0));
            }
            items.push(NewItem {
                child: c,
                parent: idx,
            });
        }
    }
    Some(items)
}

#[cfg(test)]
#[path = "patch_tests.rs"]
mod tests;
