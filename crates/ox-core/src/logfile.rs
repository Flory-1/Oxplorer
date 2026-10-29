//! Oxplorer's own log and crash reports, in the `logs` folder of the config
//! folder, so a bug report can say what went wrong.
//!
//! [`init`] makes the `log` macros write to `oxplorer.log` (warnings of other
//! crates, from info on for ours) and installs a panic hook that writes a
//! `crash-<time>.txt` report with the message, the place and a backtrace.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// The log is moved to `oxplorer.old.log` when it grows past this size.
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// Older crash reports are deleted.
const KEEP_CRASH_REPORTS: usize = 10;
const LOG_NAME: &str = "oxplorer.log";
const OLD_LOG_NAME: &str = "oxplorer.old.log";
/// Left behind when the app closed because of a crash; names the report.
const CRASH_MARKER: &str = "last-crash";

/// The folder with the log and the crash reports.
pub fn dir() -> PathBuf {
    crate::config_dir().join("logs")
}

/// Start logging to the log file and catch panics. Returns the crash report
/// of the last run if it ended in a crash (once; the next start returns
/// `None`).
pub fn init() -> Option<PathBuf> {
    let dir = dir();
    let crashed = take_crash_marker(&dir);
    match Writer::open(&dir) {
        Ok(w) => *LOGGER.writer.lock().unwrap_or_else(|e| e.into_inner()) = Some(w),
        Err(e) => eprintln!("could not open the log file: {e}"),
    }
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
    install_panic_hook(dir);
    log::info!(
        "{} {} started on {}",
        crate::APP_NAME,
        env!("CARGO_PKG_VERSION"),
        system()
    );
    crashed
}

static LOGGER: FileLogger = FileLogger {
    writer: Mutex::new(None),
};

struct FileLogger {
    writer: Mutex<Option<Writer>>,
}

impl Log for FileLogger {
    fn enabled(&self, meta: &Metadata) -> bool {
        let target = meta.target();
        let ours = target.starts_with("ox_");
        // The PDF reader of the text index complains about many odd files.
        let noisy = target.starts_with("pdf_extract") || target.starts_with("lopdf");
        meta.level()
            <= if ours {
                Level::Info
            } else if noisy {
                Level::Error
            } else {
                Level::Warn
            }
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(
            &now(),
            record.level(),
            record.target(),
            &record.args().to_string(),
        );
        if cfg!(debug_assertions) {
            eprint!("{line}");
        }
        if let Some(w) = self
            .writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            w.write(&line);
        }
    }

    fn flush(&self) {}
}

/// The open log file; it's moved aside when it gets too big.
struct Writer {
    dir: PathBuf,
    file: File,
    size: u64,
}

impl Writer {
    fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        rotate_if_large(dir, MAX_LOG_BYTES)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(LOG_NAME))?;
        let size = file.metadata().map_or(0, |m| m.len());
        Ok(Self {
            dir: dir.to_path_buf(),
            file,
            size,
        })
    }

    /// Writes straight to the file (unbuffered), so nothing is lost in a crash.
    fn write(&mut self, line: &str) {
        if self.size > MAX_LOG_BYTES {
            if let Ok(fresh) = Writer::open(&self.dir) {
                *self = fresh;
            }
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.size += line.len() as u64;
        }
    }
}

/// Move the log to `oxplorer.old.log` (replacing that) if it's bigger than `max`.
fn rotate_if_large(dir: &Path, max: u64) -> io::Result<()> {
    let log = dir.join(LOG_NAME);
    match fs::metadata(&log) {
        Ok(m) if m.len() > max => fs::rename(&log, dir.join(OLD_LOG_NAME)),
        _ => Ok(()),
    }
}

/// One line of the log; further lines of the message are indented.
fn format_line(time: &str, level: Level, target: &str, message: &str) -> String {
    let message = message.trim_end().replace('\n', "\n    ");
    format!("{time}  {level:<5}  {target}  {message}\n")
}

fn now() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string()
}

fn system() -> String {
    format!(
        "{} ({})",
        sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into()),
        std::env::consts::ARCH
    )
}

thread_local! {
    /// A panic now is caught by [`catch_panic`]: no crash report.
    static CATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` and turn a panic into `None`, with a line in the log instead of a
/// crash report: for readers of untrusted files (PDF text for the index).
pub fn catch_panic<T>(f: impl FnOnce() -> T) -> Option<T> {
    let before = CATCHING.with(|c| c.replace(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok();
    CATCHING.with(|c| c.set(before));
    result
}

fn install_panic_hook(dir: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if CATCHING.with(std::cell::Cell::get) {
            let message = info.payload_as_str().unwrap_or("(no message)");
            log::warn!("caught a panic: {message}");
            return;
        }
        let message = info.payload_as_str().unwrap_or("(no message)");
        let place = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("(unnamed)");
        log::error!(
            "panic in thread {thread} at {}: {message}",
            place.as_deref().unwrap_or("?")
        );
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let report = crash_report(message, place.as_deref(), thread, &backtrace);
        // Only a panic on the main thread closes the window.
        if let Err(e) = write_crash_report(&dir, &report, thread == "main") {
            log::error!("could not write the crash report: {e}");
        }
        default(info);
    }));
}

fn crash_report(message: &str, place: Option<&str>, thread: &str, backtrace: &str) -> String {
    format!(
        "{} {} crashed on {}\n\
         Time:    {}\n\
         System:  {}\n\
         Thread:  {thread}\n\
         Place:   {}\n\
         Message: {message}\n\
         \n\
         Backtrace:\n{backtrace}\n",
        crate::APP_NAME,
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().format("%Y-%m-%d"),
        now(),
        system(),
        place.unwrap_or("?"),
    )
}

/// Save `report` as a new `crash-<time>.txt`, delete the oldest ones, and,
/// if the app is closing because of it, leave the marker for the next start.
fn write_crash_report(dir: &Path, report: &str, closing: bool) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f").to_string();
    let path = dir.join(format!("crash-{}.txt", stamp.replace('.', "-")));
    fs::write(&path, report)?;
    prune_crash_reports(dir, KEEP_CRASH_REPORTS);
    if closing {
        fs::write(dir.join(CRASH_MARKER), path.to_string_lossy().as_bytes())?;
    }
    Ok(path)
}

/// Keep only the newest `keep` crash reports (their names sort by time).
fn prune_crash_reports(dir: &Path, keep: usize) {
    let Ok(items) = fs::read_dir(dir) else {
        return;
    };
    let mut reports: Vec<PathBuf> = items
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    reports.sort();
    let extra = reports.len().saturating_sub(keep);
    for old in &reports[..extra] {
        let _ = fs::remove_file(old);
    }
}

/// The report the marker names, if it's still there; the marker is removed.
fn take_crash_marker(dir: &Path) -> Option<PathBuf> {
    let marker = dir.join(CRASH_MARKER);
    let named = fs::read_to_string(&marker).ok()?;
    let _ = fs::remove_file(&marker);
    let report = PathBuf::from(named.trim());
    report.is_file().then_some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_have_time_level_target_and_indented_continuations() {
        let line = format_line("2026-10-29 10:00:00.000", Level::Warn, "ox_app", "a\nb\n");
        assert_eq!(line, "2026-10-29 10:00:00.000  WARN   ox_app  a\n    b\n");
    }

    #[test]
    fn only_warnings_of_other_crates_are_logged() {
        let meta = |level, target| Metadata::builder().level(level).target(target).build();
        assert!(LOGGER.enabled(&meta(Level::Info, "ox_core::git")));
        assert!(!LOGGER.enabled(&meta(Level::Debug, "ox_app")));
        assert!(LOGGER.enabled(&meta(Level::Warn, "wgpu_core")));
        assert!(!LOGGER.enabled(&meta(Level::Info, "winit")));
    }

    #[test]
    fn a_big_log_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join(LOG_NAME);
        fs::write(&log, "small").unwrap();
        rotate_if_large(dir.path(), 10).unwrap();
        assert!(log.exists());
        fs::write(&log, "more than ten bytes").unwrap();
        rotate_if_large(dir.path(), 10).unwrap();
        assert!(!log.exists());
        assert_eq!(
            fs::read_to_string(dir.path().join(OLD_LOG_NAME)).unwrap(),
            "more than ten bytes"
        );
    }

    #[test]
    fn writer_starts_a_new_file_past_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Writer::open(dir.path()).unwrap();
        w.size = MAX_LOG_BYTES + 1;
        fs::write(
            dir.path().join(LOG_NAME),
            vec![b'x'; MAX_LOG_BYTES as usize + 1],
        )
        .unwrap();
        w.write("fresh\n");
        assert_eq!(
            fs::read_to_string(dir.path().join(LOG_NAME)).unwrap(),
            "fresh\n"
        );
        assert!(dir.path().join(OLD_LOG_NAME).exists());
    }

    #[test]
    fn crash_report_names_version_place_and_message() {
        let report = crash_report("index out of bounds", Some("src/app.rs:10:5"), "main", "bt");
        assert!(report.contains(env!("CARGO_PKG_VERSION")));
        assert!(report.contains("Place:   src/app.rs:10:5"));
        assert!(report.contains("Message: index out of bounds"));
        assert!(report.contains("Thread:  main"));
        assert!(report.ends_with("Backtrace:\nbt\n"));
    }

    #[test]
    fn a_crash_that_closed_the_app_is_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let report = write_crash_report(dir.path(), "boom", true).unwrap();
        assert_eq!(fs::read_to_string(&report).unwrap(), "boom");
        assert_eq!(take_crash_marker(dir.path()), Some(report));
        assert_eq!(take_crash_marker(dir.path()), None);
        // A panic on another thread doesn't close the app: no note next time.
        write_crash_report(dir.path(), "boom", false).unwrap();
        assert_eq!(take_crash_marker(dir.path()), None);
    }

    #[test]
    fn only_the_newest_crash_reports_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            fs::write(dir.path().join(format!("crash-2026010{i}.txt")), "").unwrap();
        }
        fs::write(dir.path().join(LOG_NAME), "").unwrap();
        prune_crash_reports(dir.path(), 2);
        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["crash-20260103.txt", "crash-20260104.txt", LOG_NAME]);
    }
}
