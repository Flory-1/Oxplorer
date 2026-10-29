//! WSL: the Linux distributions of the Windows Subsystem for Linux, their
//! folders (`\\wsl.localhost\<name>\…`, or `\\wsl$\<name>\…` on older
//! systems) and Linux ↔ Windows paths.
//!
//! The distributions are read from the registry, which is fast and doesn't
//! start WSL. Only `wsl.exe --list --running` is asked which ones run; it
//! never starts a stopped one. Reading a distribution's folders starts it,
//! so nothing here touches them unless the user opens one.

use std::io;
use std::path::{Path, PathBuf};

/// The registry key with one `{GUID}` sub-key per distribution.
#[cfg_attr(not(windows), allow(dead_code))]
const LXSS: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";

/// An installed distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distro {
    pub name: String,
    /// 1 or 2.
    pub version: u8,
    /// The distribution `wsl` starts without `-d`.
    pub default: bool,
    /// Where its files are stored on Windows (`ext4.vhdx` for WSL 2).
    pub base_path: Option<PathBuf>,
    /// `None` if that couldn't be found out.
    pub running: Option<bool>,
    /// `\\wsl.localhost\<name>\`
    pub root: PathBuf,
}

/// What the distributions are read from: the `Lxss` key and its sub-keys
/// (an abstraction so the parsing is tested without a registry).
pub trait Lxss {
    /// The names of the sub-keys (one `{GUID}` per distribution).
    fn keys(&self) -> Vec<String>;
    /// A string value of sub-key `key` (`""` = the `Lxss` key itself).
    fn string(&self, key: &str, name: &str) -> Option<String>;
    /// A DWORD value of sub-key `key`.
    fn dword(&self, key: &str, name: &str) -> Option<u32>;
}

/// The distributions in `reg`, the default one first, then by name. Their
/// roots are on `host` (`wsl.localhost` or `wsl$`).
pub fn read(reg: &impl Lxss, host: &str) -> Vec<Distro> {
    let default = reg.string("", "DefaultDistribution").unwrap_or_default();
    let mut out: Vec<Distro> = reg
        .keys()
        .into_iter()
        .filter_map(|key| {
            let name = reg.string(&key, "DistributionName")?.trim().to_owned();
            if name.is_empty() {
                return None;
            }
            // 3 = being installed, 4 = being removed.
            if matches!(reg.dword(&key, "State"), Some(3 | 4)) {
                return None;
            }
            // Older installs only have the WSL 2 flag (0x8) in `Flags`.
            let version = match reg.dword(&key, "Version") {
                Some(v @ 1..=2) => v as u8,
                _ if reg.dword(&key, "Flags").is_some_and(|f| f & 0x8 != 0) => 2,
                _ => 1,
            };
            Some(Distro {
                root: root(&name, host),
                default: !default.is_empty() && key.eq_ignore_ascii_case(&default),
                base_path: reg
                    .string(&key, "BasePath")
                    .filter(|p| !p.is_empty())
                    .map(|p| PathBuf::from(p.trim_start_matches(r"\\?\"))),
                running: None,
                version,
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.default
            .cmp(&a.default)
            .then_with(|| crate::natural::natural_cmp(&a.name, &b.name))
    });
    out
}

/// The installed distributions, with whether they run (empty without WSL).
/// Takes up to a few seconds (it asks `wsl.exe`): call it off the UI thread.
pub fn distros() -> Vec<Distro> {
    #[cfg(windows)]
    {
        let mut list = read(&imp::Registry, host());
        if !list.is_empty() {
            let running = running();
            for d in &mut list {
                d.running = running
                    .as_ref()
                    .map(|r| r.iter().any(|n| n.eq_ignore_ascii_case(&d.name)));
            }
        }
        list
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// `\\<host>\<name>\`
pub fn root(name: &str, host: &str) -> PathBuf {
    PathBuf::from(format!(r"\\{host}\{name}\"))
}

/// The host the distributions are reached on: `wsl.localhost` (Windows 11
/// and the Store version of WSL), else `wsl$`. Checked without touching
/// the network share, which could start WSL.
pub fn host() -> &'static str {
    static HOST: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        #[cfg(windows)]
        {
            if prefer_localhost(imp::build_number(), imp::store_wsl()) {
                "wsl.localhost"
            } else {
                "wsl$"
            }
        }
        #[cfg(not(windows))]
        {
            "wsl.localhost"
        }
    })
}

/// `\\wsl.localhost` came with Windows 11 (build 22000) and the Store WSL.
fn prefer_localhost(build: Option<u32>, store_wsl: bool) -> bool {
    store_wsl || build.is_some_and(|b| b >= 22000)
}

/// Whether `server` is a WSL host (`wsl$` or `wsl.localhost`).
pub fn is_wsl_host(server: &str) -> bool {
    server.eq_ignore_ascii_case("wsl$") || server.eq_ignore_ascii_case("wsl.localhost")
}

/// A path inside a distribution, split up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WslPath<'a> {
    pub host: &'a str,
    pub distro: &'a str,
    /// After the distribution, without the leading separator (`home\me`).
    pub rest: &'a str,
}

/// `\\wsl.localhost\Ubuntu\home\me` → host, `Ubuntu` and `home\me`
/// (`\\wsl$\…`, `\\?\UNC\wsl$\…` and forward slashes too).
pub fn parse(path: &Path) -> Option<WslPath<'_>> {
    let s = path.to_str()?;
    let unc = s
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| s.strip_prefix(r"\\"))
        .or_else(|| s.strip_prefix("//"))?;
    let (host, after) = unc.split_once(['\\', '/'])?;
    if !is_wsl_host(host) {
        return None;
    }
    let (distro, rest) = after.split_once(['\\', '/']).unwrap_or((after, ""));
    (!distro.is_empty()).then(|| WslPath {
        host,
        distro,
        rest: rest.trim_matches(['\\', '/']),
    })
}

/// Whether `path` is inside a WSL distribution.
pub fn is_wsl_path(path: &Path) -> bool {
    parse(path).is_some()
}

/// The distribution `path` is in.
pub fn distro_of(path: &Path) -> Option<&str> {
    parse(path).map(|p| p.distro)
}

/// The distribution's name if `path` is its root (`\\wsl$\Ubuntu\`).
pub fn root_name(path: &Path) -> Option<&str> {
    parse(path).filter(|p| p.rest.is_empty()).map(|p| p.distro)
}

/// Dotfiles count as hidden inside a distribution, like on Linux.
pub fn dotfile_hidden(path: &Path, name: &str) -> bool {
    name.starts_with('.') && is_wsl_path(path)
}

/// The path as Linux sees it: `\\wsl.localhost\Ubuntu\home\me` → `/home/me`,
/// `C:\Users\me` → `/mnt/c/Users/me`. `None` for other paths (shares).
pub fn to_linux(path: &Path) -> Option<String> {
    if let Some(p) = parse(path) {
        return Some(format!("/{}", p.rest.replace('\\', "/")));
    }
    let s = path.to_str()?;
    let s = s.strip_prefix(r"\\?\").unwrap_or(s);
    let mut chars = s.chars();
    let (letter, colon) = (chars.next()?, chars.next()?);
    if !letter.is_ascii_alphabetic() || colon != ':' {
        return None;
    }
    let rest = s[2..].trim_matches(['\\', '/']).replace('\\', "/");
    let drive = letter.to_ascii_lowercase();
    Some(if rest.is_empty() {
        format!("/mnt/{drive}")
    } else {
        format!("/mnt/{drive}/{rest}")
    })
}

/// A Linux path of `distro` as a Windows path: `/mnt/c/x` → `C:\x`,
/// `/home/me` → `\\<host>\<distro>\home\me`.
pub fn to_windows(linux: &str, distro: &str, host: &str) -> PathBuf {
    let rel = linux.trim_matches('/');
    if let Some(after) = rel.strip_prefix("mnt/") {
        let (drive, rest) = after.split_once('/').unwrap_or((after, ""));
        if drive.len() == 1 && drive.chars().all(|c| c.is_ascii_alphabetic()) {
            let drive = drive.to_ascii_uppercase();
            return PathBuf::from(format!(r"{drive}:\{}", rest.replace('/', "\\")));
        }
    }
    let mut out = root(distro, host).into_os_string();
    out.push(rel.replace('/', "\\"));
    PathBuf::from(out)
}

/// Open a terminal with a shell of the distribution `dir` is in, in `dir`
/// (Windows Terminal if installed, otherwise a console window).
pub fn open_terminal(dir: &Path) -> io::Result<()> {
    let (Some(distro), Some(linux)) = (distro_of(dir), to_linux(dir)) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a folder inside a WSL distribution",
        ));
    };
    #[cfg(windows)]
    {
        imp::open_terminal(distro, &linux)
    }
    #[cfg(not(windows))]
    {
        let _ = (distro, linux);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "WSL is only available on Windows",
        ))
    }
}

/// The names `wsl.exe --list --running --quiet` printed: UTF-16LE (with or
/// without BOM) as `wsl.exe` writes to pipes, or UTF-8 with `WSL_UTF8=1`.
pub fn parse_names(out: &[u8]) -> Vec<String> {
    let utf16 = out.starts_with(&[0xff, 0xfe])
        || (out.len() >= 2
            && out.len().is_multiple_of(2)
            && out.iter().skip(1).step_by(2).all(|&b| b == 0));
    let text = if utf16 {
        let units: Vec<u16> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(out).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0'))
        // `-l -v` marks the default with `*`; names never contain spaces.
        .map(|l| l.trim_start_matches('*').trim())
        .filter(|l| !l.is_empty() && !l.contains(' '))
        .map(str::to_owned)
        .collect()
}

/// The running distributions, `None` if `wsl.exe` didn't answer in time.
#[cfg(windows)]
fn running() -> Option<Vec<String>> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = Command::new("wsl.exe")
        .args(["--list", "--running", "--quiet"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    // Read on a thread: a full pipe would keep `wsl.exe` from exiting.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = io::Read::read_to_end(&mut stdout, &mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = reader.join().ok()?;
    // Older versions fail with a message when none runs.
    Some(if status.success() {
        parse_names(&out)
    } else {
        Vec::new()
    })
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
        HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
    };

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }

    fn sub_key(key: &str) -> String {
        if key.is_empty() {
            super::LXSS.to_owned()
        } else {
            format!(r"{}\{key}", super::LXSS)
        }
    }

    fn get_string(root: HKEY, key: &str, name: &str) -> Option<String> {
        let (key, name) = (wide(key), wide(name));
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: `buf` is `size` bytes; the strings are null-terminated.
        let err = unsafe {
            RegGetValueW(
                root,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if !err.is_ok() {
            return None;
        }
        let len = (size as usize / 2).min(buf.len());
        Some(
            String::from_utf16_lossy(&buf[..len])
                .trim_end_matches('\0')
                .to_owned(),
        )
    }

    /// `HKEY_CURRENT_USER\…\Lxss`.
    pub(super) struct Registry;

    impl super::Lxss for Registry {
        fn keys(&self) -> Vec<String> {
            let path = wide(super::LXSS);
            let mut hkey = HKEY::default();
            // SAFETY: the key is closed below; buffers outlive the calls.
            unsafe {
                if !RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(path.as_ptr()),
                    None,
                    KEY_READ,
                    &mut hkey,
                )
                .is_ok()
                {
                    return Vec::new();
                }
                let mut out = Vec::new();
                for i in 0.. {
                    let mut buf = [0u16; 256];
                    let mut len = buf.len() as u32;
                    let err = RegEnumKeyExW(
                        hkey,
                        i,
                        Some(PWSTR(buf.as_mut_ptr())),
                        &mut len,
                        None,
                        None,
                        None,
                        None,
                    );
                    if !err.is_ok() {
                        break;
                    }
                    out.push(String::from_utf16_lossy(&buf[..len as usize]));
                }
                let _ = RegCloseKey(hkey);
                out
            }
        }

        fn string(&self, key: &str, name: &str) -> Option<String> {
            get_string(HKEY_CURRENT_USER, &sub_key(key), name)
        }

        fn dword(&self, key: &str, name: &str) -> Option<u32> {
            let (key, name) = (wide(sub_key(key)), wide(name));
            let mut value = 0u32;
            let mut size = 4u32;
            // SAFETY: `value` is 4 bytes; the strings are null-terminated.
            let err = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    PCWSTR(key.as_ptr()),
                    PCWSTR(name.as_ptr()),
                    RRF_RT_REG_DWORD,
                    None,
                    Some((&mut value as *mut u32).cast()),
                    Some(&mut size),
                )
            };
            err.is_ok().then_some(value)
        }
    }

    /// The Windows build (`CurrentBuildNumber`).
    pub(super) fn build_number() -> Option<u32> {
        get_string(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "CurrentBuildNumber",
        )?
        .trim()
        .parse()
        .ok()
    }

    /// Whether the Store version of WSL is installed (it has `\\wsl.localhost`
    /// on Windows 10 too).
    pub(super) fn store_wsl() -> bool {
        std::env::var_os("ProgramFiles")
            .is_some_and(|p| std::path::Path::new(&p).join(r"WSL\wsl.exe").is_file())
    }

    pub(super) fn open_terminal(distro: &str, linux: &str) -> io::Result<()> {
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        // Long options: Windows Terminal has a `-d` of its own. `;` would
        // separate its commands, so such paths go to a console window.
        let wt = (!linux.contains(';'))
            .then(|| {
                Command::new("wt")
                    .args(["new-tab", "--title", distro, "wsl.exe"])
                    .args(["--distribution", distro, "--cd", linux])
                    .spawn()
                    .ok()
            })
            .flatten();
        if wt.is_some() {
            return Ok(());
        }
        Command::new("wsl.exe")
            .args(["--distribution", distro, "--cd", linux])
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A registry in memory: `(key, name)` → value.
    #[derive(Default)]
    struct FakeLxss {
        keys: Vec<String>,
        strings: HashMap<(String, String), String>,
        dwords: HashMap<(String, String), u32>,
    }

    impl FakeLxss {
        fn distro(&mut self, key: &str, name: &str, version: Option<u32>, state: u32) {
            self.keys.push(key.into());
            self.strings
                .insert((key.into(), "DistributionName".into()), name.into());
            self.strings
                .insert((key.into(), "BasePath".into()), format!(r"C:\wsl\{name}"));
            if let Some(v) = version {
                self.dwords.insert((key.into(), "Version".into()), v);
            }
            self.dwords.insert((key.into(), "State".into()), state);
        }
    }

    impl Lxss for FakeLxss {
        fn keys(&self) -> Vec<String> {
            self.keys.clone()
        }
        fn string(&self, key: &str, name: &str) -> Option<String> {
            self.strings.get(&(key.into(), name.into())).cloned()
        }
        fn dword(&self, key: &str, name: &str) -> Option<u32> {
            self.dwords.get(&(key.into(), name.into())).copied()
        }
    }

    #[test]
    fn reads_distributions_default_first() {
        let mut reg = FakeLxss::default();
        reg.distro("{a}", "alpine", Some(2), 1);
        reg.distro("{B}", "Ubuntu", Some(1), 1);
        reg.distro("{c}", "Debian", None, 1);
        reg.dwords.insert(("{c}".into(), "Flags".into()), 0xf);
        reg.distro("{d}", "Installing", Some(2), 3);
        reg.strings
            .insert(("".into(), "DefaultDistribution".into()), "{b}".into());
        let list = read(&reg, "wsl.localhost");
        let names: Vec<_> = list.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Ubuntu", "alpine", "Debian"]);
        assert!(list[0].default && !list[1].default);
        assert_eq!(list[0].version, 1);
        assert_eq!(list[1].version, 2);
        // No `Version`: the WSL 2 flag decides.
        assert_eq!(list[2].version, 2);
        assert_eq!(list[0].root, PathBuf::from(r"\\wsl.localhost\Ubuntu\"));
        assert_eq!(list[0].base_path, Some(PathBuf::from(r"C:\wsl\Ubuntu")));
        assert_eq!(list[0].running, None);
    }

    #[test]
    fn no_wsl_no_distributions() {
        assert!(read(&FakeLxss::default(), "wsl$").is_empty());
    }

    #[test]
    fn host_choice() {
        assert!(prefer_localhost(Some(22631), false));
        assert!(prefer_localhost(Some(19045), true));
        assert!(!prefer_localhost(Some(19045), false));
        assert!(!prefer_localhost(None, false));
    }

    fn utf16(s: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xff, 0xfe] } else { Vec::new() };
        out.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        out
    }

    #[test]
    fn parses_wsl_output() {
        let quiet = "Ubuntu\r\nlisaops-engine\r\n";
        let want = ["Ubuntu", "lisaops-engine"];
        assert_eq!(parse_names(&utf16(quiet, false)), want);
        assert_eq!(parse_names(&utf16(quiet, true)), want);
        assert_eq!(parse_names(quiet.as_bytes()), want);
        assert!(parse_names(b"").is_empty());
        assert!(parse_names(&utf16("\r\n", false)).is_empty());
        // As `wsl.exe` wrote it on a real system.
        let real = [
            85, 0, 98, 0, 117, 0, 110, 0, 116, 0, 117, 0, 13, 0, 10, 0, 97, 0, 108, 0, 112, 0, 105,
            0, 110, 0, 101, 0, 13, 0, 10, 0,
        ];
        assert_eq!(parse_names(&real), ["Ubuntu", "alpine"]);
        // Messages (they have spaces) aren't names; a `*` marks the default.
        let listed = "There are no running distributions.\r\n* Debian\r\n";
        assert_eq!(parse_names(&utf16(listed, false)), ["Debian"]);
    }

    #[test]
    fn splits_wsl_paths() {
        let p = parse(Path::new(r"\\wsl.localhost\Ubuntu\home\me")).unwrap();
        assert_eq!(
            (p.host, p.distro, p.rest),
            ("wsl.localhost", "Ubuntu", r"home\me")
        );
        assert_eq!(distro_of(Path::new(r"\\WSL$\Debian\")), Some("Debian"));
        assert_eq!(
            distro_of(Path::new(r"\\?\UNC\wsl$\Debian\etc")),
            Some("Debian")
        );
        assert_eq!(
            distro_of(Path::new("//wsl.localhost/Ubuntu/etc")),
            Some("Ubuntu")
        );
        assert_eq!(root_name(Path::new(r"\\wsl$\Ubuntu")), Some("Ubuntu"));
        assert_eq!(root_name(Path::new(r"\\wsl$\Ubuntu\")), Some("Ubuntu"));
        assert_eq!(root_name(Path::new(r"\\wsl$\Ubuntu\home")), None);
        assert!(!is_wsl_path(Path::new(r"\\wsl$")));
        assert!(!is_wsl_path(Path::new(r"\\wsl.localhost\")));
        assert!(!is_wsl_path(Path::new(r"\\nas\share")));
        assert!(!is_wsl_path(Path::new(r"C:\wsl$\x")));
        assert!(dotfile_hidden(
            Path::new(r"\\wsl$\Ubuntu\home\me\.bashrc"),
            ".bashrc"
        ));
        assert!(!dotfile_hidden(Path::new(r"C:\me\.bashrc"), ".bashrc"));
    }

    #[test]
    fn windows_to_linux() {
        let l = |p: &str| to_linux(Path::new(p));
        assert_eq!(
            l(r"\\wsl.localhost\Ubuntu\home\me").as_deref(),
            Some("/home/me")
        );
        assert_eq!(l(r"\\wsl$\Ubuntu\home\me\").as_deref(), Some("/home/me"));
        assert_eq!(l(r"\\wsl$\Ubuntu\").as_deref(), Some("/"));
        assert_eq!(l(r"\\wsl$\Ubuntu").as_deref(), Some("/"));
        assert_eq!(
            l(r"C:\Users\me\a b.txt").as_deref(),
            Some("/mnt/c/Users/me/a b.txt")
        );
        assert_eq!(l(r"D:\").as_deref(), Some("/mnt/d"));
        assert_eq!(l("e:").as_deref(), Some("/mnt/e"));
        assert_eq!(l(r"\\?\C:\x").as_deref(), Some("/mnt/c/x"));
        assert_eq!(l(r"\\nas\share\x"), None);
        assert_eq!(l("relative"), None);
    }

    #[test]
    fn linux_to_windows() {
        let w = |p: &str| to_windows(p, "Ubuntu", "wsl.localhost");
        assert_eq!(
            w("/home/me"),
            PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me")
        );
        assert_eq!(w("/"), PathBuf::from(r"\\wsl.localhost\Ubuntu\"));
        assert_eq!(w("/mnt/c/Users/me"), PathBuf::from(r"C:\Users\me"));
        assert_eq!(w("/mnt/d"), PathBuf::from(r"D:\"));
        assert_eq!(
            w("/mnt/wsl/x"),
            PathBuf::from(r"\\wsl.localhost\Ubuntu\mnt\wsl\x")
        );
        // Round trip.
        let p = Path::new(r"\\wsl.localhost\Ubuntu\etc\apt");
        assert_eq!(w(&to_linux(p).unwrap()), p);
    }

    #[test]
    fn terminal_needs_a_wsl_folder() {
        assert!(open_terminal(Path::new(r"C:\x")).is_err());
    }
}
