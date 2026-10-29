//! Starting custom commands: directly or through the shell, hidden, in a
//! console window of their own, or with their output read line by line.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{line, Invocation, Window};
use crate::i18n::t as tr;

/// How a run is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// The program found, with the rest of the line as its arguments
    /// (exactly as quoted there).
    Direct(PathBuf),
    /// The whole line through `cmd /S /C "…"` (`sh -c` elsewhere).
    Shell,
}

/// Why a run failed.
#[derive(Debug)]
pub enum Failure {
    NotFound(String),
    NoFolder(PathBuf),
    Start(std::io::Error),
    /// It ended with an exit code other than 0 (`None`: it was ended) and
    /// printed `message` as its error.
    Exit {
        code: Option<i32>,
        message: String,
    },
}

impl Failure {
    /// The message for a toast.
    pub fn message(&self) -> String {
        match self {
            Self::NotFound(p) => crate::trf!("Program not found: {p}", p = p),
            Self::NoFolder(d) => crate::trf!("The folder doesn't exist: {d}", d = d.display()),
            Self::Start(e) => e.to_string(),
            Self::Exit { code, message } if !message.is_empty() => match code {
                Some(c) => format!("{message} ({c})"),
                None => message.clone(),
            },
            Self::Exit { code: Some(c), .. } => crate::trf!("Exit code {c}", c = c),
            Self::Exit { code: None, .. } => tr("It was ended").to_owned(),
        }
    }
}

/// How `inv` starts. Through the shell when the command line needs it (see
/// [`line::needs_shell`]) or when its program isn't an `.exe` or `.com` (a
/// batch file like VS Code's `code.cmd`, a script opened by its file type);
/// otherwise the program is started directly. The program is looked for in
/// the working folder and then on `PATH`, with the extensions of `PATHEXT`;
/// if it isn't found that is an error (`Err` with its name), unless the line
/// goes through the shell anyway.
pub fn launch(inv: &Invocation) -> Result<Launch, String> {
    if inv.shell_syntax {
        return Ok(Launch::Shell);
    }
    let (program, _) = line::split_program(&inv.line);
    let path = std::env::var_os("PATH");
    let pathext = std::env::var("PATHEXT").ok();
    match find_program(&program, &inv.dir, path.as_deref(), pathext.as_deref()) {
        Some(exe) if is_binary(&exe) => Ok(Launch::Direct(exe)),
        Some(_) => Ok(Launch::Shell),
        None => Err(program),
    }
}

fn is_binary(path: &Path) -> bool {
    !cfg!(windows)
        || path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("com"))
}

fn is_file(path: &Path) -> bool {
    // `symlink_metadata`: the App Execution Aliases in WindowsApps (`wt`,
    // `python`, …) are links that can't be followed like files.
    std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_dir())
}

/// Where `program` is: a path (relative to `dir`), or a name looked for in
/// `dir` (on Windows, like cmd does) and the folders of `path_var`. Without
/// an extension, the ones of `pathext` are tried (on Windows).
pub fn find_program(
    program: &str,
    dir: &Path,
    path_var: Option<&OsStr>,
    pathext: Option<&str>,
) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let exts: Vec<String> = if cfg!(windows) {
        pathext
            .unwrap_or(".COM;.EXE;.BAT;.CMD")
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_lowercase)
            .collect()
    } else {
        Vec::new()
    };
    let candidate = |base: PathBuf| -> Option<PathBuf> {
        if (exts.is_empty() || base.extension().is_some()) && is_file(&base) {
            return Some(base);
        }
        exts.iter().find_map(|e| {
            let mut name = base.clone().into_os_string();
            name.push(e);
            let p = PathBuf::from(name);
            is_file(&p).then_some(p)
        })
    };
    if program.contains(['/', '\\']) || Path::new(program).is_absolute() {
        return candidate(dir.join(program));
    }
    let here = cfg!(windows).then(|| dir.to_path_buf());
    here.into_iter()
        .chain(path_var.map(std::env::split_paths).into_iter().flatten())
        .find_map(|d| candidate(d.join(program)))
}

/// The process for `inv`, not started yet.
pub fn command(inv: &Invocation, launch: &Launch, window: Window) -> Command {
    let mut cmd = match launch {
        Launch::Direct(exe) => {
            let mut c = Command::new(exe);
            let (_, rest) = line::split_program(&inv.line);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                if !rest.trim().is_empty() {
                    c.raw_arg(rest.trim_start());
                }
            }
            #[cfg(not(windows))]
            c.args(line::split_args(rest));
            c
        }
        Launch::Shell => {
            #[cfg(windows)]
            let c = {
                use std::os::windows::process::CommandExt;
                let mut c = Command::new("cmd");
                // /S: the line in the outer quotes is taken as it is.
                c.raw_arg(format!("/S /C \"{}\"", inv.line));
                c
            };
            #[cfg(not(windows))]
            let c = {
                let mut c = Command::new("sh");
                c.arg("-c").arg(&inv.line);
                c
            };
            c
        }
    };
    cmd.current_dir(&inv.dir)
        .envs(inv.env.iter().map(|(k, v)| (k, v)));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(match window {
            Window::Console => CREATE_NEW_CONSOLE,
            Window::Hidden | Window::Output => CREATE_NO_WINDOW,
        });
    }
    #[cfg(not(windows))]
    let _ = window;
    cmd
}

/// Run `inv` and wait until it ends. With [`Window::Output`], what it prints
/// (output and errors) is given to `on_line` line by line as it comes.
pub fn run(
    inv: &Invocation,
    window: Window,
    on_line: &mut dyn FnMut(String),
) -> Result<(), Failure> {
    if !inv.dir.is_dir() {
        return Err(Failure::NoFolder(inv.dir.clone()));
    }
    let launch = launch(inv).map_err(Failure::NotFound)?;
    let mut cmd = command(inv, &launch, window);
    let status = match window {
        Window::Console => cmd.status().map_err(Failure::Start)?,
        Window::Hidden => {
            let out = cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .map_err(Failure::Start)?;
            if !out.status.success() {
                return Err(Failure::Exit {
                    code: out.status.code(),
                    message: one_line(&decode(&out.stderr)),
                });
            }
            out.status
        }
        Window::Output => {
            let (mut reader, writer) = std::io::pipe().map_err(Failure::Start)?;
            let writer2 = writer.try_clone().map_err(Failure::Start)?;
            cmd.stdin(Stdio::null()).stdout(writer).stderr(writer2);
            let mut child = cmd.spawn().map_err(Failure::Start)?;
            // The command keeps the pipe's writing ends: without them gone
            // the reading would never see the end.
            drop(cmd);
            read_lines(&mut reader, on_line);
            child.wait().map_err(Failure::Start)?
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err(Failure::Exit {
            code: status.code(),
            message: String::new(),
        })
    }
}

/// Read `reader` to its end, line by line.
fn read_lines(reader: &mut impl Read, on_line: &mut dyn FnMut(String)) {
    let mut pending = Vec::new();
    let mut buf = [0u8; 8192];
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buf[..n]);
        while let Some(end) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            on_line(shown_line(&line[..end]));
        }
    }
    if !pending.is_empty() {
        on_line(shown_line(&pending));
    }
}

/// A printed line as a console shows it: a carriage return (progress
/// shown in place, like ffmpeg's) starts the line again.
fn shown_line(bytes: &[u8]) -> String {
    let text = decode(bytes);
    let text = text.strip_suffix('\r').unwrap_or(&text);
    text.rsplit('\r')
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// The last lines of an error output as one line, without the final full
/// stop (cmd's "… is not recognized …" message has both).
fn one_line(text: &str) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = words.trim_end_matches('.').to_owned();
    if s.chars().count() > 300 {
        s = s.chars().take(300).collect::<String>() + "…";
    }
    s
}

/// Text printed by a program: UTF-8, else (on Windows) the console's code
/// page, which cmd and older programs use.
pub fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => decode_oem(bytes),
    }
}

#[cfg(windows)]
fn decode_oem(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP};
    let Ok(len) = i32::try_from(bytes.len()) else {
        return String::from_utf8_lossy(bytes).into_owned();
    };
    // SAFETY: the buffers are valid for the lengths given.
    unsafe {
        let n = MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
        if n <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut wide = vec![0u16; n as usize];
        let n = MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), n);
        String::from_utf16_lossy(&wide[..n.max(0) as usize])
    }
}

#[cfg(not(windows))]
fn decode_oem(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CustomCommand, Many};

    #[test]
    fn programs_are_found_with_their_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::write(bin.join("tool.exe"), "").unwrap();
        std::fs::write(bin.join("code.cmd"), "").unwrap();
        std::fs::write(dir.path().join("local.bat"), "").unwrap();
        let path = std::env::join_paths([&bin]).unwrap();
        let find = |p: &str| find_program(p, dir.path(), Some(&path), Some(".COM;.EXE;.BAT;.CMD"));
        assert!(find("missing").is_none());
        assert!(find("").is_none());
        if cfg!(windows) {
            assert_eq!(find("tool"), Some(bin.join("tool.exe")));
            assert_eq!(find("TOOL.EXE"), Some(bin.join("TOOL.EXE")));
            assert_eq!(find("code"), Some(bin.join("code.cmd")));
            assert_eq!(find("local"), Some(dir.path().join("local.bat")));
            assert_eq!(find(r"bin\tool"), Some(dir.path().join(r"bin\tool.exe")));
            assert!(is_binary(&bin.join("tool.exe")) && !is_binary(&bin.join("code.cmd")));
        } else {
            assert_eq!(find("tool.exe"), Some(bin.join("tool.exe")));
            assert!(find("local.bat").is_none(), "not in the working folder");
        }
    }

    #[test]
    fn lines_as_a_console_shows_them() {
        let mut lines = Vec::new();
        let text = b"one\r\ntwo\nprogress 10%\rprogress 100%\r\nlast";
        read_lines(&mut &text[..], &mut |l| lines.push(l));
        assert_eq!(lines, ["one", "two", "progress 100%", "last"]);
        assert_eq!(decode("Grüße".as_bytes()), "Grüße");
        assert_eq!(
            one_line("'x' is not recognized as an internal\r\nor external command.\r\n"),
            "'x' is not recognized as an internal or external command"
        );
    }

    fn invocation(command: &str, dir: &Path, items: &[PathBuf]) -> Invocation {
        let c = CustomCommand {
            command: command.into(),
            many: Many::All,
            ..Default::default()
        };
        c.invocations(items, Some(dir)).remove(0)
    }

    #[test]
    fn missing_programs_and_folders_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let inv = invocation("no-such-program-ox {path}", dir.path(), &[]);
        assert!(matches!(
            run(&inv, Window::Hidden, &mut |_| {}),
            Err(Failure::NotFound(p)) if p == "no-such-program-ox"
        ));
        let gone = dir.path().join("gone");
        let inv = invocation("x", &gone, &[]);
        assert!(matches!(
            run(&inv, Window::Hidden, &mut |_| {}),
            Err(Failure::NoFolder(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn runs_through_the_shell_and_directly() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("A & B (x) ü");
        std::fs::create_dir(&sub).unwrap();
        let file = sub.join("my file.txt");
        std::fs::write(&file, "hi").unwrap();
        let items = [file.clone()];

        // The shell, with a path cmd would otherwise split at `&`.
        let inv = invocation("echo {path}& set OX_FOLDER", &sub, &items);
        assert_eq!(launch(&inv), Ok(Launch::Shell));
        let mut out = Vec::new();
        run(&inv, Window::Output, &mut |l| out.push(l)).unwrap();
        assert_eq!(out[0].trim(), format!("\"{}\"", file.display()));
        assert_eq!(out[1].trim(), format!("OX_FOLDER={}", sub.display()));

        // Directly: the arguments arrive as they were.
        let script = dir.path().join("args.ps1");
        std::fs::write(&script, "foreach ($a in $args) { \"<$a>\" }").unwrap();
        let line = format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File {} {{path}} {{name}}",
            line::quote(&script.to_string_lossy())
        );
        let inv = invocation(&line, &sub, &items);
        assert!(matches!(launch(&inv), Ok(Launch::Direct(_))));
        let mut out = Vec::new();
        run(&inv, Window::Output, &mut |l| out.push(l)).unwrap();
        assert_eq!(
            out,
            [format!("<{}>", file.display()), "<my file.txt>".to_owned()]
        );

        // A failing command reports its exit code and error.
        let inv = invocation("cmd /C exit 3", &sub, &items);
        match run(&inv, Window::Hidden, &mut |_| {}) {
            Err(Failure::Exit { code: Some(3), .. }) => {}
            other => panic!("{other:?}"),
        }
        let inv = invocation("dir {path}\\nothing-here", &sub, &items);
        match run(&inv, Window::Hidden, &mut |_| {}) {
            Err(Failure::Exit { message, .. }) => assert!(!message.is_empty()),
            other => panic!("{other:?}"),
        }
    }
}
