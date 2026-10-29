//! Compare two folders: what only one side has, what is newer on one side
//! and what differs, optionally including sub-folders and file contents.
//!
//! Items are matched by name, ignoring case (as Windows does). Files count
//! as the same when size and date modified match; dates within two seconds
//! count as equal, since FAT and exFAT drives store them that coarsely.
//! With `content` on, files of the same size are compared byte by byte
//! instead and the date only decides which one is newer.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Receiver, TryRecvError};

use crate::transfer::{ConflictPolicy, TransferKind, TransferRequest};

/// Dates closer than this count as equal (FAT/exFAT store 2-second steps).
const TOLERANCE: Duration = Duration::from_secs(2);

/// How an item compares between the left and the right folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    OnlyLeft,
    OnlyRight,
    Same,
    NewerLeft,
    NewerRight,
    /// Same date but different size or contents, a file on one side and a
    /// folder on the other, or (with sub-folders) a folder with differences
    /// inside.
    Different,
}

/// How an item looks from one side (the pane it is shown in).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mark {
    OnlyHere,
    Newer,
    Older,
    Different,
    Same,
}

impl Status {
    /// The status as seen from the left (`true`) or right side.
    pub fn mark(self, left: bool) -> Mark {
        match (self, left) {
            (Status::OnlyLeft, true) | (Status::OnlyRight, false) => Mark::OnlyHere,
            (Status::NewerLeft, true) | (Status::NewerRight, false) => Mark::Newer,
            (Status::NewerLeft, false) | (Status::NewerRight, true) => Mark::Older,
            // Not shown on that side at all.
            (Status::OnlyLeft, false) | (Status::OnlyRight, true) => Mark::Different,
            (Status::Same, _) => Mark::Same,
            (Status::Different, _) => Mark::Different,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    /// Look inside folders both sides have.
    pub subfolders: bool,
    /// Compare files of equal size byte by byte instead of by date.
    pub content: bool,
    /// Include hidden and system items.
    pub hidden: bool,
}

/// One side of an item.
#[derive(Debug, Clone)]
pub struct Side {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct Item {
    /// Path below the compared folders (the left name if both have it).
    pub rel: PathBuf,
    pub left: Option<Side>,
    pub right: Option<Side>,
    pub status: Status,
}

impl Item {
    pub fn is_dir(&self) -> bool {
        self.left
            .as_ref()
            .or(self.right.as_ref())
            .is_some_and(|s| s.is_dir)
    }

    /// Directly in the compared folders (not in a sub-folder).
    pub fn is_top_level(&self) -> bool {
        self.rel.components().count() == 1
    }

    /// A folder both sides have: it only sums up what's inside.
    fn is_shared_folder(&self) -> bool {
        matches!((&self.left, &self.right), (Some(l), Some(r)) if l.is_dir && r.is_dir)
    }
}

/// How many items have each status (folders both sides have aren't counted:
/// they only sum up their contents).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub only_left: usize,
    pub only_right: usize,
    pub newer_left: usize,
    pub newer_right: usize,
    pub different: usize,
    pub same: usize,
}

impl Counts {
    pub fn differences(&self) -> usize {
        self.only_left + self.only_right + self.newer_left + self.newer_right + self.different
    }
}

#[derive(Debug, Clone)]
pub struct Comparison {
    pub left: PathBuf,
    pub right: PathBuf,
    pub options: Options,
    /// Every item, folders before what's inside them.
    pub items: Vec<Item>,
    /// Folders or files that couldn't be read.
    pub errors: usize,
}

/// Transfers that copy items from one side to the other.
#[derive(Debug, Clone, Default)]
pub struct CopyPlan {
    pub requests: Vec<TransferRequest>,
    /// Items copied to the right and to the left.
    pub to_right: usize,
    pub to_left: usize,
    /// Bytes of the files among them (folders not included).
    pub bytes: u64,
}

impl CopyPlan {
    pub fn is_empty(&self) -> bool {
        self.to_right + self.to_left == 0
    }
}

impl Comparison {
    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for item in self.items.iter().filter(|i| !i.is_shared_folder()) {
            match item.status {
                Status::OnlyLeft => c.only_left += 1,
                Status::OnlyRight => c.only_right += 1,
                Status::NewerLeft => c.newer_left += 1,
                Status::NewerRight => c.newer_right += 1,
                Status::Different => c.different += 1,
                Status::Same => c.same += 1,
            }
        }
        c
    }

    /// Marks for the items directly in the left (`true`) or right folder, by path.
    pub fn marks(&self, left: bool) -> HashMap<PathBuf, Mark> {
        self.items
            .iter()
            .filter(|i| i.is_top_level())
            .filter_map(|i| {
                let side = if left { &i.left } else { &i.right };
                side.as_ref().map(|s| (s.path.clone(), i.status.mark(left)))
            })
            .collect()
    }

    /// Copy what only the left side has to the right (`to_right`), or the other way.
    pub fn copy_missing(&self, to_right: bool, conflict: ConflictPolicy) -> CopyPlan {
        let (wanted, dest) = if to_right {
            (Status::OnlyLeft, &self.right)
        } else {
            (Status::OnlyRight, &self.left)
        };
        let mut main = TransferRequest::new(TransferKind::Copy, Vec::new(), dest.clone(), conflict);
        // Folders inside sub-folders, by the folder they go into.
        let mut nested: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        let mut plan = CopyPlan::default();
        for item in self.items.iter().filter(|i| i.status == wanted) {
            let Some(src) = (if to_right { &item.left } else { &item.right }) else {
                continue;
            };
            if item.is_top_level() {
                main.sources.push(src.path.clone());
            } else if !src.is_dir {
                main.pairs.push((src.path.clone(), dest.join(&item.rel)));
            } else {
                let parent = item.rel.parent().unwrap_or(Path::new(""));
                nested
                    .entry(dest.join(parent))
                    .or_default()
                    .push(src.path.clone());
            }
            if !src.is_dir {
                plan.bytes += src.size;
            }
            if to_right {
                plan.to_right += 1;
            } else {
                plan.to_left += 1;
            }
        }
        if !main.sources.is_empty() || !main.pairs.is_empty() {
            plan.requests.push(main);
        }
        for (into, sources) in nested {
            plan.requests.push(TransferRequest::new(
                TransferKind::Copy,
                sources,
                into,
                conflict,
            ));
        }
        plan
    }

    /// Replace older files with the newer ones, in both directions.
    pub fn copy_newer(&self) -> CopyPlan {
        let mut plan = CopyPlan::default();
        for to_right in [true, false] {
            let (wanted, dest) = if to_right {
                (Status::NewerLeft, &self.right)
            } else {
                (Status::NewerRight, &self.left)
            };
            let mut req = TransferRequest::new(
                TransferKind::Copy,
                Vec::new(),
                dest.clone(),
                ConflictPolicy::Overwrite,
            );
            for item in self.items.iter().filter(|i| i.status == wanted) {
                let (Some(l), Some(r)) = (&item.left, &item.right) else {
                    continue;
                };
                let (src, dst) = if to_right { (l, r) } else { (r, l) };
                req.pairs.push((src.path.clone(), dst.path.clone()));
                plan.bytes += src.size;
            }
            if to_right {
                plan.to_right = req.pairs.len();
            } else {
                plan.to_left = req.pairs.len();
            }
            if !req.pairs.is_empty() {
                plan.requests.push(req);
            }
        }
        plan
    }
}

/// The comparison was cancelled.
struct Cancelled;

struct Walker<'a> {
    options: Options,
    cancel: &'a AtomicBool,
    scanned: &'a AtomicU64,
    items: Vec<Item>,
    errors: usize,
    buf: (Vec<u8>, Vec<u8>),
}

/// A folder's items by lower-cased name: (name, side).
type Listing = BTreeMap<String, (String, Side)>;

impl Walker<'_> {
    fn check(&self) -> Result<(), Cancelled> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    fn list(&self, dir: &Path) -> io::Result<Listing> {
        let mut out = Listing::new();
        for item in fs::read_dir(dir)? {
            let Ok(item) = item else { continue };
            self.scanned.fetch_add(1, Ordering::Relaxed);
            let Ok(meta) = item.metadata() else { continue };
            let name = item.file_name().to_string_lossy().into_owned();
            if !self.options.hidden && crate::entry::is_hidden(&name, &meta) {
                continue;
            }
            // `file_type` doesn't follow links: links and junctions aren't entered.
            let is_dir = item.file_type().is_ok_and(|t| t.is_dir());
            let side = Side {
                path: item.path(),
                is_dir,
                size: if is_dir { 0 } else { meta.len() },
                modified: meta.modified().ok(),
            };
            out.insert(name.to_lowercase(), (name, side));
        }
        Ok(out)
    }

    /// Compare two folders; `true` if everything in them is the same.
    fn folder(&mut self, left: &Path, right: &Path, rel: &Path) -> Result<bool, Cancelled> {
        self.check()?;
        let (l, r) = match (self.list(left), self.list(right)) {
            (Ok(l), Ok(r)) => (l, r),
            _ => {
                self.errors += 1;
                return Ok(false);
            }
        };
        let mut right_items = r;
        let mut all_same = true;
        let mut keys: Vec<String> = l.keys().cloned().collect();
        keys.extend(right_items.keys().filter(|k| !l.contains_key(*k)).cloned());
        keys.sort();
        let mut left_items = l;
        for key in keys {
            self.check()?;
            let a = left_items.remove(&key);
            let b = right_items.remove(&key);
            let name = match (&a, &b) {
                (Some((n, _)), _) | (None, Some((n, _))) => n.clone(),
                (None, None) => continue,
            };
            let rel = rel.join(name);
            let (a, b) = (a.map(|x| x.1), b.map(|x| x.1));
            let at = self.items.len();
            self.items.push(Item {
                rel: rel.clone(),
                left: a.clone(),
                right: b.clone(),
                status: Status::Same,
            });
            let status = match (&a, &b) {
                (Some(_), None) => Status::OnlyLeft,
                (None, Some(_)) => Status::OnlyRight,
                (Some(a), Some(b)) if a.is_dir && b.is_dir => {
                    if !self.options.subfolders || self.folder(&a.path, &b.path, &rel)? {
                        Status::Same
                    } else {
                        Status::Different
                    }
                }
                (Some(a), Some(b)) if a.is_dir != b.is_dir => Status::Different,
                (Some(a), Some(b)) => self.files(a, b)?,
                (None, None) => unreachable!(),
            };
            self.items[at].status = status;
            all_same &= status == Status::Same;
        }
        Ok(all_same)
    }

    fn files(&mut self, a: &Side, b: &Side) -> Result<Status, Cancelled> {
        let same_date = match (a.modified, b.modified) {
            (Some(x), Some(y)) => {
                let d = x.duration_since(y).or_else(|_| y.duration_since(x));
                d.is_ok_and(|d| d <= TOLERANCE)
            }
            _ => true,
        };
        let same = if a.size != b.size {
            false
        } else if self.options.content {
            match self.same_contents(&a.path, &b.path) {
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
            Status::Same
        } else if same_date {
            Status::Different
        } else if a.modified > b.modified {
            Status::NewerLeft
        } else {
            Status::NewerRight
        })
    }

    /// Byte-by-byte comparison; stops at the first difference.
    /// A cancel shows up as `ErrorKind::Interrupted`.
    fn same_contents(&mut self, a: &Path, b: &Path) -> io::Result<bool> {
        let (mut fa, mut fb) = (File::open(a)?, File::open(b)?);
        let (ba, bb) = (&mut self.buf.0, &mut self.buf.1);
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let n = read_full(&mut fa, ba)?;
            let m = read_full(&mut fb, bb)?;
            if n != m || ba[..n] != bb[..m] {
                return Ok(false);
            }
            if n == 0 {
                return Ok(true);
            }
        }
    }
}

/// Fill `buf` as far as the file goes.
fn read_full(f: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Compare `left` with `right`. `None` if `cancel` was set; `scanned` counts
/// the items looked at so far.
pub fn compare(
    left: &Path,
    right: &Path,
    options: Options,
    cancel: &AtomicBool,
    scanned: &AtomicU64,
) -> Result<Option<Comparison>, String> {
    for dir in [left, right] {
        if let Err(e) = fs::read_dir(dir) {
            return Err(format!("{}: {e}", dir.display()));
        }
    }
    let mut w = Walker {
        options,
        cancel,
        scanned,
        items: Vec::new(),
        errors: 0,
        buf: (vec![0; 1 << 20], vec![0; 1 << 20]),
    };
    match w.folder(left, right, Path::new("")) {
        Ok(_) => Ok(Some(Comparison {
            left: left.to_path_buf(),
            right: right.to_path_buf(),
            options,
            items: w.items,
            errors: w.errors,
        })),
        Err(Cancelled) => Ok(None),
    }
}

/// A comparison running on its own thread. Dropping it cancels it.
pub struct CompareJob {
    rx: Receiver<Result<Comparison, String>>,
    cancel: Arc<AtomicBool>,
    scanned: Arc<AtomicU64>,
}

impl CompareJob {
    /// Start comparing; `notify` is called when the result is ready.
    pub fn start(
        left: PathBuf,
        right: PathBuf,
        options: Options,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let scanned = Arc::new(AtomicU64::new(0));
        let (c, s) = (cancel.clone(), scanned.clone());
        std::thread::Builder::new()
            .name("compare".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                if let Some(result) = compare(&left, &right, options, &c, &s).transpose() {
                    let _ = tx.send(result);
                    notify();
                }
            })
            .expect("spawn compare thread");
        Self {
            rx,
            cancel,
            scanned,
        }
    }

    /// The result once it's there (`Some` only once).
    pub fn poll(&self) -> Option<Result<Comparison, String>> {
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

impl Drop for CompareJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn write(path: &Path, text: &str, age_secs: u64) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, text).unwrap();
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 - age_secs);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    fn run(l: &Path, r: &Path, options: Options) -> Comparison {
        compare(l, r, options, &AtomicBool::new(false), &AtomicU64::new(0))
            .unwrap()
            .unwrap()
    }

    fn status(c: &Comparison, rel: &str) -> Option<Status> {
        c.items
            .iter()
            .find(|i| i.rel == Path::new(rel))
            .map(|i| i.status)
    }

    /// left/ and right/ with one item of every kind, and a sub-folder.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let (l, r) = (tmp.path().join("left"), tmp.path().join("right"));
        write(&l.join("same.txt"), "same", 100);
        write(&r.join("same.txt"), "same", 100);
        write(&l.join("only-left.txt"), "l", 100);
        write(&r.join("only-right.txt"), "r", 100);
        write(&l.join("newer-left.txt"), "new", 10);
        write(&r.join("newer-left.txt"), "old", 100);
        write(&l.join("newer-right.txt"), "old", 100);
        write(&r.join("newer-right.txt"), "newer", 10);
        write(&l.join("different.txt"), "short", 100);
        write(&r.join("different.txt"), "longer", 100);
        // FAT/exFAT round dates to two seconds.
        write(&l.join("fat.txt"), "fat", 100);
        write(&r.join("fat.txt"), "fat", 99);
        write(&l.join("sub/inner.txt"), "a", 100);
        write(&r.join("sub/inner.txt"), "b", 50);
        write(&l.join("sub/deep/missing.txt"), "m", 100);
        fs::create_dir_all(r.join("sub/deep")).unwrap();
        write(&l.join("onlydir/x.txt"), "x", 100);
        (tmp, l, r)
    }

    #[test]
    fn classifies_this_level() {
        let (_tmp, l, r) = setup();
        let c = run(&l, &r, Options::default());
        assert_eq!(status(&c, "same.txt"), Some(Status::Same));
        assert_eq!(status(&c, "only-left.txt"), Some(Status::OnlyLeft));
        assert_eq!(status(&c, "only-right.txt"), Some(Status::OnlyRight));
        assert_eq!(status(&c, "newer-left.txt"), Some(Status::NewerLeft));
        assert_eq!(status(&c, "newer-right.txt"), Some(Status::NewerRight));
        assert_eq!(status(&c, "different.txt"), Some(Status::Different));
        assert_eq!(status(&c, "fat.txt"), Some(Status::Same));
        // Without sub-folders a folder both have isn't looked into.
        assert_eq!(status(&c, "sub"), Some(Status::Same));
        assert_eq!(status(&c, "sub/inner.txt"), None);
        assert_eq!(status(&c, "onlydir"), Some(Status::OnlyLeft));
        let counts = c.counts();
        assert_eq!(
            counts,
            Counts {
                only_left: 2,
                only_right: 1,
                newer_left: 1,
                newer_right: 1,
                different: 1,
                same: 2,
            }
        );
        assert_eq!(counts.differences(), 6);

        let marks = c.marks(false);
        assert_eq!(marks[&r.join("only-right.txt")], Mark::OnlyHere);
        assert_eq!(marks[&r.join("newer-left.txt")], Mark::Older);
        assert_eq!(marks[&r.join("newer-right.txt")], Mark::Newer);
        assert!(!marks.contains_key(&l.join("only-left.txt")));
        assert_eq!(c.marks(true)[&l.join("newer-left.txt")], Mark::Newer);
    }

    #[test]
    fn looks_into_sub_folders() {
        let (_tmp, l, r) = setup();
        let options = Options {
            subfolders: true,
            ..Options::default()
        };
        let c = run(&l, &r, options);
        assert_eq!(status(&c, "sub"), Some(Status::Different));
        assert_eq!(status(&c, "sub/inner.txt"), Some(Status::NewerRight));
        assert_eq!(status(&c, "sub/deep"), Some(Status::Different));
        assert_eq!(status(&c, "sub/deep/missing.txt"), Some(Status::OnlyLeft));
        // A folder only one side has isn't entered.
        assert_eq!(status(&c, "onlydir/x.txt"), None);
        // Folders both have only sum up their contents.
        assert_eq!(c.counts().only_left, 3);
        assert_eq!(c.counts().newer_right, 2);
        assert_eq!(c.marks(true)[&l.join("sub")], Mark::Different);
    }

    #[test]
    fn compares_contents_when_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let (l, r) = (tmp.path().join("l"), tmp.path().join("r"));
        // Same contents, different dates: the same.
        write(&l.join("a.txt"), "hello", 1000);
        write(&r.join("a.txt"), "hello", 10);
        // Same size and date, different contents.
        write(&l.join("b.txt"), "hello", 100);
        write(&r.join("b.txt"), "world", 100);
        // Different contents and dates: newer.
        write(&l.join("c.txt"), "hello", 10);
        write(&r.join("c.txt"), "world", 100);
        let content = Options {
            content: true,
            ..Options::default()
        };
        let c = run(&l, &r, content);
        assert_eq!(status(&c, "a.txt"), Some(Status::Same));
        assert_eq!(status(&c, "b.txt"), Some(Status::Different));
        assert_eq!(status(&c, "c.txt"), Some(Status::NewerLeft));
        let c = run(&l, &r, Options::default());
        assert_eq!(status(&c, "a.txt"), Some(Status::NewerRight));
        assert_eq!(status(&c, "b.txt"), Some(Status::Same));
    }

    #[test]
    fn names_match_ignoring_case_and_kinds_must_match() {
        let tmp = tempfile::tempdir().unwrap();
        let (l, r) = (tmp.path().join("l"), tmp.path().join("r"));
        write(&l.join("Readme.TXT"), "x", 100);
        write(&r.join("readme.txt"), "x", 100);
        write(&l.join("thing"), "file", 100);
        fs::create_dir_all(r.join("thing")).unwrap();
        let c = run(&l, &r, Options::default());
        assert_eq!(status(&c, "Readme.TXT"), Some(Status::Same));
        assert_eq!(status(&c, "thing"), Some(Status::Different));
        assert_eq!(c.items.len(), 2);
    }

    #[test]
    fn plans_copies() {
        let (_tmp, l, r) = setup();
        let options = Options {
            subfolders: true,
            ..Options::default()
        };
        let c = run(&l, &r, options);
        let plan = c.copy_missing(true, ConflictPolicy::Skip);
        assert_eq!((plan.to_right, plan.to_left), (3, 0));
        let main = &plan.requests[0];
        assert_eq!(main.destination, r);
        let mut sources = main.sources.clone();
        sources.sort();
        assert_eq!(sources, vec![l.join("only-left.txt"), l.join("onlydir")]);
        assert_eq!(
            main.pairs,
            vec![(
                l.join("sub/deep/missing.txt"),
                r.join("sub/deep/missing.txt")
            )]
        );
        let plan = c.copy_missing(false, ConflictPolicy::Skip);
        assert_eq!(plan.to_left, 1);
        assert_eq!(plan.requests[0].sources, vec![r.join("only-right.txt")]);

        let newer = c.copy_newer();
        assert_eq!((newer.to_right, newer.to_left), (1, 2));
        assert!(newer
            .requests
            .iter()
            .all(|q| q.conflict == ConflictPolicy::Overwrite));
        assert!(newer.requests[1]
            .pairs
            .contains(&(r.join("sub/inner.txt"), l.join("sub/inner.txt"))));
    }

    #[test]
    fn nested_missing_folders_go_into_their_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let (l, r) = (tmp.path().join("l"), tmp.path().join("r"));
        write(&l.join("a/new/x.txt"), "x", 100);
        fs::create_dir_all(r.join("a")).unwrap();
        let c = run(
            &l,
            &r,
            Options {
                subfolders: true,
                ..Options::default()
            },
        );
        let plan = c.copy_missing(true, ConflictPolicy::Ask);
        assert_eq!(plan.requests.len(), 1);
        assert_eq!(plan.requests[0].destination, r.join("a"));
        assert_eq!(plan.requests[0].sources, vec![l.join("a/new")]);
    }

    #[test]
    fn copying_the_plan_makes_both_sides_the_same() {
        let (_tmp, l, r) = setup();
        let options = Options {
            subfolders: true,
            ..Options::default()
        };
        let c = run(&l, &r, options);
        let engine = crate::TransferEngine::new(Default::default());
        let mut plans = vec![c.copy_missing(true, ConflictPolicy::Skip)];
        plans.push(c.copy_missing(false, ConflictPolicy::Skip));
        plans.push(c.copy_newer());
        for req in plans.into_iter().flat_map(|p| p.requests) {
            let job = engine.submit(req);
            let start = Instant::now();
            while !job.state().is_finished() {
                assert!(start.elapsed() < Duration::from_secs(20), "copy timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(job.errors().is_empty(), "{:?}", job.errors());
        }
        let c = run(&l, &r, options);
        // Only the files with the same date and a different size are left.
        let left: Vec<_> = c
            .items
            .iter()
            .filter(|i| i.status != Status::Same && !i.is_dir())
            .map(|i| i.rel.clone())
            .collect();
        assert_eq!(left, vec![PathBuf::from("different.txt")]);
    }

    #[test]
    fn can_be_cancelled_and_runs_in_the_background() {
        let (_tmp, l, r) = setup();
        let cancelled = compare(
            &l,
            &r,
            Options::default(),
            &AtomicBool::new(true),
            &AtomicU64::new(0),
        );
        assert!(matches!(cancelled, Ok(None)));
        assert!(compare(
            &l.join("nope"),
            &r,
            Options::default(),
            &AtomicBool::new(false),
            &AtomicU64::new(0)
        )
        .is_err());

        let job = CompareJob::start(l, r, Options::default(), || {});
        let start = Instant::now();
        let result = loop {
            if let Some(r) = job.poll() {
                break r;
            }
            assert!(start.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(result.unwrap().counts().same, 2);
        assert!(job.scanned() > 0);
    }
}
