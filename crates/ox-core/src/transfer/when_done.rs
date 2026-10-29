//! What happens when all transfers are done: nothing, a notification, or sleep.
//!
//! [`BatchWatch`] notices the moment the last running transfer finishes and
//! sums up the batch; the app then acts on [`WhenDone`]. Sleep only follows a
//! batch in which no job was cancelled, and only after a [`Countdown`] the
//! user can cancel.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::JobState;

/// The setting "When all transfers are done".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WhenDone {
    #[default]
    Nothing,
    Notify,
    Sleep,
}

impl WhenDone {
    pub const ALL: [WhenDone; 3] = [Self::Nothing, Self::Notify, Self::Sleep];

    pub fn label(self) -> &'static str {
        crate::i18n::t(match self {
            Self::Nothing => "Do nothing",
            Self::Notify => "Show a notification",
            Self::Sleep => "Put the computer to sleep",
        })
    }
}

/// How long the sleep countdown runs before the computer goes to sleep.
pub const SLEEP_DELAY: Duration = Duration::from_secs(30);

/// A batch of transfers that just finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchDone {
    pub jobs: usize,
    pub cancelled: usize,
    pub errors: usize,
}

impl BatchDone {
    /// Sleep only follows transfers that all ran to the end.
    pub fn may_sleep(&self) -> bool {
        self.cancelled == 0
    }
}

/// Watches the job list frame by frame and reports when a batch is done.
#[derive(Debug, Default)]
pub struct BatchWatch {
    /// Jobs seen unfinished since the last batch ended: (cancelled, errors)
    /// once they finished.
    batch: HashMap<u64, Option<(bool, usize)>>,
}

impl BatchWatch {
    /// Feed the current jobs as `(id, state, error count)`. Returns the
    /// summary once, when the last unfinished job of a batch has finished.
    /// Jobs that were already finished when first seen don't count.
    pub fn update(
        &mut self,
        jobs: impl IntoIterator<Item = (u64, JobState, usize)>,
    ) -> Option<BatchDone> {
        let mut active = false;
        for (id, state, errors) in jobs {
            if !state.is_finished() {
                active = true;
                self.batch.insert(id, None);
            } else if let Some(slot) = self.batch.get_mut(&id) {
                *slot = Some((state == JobState::Cancelled, errors));
            }
        }
        if active || self.batch.is_empty() {
            return None;
        }
        let mut done = BatchDone::default();
        // A job that vanished from the list was dismissed after it finished.
        for (cancelled, errors) in self.batch.drain().map(|(_, r)| r.unwrap_or_default()) {
            done.jobs += 1;
            done.cancelled += usize::from(cancelled);
            done.errors += errors;
        }
        Some(done)
    }
}

/// The visible countdown before the computer goes to sleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Countdown {
    end: Instant,
}

impl Countdown {
    pub fn start(now: Instant) -> Self {
        Self {
            end: now + SLEEP_DELAY,
        }
    }

    /// Whole seconds left, rounded up (30 right after the start, 0 when over).
    pub fn seconds_left(&self, now: Instant) -> u64 {
        let left = self.end.saturating_duration_since(now);
        left.as_secs() + u64::from(left.subsec_nanos() > 0)
    }

    pub fn is_over(&self, now: Instant) -> bool {
        now >= self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_once_when_the_last_job_finishes() {
        let mut w = BatchWatch::default();
        assert_eq!(w.update([]), None);
        assert_eq!(w.update([(1, JobState::Running, 0)]), None);
        assert_eq!(
            w.update([(1, JobState::Completed, 0), (2, JobState::Queued, 0)]),
            None
        );
        let done = w
            .update([(1, JobState::Completed, 0), (2, JobState::Completed, 3)])
            .unwrap();
        assert_eq!(
            done,
            BatchDone {
                jobs: 2,
                cancelled: 0,
                errors: 3
            }
        );
        assert!(done.may_sleep());
        // Nothing new: no second report.
        assert_eq!(w.update([(1, JobState::Completed, 0)]), None);
    }

    #[test]
    fn a_cancelled_job_prevents_sleep() {
        let mut w = BatchWatch::default();
        w.update([(1, JobState::Running, 0), (2, JobState::Running, 0)]);
        let done = w
            .update([(1, JobState::Cancelled, 0), (2, JobState::Completed, 0)])
            .unwrap();
        assert_eq!(done.cancelled, 1);
        assert!(!done.may_sleep());
    }

    #[test]
    fn old_finished_jobs_and_dismissed_jobs() {
        let mut w = BatchWatch::default();
        // Finished before the watch saw it run: not a batch.
        assert_eq!(w.update([(1, JobState::Completed, 0)]), None);
        w.update([(1, JobState::Completed, 0), (2, JobState::Scanning, 0)]);
        // Job 2 finished and was dismissed between two frames.
        let done = w.update([(1, JobState::Completed, 0)]).unwrap();
        assert_eq!(done.jobs, 1);
        assert!(done.may_sleep());
    }

    #[test]
    fn countdown_runs_thirty_seconds() {
        let t0 = Instant::now();
        let c = Countdown::start(t0);
        assert_eq!(c.seconds_left(t0), 30);
        assert_eq!(c.seconds_left(t0 + Duration::from_millis(500)), 30);
        assert_eq!(c.seconds_left(t0 + Duration::from_secs(29)), 1);
        assert!(!c.is_over(t0 + Duration::from_millis(29_999)));
        assert!(c.is_over(t0 + SLEEP_DELAY));
        assert_eq!(c.seconds_left(t0 + Duration::from_secs(40)), 0);
    }
}
