//! `oxplorer-indexer.exe`: the Oxplorer index service.
//!
//! Runs as a LocalSystem service (started on demand by the app, stopped
//! again after a while without connections) and lets one user read the
//! NTFS master file tables and change journals through a named pipe, so
//! Oxplorer's index doesn't need administrator rights. The protocol, the
//! request handling and the security model are in `ox_core::index::helper`;
//! this binary only hosts them: the service control glue, the pipe server,
//! and dropping the privileges it doesn't need.
//!
//! ```text
//! oxplorer-indexer                        run as the service (started by Windows)
//! oxplorer-indexer --console [SID]        run in this console for debugging (as
//!                                         administrator; allows SID or yourself)
//! oxplorer-indexer --session-user-sid F   write the SID of the user logged on to
//!                                         this session to file F (for the installer)
//! ```

#[cfg(windows)]
mod account;
#[cfg(windows)]
mod privileges;
#[cfg(windows)]
mod server;
#[cfg(windows)]
mod service;

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
enum Mode {
    Service,
    Console { allow: Option<String> },
    SessionUserSid { out: String },
}

fn parse_args(args: &[String]) -> Option<Mode> {
    match args {
        [] => Some(Mode::Service),
        [c] if c == "--console" => Some(Mode::Console { allow: None }),
        [c, sid] if c == "--console" => Some(Mode::Console {
            allow: Some(sid.clone()),
        }),
        [c, out] if c == "--session-user-sid" => Some(Mode::SessionUserSid { out: out.clone() }),
        _ => None,
    }
}

const USAGE: &str = "usage: oxplorer-indexer [--console [SID] | --session-user-sid FILE]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(mode) = parse_args(&args) else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    #[cfg(windows)]
    std::process::exit(run(mode));
    #[cfg(not(windows))]
    {
        let _ = mode;
        eprintln!("oxplorer-indexer only runs on Windows.");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn run(mode: Mode) -> i32 {
    use ox_core::index::helper::security::{is_user_sid, pipe_sddl};

    server::harden();
    match mode {
        Mode::Service => service::run_dispatcher(),
        Mode::Console { allow } => {
            let Some(user) = allow.or_else(account::current_user_sid) else {
                eprintln!("Couldn't tell who you are.");
                return 1;
            };
            if !is_user_sid(&user) || !account::is_user_account(&user) {
                eprintln!("{user} isn't a user account.");
                return 1;
            }
            if !privileges::drop_privileges() {
                eprintln!("Couldn't drop privileges.");
                return 1;
            }
            let Some(sddl) = pipe_sddl(&user, "BA") else {
                return 1;
            };
            eprintln!(
                "Serving {} for {user} (Ctrl+C stops).",
                ox_core::index::helper::PIPE_NAME
            );
            match server::run(&sddl, None, true) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("The pipe server failed: {e}");
                    1
                }
            }
        }
        Mode::SessionUserSid { out } => match account::session_user_sid() {
            Some(sid) if is_user_sid(&sid) => match std::fs::write(&out, sid) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("Couldn't write {out}: {e}");
                    1
                }
            },
            other => {
                eprintln!("No user account logged on to this session ({other:?}).");
                1
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_command_line() {
        assert_eq!(parse_args(&[]), Some(Mode::Service));
        assert_eq!(
            parse_args(&args(&["--console"])),
            Some(Mode::Console { allow: None })
        );
        assert_eq!(
            parse_args(&args(&["--console", "S-1-5-21-1-2-3-1001"])),
            Some(Mode::Console {
                allow: Some("S-1-5-21-1-2-3-1001".into())
            })
        );
        assert_eq!(
            parse_args(&args(&["--session-user-sid", "x.txt"])),
            Some(Mode::SessionUserSid {
                out: "x.txt".into()
            })
        );
        assert_eq!(parse_args(&args(&["--install"])), None);
        assert_eq!(parse_args(&args(&["--console", "a", "b"])), None);
        assert_eq!(parse_args(&args(&["--session-user-sid"])), None);
    }
}
