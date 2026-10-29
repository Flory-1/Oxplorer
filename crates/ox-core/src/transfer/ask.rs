//! Asking about a failed file while the job runs.
//!
//! When a file can't be copied, the worker that hit the error posts a
//! question and waits for the answer; the other workers of the job hold at
//! their next checkpoint until the question is answered. Only one question is
//! open at a time. "Skip all" answers every later error of the job, which are
//! then collected for the end like before.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use super::Job;

/// A file that failed and what went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorQuestion {
    pub path: PathBuf,
    pub dest: PathBuf,
    pub message: String,
}

/// The answer to an [`ErrorQuestion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorChoice {
    /// Try the same file again.
    Retry,
    /// Leave this file out (it is listed with the errors).
    Skip,
    /// Leave this and every later failing file of the job out without asking.
    SkipAll,
}

/// The open question of a job, if any.
#[derive(Default)]
pub(super) struct ErrorPrompt {
    question: Mutex<Option<ErrorQuestion>>,
    answer: Mutex<Option<ErrorChoice>>,
    skip_all: AtomicBool,
    /// Taken by the worker that asks, so questions come one at a time.
    turn: Mutex<()>,
}

impl ErrorPrompt {
    pub(super) fn is_open(&self) -> bool {
        self.question.lock().unwrap().is_some()
    }
}

impl Job {
    /// The failed file the job is waiting about, if any.
    pub fn error_question(&self) -> Option<ErrorQuestion> {
        self.prompt.question.lock().unwrap().clone()
    }

    /// The job is paused until its [`error_question`](Self::error_question) is answered.
    pub fn is_asking(&self) -> bool {
        self.prompt.is_open()
    }

    /// Answer the open error question. Does nothing if there is none.
    pub fn answer_error(&self, choice: ErrorChoice) {
        if self.prompt.is_open() {
            *self.prompt.answer.lock().unwrap() = Some(choice);
        }
    }

    /// Called by a worker whose file failed: ask and wait for the answer.
    /// Cancelling the job counts as [`ErrorChoice::Skip`].
    pub(super) fn ask_about_error(&self, path: &Path, dest: &Path, message: &str) -> ErrorChoice {
        let prompt = &self.prompt;
        if prompt.skip_all.load(Ordering::Relaxed) {
            return ErrorChoice::SkipAll;
        }
        let _turn = prompt.turn.lock().unwrap();
        // Answered while this worker waited for its turn.
        if prompt.skip_all.load(Ordering::Relaxed) {
            return ErrorChoice::SkipAll;
        }
        if self.is_cancelled() {
            return ErrorChoice::Skip;
        }
        *prompt.answer.lock().unwrap() = None;
        *prompt.question.lock().unwrap() = Some(ErrorQuestion {
            path: path.to_path_buf(),
            dest: dest.to_path_buf(),
            message: message.to_owned(),
        });
        let choice = loop {
            if self.is_cancelled() {
                break ErrorChoice::Skip;
            }
            if let Some(a) = prompt.answer.lock().unwrap().take() {
                break a;
            }
            thread::sleep(Duration::from_millis(50));
        };
        if choice == ErrorChoice::SkipAll {
            prompt.skip_all.store(true, Ordering::Relaxed);
        }
        *prompt.question.lock().unwrap() = None;
        choice
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::fs;
    use std::time::Instant;

    fn engine(ask_on_error: bool) -> TransferEngine {
        TransferEngine::new(EngineConfig {
            workers: 1,
            buffer_size: 64 * 1024,
            ask_on_error,
            ..EngineConfig::default()
        })
    }

    fn wait_until(job: &Job, done: impl Fn(&Job) -> bool) {
        let start = Instant::now();
        while !done(job) {
            assert!(start.elapsed() < Duration::from_secs(20), "job timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_question(job: &Job) -> ErrorQuestion {
        wait_until(job, |j| j.is_asking() || j.state().is_finished());
        job.error_question()
            .expect("the job asks about the failed file")
    }

    /// Files whose target folder `out` is blocked by a file, so copying fails
    /// until the test turns `out` into a folder.
    fn blocked(tmp: &Path, names: &[&str]) -> (TransferRequest, PathBuf) {
        let out = tmp.join("out");
        fs::write(&out, "not a folder").unwrap();
        let pairs = names
            .iter()
            .map(|n| {
                let src = tmp.join(n);
                fs::write(&src, format!("contents of {n}")).unwrap();
                (src, out.join(n))
            })
            .collect();
        let req = TransferRequest {
            pairs,
            ..TransferRequest::new(
                TransferKind::Copy,
                Vec::new(),
                tmp.to_path_buf(),
                ConflictPolicy::Ask,
            )
        };
        (req, out)
    }

    /// Errors that name a file target (the failed files, not the folder).
    fn file_errors(job: &Job) -> usize {
        job.errors().iter().filter(|e| e.dest.is_some()).count()
    }

    #[test]
    fn retry_after_fixing_the_cause_copies_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (req, out) = blocked(tmp.path(), &["a.txt"]);
        let job = engine(true).submit(req);

        let q = wait_question(&job);
        assert_eq!(q.path, tmp.path().join("a.txt"));
        assert_eq!(q.dest, out.join("a.txt"));
        assert!(!q.message.is_empty());
        assert_eq!(job.state(), JobState::Running);
        assert_eq!(file_errors(&job), 0, "nothing collected while asking");

        fs::remove_file(&out).unwrap();
        fs::create_dir(&out).unwrap();
        job.answer_error(ErrorChoice::Retry);
        wait_until(&job, |j| j.state().is_finished());

        assert_eq!(job.state(), JobState::Completed);
        assert_eq!(file_errors(&job), 0, "{:?}", job.errors());
        assert_eq!(
            fs::read_to_string(out.join("a.txt")).unwrap(),
            "contents of a.txt"
        );
        assert_eq!(job.done_files.load(Ordering::Relaxed), 1);
        assert_eq!(
            job.done_bytes.load(Ordering::Relaxed),
            job.total_bytes.load(Ordering::Relaxed)
        );
        assert!(!job.is_asking());
    }

    #[test]
    fn skip_asks_again_and_skip_all_collects_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let (req, _) = blocked(tmp.path(), &["a.txt", "b.txt", "c.txt", "d.txt"]);
        let job = engine(true).submit(req);

        let first = wait_question(&job);
        job.answer_error(ErrorChoice::Skip);
        wait_until(&job, |j| {
            j.error_question().is_some_and(|q| q.path != first.path)
        });
        assert_eq!(file_errors(&job), 1);
        job.answer_error(ErrorChoice::SkipAll);
        wait_until(&job, |j| j.state().is_finished());

        assert_eq!(job.state(), JobState::Completed);
        assert_eq!(file_errors(&job), 4);
        assert_eq!(job.retry_request().unwrap().pairs.len(), 4);
        assert!(!job.is_asking());
    }

    #[test]
    fn cancel_while_asking() {
        let tmp = tempfile::tempdir().unwrap();
        let (req, _) = blocked(tmp.path(), &["a.txt", "b.txt"]);
        let job = engine(true).submit(req);
        wait_question(&job);
        job.cancel();
        wait_until(&job, |j| j.state().is_finished());
        assert_eq!(job.state(), JobState::Cancelled);
        assert!(!job.is_asking());
    }

    #[test]
    fn without_asking_errors_are_collected() {
        let tmp = tempfile::tempdir().unwrap();
        let (req, _) = blocked(tmp.path(), &["a.txt", "b.txt"]);
        let job = engine(false).submit(req);
        wait_until(&job, |j| j.state().is_finished());
        assert_eq!(job.state(), JobState::Completed);
        assert_eq!(file_errors(&job), 2);
    }

    #[test]
    fn answers_without_a_question_are_ignored() {
        let req = TransferRequest::new(
            TransferKind::Copy,
            Vec::new(),
            PathBuf::new(),
            ConflictPolicy::Ask,
        );
        let job = Job::new(1, &req);
        job.answer_error(ErrorChoice::SkipAll);
        assert!(job.prompt.answer.lock().unwrap().is_none());
        assert!(job.error_question().is_none());
    }
}
