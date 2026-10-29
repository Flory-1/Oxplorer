//! Reacting to the transfer queue every frame: show questions about failed
//! files, and do what the settings say once all transfers are done.

use std::sync::atomic::Ordering;

use super::*;
use ox_core::transfer::when_done::{BatchDone, BatchWatch, Countdown, WhenDone};

#[derive(Default)]
pub struct TransferWatch {
    batch: BatchWatch,
    /// The sleep countdown and the flag that stops its timer thread.
    sleep: Option<(Countdown, Arc<AtomicBool>)>,
    /// Jobs that asked about a failed file last frame.
    asking: HashSet<u64>,
    /// The next transfer started waits for the current ones.
    pub(super) run_after_next: bool,
}

impl TransferWatch {
    /// Seconds until the computer goes to sleep, while the countdown runs.
    pub fn seconds_to_sleep(&self, now: Instant) -> Option<u64> {
        self.sleep.as_ref().map(|(c, _)| c.seconds_left(now))
    }

    pub fn cancel_sleep(&mut self) {
        if let Some((_, stop)) = self.sleep.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    /// The timer runs on its own thread so the computer also goes to sleep
    /// when the window is minimized and doesn't redraw.
    fn start_sleep(&mut self) {
        self.cancel_sleep();
        let stop = Arc::new(AtomicBool::new(false));
        let countdown = Countdown::start(Instant::now());
        let flag = stop.clone();
        std::thread::spawn(move || {
            while !countdown.is_over(Instant::now()) {
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            if !flag.swap(true, Ordering::Relaxed) {
                if let Err(e) = ox_core::power::sleep() {
                    log::warn!("could not put the computer to sleep: {e}");
                }
            }
        });
        self.sleep = Some((countdown, stop));
    }
}

impl OxApp {
    pub(super) fn poll_transfer_watch(&mut self, ctx: &Context) {
        let jobs = self.engine.jobs();

        // A failed file waits for an answer: show the panel, flash the taskbar once.
        let asking: HashSet<u64> = jobs
            .iter()
            .filter(|j| j.is_asking())
            .map(|j| j.id)
            .collect();
        if !asking.is_empty() {
            self.ui.show_transfers = true;
            if asking
                .iter()
                .any(|id| !self.transfer_watch.asking.contains(id))
            {
                request_attention(ctx);
            }
        }
        self.transfer_watch.asking = asking;

        let watch = &mut self.transfer_watch;
        if let Some((_, stop)) = &watch.sleep {
            // Over (the timer thread took it) or a new transfer started.
            if stop.load(Ordering::Relaxed) || self.engine.has_active() {
                watch.cancel_sleep();
            }
        }
        let done = watch
            .batch
            .update(jobs.iter().map(|j| (j.id, j.state(), j.error_count())));
        if let Some(done) = done {
            self.when_done(ctx, done);
        }
    }

    fn when_done(&mut self, ctx: &Context, done: BatchDone) {
        match self.settings.transfer.when_done {
            WhenDone::Nothing => {}
            WhenDone::Notify => {
                self.toast(if done.errors == 0 {
                    tr("All transfers are done.").to_owned()
                } else {
                    ox_core::trf!(
                        "All transfers are done, with {v1}.",
                        v1 = ox_core::i18n::count(done.errors as u64, "error")
                    )
                });
                request_attention(ctx);
            }
            WhenDone::Sleep if done.may_sleep() => {
                self.transfer_watch.start_sleep();
                request_attention(ctx);
            }
            WhenDone::Sleep => {
                self.toast(tr("A transfer was cancelled, so the computer stays awake."));
            }
        }
    }
}

/// Flash the taskbar button (until the window is activated).
fn request_attention(ctx: &Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
        egui::UserAttentionType::Informational,
    ));
}
