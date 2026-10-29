//! Values of the extra details columns (dimensions, length, authors, …),
//! read in the background for the rows on screen and kept per file and
//! modification time, so the frame never waits for a file. Sorting and
//! grouping by them asks for all items of a folder too, after the rows on
//! screen ([`ColumnValues::get_later`]).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

use crossbeam_channel::{Receiver, Sender};
use egui::Context;
use ox_core::entry::FileEntry;
use ox_core::props::Props;

const WORKERS: usize = 2;
/// Requests beyond this are dropped, oldest first (long scrolled past).
const MAX_QUEUED: usize = 512;
/// Files whose values are kept; the oldest are forgotten first.
const MAX_KEPT: usize = 20_000;

struct Job {
    path: PathBuf,
    is_dir: bool,
    modified: Option<SystemTime>,
}

enum Slot {
    /// Asked for; `later`: only by [`ColumnValues::get_later`] so far.
    Loading {
        later: bool,
    },
    Ready(Arc<Props>, Option<SystemTime>),
}

struct Jobs {
    /// The rows on screen, newest first.
    now: Vec<Job>,
    /// All items of a sorted folder, in order, when `now` is empty.
    later: VecDeque<Job>,
}

struct Queue {
    jobs: Mutex<Jobs>,
    ready: Condvar,
}

/// `None` props: dropped from the queue before it was read.
struct Done {
    path: PathBuf,
    modified: Option<SystemTime>,
    props: Option<Props>,
}

pub struct ColumnValues {
    queue: Arc<Queue>,
    done_tx: Sender<Done>,
    done: Receiver<Done>,
    slots: RefCell<HashMap<PathBuf, Slot>>,
    kept: RefCell<VecDeque<PathBuf>>,
    /// Bumped whenever values arrive (or a request is dropped).
    arrived: Cell<u64>,
    /// Who the `later` jobs are for (see [`ColumnValues::later_for`]).
    later_owner: RefCell<Vec<(u64, u64)>>,
}

impl ColumnValues {
    pub fn new(ctx: &Context) -> Self {
        let queue = Arc::new(Queue {
            jobs: Mutex::new(Jobs {
                now: Vec::new(),
                later: VecDeque::new(),
            }),
            ready: Condvar::new(),
        });
        let (done_tx, done) = crossbeam_channel::unbounded();
        for i in 0..WORKERS {
            let queue = queue.clone();
            let tx = done_tx.clone();
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new()
                .name(format!("column-values-{i}"))
                .spawn(move || worker(&queue, &tx, &ctx));
        }
        Self {
            queue,
            done_tx,
            done,
            slots: RefCell::default(),
            kept: RefCell::default(),
            arrived: Cell::new(0),
            later_owner: RefCell::default(),
        }
    }

    /// Take in what the workers have read. Call once per frame.
    pub fn poll(&self) {
        let mut slots = self.slots.borrow_mut();
        let mut kept = self.kept.borrow_mut();
        for done in self.done.try_iter() {
            // Dropped ones count too: they can be asked for again.
            self.arrived.set(self.arrived.get() + 1);
            match done.props {
                Some(props) => {
                    kept.push_back(done.path.clone());
                    slots.insert(done.path, Slot::Ready(Arc::new(props), done.modified));
                }
                None => {
                    slots.remove(&done.path);
                }
            }
        }
        while kept.len() > MAX_KEPT {
            if let Some(old) = kept.pop_front() {
                slots.remove(&old);
            }
        }
    }

    /// The values of `e`, if read. Asks for them otherwise (again when the
    /// file changed).
    pub fn get(&self, e: &FileEntry) -> Option<Arc<Props>> {
        match self.slots.borrow().get(&e.path) {
            Some(Slot::Loading { later: false }) => return None,
            Some(Slot::Ready(props, modified)) if *modified == e.modified => {
                return Some(props.clone());
            }
            // Asked for later only: on screen now, so it goes first (it may
            // be read twice, which is harmless).
            _ => {}
        }
        self.request(e, false);
        None
    }

    /// Like [`ColumnValues::get`], but read after the rows on screen and
    /// never dropped: for all items of a folder sorted or grouped by these
    /// values.
    pub fn get_later(&self, e: &FileEntry) -> Option<Arc<Props>> {
        match self.slots.borrow().get(&e.path) {
            Some(Slot::Loading { .. }) => return None,
            Some(Slot::Ready(props, modified)) if *modified == e.modified => {
                return Some(props.clone());
            }
            _ => {}
        }
        self.request(e, true);
        None
    }

    /// Bumped whenever values arrived or a request was dropped (to see
    /// that something changed).
    pub fn arrived(&self) -> u64 {
        self.arrived.get()
    }

    /// The `later` requests are for these tabs (id and listing version)
    /// now; those made for others are dropped.
    pub fn later_for(&self, owners: Vec<(u64, u64)>) {
        if *self.later_owner.borrow() == owners {
            return;
        }
        *self.later_owner.borrow_mut() = owners;
        let dropped: Vec<Job> = {
            let mut jobs = self.queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
            jobs.later.drain(..).collect()
        };
        let mut slots = self.slots.borrow_mut();
        for job in dropped {
            if matches!(slots.get(&job.path), Some(Slot::Loading { later: true })) {
                slots.remove(&job.path);
            }
        }
    }

    fn request(&self, e: &FileEntry, later: bool) {
        self.slots
            .borrow_mut()
            .insert(e.path.clone(), Slot::Loading { later });
        let mut jobs = self.queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let job = Job {
            path: e.path.clone(),
            is_dir: e.is_dir,
            modified: e.modified,
        };
        if later {
            jobs.later.push_back(job);
            self.queue.ready.notify_one();
            return;
        }
        jobs.now.push(job);
        if jobs.now.len() > MAX_QUEUED {
            let old = jobs.now.remove(0);
            let _ = self.done_tx.send(Done {
                path: old.path,
                modified: old.modified,
                props: None,
            });
        }
        self.queue.ready.notify_one();
    }
}

fn worker(queue: &Queue, tx: &Sender<Done>, ctx: &Context) {
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                // Newest first: the rows on screen now.
                if let Some(job) = jobs.now.pop().or_else(|| jobs.later.pop_front()) {
                    break job;
                }
                jobs = queue.ready.wait(jobs).unwrap_or_else(|p| p.into_inner());
            }
        };
        let props = ox_core::props::read(&job.path, job.is_dir);
        let done = Done {
            path: job.path,
            modified: job.modified,
            props: Some(props),
        };
        if tx.send(done).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_arrive_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "x").unwrap();
        let entry = FileEntry::from_path(&path).unwrap();
        let values = ColumnValues::new(&Context::default());
        assert!(values.get(&entry).is_none());
        let start = std::time::Instant::now();
        let props = loop {
            values.poll();
            if let Some(p) = values.get(&entry) {
                break p;
            }
            assert!(start.elapsed().as_secs() < 20, "never read");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(props.attributes.is_some(), cfg!(windows));
    }

    #[test]
    fn values_asked_for_later_arrive_too_unless_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let entries: Vec<FileEntry> = (0..3)
            .map(|i| {
                let path = dir.path().join(format!("{i}.txt"));
                std::fs::write(&path, "x").unwrap();
                FileEntry::from_path(&path).unwrap()
            })
            .collect();
        let values = ColumnValues::new(&Context::default());
        values.later_for(vec![(1, 1)]);
        assert!(entries.iter().all(|e| values.get_later(e).is_none()));
        let start = std::time::Instant::now();
        loop {
            values.poll();
            if entries.iter().all(|e| values.get_later(e).is_some()) {
                break;
            }
            assert!(start.elapsed().as_secs() < 20, "never read");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(values.arrived() >= 3);
        // Another folder: what's still waiting for the old one is dropped
        // (asking again queues it again).
        values.later_for(vec![(2, 1)]);
        assert!(values.later_owner.borrow().as_slice() == [(2, 1)]);
    }
}
