//! The copy tools being measured: Oxplorer's engine and, for comparison,
//! robocopy, xcopy and a plain sequential `std::fs::copy`.

use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ox_core::transfer::{EngineConfig, JobState, TransferEngine};
use ox_core::{ConflictPolicy, TransferKind, TransferRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Oxplorer,
    OxplorerMove,
    Robocopy,
    Xcopy,
    Std,
}

impl Tool {
    /// Tools `--compare` accepts.
    pub const COMPARE: [Tool; 3] = [Tool::Robocopy, Tool::Xcopy, Tool::Std];

    /// Short name, used on the command line and in the baseline file.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Oxplorer => "oxplorer",
            Tool::OxplorerMove => "oxplorer-move",
            Tool::Robocopy => "robocopy",
            Tool::Xcopy => "xcopy",
            Tool::Std => "std",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Oxplorer => "Oxplorer copy",
            Tool::OxplorerMove => "Oxplorer move",
            Tool::Robocopy => "robocopy /MT:8",
            Tool::Xcopy => "xcopy",
            Tool::Std => "std::fs::copy",
        }
    }

    pub fn parse(s: &str) -> Option<Tool> {
        Tool::COMPARE.into_iter().find(|t| t.name() == s)
    }

    /// Only Oxplorer's own results can fail the baseline check; the others
    /// are there to put the numbers into perspective.
    pub fn is_checked(self) -> bool {
        matches!(self, Tool::Oxplorer | Tool::OxplorerMove)
    }

    pub fn needs_windows(self) -> bool {
        matches!(self, Tool::Robocopy | Tool::Xcopy)
    }
}

pub struct Runner {
    engine: TransferEngine,
}

impl Runner {
    pub fn new(workers: usize) -> Self {
        Self {
            engine: TransferEngine::new(EngineConfig {
                workers,
                ..EngineConfig::default()
            }),
        }
    }

    /// Copy (or move) the folder `src` into `dest_parent` and return how long it took.
    /// A move is undone afterwards (not timed), so `src` is there for the next run.
    pub fn run(&self, tool: Tool, src: &Path, dest_parent: &Path) -> Result<Duration, String> {
        let target = dest_parent.join(src.file_name().unwrap());
        match tool {
            Tool::Oxplorer => self.engine_job(TransferKind::Copy, src, dest_parent),
            Tool::OxplorerMove => {
                let took = self.engine_job(TransferKind::Move, src, dest_parent)?;
                self.engine_job(TransferKind::Move, &target, src.parent().unwrap())
                    .map_err(|e| format!("moving the files back failed: {e}"))?;
                Ok(took)
            }
            Tool::Robocopy => {
                let mut cmd = Command::new("robocopy");
                cmd.arg(src).arg(&target).args([
                    "/E", "/MT:8", "/R:0", "/W:0", "/NJH", "/NJS", "/NFL", "/NDL", "/NP",
                ]);
                // robocopy: exit codes below 8 mean success.
                time_command(cmd, |code| code < 8)
            }
            Tool::Xcopy => {
                let mut dest = target.into_os_string();
                dest.push("\\");
                let mut cmd = Command::new("xcopy");
                cmd.arg(src).arg(dest).args(["/E", "/I", "/Q", "/Y", "/H"]);
                time_command(cmd, |code| code == 0)
            }
            Tool::Std => {
                let start = Instant::now();
                std_copy_tree(src, &target).map_err(|e| e.to_string())?;
                Ok(start.elapsed())
            }
        }
    }

    fn engine_job(&self, kind: TransferKind, src: &Path, dest: &Path) -> Result<Duration, String> {
        let req = TransferRequest::new(
            kind,
            vec![src.to_path_buf()],
            dest.to_path_buf(),
            ConflictPolicy::Overwrite,
        );
        let start = Instant::now();
        let job = self.engine.submit(req);
        let state = loop {
            let s = job.state();
            if s.is_finished() || s == JobState::AwaitingDecision {
                break s;
            }
            thread::sleep(Duration::from_millis(1));
        };
        let took = start.elapsed();
        let errors = job.errors();
        self.engine.clear_finished();
        match state {
            JobState::Completed if errors.is_empty() => Ok(took),
            JobState::Completed => Err(format!(
                "{} error(s), first: {}: {}",
                errors.len(),
                errors[0].path.display(),
                errors[0].message
            )),
            JobState::AwaitingDecision => {
                job.cancel();
                Err("the job stopped to ask about conflicts".into())
            }
            _ => Err("the job was cancelled".into()),
        }
    }
}

fn time_command(mut cmd: Command, ok: impl Fn(i32) -> bool) -> Result<Duration, String> {
    let start = Instant::now();
    let status = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("could not start {:?}: {e}", cmd.get_program()))?;
    let took = start.elapsed();
    match status.code() {
        Some(code) if ok(code) => Ok(took),
        code => Err(format!(
            "{:?} failed (exit code {code:?})",
            cmd.get_program()
        )),
    }
}

/// One file after the other, the way a naive copy loop would do it.
fn std_copy_tree(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std_copy_tree(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{generate, totals, Kind, Sizes};

    #[test]
    fn engine_copy_and_move_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        let sizes = Sizes {
            small_count: 30,
            small_size: 10,
            large_size: 0,
        };
        let s = generate(Kind::Small, &src, &sizes).unwrap();
        let runner = Runner::new(0);
        for tool in [Tool::Oxplorer, Tool::Std] {
            runner.run(tool, &s.root, &dest).unwrap();
            assert_eq!(totals(&dest.join("small")).unwrap(), (30, 300));
            fs::remove_dir_all(dest.join("small")).unwrap();
        }
        runner.run(Tool::OxplorerMove, &s.root, &dest).unwrap();
        assert_eq!(totals(&s.root).unwrap(), (30, 300));
        assert!(!dest.join("small").exists());
    }
}
