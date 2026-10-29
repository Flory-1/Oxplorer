//! Values of the extra details columns (dimensions, length, authors, …),
//! read in the background for the rows on screen and kept per file and
//! modification time, so the frame never waits for a file.

use std::cell::RefCell;
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
    Loading,
    Ready(Arc<Props>, Option<SystemTime>),
}

struct Queue {
    jobs: Mutex<Vec<Job>>,
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
}

impl ColumnValues {
    pub fn new(ctx: &Context) -> Self {
        let queue = Arc::new(Queue {
            jobs: Mutex::new(Vec::new()),
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
        }
    }

    /// Take in what the workers have read. Call once per frame.
    pub fn poll(&self) {
        let mut slots = self.slots.borrow_mut();
        let mut kept = self.kept.borrow_mut();
        for done in self.done.try_iter() {
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
            Some(Slot::Loading) => return None,
            Some(Slot::Ready(props, modified)) if *modified == e.modified => {
                return Some(props.clone());
            }
            _ => {}
        }
        self.slots
            .borrow_mut()
            .insert(e.path.clone(), Slot::Loading);
        let mut jobs = self.queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
        jobs.push(Job {
            path: e.path.clone(),
            is_dir: e.is_dir,
            modified: e.modified,
        });
        if jobs.len() > MAX_QUEUED {
            let old = jobs.remove(0);
            let _ = self.done_tx.send(Done {
                path: old.path,
                modified: old.modified,
                props: None,
            });
        }
        self.queue.ready.notify_one();
        None
    }
}

fn worker(queue: &Queue, tx: &Sender<Done>, ctx: &Context) {
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                // Newest first: the rows on screen now.
                if let Some(job) = jobs.pop() {
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
}
