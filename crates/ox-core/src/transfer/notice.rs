//! The Windows notification when transfers are done (Settings → Transfers
//! → Notifications when transfers are done): whether to show one and what
//! it says. Shown by the app with [`crate::toast`].

use std::path::PathBuf;

use super::TransferKind;
use crate::i18n::{count, t};
use crate::trf;

/// One job of a finished batch, as far as the notification tells of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    pub kind: TransferKind,
    /// How many items it started with, and the name of the first.
    pub items: usize,
    pub first: Option<String>,
    pub destination: PathBuf,
    pub cancelled: bool,
    /// Files that failed.
    pub errors: usize,
}

/// A notification's title and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// Whether to show a notification for a batch: only when it's switched on,
/// Oxplorer isn't the window in front (there the app shows its own
/// message) and a job ran to the end (cancelling was the user's doing).
pub fn wanted(enabled: bool, in_front: bool, jobs: &[Finished]) -> bool {
    enabled && !in_front && jobs.iter().any(|j| !j.cancelled)
}

/// What the notification says about the jobs that ran to the end; `None`
/// when all were cancelled.
pub fn notice(jobs: &[Finished]) -> Option<Notice> {
    let done: Vec<&Finished> = jobs.iter().filter(|j| !j.cancelled).collect();
    let first = *done.first()?;
    let errors: usize = done.iter().map(|j| j.errors).sum();
    let moves = done.iter().all(|j| j.kind == TransferKind::Move);
    let title = if errors > 0 {
        let files = count(errors as u64, "file");
        match moves {
            true => trf!("{v1} couldn't be moved", v1 = files),
            false => trf!("{v1} couldn't be copied", v1 = files),
        }
    } else if done.len() > 1 {
        trf!("{v1} finished", v1 = count(done.len() as u64, "transfer"))
    } else {
        match first.kind {
            TransferKind::Copy => t("Copy finished"),
            TransferKind::Move => t("Move finished"),
        }
        .to_owned()
    };
    let items: usize = done.iter().map(|j| j.items).sum();
    let what = match (&first.first, done.len(), items) {
        (Some(name), 1, 1) => name.clone(),
        _ => count(items as u64, "item"),
    };
    let body = if done.iter().all(|j| j.destination == first.destination) {
        trf!("{v1} to {v2}", v1 = what, v2 = first.destination.display())
    } else {
        trf!("{v1} to several folders", v1 = what)
    };
    Some(Notice { title, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(kind: TransferKind, items: usize, first: &str, dest: &str) -> Finished {
        Finished {
            kind,
            items,
            first: Some(first.into()),
            destination: PathBuf::from(dest),
            cancelled: false,
            errors: 0,
        }
    }

    // Tests run in English.
    #[test]
    fn says_what_went_where() {
        let one = job(TransferKind::Copy, 1, "report.pdf", r"D:\Backup");
        assert_eq!(
            notice(std::slice::from_ref(&one)),
            Some(Notice {
                title: "Copy finished".into(),
                body: r"report.pdf to D:\Backup".into(),
            })
        );
        let moved = job(TransferKind::Move, 12, "a.jpg", r"E:\Pictures");
        let n = notice(std::slice::from_ref(&moved)).unwrap();
        assert_eq!(n.title, "Move finished");
        assert_eq!(n.body, r"12 items to E:\Pictures");
        // Two jobs into the same folder, and into different ones.
        let other = job(TransferKind::Copy, 2, "b", r"D:\Backup");
        let n = notice(&[one.clone(), other]).unwrap();
        assert_eq!(n.title, "2 transfers finished");
        assert_eq!(n.body, r"3 items to D:\Backup");
        let n = notice(&[one.clone(), moved.clone()]).unwrap();
        assert_eq!(n.body, "13 items to several folders");
    }

    #[test]
    fn errors_and_cancelled_jobs() {
        let mut failed = job(TransferKind::Copy, 4, "x", r"D:\");
        failed.errors = 3;
        assert_eq!(
            notice(&[failed.clone()]).unwrap().title,
            "3 files couldn't be copied"
        );
        let mut moved = job(TransferKind::Move, 1, "y", r"D:\");
        moved.errors = 1;
        assert_eq!(
            notice(&[moved.clone()]).unwrap().title,
            "1 file couldn't be moved"
        );
        // Copies and moves together count as copies.
        let n = notice(&[failed.clone(), moved]).unwrap();
        assert_eq!(n.title, "4 files couldn't be copied");
        // A cancelled job isn't told of; all cancelled: nothing to say.
        let mut cancelled = job(TransferKind::Copy, 9, "z", r"C:\");
        cancelled.cancelled = true;
        cancelled.errors = 5;
        let n = notice(&[cancelled.clone(), failed.clone()]).unwrap();
        assert_eq!(n.title, "3 files couldn't be copied");
        assert_eq!(n.body, r"4 items to D:\");
        assert_eq!(notice(&[cancelled.clone()]), None);
        assert_eq!(notice(&[]), None);
    }

    #[test]
    fn only_in_the_background() {
        let one = [job(TransferKind::Copy, 1, "a", r"D:\")];
        assert!(wanted(true, false, &one));
        assert!(!wanted(true, true, &one), "in front: the app's own message");
        assert!(!wanted(false, false, &one), "switched off");
        let mut cancelled = one.clone();
        cancelled[0].cancelled = true;
        assert!(!wanted(true, false, &cancelled));
        assert!(!wanted(true, false, &[]));
    }
}
