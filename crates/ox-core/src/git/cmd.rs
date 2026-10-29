//! Running the `git` command line: without a console window on Windows,
//! without input (nothing can wait for an answer) and with a time limit.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::RecvTimeoutError;

use super::progress::Lines;

/// How long reading git (status, log, diff) may take.
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// How long changing something may take (hooks, fetching, pushing).
pub(super) const WRITE_TIMEOUT: Duration = Duration::from_secs(600);

/// `git -C root` with the environment every call shares.
pub(super) fn command(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root)
        // No terminal: git and its helpers must not ask on one (the
        // credential manager shows its own window), nor open an editor.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Run `cmd` and collect its output; stop it after `timeout`.
pub(super) fn output(cmd: &mut Command, timeout: Duration) -> io::Result<Output> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = read_all(child.stdout.take());
    let stderr = read_all(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let mut pause = Duration::from_millis(1);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            // The readers are left alone: a hook started by git may still
            // hold the pipes open.
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                crate::trf!(
                    "Git didn't finish within {n} seconds and was stopped.",
                    n = timeout.as_secs()
                ),
            ));
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(50));
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Like [`output`], but every line git writes to stderr goes to `on_line`
/// as it comes, progress updates too (see [`Lines`]), and setting `cancel`
/// stops git. The stderr returned has the lines that stay only.
pub(super) fn output_watched(
    cmd: &mut Command,
    timeout: Duration,
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
) -> io::Result<Output> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = read_all(child.stdout.take());
    let (tx, rx) = crossbeam_channel::unbounded::<Vec<u8>>();
    if let Some(mut pipe) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
    }
    let mut lines = Lines::default();
    let mut stays = Vec::new();
    let mut take = |bytes: &[u8], stays: &mut Vec<String>| {
        for (text, keep) in lines.feed(bytes) {
            on_line(&text);
            if keep {
                stays.push(text);
            }
        }
    };
    let deadline = Instant::now() + timeout;
    let status = loop {
        // Cancelled counts even if git just finished: the user said stop.
        let stop = if cancel.load(Ordering::Relaxed) {
            Some(io::Error::new(
                io::ErrorKind::Interrupted,
                crate::i18n::t("Cancelled"),
            ))
        } else if Instant::now() >= deadline {
            Some(io::Error::new(
                io::ErrorKind::TimedOut,
                crate::trf!(
                    "Git didn't finish within {n} seconds and was stopped.",
                    n = timeout.as_secs()
                ),
            ))
        } else {
            None
        };
        if let Some(e) = stop {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => take(&bytes, &mut stays),
            // stderr is closed already.
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(20)),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
    };
    // The rest of stderr; not for long, a helper git started may still
    // hold the pipe open.
    let until = Instant::now() + Duration::from_millis(300);
    while let Ok(bytes) = rx.recv_deadline(until) {
        take(&bytes, &mut stays);
    }
    stays.extend(lines.finish());
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stays.join("\n").into_bytes(),
    })
}

/// Read a pipe to its end on a thread of its own.
fn read_all<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_commands_that_take_too_long() {
        // Something that waits on every system: git's own `--help` would
        // open a browser, so use a shell sleep where there is one.
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("powershell");
            c.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 5"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("5");
            c
        };
        let started = Instant::now();
        match output(cmd.stdin(Stdio::null()), Duration::from_millis(300)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                eprintln!("no shell to wait with; skipping");
                return;
            }
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::TimedOut),
            Ok(_) => panic!("finished although it should have been stopped"),
        }
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
