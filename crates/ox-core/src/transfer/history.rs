//! A plain-text log of finished transfers (`transfers.log` in the config folder).

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use super::{Job, JobState};
use crate::format;

/// The log is rotated to `transfers.old.log` when it grows past this size.
const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub fn log_path() -> PathBuf {
    crate::config_dir().join("transfers.log")
}

/// One human-readable line describing a finished job.
pub fn describe(job: &Job) -> String {
    let files = job.total_files.load(Ordering::Relaxed);
    let done = job.done_files.load(Ordering::Relaxed);
    let bytes = job.done_bytes.load(Ordering::Relaxed);
    let mut line = format!(
        "{}  {:<4}  {} of {} ({}) to {}  in {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        match job.kind {
            super::TransferKind::Copy => "Copy",
            super::TransferKind::Move => "Move",
        },
        done,
        format::count(files, "file"),
        format::size(bytes),
        job.destination.display(),
        format::duration(job.elapsed()),
    );
    if job.state() == JobState::Cancelled {
        line.push_str("  · cancelled");
    }
    let skipped = job.skipped_files.load(Ordering::Relaxed);
    if skipped > 0 {
        line.push_str(&format!("  · {skipped} skipped"));
    }
    let verified = job.verified_files.load(Ordering::Relaxed);
    if verified > 0 {
        line.push_str(&format!("  · {verified} verified"));
    }
    let errors = job.errors();
    if !errors.is_empty() {
        line.push_str(&format!(
            "  · {}",
            format::count(errors.len() as u64, "error")
        ));
        for e in errors.iter().take(20) {
            line.push_str(&format!("\n      {} — {}", e.path.display(), e.message));
        }
        if errors.len() > 20 {
            line.push_str(&format!("\n      … and {} more", errors.len() - 20));
        }
    }
    line
}

/// Append a finished job to the log.
pub fn append(job: &Job) -> io::Result<()> {
    append_line(&log_path(), &describe(job))
}

fn append_line(path: &std::path::Path, line: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = fs::rename(path, path.with_extension("old.log"));
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")
}

#[cfg(test)]
mod tests {
    use super::super::{ConflictPolicy, TransferKind, TransferRequest};
    use super::*;

    #[test]
    fn describes_and_appends() {
        let dir = tempfile::tempdir().unwrap();
        let job = Job::new(
            7,
            &TransferRequest::new(
                TransferKind::Move,
                vec![],
                dir.path().into(),
                ConflictPolicy::Ask,
            ),
        );
        job.total_files.store(3, Ordering::Relaxed);
        job.done_files.store(2, Ordering::Relaxed);
        job.push_file_error(&dir.path().join("x"), None, "Access denied.");
        let line = describe(&job);
        assert!(line.contains("Move  2 of 3 files"), "{line}");
        assert!(
            line.contains("1 error") && line.contains("Access denied."),
            "{line}"
        );

        let log = dir.path().join("logs/transfers.log");
        append_line(&log, "first").unwrap();
        append_line(&log, "second").unwrap();
        assert_eq!(fs::read_to_string(&log).unwrap(), "first\nsecond\n");
    }
}
