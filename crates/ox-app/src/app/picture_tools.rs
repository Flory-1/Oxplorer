//! Picture tools: rotate, flip, convert and resize the selected pictures on
//! helper threads, with progress and Undo. The work is done by
//! `ox_core::picture_tools`; the dialog and the progress window are in
//! `ui::picture_tools`.

use super::*;
use ox_core::i18n::count;
use ox_core::picture_tools::{self as pt, Convert, Job, Task, Transform};
use ox_core::undo::{Kind, Op};

use crate::ui::picture_tools::PictureDialog;

pub enum PictureAction {
    /// Rotate or flip pictures in place.
    Transform(Vec<PathBuf>, Transform),
    /// Open the Convert/Resize dialog for pictures (`resize`: with
    /// resizing ticked).
    Dialog(Vec<PathBuf>, bool),
    /// Convert pictures as set in the dialog.
    Convert(Vec<PathBuf>, Convert),
    /// Stop the running job after the pictures it is working on.
    Cancel,
    /// Close the list of pictures that couldn't be done.
    DismissFailures,
}

/// The picture job that is running, and how the last one went.
#[derive(Default)]
pub struct PictureTools {
    pub running: Option<RunningPictures>,
    /// What the last job couldn't do: a title and "name: reason" lines.
    pub failures: Option<(String, Vec<String>)>,
    /// The dialog's last settings, offered again next time.
    last: Option<Convert>,
}

pub struct RunningPictures {
    pub job: Arc<Job>,
    pub kind: Kind,
}

/// "Rotating pictures", for the progress window.
pub fn doing(kind: Kind) -> &'static str {
    match kind {
        Kind::Rotate => tr("Rotating pictures"),
        Kind::Flip => tr("Flipping pictures"),
        Kind::Resize => tr("Resizing pictures"),
        _ => tr("Converting pictures"),
    }
}

fn convert_kind(c: &Convert) -> Kind {
    if c.resize.is_some() {
        Kind::Resize
    } else {
        Kind::Convert
    }
}

impl OxApp {
    pub(super) fn process_pictures(&mut self, a: PictureAction) {
        match a {
            PictureAction::Transform(paths, tf) => {
                let kind = if tf.is_flip() {
                    Kind::Flip
                } else {
                    Kind::Rotate
                };
                self.start_pictures(Task::Transform(tf), paths, kind);
            }
            PictureAction::Dialog(paths, resize) => {
                let last = self.pictures.last.clone().unwrap_or_default();
                let destinations = ui::quick_destinations(self);
                self.dialog = Some(Dialog::Pictures(Box::new(PictureDialog::new(
                    paths,
                    last,
                    resize,
                    destinations,
                ))));
            }
            PictureAction::Convert(paths, c) => {
                let kind = convert_kind(&c);
                self.pictures.last = Some(c.clone());
                self.start_pictures(Task::Convert(c), paths, kind);
            }
            PictureAction::Cancel => {
                if let Some(r) = &self.pictures.running {
                    r.job.cancel();
                }
            }
            PictureAction::DismissFailures => self.pictures.failures = None,
        }
    }

    fn start_pictures(&mut self, task: Task, mut paths: Vec<PathBuf>, kind: Kind) {
        if self.pictures.running.is_some() {
            self.toast(tr(
                "Other pictures are being worked on. Try again when they are done.",
            ));
            return;
        }
        paths.retain(|p| pt::is_supported(p));
        if paths.is_empty() {
            return;
        }
        let ctx = self.ctx.clone();
        let job = pt::start(
            task,
            paths,
            Arc::new(ox_core::undo::RecycleBin),
            move || ctx.request_repaint(),
        );
        self.pictures.failures = None;
        self.pictures.running = Some(RunningPictures { job, kind });
    }

    /// Every frame: when the job has finished, record it for Undo and say
    /// how it went.
    pub(super) fn poll_pictures(&mut self) {
        if !self
            .pictures
            .running
            .as_ref()
            .is_some_and(|r| r.job.is_finished())
        {
            return;
        }
        let Some(RunningPictures { job, kind }) = self.pictures.running.take() else {
            return;
        };
        let Some(s) = job.take_summary() else {
            return;
        };
        let done = s.created.len() + s.replaced.len();
        let mut dirs: Vec<PathBuf> = s
            .created
            .iter()
            .map(|p| &p.path)
            .chain(s.replaced.iter().map(|r| &r.new.path))
            .filter_map(|p| p.parent().map(Path::to_path_buf))
            .collect();
        dirs.sort();
        dirs.dedup();
        if !s.created.is_empty() {
            self.undo.record(kind, Op::Placed(s.created));
        }
        if !s.replaced.is_empty() {
            self.undo.record(
                kind,
                Op::Replaced {
                    items: s.replaced,
                    since: s.since,
                },
            );
        }
        for d in &dirs {
            self.mark_changed(Some(d));
        }
        let n = count(done as u64, "picture");
        if !s.failed.is_empty() {
            let failed = count(s.failed.len() as u64, "picture");
            let title = match kind {
                Kind::Rotate => ox_core::trf!("Couldn't rotate {n}:", n = failed),
                Kind::Flip => ox_core::trf!("Couldn't flip {n}:", n = failed),
                Kind::Resize => ox_core::trf!("Couldn't resize {n}:", n = failed),
                _ => ox_core::trf!("Couldn't convert {n}:", n = failed),
            };
            let lines = s
                .failed
                .iter()
                .map(|(p, e)| format!("{}: {e}", entry::display_name(p)))
                .collect();
            self.pictures.failures = Some((title, lines));
        } else if s.cancelled {
            self.toast(ox_core::trf!("Stopped after {n}", n = n));
        } else {
            self.toast(match kind {
                Kind::Rotate => ox_core::trf!("Rotated {n}", n = n),
                Kind::Flip => ox_core::trf!("Flipped {n}", n = n),
                Kind::Resize => ox_core::trf!("Resized {n}", n = n),
                _ => ox_core::trf!("Converted {n}", n = n),
            });
        }
    }
}
