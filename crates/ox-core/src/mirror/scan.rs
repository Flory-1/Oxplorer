//! Walking the source and the target for a mirror plan.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;

use super::{CompareBy, Exclude, Group, Mode, Options, Row, Why};
use crate::compare::{same_contents, same_time};

/// The plan was cancelled.
pub(super) struct Cancelled;

/// One item of a folder.
#[derive(Debug, Clone)]
struct Item {
    path: PathBuf,
    is_dir: bool,
    size: u64,
    modified: Option<SystemTime>,
}

/// A folder's items by lower-cased name: (name, item). `excluded`: some
/// items were left out.
struct Listing {
    items: BTreeMap<String, (String, Item)>,
    excluded: bool,
}

/// Files and bytes in a folder; `excluded`: something in it is left out.
#[derive(Default)]
struct Tree {
    files: u64,
    bytes: u64,
    excluded: bool,
}

pub(super) struct Scanner<'a> {
    mode: Mode,
    contents: bool,
    exclude: &'a Exclude,
    cancel: &'a AtomicBool,
    scanned: &'a AtomicU64,
    buf: Option<(Vec<u8>, Vec<u8>)>,
    pub rows: Vec<Row>,
    pub target_files: u64,
    pub errors: usize,
    pub kept_extras: usize,
    pub conflicts: usize,
}

impl<'a> Scanner<'a> {
    pub fn new(
        options: &Options,
        exclude: &'a Exclude,
        cancel: &'a AtomicBool,
        scanned: &'a AtomicU64,
    ) -> Self {
        Self {
            mode: options.mode,
            contents: options.by == CompareBy::Contents,
            exclude,
            cancel,
            scanned,
            buf: None,
            rows: Vec::new(),
            target_files: 0,
            errors: 0,
            kept_extras: 0,
            conflicts: 0,
        }
    }

    fn check(&self) -> Result<(), Cancelled> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    /// Everything in `dir` (hidden items too) that isn't excluded.
    fn list(&self, dir: &Path, rel: &Path) -> io::Result<Listing> {
        let mut out = Listing {
            items: BTreeMap::new(),
            excluded: false,
        };
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            self.scanned.fetch_add(1, Ordering::Relaxed);
            let name = entry.file_name().to_string_lossy().into_owned();
            if self.exclude.matches(&rel.join(&name)) {
                out.excluded = true;
                continue;
            }
            let meta = entry.metadata()?;
            // `file_type` doesn't follow links: links and junctions are
            // items of their own, never entered.
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            let item = Item {
                path: entry.path(),
                is_dir,
                size: if is_dir { 0 } else { meta.len() },
                modified: meta.modified().ok(),
            };
            out.items.insert(name.to_lowercase(), (name, item));
        }
        Ok(out)
    }

    /// Files and bytes in a folder, without what is excluded.
    fn tree(&mut self, dir: &Path, rel: &Path) -> Result<Tree, Cancelled> {
        self.check()?;
        let mut tree = Tree::default();
        let Ok(listing) = self.list(dir, rel) else {
            self.errors += 1;
            // Unreadable: treated like one with excluded items, so it is
            // never removed or copied as a whole.
            tree.excluded = true;
            return Ok(tree);
        };
        tree.excluded = listing.excluded;
        for (_, (name, item)) in listing.items {
            if item.is_dir {
                let sub = self.tree(&item.path, &rel.join(&name))?;
                tree.files += sub.files;
                tree.bytes += sub.bytes;
                tree.excluded |= sub.excluded;
            } else {
                tree.files += 1;
                tree.bytes += item.size;
            }
        }
        Ok(tree)
    }

    fn row(&mut self, rel: PathBuf, group: Group, why: Why, item: &Item, tree: (u64, u64)) {
        let include = match group {
            Group::Same => false,
            Group::Update => !(why == Why::TargetNewer && self.mode == Mode::Update),
            _ => true,
        };
        self.rows.push(Row {
            rel,
            group,
            why,
            is_dir: item.is_dir,
            files: tree.0,
            bytes: tree.1,
            source: None,
            target: PathBuf::new(),
            include,
        });
    }

    /// Compare the folder `src` with `tgt` (`None`: the target doesn't have
    /// it); `dst` is where the target's folder is or will be.
    pub fn pair(
        &mut self,
        src: &Path,
        tgt: Option<&Path>,
        dst: &Path,
        rel: &Path,
    ) -> Result<(), Cancelled> {
        self.check()?;
        let Ok(source) = self.list(src, rel) else {
            // Never take the target's items for extras when the source
            // can't be read.
            self.errors += 1;
            return Ok(());
        };
        let target = match tgt.map(|t| self.list(t, rel)) {
            None => Listing {
                items: BTreeMap::new(),
                excluded: false,
            },
            Some(Ok(l)) => l,
            Some(Err(_)) => {
                self.errors += 1;
                return Ok(());
            }
        };
        let mut a = source.items;
        let mut b = target.items;
        let mut keys: Vec<String> = a.keys().cloned().collect();
        keys.extend(b.keys().filter(|k| !a.contains_key(*k)).cloned());
        keys.sort();
        for key in keys {
            self.check()?;
            match (a.remove(&key), b.remove(&key)) {
                (Some((name, s)), None) => {
                    let to = dst.join(&name);
                    self.new_item(&s, to, rel.join(&name), Why::New)?;
                }
                (None, Some((name, t))) => self.extra(&t, rel.join(&name), Why::Extra)?,
                (Some((name, s)), Some((_, t))) if s.is_dir && t.is_dir => {
                    self.pair(&s.path, Some(&t.path), &t.path, &rel.join(&name))?;
                }
                (Some((name, s)), Some((_, t))) if s.is_dir != t.is_dir => {
                    if self.mode == Mode::Update {
                        self.conflicts += 1;
                        self.count_target(&t, &rel.join(&name))?;
                    } else {
                        let rel = rel.join(&name);
                        self.extra(&t, rel.clone(), Why::Kind)?;
                        self.new_item(&s, t.path.clone(), rel, Why::Kind)?;
                    }
                }
                (Some((name, s)), Some((_, t))) => {
                    self.target_files += 1;
                    let why = self.files(&s, &t)?;
                    let group = if why == Why::Same {
                        Group::Same
                    } else {
                        Group::Update
                    };
                    self.row(rel.join(&name), group, why, &s, (1, s.size));
                    let last = self.rows.last_mut().expect("just pushed");
                    last.source = Some(s.path);
                    last.target = t.path;
                }
                (None, None) => {}
            }
        }
        Ok(())
    }

    /// Count the target's files in `t` (an item that is kept).
    fn count_target(&mut self, t: &Item, rel: &Path) -> Result<(), Cancelled> {
        self.target_files += if t.is_dir {
            self.tree(&t.path, rel)?.files
        } else {
            1
        };
        Ok(())
    }

    /// An item only the source has, copied to `to`. A folder with excluded
    /// items is gone into, so those stay behind.
    fn new_item(&mut self, s: &Item, to: PathBuf, rel: PathBuf, why: Why) -> Result<(), Cancelled> {
        let tree = if s.is_dir {
            let tree = self.tree(&s.path, &rel)?;
            if tree.excluded {
                return self.pair(&s.path, None, &to, &rel);
            }
            (tree.files, tree.bytes)
        } else {
            (1, s.size)
        };
        self.row(rel, Group::New, why, s, tree);
        let last = self.rows.last_mut().expect("just pushed");
        last.source = Some(s.path.clone());
        last.target = to;
        Ok(())
    }

    /// An item only the target has. Mirroring removes it; a folder with
    /// excluded items is gone into, so those are kept.
    fn extra(&mut self, t: &Item, rel: PathBuf, why: Why) -> Result<(), Cancelled> {
        if self.mode == Mode::Update {
            self.kept_extras += 1;
            return self.count_target(t, &rel);
        }
        let tree = if t.is_dir {
            let tree = self.tree(&t.path, &rel)?;
            if tree.excluded {
                return self.extras_in(&t.path, &rel);
            }
            (tree.files, tree.bytes)
        } else {
            (1, t.size)
        };
        self.target_files += tree.0;
        self.row(rel, Group::Remove, why, t, tree);
        self.rows.last_mut().expect("just pushed").target = t.path.clone();
        Ok(())
    }

    /// Every item in a target folder the source doesn't have (the folder
    /// itself stays: it holds excluded items).
    fn extras_in(&mut self, dir: &Path, rel: &Path) -> Result<(), Cancelled> {
        self.check()?;
        let Ok(listing) = self.list(dir, rel) else {
            self.errors += 1;
            return Ok(());
        };
        for (_, (name, item)) in listing.items {
            self.extra(&item, rel.join(&name), Why::Extra)?;
        }
        Ok(())
    }

    /// How a file in the source compares with the target's.
    fn files(&mut self, s: &Item, t: &Item) -> Result<Why, Cancelled> {
        let same_date = same_time(s.modified, t.modified);
        let same = if s.size != t.size {
            false
        } else if self.contents {
            let buf = self
                .buf
                .get_or_insert_with(|| (vec![0; 1 << 20], vec![0; 1 << 20]));
            match same_contents(&s.path, &t.path, self.cancel, buf) {
                Ok(same) => same,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(Cancelled),
                Err(_) => {
                    self.errors += 1;
                    false
                }
            }
        } else {
            same_date
        };
        Ok(if same {
            Why::Same
        } else if same_date {
            Why::Different
        } else if s.modified > t.modified {
            Why::Newer
        } else {
            Why::TargetNewer
        })
    }
}
