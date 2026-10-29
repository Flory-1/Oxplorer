//! Running the `git` command line: without a console window on Windows,
//! without input (nothing can wait for an answer) and with a time limit.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

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
