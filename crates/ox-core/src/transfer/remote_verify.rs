//! Verification of downloads, uploads and copies between servers (the
//! *verify* setting), before the copy gets its real name.
//!
//! The copy must have the size of the original, and the original must not
//! have changed during the transfer. Where the server computes checksums
//! (FTP `HASH`/`XSHA256`/…, WebDAV `OC-Checksum`), the copy's content is
//! compared too and the file counts as verified; otherwise (SFTP has no
//! standard checksum) it counts as "size checked only". Reading the file
//! back from the server to hash it would double the traffic, so that isn't
//! done.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use super::super::Job;
use super::file::{Ctx, Failed};
use super::{End, Task};
use crate::remote::checksum::{self, Checksum};

fn failed(message: &str) -> Failed {
    Failed {
        message: message.to_owned(),
        transient: false,
    }
}

/// Same date to the second (servers round them).
fn same_time(a: Option<SystemTime>, b: Option<SystemTime>) -> bool {
    let secs = |t: SystemTime| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .ok()
    };
    match (a, b) {
        (Some(a), Some(b)) => secs(a) == secs(b),
        _ => true,
    }
}

/// The server's checksum of `p` (`None` when it has none, or fails to
/// say: then only the size is checked).
fn server_sum(end: End<'_>, p: &Path) -> Option<Checksum> {
    let End::Remote(side) = end else {
        return None;
    };
    let path = End::rpath(side, p).ok()?;
    side.fs.checksum(&path).ok().flatten()
}

/// Check `copy` (the finished data of `task` on the destination side).
/// Counts the file as verified or as size checked only.
pub(super) fn check(job: &Job, task: &Task, ctx: &Ctx<'_>, copy: &Path) -> Result<(), Failed> {
    let size = ctx.dst.meta(copy)?.map(|m| m.size);
    if size != Some(task.size) {
        return Err(failed(crate::i18n::t(
            "Verification failed: the copy differs from the original.",
        )));
    }
    let now = ctx.src.meta(&task.src)?;
    if !now.is_some_and(|m| m.size == task.size && same_time(m.modified, task.modified)) {
        return Err(failed(crate::i18n::t(
            "The original changed during the transfer. Try again.",
        )));
    }
    let go = || job.checkpoint();
    let local = |p: &Path, sum: &Checksum| checksum::hash_file(p, sum.kind, &go);
    let matched = match (ctx.src, ctx.dst) {
        (End::Remote(_), End::Local) => match server_sum(ctx.src, &task.src) {
            Some(sum) => Some(local(copy, &sum)? == sum),
            None => None,
        },
        (End::Local, End::Remote(_)) => match server_sum(ctx.dst, copy) {
            Some(sum) => Some(local(&task.src, &sum)? == sum),
            None => None,
        },
        (End::Remote(_), End::Remote(_)) => {
            match (server_sum(ctx.src, &task.src), server_sum(ctx.dst, copy)) {
                (Some(a), Some(b)) if a.kind == b.kind => Some(a == b),
                _ => None,
            }
        }
        (End::Local, End::Local) => None,
    };
    match matched {
        Some(false) => Err(failed(crate::i18n::t(
            "Verification failed: the copy differs from the original.",
        ))),
        Some(true) => {
            job.verified_files.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        None => {
            job.size_checked_files.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
}
