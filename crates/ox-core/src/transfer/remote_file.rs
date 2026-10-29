//! One file of a remote job: downloaded, uploaded or streamed from one
//! server to another, into a `.name.oxpart` that gets the real name when
//! complete.
//!
//! A partial `.oxpart` is continued from where it stopped instead of
//! starting over: downloads read from that offset (SFTP seek, FTP `REST`,
//! WebDAV `Range`), uploads append (SFTP write at the offset, FTP `REST` +
//! `STOR`; WebDAV can't, so it starts over). That happens on retries in
//! the same run (automatic ones when the connection dropped mid-file, and
//! the "Retry" answer) and when a job is resumed from its journal and the
//! original is unchanged. A cancelled job removes its partial files, like
//! local jobs remove their journal; a file that failed while its server
//! stayed reachable does too. When the server is gone, they stay with the
//! journal, so the transfer can be resumed later.

use std::fs::{self, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::super::{ErrorChoice, Job, TransferKind};
use super::{name_of, parent_of, remote_err, verify, End, RemoteSide, Stamp, Task, Throttle};
use crate::fsops;
use crate::remote::RemoteError;

pub(super) struct Ctx<'a> {
    pub src: End<'a>,
    pub dst: End<'a>,
    pub throttle: Option<&'a Throttle>,
    pub ask: bool,
    pub keep_dates: bool,
    pub verify: bool,
}

/// Automatic retries of a file whose connection dropped, before the user
/// is asked (or the error is collected).
const AUTO_RETRIES: u32 = 3;

/// Blocks a copy between servers keeps in memory (each up to 256 KiB).
const PIPE_BLOCKS: usize = 8;

/// The wait before automatic retry `n` (0, 1, …).
fn backoff(n: u32) -> Duration {
    if cfg!(test) {
        Duration::from_millis(10)
    } else {
        Duration::from_secs(1 << n.min(4))
    }
}

/// Why an attempt failed.
pub(super) struct Failed {
    pub message: String,
    /// Trying again may help (the transfer itself broke, not a check).
    pub transient: bool,
}

impl From<String> for Failed {
    fn from(message: String) -> Self {
        Self {
            message,
            transient: true,
        }
    }
}

impl From<RemoteError> for Failed {
    fn from(e: RemoteError) -> Self {
        remote_err(e).into()
    }
}

impl From<io::Error> for Failed {
    fn from(e: io::Error) -> Self {
        e.to_string().into()
    }
}

/// Counts the bytes, applies the speed limit and stops when cancelled.
struct Meter<'a, T> {
    inner: T,
    job: &'a Job,
    throttle: Option<&'a Throttle>,
    reported: u64,
}

impl<'a, T> Meter<'a, T> {
    fn new(inner: T, job: &'a Job, ctx: &Ctx<'a>) -> Self {
        Self {
            inner,
            job,
            throttle: ctx.throttle,
            reported: 0,
        }
    }

    fn count(&mut self, n: usize) -> io::Result<()> {
        self.reported += n as u64;
        self.job.done_bytes.fetch_add(n as u64, Ordering::Relaxed);
        if let Some(t) = self.throttle {
            t.consume(n as u64);
        }
        if self.job.checkpoint() {
            Ok(())
        } else {
            // Not `Interrupted`: copy loops retry those.
            Err(io::Error::other(crate::i18n::t("Cancelled")))
        }
    }
}

impl<T: Write> Write for Meter<'_, T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count(n)?;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T: Read> Read for Meter<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count(n)?;
        Ok(n)
    }
}

/// The temporary name a file is written to before it gets its real name.
pub(super) fn temp_name(end: End<'_>, dst: &Path) -> PathBuf {
    let parent = parent_of(dst).unwrap_or_default();
    let name = name_of(dst).unwrap_or_default();
    end.join(&parent, &format!(".{name}.oxpart"))
}

/// Where the data of `task` is written first (`None`: right to its name,
/// for servers that replace a file only once it is complete).
fn target(ctx: &Ctx<'_>, task: &Task) -> Option<PathBuf> {
    match ctx.dst {
        End::Remote(side) if !side.fs.upload_via_temp() => None,
        end => Some(temp_name(end, &task.dst)),
    }
}

/// Remove a partial copy (after a cancel, or a failure that isn't the
/// connection's).
fn remove_partial(ctx: &Ctx<'_>, task: &Task) {
    if let Some(tmp) = target(ctx, task) {
        let _ = match ctx.dst {
            End::Local => fs::remove_file(&tmp).map_err(|e| e.to_string()),
            end => end.remove_file(&tmp),
        };
    }
}

/// Both servers still answer (a failure wasn't the connection's).
fn reachable(ctx: &Ctx<'_>, task: &Task) -> bool {
    let probe = |end: End<'_>, p: &Path| match end {
        End::Local => true,
        End::Remote(_) => parent_of(p).is_none_or(|dir| end.meta(&dir).is_ok()),
    };
    probe(ctx.src, &task.src) && probe(ctx.dst, &task.dst)
}

/// Transfer one file: retried automatically when the connection dropped,
/// then asked about (or collected) when it still fails.
pub(super) fn transfer(job: &Job, task: &Task, ctx: &Ctx<'_>) {
    job.journal
        .started_part(&task.src, &task.dst, Stamp::new(task.size, task.modified));
    let mut may_resume = task.resume;
    let mut auto = 0;
    loop {
        let mut counted = 0u64;
        let mut offset = 0u64;
        let result = attempt(job, task, ctx, may_resume, &mut offset, &mut counted);
        match result {
            Ok(()) => {
                top_up(job, task, counted);
                return finished(job, task, ctx);
            }
            Err(_) if job.is_cancelled() => {
                top_up(job, task, counted);
                remove_partial(ctx, task);
                return;
            }
            Err(failed) => {
                let lost = !reachable(ctx, task);
                // Broken in the middle (a dropped connection), or still
                // waiting for the server to come back after that.
                let moved = counted > offset;
                if failed.transient && auto < AUTO_RETRIES && (moved || (auto > 0 && lost)) {
                    auto += 1;
                    job.done_bytes.fetch_sub(counted, Ordering::Relaxed);
                    may_resume = true;
                    if wait(job, backoff(auto - 1)) {
                        continue;
                    }
                    return remove_partial(ctx, task);
                }
                if ctx.ask
                    && job.ask_about_error(&task.src, &task.dst, &failed.message)
                        == ErrorChoice::Retry
                {
                    job.done_bytes.fetch_sub(counted, Ordering::Relaxed);
                    may_resume = failed.transient;
                    auto = 0;
                    continue;
                }
                top_up(job, task, counted);
                if lost && failed.transient {
                    // Kept for resuming once the server is back.
                    job.journal.lost.store(true, Ordering::Relaxed);
                } else {
                    remove_partial(ctx, task);
                }
                job.push_file_error(&task.src, Some(&task.dst), failed.message);
                return;
            }
        }
    }
}

/// Wait `d` (unless cancelled meanwhile); `false` if cancelled.
fn wait(job: &Job, d: Duration) -> bool {
    let until = std::time::Instant::now() + d;
    while std::time::Instant::now() < until {
        if !job.checkpoint() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10).min(d));
    }
    job.checkpoint()
}

/// Keep the byte counter consistent however the file ended.
fn top_up(job: &Job, task: &Task, counted: u64) {
    if task.size > counted {
        job.done_bytes
            .fetch_add(task.size - counted, Ordering::Relaxed);
    }
}

/// The file is in place: the journal, and for a move the original goes.
fn finished(job: &Job, task: &Task, ctx: &Ctx<'_>) {
    // `done` comes after the original is gone (see the journal).
    if job.kind == TransferKind::Move {
        if let Err(e) = ctx.src.remove_file(&task.src) {
            return job.push_file_error(&task.src, None, e);
        }
    }
    let orig = Stamp::new(task.size, task.modified);
    match ctx.dst {
        End::Local => job
            .journal
            .finished(&task.src, &task.dst, task.size, task.modified),
        // Server dates are rounded: the size is what is checked on resume.
        End::Remote(_) => job.journal.finished_as(
            &task.src,
            &task.dst,
            orig,
            Some(Stamp::new(task.size, None)),
        ),
    }
    job.done_files.fetch_add(1, Ordering::Relaxed);
}

/// One try. `offset`: where it continued from; `counted`: the bytes it
/// added to the job's counter (the offset included).
fn attempt(
    job: &Job,
    task: &Task,
    ctx: &Ctx<'_>,
    may_resume: bool,
    offset: &mut u64,
    counted: &mut u64,
) -> Result<(), Failed> {
    match (ctx.src, ctx.dst) {
        (End::Remote(side), End::Local) => {
            download(job, side, task, ctx, may_resume, offset, counted)
        }
        (End::Local, End::Remote(side)) => {
            upload(job, side, task, ctx, may_resume, offset, counted)
        }
        (End::Remote(from), End::Remote(to)) => {
            stream(job, from, to, task, ctx, may_resume, offset, counted)
        }
        (End::Local, End::Local) => Err("unsupported".to_owned().into()),
    }
}

/// Count the bytes a continued file already has.
fn count_offset(job: &Job, offset: u64, counted: &mut u64) {
    job.done_bytes.fetch_add(offset, Ordering::Relaxed);
    *counted += offset;
}

fn download(
    job: &Job,
    side: &RemoteSide,
    task: &Task,
    ctx: &Ctx<'_>,
    may_resume: bool,
    offset: &mut u64,
    counted: &mut u64,
) -> Result<(), Failed> {
    let path = End::rpath(side, &task.src)?;
    let tmp = temp_name(End::Local, &task.dst);
    // Downloads aren't pre-allocated: the partial file's length is what arrived.
    let have = fs::metadata(&tmp)
        .ok()
        .filter(|m| m.is_file() && may_resume)
        .map(|m| m.len())
        .filter(|&n| n <= task.size)
        .unwrap_or(0);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(have == 0)
        .open(&tmp)?;
    if have > 0 {
        file.set_len(have)?;
        file.seek(SeekFrom::End(0))?;
    }
    *offset = have;
    count_offset(job, have, counted);
    let mut meter = Meter::new(BufWriter::with_capacity(1024 * 1024, file), job, ctx);
    let got = if have == task.size && have > 0 {
        Ok(0)
    } else {
        side.fs.read_from(&path, have, &mut meter)
    };
    *counted += meter.reported;
    // Written out also after a failure: it is continued from there.
    let file = meter.inner.into_inner().map_err(|e| e.error().to_string());
    got?;
    let file = file?;
    if ctx.verify {
        drop(file);
        verify::check(job, task, ctx, &tmp)?;
    } else {
        drop(file);
    }
    if ctx.keep_dates {
        if let Some(t) = task.modified {
            if let Ok(f) = OpenOptions::new().write(true).open(&tmp) {
                let _ = f.set_modified(t);
            }
        }
    }
    if task.conflict || task.replace {
        let _ = fsops::remove_file_forced(&task.dst);
    }
    fs::rename(&tmp, &task.dst)?;
    Ok(())
}

/// How much of `target` is already on the server (a partial upload that
/// may be continued).
fn uploaded_part(side: &RemoteSide, target: &str, task: &Task, may_resume: bool) -> u64 {
    if !may_resume || !side.fs.can_append() || !side.fs.upload_via_temp() {
        return 0;
    }
    match side.fs.stat(target) {
        Ok(Some(e)) if !e.is_dir && e.size <= task.size => e.size,
        _ => 0,
    }
}

/// Give the finished upload its name and date, then verify it (at its
/// name: some FTP servers don't list the hidden `.oxpart` files).
fn finish_upload(
    job: &Job,
    side: &RemoteSide,
    task: &Task,
    ctx: &Ctx<'_>,
    target: &str,
) -> Result<(), Failed> {
    let dst = End::rpath(side, &task.dst)?;
    if target != dst {
        if task.conflict || task.replace {
            let _ = side.fs.remove_file(&dst);
        }
        side.fs.rename(target, &dst)?;
    }
    if ctx.keep_dates {
        if let Some(t) = task.modified {
            let _ = side.fs.set_modified(&dst, t);
        }
    }
    if ctx.verify {
        if let Err(e) = verify::check(job, task, ctx, &task.dst) {
            // Never keep (or move away the original of) a bad copy.
            let _ = side.fs.remove_file(&dst);
            return Err(e);
        }
    }
    Ok(())
}

fn upload(
    job: &Job,
    side: &RemoteSide,
    task: &Task,
    ctx: &Ctx<'_>,
    may_resume: bool,
    offset: &mut u64,
    counted: &mut u64,
) -> Result<(), Failed> {
    let target_path = target(ctx, task).unwrap_or_else(|| task.dst.clone());
    let target = End::rpath(side, &target_path)?;
    let have = uploaded_part(side, &target, task, may_resume);
    let mut file = fs::File::open(&task.src)?;
    if have > 0 {
        file.seek(SeekFrom::Start(have))?;
    }
    *offset = have;
    count_offset(job, have, counted);
    let mut meter = Meter::new(BufReader::with_capacity(1024 * 1024, file), job, ctx);
    let sent = if have > 0 {
        side.fs
            .append_from(&target, have, &mut meter, task.size - have)
    } else {
        side.fs.write_from(&target, &mut meter, task.size)
    };
    *counted += meter.reported;
    sent?;
    finish_upload(job, side, task, ctx, &target)
}

/// Hands the blocks of a download to the upload (bounded: the download
/// waits when the upload is slower).
struct PipeWriter {
    tx: crossbeam_channel::Sender<Result<Vec<u8>, String>>,
}

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tx
            .send(Ok(buf.to_vec()))
            .map_err(|_| io::Error::other(crate::i18n::t("Cancelled")))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct PipeReader {
    rx: crossbeam_channel::Receiver<Result<Vec<u8>, String>>,
    block: Vec<u8>,
    pos: usize,
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pos == self.block.len() {
            match self.rx.recv() {
                Ok(Ok(block)) => {
                    self.block = block;
                    self.pos = 0;
                }
                // The download failed: so does the upload (not a short file).
                Ok(Err(e)) => return Err(io::Error::other(e)),
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.block.len() - self.pos);
        buf[..n].copy_from_slice(&self.block[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// From one server to another (or within one): the download feeds the
/// upload through a small buffer, no temporary file.
#[allow(clippy::too_many_arguments)]
fn stream(
    job: &Job,
    from: &RemoteSide,
    to: &RemoteSide,
    task: &Task,
    ctx: &Ctx<'_>,
    may_resume: bool,
    offset: &mut u64,
    counted: &mut u64,
) -> Result<(), Failed> {
    let src = End::rpath(from, &task.src)?;
    let target_path = target(ctx, task).unwrap_or_else(|| task.dst.clone());
    let target = End::rpath(to, &target_path)?;
    let have = uploaded_part(to, &target, task, may_resume);
    *offset = have;
    count_offset(job, have, counted);
    let (tx, rx) = crossbeam_channel::bounded(PIPE_BLOCKS);
    let (got, sent, reported) = std::thread::scope(|scope| {
        let reader = scope.spawn(move || {
            let mut pipe = PipeWriter { tx };
            let got = from.fs.read_from(&src, have, &mut pipe);
            if let Err(e) = &got {
                let _ = pipe.tx.send(Err(e.to_string()));
            }
            got
        });
        let pipe = PipeReader {
            rx,
            block: Vec::new(),
            pos: 0,
        };
        let mut meter = Meter::new(pipe, job, ctx);
        let sent = if have > 0 {
            to.fs
                .append_from(&target, have, &mut meter, task.size - have)
        } else {
            to.fs.write_from(&target, &mut meter, task.size)
        };
        let reported = meter.reported;
        // The download stops once nobody reads (cancelled, failed upload).
        drop(meter);
        let got = reader
            .join()
            .unwrap_or_else(|_| Err(RemoteError::Other("download thread".into())));
        (got, sent, reported)
    });
    *counted += reported;
    // The upload's error says most (it includes a failed download's).
    sent?;
    got?;
    finish_upload(job, to, task, ctx, &target)
}
