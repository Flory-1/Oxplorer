//! Reacting to the transfer queue every frame: show questions about failed
//! files, and do what the settings say once all transfers are done (and
//! a Windows notification when Oxplorer is in the background).

use std::sync::atomic::Ordering;

use super::*;
use ox_core::transfer::notice::{self, Finished};
use ox_core::transfer::when_done::{BatchDone, BatchWatch, Countdown, WhenDone};
use ox_core::transfer::JobState;

/// Notifications kept so that clicking them still reaches the window.
const KEEP_TOASTS: usize = 5;

#[derive(Default)]
pub struct TransferWatch {
    batch: BatchWatch,
    /// The sleep countdown and the flag that stops its timer thread.
    sleep: Option<(Countdown, Arc<AtomicBool>)>,
    /// Jobs that asked about a failed file last frame.
    asking: HashSet<u64>,
    /// The next transfer started waits for the current ones.
    pub(super) run_after_next: bool,
    /// The last Windows notifications, and whether one was clicked.
    toasts: Vec<ox_core::toast::Toast>,
    clicked: Arc<AtomicBool>,
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
        if self.transfer_watch.clicked.swap(false, Ordering::Relaxed) {
            // A notification was clicked: bring the window to the front.
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.ui.show_transfers = true;
        }
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
        let in_front = ctx.input(|i| {
            let v = i.viewport();
            v.focused.unwrap_or(true) && v.minimized != Some(true)
        });
        let finished = self.finished_jobs(&done);
        if notice::wanted(self.settings.transfer.notify_done, in_front, &finished) {
            self.show_toast(ctx, &finished);
        }
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

    /// The jobs of `done` still in the list, for the notification.
    fn finished_jobs(&self, done: &BatchDone) -> Vec<Finished> {
        let jobs = self.engine.jobs();
        done.ids
            .iter()
            .filter_map(|id| jobs.iter().find(|j| j.id == *id))
            .map(|j| Finished {
                kind: j.kind,
                items: j.item_count(),
                first: j.sources.first().map(|p| entry::display_name(p)),
                destination: j.destination.clone(),
                cancelled: j.state() == JobState::Cancelled,
                errors: j.error_count(),
            })
            .collect()
    }

    /// A Windows notification about `finished`; clicking it brings the
    /// window back (see [`Self::poll_transfer_watch`]).
    fn show_toast(&mut self, ctx: &Context, finished: &[Finished]) {
        let Some(n) = notice::notice(finished) else {
            return;
        };
        let (clicked, repaint) = (self.transfer_watch.clicked.clone(), ctx.clone());
        let icon = include_bytes!("../../assets/oxplorer.png");
        match ox_core::toast::show(&n.title, &n.body, icon, move || {
            clicked.store(true, Ordering::Relaxed);
            repaint.request_repaint();
        }) {
            Ok(t) => {
                let toasts = &mut self.transfer_watch.toasts;
                toasts.push(t);
                if toasts.len() > KEEP_TOASTS {
                    toasts.remove(0);
                }
            }
            Err(e) => log::warn!("could not show a notification: {e}"),
        }
    }
}

/// Flash the taskbar button (until the window is activated).
fn request_attention(ctx: &Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
        egui::UserAttentionType::Informational,
    ));
}
