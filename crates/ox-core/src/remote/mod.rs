//! Remote locations: SFTP, FTP/FTPS and WebDAV servers, browsed like
//! folders and copied through the transfer engine.
//!
//! Every protocol implements [`RemoteFs`], a small blocking file system
//! API on remote paths (`/home/me/file.txt`). [`Connections`] keeps one
//! connection (or pool) per server login, reconnects when it dropped and
//! knows the passwords of this session. All calls block, so they run on
//! background threads, and each network step has a timeout.

pub mod checksum;
pub mod credentials;
pub mod edits;
mod ftp;
mod ftp_list;
pub mod known_hosts;
pub mod mtp;
mod sftp;
pub mod thumbs;
mod tls;
pub mod url;
mod webdav;
mod webdav_auth;
mod webdav_xml;

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::entry::FileEntry;
use crate::i18n::t;
pub use url::{is_remote_path, Protocol, RemoteAddr};

/// How long one network step (connecting, a command, a block of data) may take.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// A file or folder in a remote listing.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Unix permission bits, where the server tells them.
    pub permissions: Option<u32>,
}

impl RemoteEntry {
    pub fn file(name: &str, size: u64, modified: Option<SystemTime>) -> Self {
        Self {
            name: name.to_owned(),
            is_dir: false,
            is_symlink: false,
            size,
            modified,
            permissions: None,
        }
    }

    pub fn dir(name: &str, modified: Option<SystemTime>) -> Self {
        Self {
            is_dir: true,
            ..Self::file(name, 0, modified)
        }
    }

    /// The entry as the file view shows it; `dir` is the folder it was listed in.
    pub fn to_file_entry(&self, dir: &RemoteAddr) -> FileEntry {
        FileEntry {
            path: dir.join(&self.name).to_path(),
            name: self.name.clone(),
            is_dir: self.is_dir,
            is_symlink: self.is_symlink,
            size: if self.is_dir { 0 } else { self.size },
            modified: self.modified,
            created: None,
            hidden: self.name.starts_with('.'),
            // No write permission for the owner.
            readonly: self.permissions.is_some_and(|p| p & 0o200 == 0),
            cloud: None,
        }
    }
}

/// An SSH host key that isn't trusted yet: the user decides.
#[derive(Debug, Clone, PartialEq)]
pub struct HostKeyQuestion {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    /// The key (base64), saved when the user trusts it.
    pub key: String,
    pub fingerprint: String,
    /// The server had another key before (or it is revoked): a warning.
    pub changed: bool,
}

impl HostKeyQuestion {
    /// Trust the key from now on (saved in Oxplorer's `known_hosts`).
    pub fn trust(&self) -> std::io::Result<()> {
        known_hosts::trust(&self.host, self.port, &self.key_type, &self.key)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RemoteError {
    /// A password (or the passphrase of a key) is needed. `rejected`: the
    /// one given was wrong.
    NeedPassword {
        rejected: bool,
    },
    /// The server's host key isn't trusted yet or has changed.
    HostKey(Box<HostKeyQuestion>),
    NotFound,
    Cancelled,
    Other(String),
}

impl RemoteError {
    pub fn other(e: impl fmt::Display) -> Self {
        Self::Other(e.to_string())
    }

    /// Needs an answer from the user before connecting again.
    pub fn needs_user(&self) -> bool {
        matches!(self, Self::NeedPassword { .. } | Self::HostKey(_))
    }
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedPassword { rejected: false } => f.write_str(t("A password is needed.")),
            Self::NeedPassword { rejected: true } => {
                f.write_str(t("The user name or password is wrong."))
            }
            Self::HostKey(q) if q.changed => {
                f.write_str(t("The server's identity has changed. Not connected."))
            }
            Self::HostKey(_) => f.write_str(t("The server's identity isn't confirmed yet.")),
            Self::NotFound => f.write_str(t("The file no longer exists.")),
            Self::Cancelled => f.write_str(t("Cancelled")),
            Self::Other(msg) => f.write_str(msg),
        }
    }
}

impl From<std::io::Error> for RemoteError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound,
            std::io::ErrorKind::Interrupted => Self::Cancelled,
            _ => Self::Other(e.to_string()),
        }
    }
}

pub type RemoteResult<T> = Result<T, RemoteError>;

/// A remote file system (one server login). Paths are absolute, with `/`.
pub trait RemoteFs: Send + Sync {
    /// The folder a login starts in.
    fn home(&self) -> RemoteResult<String>;
    fn list(&self, dir: &str) -> RemoteResult<Vec<RemoteEntry>>;
    /// `Ok(None)` when there is nothing at `path`.
    fn stat(&self, path: &str) -> RemoteResult<Option<RemoteEntry>> {
        let parent = url::parent_path(path).unwrap_or_else(|| "/".into());
        let name = url::file_name(path);
        Ok(self.list(&parent)?.into_iter().find(|e| e.name == name))
    }
    /// Download the file into `out`; returns the bytes written. An error
    /// from `out` stops the download (that is how it is cancelled).
    fn read_to(&self, path: &str, out: &mut dyn Write) -> RemoteResult<u64>;
    /// Upload `size` bytes from `src` to `path` (replacing a file there).
    fn write_from(&self, path: &str, src: &mut dyn Read, size: u64) -> RemoteResult<()>;
    /// Download from byte `offset` on (to continue a download); returns
    /// the bytes written to `out`. By default the file is read from the
    /// start and the first `offset` bytes are skipped.
    fn read_from(&self, path: &str, offset: u64, out: &mut dyn Write) -> RemoteResult<u64> {
        read_skipping(&|out| self.read_to(path, out), offset, out)
    }
    /// Uploads can be continued: [`RemoteFs::append_from`] works.
    fn can_append(&self) -> bool {
        false
    }
    /// Continue an upload: write the `size` bytes of `src` to `path` from
    /// byte `offset` on, keeping the bytes before.
    fn append_from(
        &self,
        _path: &str,
        _offset: u64,
        _src: &mut dyn Read,
        _size: u64,
    ) -> RemoteResult<()> {
        Err(RemoteError::Other("append isn't supported".into()))
    }
    /// The checksum the server computes for a file, where the protocol
    /// offers one (`Ok(None)`: it doesn't, or not for this file).
    fn checksum(&self, _path: &str) -> RemoteResult<Option<checksum::Checksum>> {
        Ok(None)
    }
    fn mkdir(&self, path: &str) -> RemoteResult<()>;
    fn rename(&self, from: &str, to: &str) -> RemoteResult<()>;
    fn remove_file(&self, path: &str) -> RemoteResult<()>;
    /// Remove an empty folder.
    fn remove_dir(&self, path: &str) -> RemoteResult<()>;
    /// Give a file its date, where the protocol can.
    fn set_modified(&self, _path: &str, _time: SystemTime) -> RemoteResult<()> {
        Ok(())
    }
    /// Uploads go to a temporary name and are renamed when complete (a
    /// broken upload never looks like a finished file).
    fn upload_via_temp(&self) -> bool {
        true
    }
    /// Files a transfer may move at the same time.
    fn parallel(&self) -> usize {
        2
    }
    /// Close the connection(s).
    fn close(&self) {}
}

/// Download from the start with `read`, passing on what follows `offset`.
fn read_skipping(
    read: &dyn Fn(&mut dyn Write) -> RemoteResult<u64>,
    offset: u64,
    out: &mut dyn Write,
) -> RemoteResult<u64> {
    if offset == 0 {
        return read(out);
    }
    let mut skip = Skip {
        inner: out,
        left: offset,
        written: 0,
    };
    read(&mut skip)?;
    Ok(skip.written)
}

/// Drops the first `left` bytes, passes the rest on.
struct Skip<'a> {
    inner: &'a mut dyn Write,
    left: u64,
    written: u64,
}

impl Write for Skip<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let skip = (self.left.min(data.len() as u64)) as usize;
        self.left -= skip as u64;
        if skip < data.len() {
            self.inner.write_all(&data[skip..])?;
            self.written += (data.len() - skip) as u64;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Remove a file or a folder with everything in it.
pub fn remove_all(fs: &dyn RemoteFs, path: &str, is_dir: bool) -> RemoteResult<()> {
    if !is_dir {
        return fs.remove_file(path);
    }
    for entry in fs.list(path)? {
        let child = url::join_path(path, &entry.name);
        // A link to a folder is removed, not what it points to.
        remove_all(fs, &child, entry.is_dir && !entry.is_symlink)?;
    }
    fs.remove_dir(path)
}

/// How to log in to an SFTP server (FTP and WebDAV always use a password).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AuthMethod {
    /// SSH agent (OpenSSH or Pageant), then the keys in `~/.ssh`, then a password.
    #[default]
    Auto,
    Password,
    /// This private key (its passphrase is asked like a password).
    KeyFile(PathBuf),
}

/// A saved remote place (the sidebar's "Remote locations"). Passwords are
/// never part of it (see [`credentials`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemotePlace {
    pub name: String,
    pub addr: RemoteAddr,
    #[serde(default)]
    pub auth: AuthMethod,
    /// The password is saved in the Windows Credential Manager.
    #[serde(default)]
    pub remember_password: bool,
}

impl RemotePlace {
    pub fn title(&self) -> String {
        if self.name.trim().is_empty() {
            self.addr.title()
        } else {
            self.name.clone()
        }
    }
}

/// The saved remote places (in the settings, as `[[remote]]` tables).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct RemotePlaces(pub Vec<RemotePlace>);

impl RemotePlaces {
    /// The saved place for the server of `addr`.
    pub fn for_addr(&self, addr: &RemoteAddr) -> Option<&RemotePlace> {
        self.0.iter().find(|p| p.addr.same_server(addr))
    }
}

/// Everything a connection needs besides the address.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    pub auth: AuthMethod,
}

impl ConnectOptions {
    pub fn for_addr(places: &RemotePlaces, addr: &RemoteAddr) -> Self {
        match places.for_addr(addr) {
            Some(p) => Self {
                auth: p.auth.clone(),
            },
            None => Self::default(),
        }
    }
}

/// What a backend needs to (re)connect.
#[derive(Clone)]
pub(crate) struct Login {
    pub addr: RemoteAddr,
    pub password: Option<String>,
    pub auth: AuthMethod,
}

fn connect(login: Login) -> RemoteResult<Arc<dyn RemoteFs>> {
    Ok(match login.addr.protocol {
        Protocol::Sftp => Arc::new(sftp::SftpFs::connect(login)?),
        Protocol::Ftp | Protocol::Ftps => Arc::new(ftp::FtpFs::connect(login)?),
        Protocol::WebDav | Protocol::WebDavs => Arc::new(webdav::DavFs::connect(login)?),
        Protocol::Mtp => mtp::connect(&login.addr)?,
    })
}

/// The open connections and this session's passwords, by server.
#[derive(Default)]
pub struct Connections {
    open: Mutex<HashMap<String, Arc<dyn RemoteFs>>>,
    passwords: Mutex<HashMap<String, String>>,
}

impl Connections {
    /// The connection to the server of `addr`, connecting if needed.
    pub fn get(&self, addr: &RemoteAddr, opts: &ConnectOptions) -> RemoteResult<Arc<dyn RemoteFs>> {
        let key = addr.server_key();
        if let Some(fs) = self.open.lock().unwrap().get(&key) {
            return Ok(fs.clone());
        }
        let mut password = self.passwords.lock().unwrap().get(&key).cloned();
        if password.is_none() {
            // Saved only when the user asked to remember it.
            password = credentials::load(addr);
        }
        let had_password = password.is_some();
        let login = Login {
            // WebDAV checks the login at this folder (its root may be closed).
            addr: addr.clone(),
            password,
            auth: opts.auth.clone(),
        };
        let fs = match connect(login) {
            Ok(fs) => fs,
            Err(RemoteError::NeedPassword { rejected }) => {
                // A wrong saved password is forgotten for this session.
                self.passwords.lock().unwrap().remove(&key);
                return Err(RemoteError::NeedPassword {
                    rejected: rejected || had_password,
                });
            }
            Err(e) => return Err(e),
        };
        let mut open = self.open.lock().unwrap();
        // Another thread may have connected meanwhile: keep the first.
        Ok(open.entry(key).or_insert(fs).clone())
    }

    /// The connection, if one is open.
    pub fn cached(&self, addr: &RemoteAddr) -> Option<Arc<dyn RemoteFs>> {
        self.open.lock().unwrap().get(&addr.server_key()).cloned()
    }

    pub fn is_connected(&self, addr: &RemoteAddr) -> bool {
        self.open.lock().unwrap().contains_key(&addr.server_key())
    }

    pub fn disconnect(&self, addr: &RemoteAddr) {
        let fs = self.open.lock().unwrap().remove(&addr.server_key());
        if let Some(fs) = fs {
            // Closing may wait for the network: not on the caller's thread.
            std::thread::spawn(move || fs.close());
        }
    }

    pub fn disconnect_all(&self) {
        let all: Vec<_> = self
            .open
            .lock()
            .unwrap()
            .drain()
            .map(|(_, fs)| fs)
            .collect();
        for fs in all {
            fs.close();
        }
    }

    /// Use `password` for the server from now on (this session; also in
    /// the Credential Manager when `remember`).
    pub fn set_password(
        &self,
        addr: &RemoteAddr,
        password: &str,
        remember: bool,
    ) -> Result<(), String> {
        self.passwords
            .lock()
            .unwrap()
            .insert(addr.server_key(), password.to_owned());
        if remember {
            credentials::save(addr, password)
        } else {
            Ok(())
        }
    }

    /// Forget the password of the server (this session and the saved one).
    pub fn forget_password(&self, addr: &RemoteAddr) {
        self.passwords.lock().unwrap().remove(&addr.server_key());
        credentials::delete(addr);
    }

    /// Use this connection for the server (tests).
    #[cfg(test)]
    pub(crate) fn insert(&self, addr: &RemoteAddr, fs: Arc<dyn RemoteFs>) {
        self.open.lock().unwrap().insert(addr.server_key(), fs);
    }
}

/// List a remote folder as file view entries (hidden ones only when asked).
pub fn list_entries(
    fs: &dyn RemoteFs,
    dir: &RemoteAddr,
    show_hidden: bool,
) -> RemoteResult<Vec<FileEntry>> {
    Ok(fs
        .list(&dir.path)?
        .iter()
        .filter(|e| e.name != "." && e.name != "..")
        .filter(|e| show_hidden || !e.name.starts_with('.'))
        .map(|e| e.to_file_entry(dir))
        .collect())
}

/// Download at most `limit` bytes of a file (for the preview).
pub fn read_head(fs: &dyn RemoteFs, path: &str, limit: usize) -> RemoteResult<Vec<u8>> {
    struct Head<'a> {
        buf: &'a mut Vec<u8>,
        limit: usize,
    }
    impl Write for Head<'_> {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            let room = self.limit - self.buf.len();
            self.buf.extend_from_slice(&data[..data.len().min(room)]);
            if self.buf.len() >= self.limit {
                // Enough: stop the download.
                return Err(std::io::Error::other("limit"));
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buf = Vec::new();
    let result = fs.read_to(
        path,
        &mut Head {
            buf: &mut buf,
            limit,
        },
    );
    match result {
        Ok(_) => Ok(buf),
        Err(_) if buf.len() >= limit => Ok(buf),
        Err(e) => Err(e),
    }
}

/// Download a file into a temporary folder (to open it with its app).
/// Returns the local copy.
pub fn download_to_temp(fs: &dyn RemoteFs, addr: &RemoteAddr) -> RemoteResult<PathBuf> {
    let dir = std::env::temp_dir()
        .join("Oxplorer remote")
        .join(sanitize(&addr.host));
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(sanitize(&addr.name()));
    let mut file = std::fs::File::create(&target)?;
    if let Err(e) = fs.read_to(&addr.path, &mut file) {
        drop(file);
        let _ = std::fs::remove_file(&target);
        return Err(e);
    }
    Ok(target)
}

/// A name that is valid on Windows (remote names may contain `:` or `?`).
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        "_".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
pub(crate) mod memory;

#[cfg(test)]
mod tests {
    use super::memory::MemoryFs;
    use super::*;

    #[test]
    fn entries_get_their_address_as_path() {
        let dir = RemoteAddr::parse("sftp://me@h/home/me").unwrap();
        let mut e = RemoteEntry::file(".bashrc", 12, None);
        e.permissions = Some(0o444);
        let fe = e.to_file_entry(&dir);
        assert_eq!(
            fe.path,
            std::path::PathBuf::from("sftp://me@h/home/me/.bashrc")
        );
        assert!(fe.hidden && fe.readonly && !fe.is_dir);
        assert_eq!(fe.size, 12);
    }

    #[test]
    fn lists_hides_and_removes_recursively() {
        let fs = MemoryFs::default();
        fs.add_dir("/a/b");
        fs.add_file("/a/b/c.txt", b"hello");
        fs.add_file("/a/.hidden", b"x");
        let dir = RemoteAddr::parse("sftp://h/a").unwrap();
        let names: Vec<String> = list_entries(&fs, &dir, false)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["b"]);
        assert_eq!(list_entries(&fs, &dir, true).unwrap().len(), 2);
        assert_eq!(fs.stat("/a/b/c.txt").unwrap().unwrap().size, 5);
        assert_eq!(read_head(&fs, "/a/b/c.txt", 3).unwrap(), b"hel");
        assert_eq!(read_head(&fs, "/a/b/c.txt", 100).unwrap(), b"hello");
        remove_all(&fs, "/a", true).unwrap();
        assert_eq!(fs.stat("/a").unwrap(), None);
    }

    #[test]
    fn connections_are_shared_per_server() {
        let conns = Connections::default();
        let a = RemoteAddr::parse("sftp://me@h/x").unwrap();
        assert!(!conns.is_connected(&a));
        conns.insert(&a, Arc::new(MemoryFs::default()));
        let b = RemoteAddr::parse("sftp://me@h:22/y").unwrap();
        assert!(conns.is_connected(&b));
        assert!(conns.get(&b, &ConnectOptions::default()).is_ok());
        conns.disconnect(&b);
        assert!(!conns.is_connected(&a));
    }

    #[test]
    fn places_are_found_by_server() {
        let place = RemotePlace {
            name: String::new(),
            addr: RemoteAddr::parse("sftp://me@nas/srv").unwrap(),
            auth: AuthMethod::KeyFile("C:/k".into()),
            remember_password: true,
        };
        let places = RemotePlaces(vec![place]);
        let addr = RemoteAddr::parse("sftp://me@nas/other/folder").unwrap();
        let opts = ConnectOptions::for_addr(&places, &addr);
        assert_eq!(opts.auth, AuthMethod::KeyFile("C:/k".into()));
        assert_eq!(places.0[0].title(), "srv");
        let other = RemoteAddr::parse("sftp://you@nas/").unwrap();
        assert_eq!(
            ConnectOptions::for_addr(&places, &other).auth,
            AuthMethod::Auto
        );
    }

    #[test]
    fn places_round_trip_through_toml() {
        #[derive(Serialize, Deserialize, Default)]
        struct S {
            remote: RemotePlaces,
        }
        let s = S {
            remote: RemotePlaces(vec![RemotePlace {
                name: "NAS".into(),
                addr: RemoteAddr::parse("ftps://me@nas:2121/pub").unwrap(),
                auth: AuthMethod::Password,
                remember_password: false,
            }]),
        };
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("[[remote]]"), "{text}");
        assert!(!text.lines().any(|l| l.starts_with("password")), "{text}");
        let back: S = toml::from_str(&text).unwrap();
        assert_eq!(back.remote, s.remote);
    }

    #[test]
    fn local_names_are_valid_on_windows() {
        assert_eq!(sanitize("a:b?.txt"), "a_b_.txt");
        assert_eq!(sanitize("dots..."), "dots");
        assert_eq!(sanitize(""), "_");
    }
}
