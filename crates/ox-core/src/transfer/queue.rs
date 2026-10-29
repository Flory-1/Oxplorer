//! The transfer queue: which waiting job may start next.
//!
//! Waiting jobs start in queue order (submission order, changed with
//! [`TransferEngine::move_in_queue`]). A job starts when a slot is free and,
//! with the per-disk queue, none of its drives is busy. A job further back
//! never takes a drive or slot that a job ahead of it is waiting for, so jobs
//! can't overtake each other on the same drive.
//!
//! A job on hold (paused while waiting) keeps its place but is passed over.
//! A job marked *run after the current ones* starts only when nothing else
//! runs and nothing ahead of it waits; while it waits, the jobs behind it
//! wait too, so it works like a sequential queue.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::{EngineConfig, Job, TransferEngine, TransferRequest};

/// Which jobs are running, on which volumes, and which are waiting.
#[derive(Default)]
pub(super) struct Scheduler {
    running: usize,
    busy: HashMap<String, usize>,
    /// Jobs that haven't started yet, in the order they should start.
    waiting: Vec<Arc<Job>>,
}

impl Scheduler {
    pub(super) fn enqueue(&mut self, job: Arc<Job>) {
        self.waiting.push(job);
    }

    /// Take a job out of the queue without starting it (it was cancelled).
    pub(super) fn dequeue(&mut self, id: u64) {
        self.waiting.retain(|j| j.id != id);
    }

    /// Whether `job` may start now. Walks the queue in order and lets every
    /// job ahead of `job` that could start take its slot and drives first.
    pub(super) fn can_start(&self, job: &Job, cfg: &EngineConfig) -> bool {
        let max = cfg.max_concurrent_jobs.max(1);
        let mut running = self.running;
        let mut taken: HashSet<&str> = self.busy.keys().map(String::as_str).collect();
        let mut waiting_ahead = false;
        let mut barrier = false;
        let queue = self.waiting.iter().map(Arc::as_ref);
        // A job missing from the queue (shouldn't happen) is checked as if last.
        let in_queue = self.waiting.iter().any(|j| j.id == job.id);
        let queue = queue.chain((!in_queue).then_some(job));
        for w in queue {
            let mine = w.id == job.id;
            if w.is_paused() && !mine {
                continue; // on hold: keeps its place, blocks nobody
            }
            let drives_free =
                !cfg.per_disk_queue || w.volumes.iter().all(|v| !taken.contains(v.as_str()));
            let ok = !barrier
                && running < max
                && drives_free
                && (!w.runs_after_current() || (running == 0 && !waiting_ahead));
            if mine {
                return ok && !w.is_paused();
            }
            if ok {
                running += 1;
            } else if w.runs_after_current() {
                barrier = true;
            }
            // Started or not, the drives are spoken for: later jobs don't overtake.
            taken.extend(w.volumes.iter().map(String::as_str));
            waiting_ahead = true;
        }
        false
    }

    pub(super) fn start(&mut self, job: &Job) {
        self.dequeue(job.id);
        self.running += 1;
        for v in &job.volumes {
            *self.busy.entry(v.clone()).or_default() += 1;
        }
    }

    pub(super) fn finish(&mut self, job: &Job) {
        self.running -= 1;
        for v in &job.volumes {
            if let Some(n) = self.busy.get_mut(v) {
                *n -= 1;
                if *n == 0 {
                    self.busy.remove(v);
                }
            }
        }
    }

    /// Move a waiting job `delta` places (negative = earlier).
    fn move_job(&mut self, id: u64, delta: isize) {
        let Some(from) = self.waiting.iter().position(|j| j.id == id) else {
            return;
        };
        let to = from
            .saturating_add_signed(delta)
            .min(self.waiting.len() - 1);
        let job = self.waiting.remove(from);
        self.waiting.insert(to, job);
    }
}

impl Job {
    /// The job waits until all transfers ahead of it are done and then runs alone.
    pub fn runs_after_current(&self) -> bool {
        self.run_after.load(Ordering::Relaxed)
    }

    /// Change whether a waiting job runs after the current ones (no effect once it started).
    pub fn set_run_after_current(&self, on: bool) {
        self.run_after.store(on, Ordering::Relaxed);
    }

    /// A paused job that hasn't started yet: it keeps its place in the queue.
    pub fn is_on_hold(&self) -> bool {
        self.state() == super::JobState::Queued && self.is_paused()
    }
}

impl TransferEngine {
    /// Queue a job that starts only after every running and waiting job is done.
    pub fn submit_after_current(&self, req: TransferRequest) -> Arc<Job> {
        self.submit_job(req, true)
    }

    /// Jobs that haven't started yet, in the order they will start.
    pub fn queue(&self) -> Vec<Arc<Job>> {
        self.scheduler.0.lock().unwrap().waiting.clone()
    }

    /// Move a waiting job `delta` places in the queue (negative = earlier).
    pub fn move_in_queue(&self, id: u64, delta: isize) {
        self.scheduler.0.lock().unwrap().move_job(id, delta);
        self.wake_queue();
    }

    /// Let waiting jobs check again whether they may start (after a job was
    /// put on hold, released or marked to run after the current ones).
    pub fn wake_queue(&self) {
        self.scheduler.1.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ConflictPolicy, TransferKind};
    use super::*;
    use std::path::PathBuf;

    fn job(id: u64, volumes: &[&str]) -> Arc<Job> {
        let req = TransferRequest::new(
            TransferKind::Copy,
            vec![],
            PathBuf::new(),
            ConflictPolicy::Ask,
        );
        let mut job = Job::new(id, &req);
        job.volumes = volumes.iter().map(|v| v.to_string()).collect();
        Arc::new(job)
    }

    fn cfg(max: usize, per_disk_queue: bool) -> EngineConfig {
        EngineConfig {
            max_concurrent_jobs: max,
            per_disk_queue,
            ..EngineConfig::default()
        }
    }

    /// Ids of the waiting jobs that may start now.
    fn startable(s: &Scheduler, cfg: &EngineConfig) -> Vec<u64> {
        s.waiting
            .iter()
            .filter(|j| s.can_start(j, cfg))
            .map(|j| j.id)
            .collect()
    }

    fn sched(jobs: &[Arc<Job>]) -> Scheduler {
        let mut s = Scheduler::default();
        for j in jobs {
            s.enqueue(j.clone());
        }
        s
    }

    #[test]
    fn jobs_start_in_queue_order() {
        let jobs = [job(1, &["C"]), job(2, &["C"]), job(3, &["C"])];
        let mut s = sched(&jobs);
        let c = cfg(1, true);
        assert_eq!(startable(&s, &c), vec![1]);
        s.move_job(3, -2);
        assert_eq!(ids(&s), vec![3, 1, 2]);
        assert_eq!(startable(&s, &c), vec![3]);
        s.start(&jobs[2]);
        assert!(startable(&s, &c).is_empty());
        s.finish(&jobs[2]);
        assert_eq!(startable(&s, &c), vec![1]);
    }

    #[test]
    fn later_jobs_dont_take_a_drive_an_earlier_one_waits_for() {
        // 1 runs on C. 2 needs C and D, 3 only D: 3 must not overtake 2 on D.
        let jobs = [
            job(1, &["C"]),
            job(2, &["C", "D"]),
            job(3, &["D"]),
            job(4, &["E"]),
        ];
        let mut s = sched(&jobs);
        let c = cfg(4, true);
        s.start(&jobs[0]);
        assert_eq!(startable(&s, &c), vec![4]);
        // Without the per-disk queue everything runs at once.
        assert_eq!(startable(&s, &cfg(4, false)), vec![2, 3, 4]);
    }

    #[test]
    fn jobs_on_hold_keep_their_place_but_block_nobody() {
        let jobs = [job(1, &["C"]), job(2, &["C"])];
        let s = sched(&jobs);
        let c = cfg(1, true);
        jobs[0].pause();
        assert!(jobs[0].is_on_hold());
        assert_eq!(startable(&s, &c), vec![2]);
        jobs[0].resume();
        assert_eq!(startable(&s, &c), vec![1]);
    }

    #[test]
    fn run_after_current_waits_for_everything_ahead() {
        let jobs = [job(1, &["C"]), job(2, &["D"]), job(3, &["E"])];
        jobs[1].set_run_after_current(true);
        let mut s = sched(&jobs);
        let c = cfg(4, true);
        // 2 waits for 1 even though it uses another drive; 3 waits behind 2.
        assert_eq!(startable(&s, &c), vec![1]);
        s.start(&jobs[0]);
        assert!(startable(&s, &c).is_empty());
        // Once nothing runs it starts, and the normal jobs behind it may join it.
        s.finish(&jobs[0]);
        assert_eq!(startable(&s, &c), vec![2, 3]);
    }

    #[test]
    fn run_after_current_on_hold_doesnt_block() {
        let jobs = [job(1, &["C"]), job(2, &["D"]), job(3, &["E"])];
        jobs[1].set_run_after_current(true);
        jobs[1].pause();
        let s = sched(&jobs);
        assert_eq!(startable(&s, &cfg(4, true)), vec![1, 3]);
    }

    #[test]
    fn moving_is_clamped_to_the_queue() {
        let jobs = [job(1, &[]), job(2, &[]), job(3, &[])];
        let mut s = sched(&jobs);
        s.move_job(1, -5);
        assert_eq!(ids(&s), vec![1, 2, 3]);
        s.move_job(1, 10);
        assert_eq!(ids(&s), vec![2, 3, 1]);
        s.move_job(3, 1);
        assert_eq!(ids(&s), vec![2, 1, 3]);
        s.move_job(99, 1);
        assert_eq!(ids(&s), vec![2, 1, 3]);
    }

    /// With real jobs: one that runs after the current one, and one on hold.
    #[test]
    fn engine_runs_queued_jobs_after_the_current_one_and_holds() {
        use super::super::JobState;
        use std::fs;
        use std::time::{Duration, Instant};

        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("f.bin"), vec![1u8; 1024 * 1024]).unwrap();
        let engine = TransferEngine::new(EngineConfig {
            speed_limit: 1024 * 1024,
            max_concurrent_jobs: 8,
            per_disk_queue: false,
            ..EngineConfig::default()
        });
        let req = |i: usize| {
            let dest = tmp.path().join(format!("d{i}"));
            fs::create_dir(&dest).unwrap();
            TransferRequest::new(
                TransferKind::Copy,
                vec![tmp.path().join("f.bin")],
                dest,
                ConflictPolicy::Ask,
            )
        };
        let first = engine.submit(req(0));
        let after = engine.submit_after_current(req(1));
        let wait = |job: &Job, s: JobState| {
            let start = Instant::now();
            while job.state() != s {
                assert!(
                    start.elapsed() < Duration::from_secs(20),
                    "{:?}",
                    job.state()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait(&first, JobState::Running);
        std::thread::sleep(Duration::from_millis(300));
        // The copy takes ~1 s; without the flag both would run in parallel.
        assert_eq!(after.state(), JobState::Queued);
        assert_eq!(ids_of(&engine.queue()), vec![after.id]);
        wait(&first, JobState::Completed);
        wait(&after, JobState::Completed);
        assert!(engine.queue().is_empty());

        // A job on hold lets the jobs behind it pass until it is released.
        let cfg = engine.config.read().unwrap().clone();
        engine.set_config(EngineConfig {
            max_concurrent_jobs: 1,
            ..cfg
        });
        let slow = engine.submit(req(2));
        let held = engine.submit(req(3));
        held.pause();
        let next = engine.submit(req(4));
        wait(&slow, JobState::Completed);
        wait(&next, JobState::Completed);
        assert!(held.is_on_hold());
        held.resume();
        engine.wake_queue();
        wait(&held, JobState::Completed);
    }

    fn ids_of(jobs: &[Arc<Job>]) -> Vec<u64> {
        jobs.iter().map(|j| j.id).collect()
    }

    fn ids(s: &Scheduler) -> Vec<u64> {
        s.waiting.iter().map(|j| j.id).collect()
    }
}
