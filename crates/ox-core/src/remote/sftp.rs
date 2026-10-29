//! SFTP through `russh` (pure Rust SSH) and `russh-sftp`. Both are async:
//! a small tokio runtime runs on two background threads and the calls here
//! block on it, each step with a timeout.
//!
//! The server's host key is checked against the `known_hosts` files before
//! anything else is sent; an unknown or changed key stops the connection
//! with [`RemoteError::HostKey`] so the user can decide.

use std::borrow::Cow;
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{
    self, Algorithm, HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate,
};
use russh::MethodKind;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::known_hosts::{self, HostKeyStatus};
use super::{
    AuthMethod, HostKeyQuestion, Login, RemoteEntry, RemoteError, RemoteFs, RemoteResult, TIMEOUT,
};

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("ox-ssh")
        .enable_all()
        .build()
        .expect("couldn't start the SSH runtime")
});

/// Run one step with the timeout.
fn run<T, E: Into<RemoteError>>(f: impl Future<Output = Result<T, E>>) -> RemoteResult<T> {
    RUNTIME.block_on(async {
        match tokio::time::timeout(TIMEOUT, f).await {
            Ok(r) => r.map_err(Into::into),
            Err(_) => Err(timed_out()),
        }
    })
}

fn timed_out() -> RemoteError {
    RemoteError::Other(crate::i18n::t("The server didn't answer in time.").to_owned())
}

impl From<SftpError> for RemoteError {
    fn from(e: SftpError) -> Self {
        match &e {
            SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile => {
                RemoteError::NotFound
            }
            SftpError::Status(s) if s.status_code == StatusCode::PermissionDenied => {
                RemoteError::Other(crate::i18n::t("Access denied.").to_owned())
            }
            SftpError::Timeout => timed_out(),
            _ => RemoteError::other(e),
        }
    }
}

impl From<keys::Error> for RemoteError {
    fn from(e: keys::Error) -> Self {
        RemoteError::other(e)
    }
}

impl From<russh::Error> for RemoteError {
    fn from(e: russh::Error) -> Self {
        RemoteError::other(e)
    }
}

/// The SSH side: checks the host key.
struct Client {
    host: String,
    port: u16,
    /// Set when the key isn't trusted (read after the connection failed).
    question: Arc<Mutex<Option<HostKeyQuestion>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key: Cow<'_, PublicKey> = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => Cow::Borrowed(key),
            // Only the key inside counts (no certificate authorities).
            PublicKeyOrCertificate::Certificate(c) => {
                Cow::Owned(PublicKey::new(c.public_key().clone(), ""))
            }
        };
        let Some((key_type, blob)) = openssh_parts(&key) else {
            return Ok(false);
        };
        let status = known_hosts::check(&self.host, self.port, &key_type, &blob);
        if status == HostKeyStatus::Trusted {
            return Ok(true);
        }
        *self.question.lock().unwrap() = Some(HostKeyQuestion {
            host: self.host.clone(),
            port: self.port,
            fingerprint: known_hosts::fingerprint(&blob),
            key_type,
            key: blob,
            changed: matches!(status, HostKeyStatus::Changed | HostKeyStatus::Revoked),
        });
        Ok(false)
    }
}

/// `("ssh-ed25519", "AAAA…")` of a public key.
fn openssh_parts(key: &PublicKey) -> Option<(String, String)> {
    let line = key.to_openssh().ok()?;
    let mut parts = line.split_whitespace();
    Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
}

struct Session {
    ssh: Handle<Client>,
    sftp: SftpSession,
}

pub(super) struct SftpFs {
    login: Login,
    session: Mutex<Option<Arc<Session>>>,
}

impl SftpFs {
    pub fn connect(login: Login) -> RemoteResult<Self> {
        let session = Arc::new(open(&login)?);
        Ok(Self {
            login,
            session: Mutex::new(Some(session)),
        })
    }

    /// The session; a new one when the old one was closed.
    fn session(&self) -> RemoteResult<Arc<Session>> {
        let mut guard = self.session.lock().unwrap();
        if let Some(s) = guard.as_ref().filter(|s| !s.ssh.is_closed()) {
            return Ok(s.clone());
        }
        let s = Arc::new(open(&self.login)?);
        *guard = Some(s.clone());
        Ok(s)
    }

    /// Run `f` on the session, once more on a new session if the
    /// connection dropped meanwhile.
    fn with<T>(&self, f: impl Fn(&SftpSession) -> RemoteResult<T>) -> RemoteResult<T> {
        let s = self.session()?;
        match f(&s.sftp) {
            Err(e) if s.ssh.is_closed() => {
                drop(s);
                let s = self.session().map_err(|_| e)?;
                f(&s.sftp)
            }
            r => r,
        }
    }
}

fn open(login: &Login) -> RemoteResult<Session> {
    let addr = &login.addr;
    let mut config = client::Config {
        inactivity_timeout: Some(Duration::from_secs(15 * 60)),
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    };
    // Ask for a key type that is already saved for the host first (as
    // OpenSSH does): no needless "unknown key" question for a known server.
    let known = known_hosts::known_types(&addr.host, addr.port);
    if !known.is_empty() {
        let mut algs: Vec<Algorithm> = config.preferred.key.to_vec();
        algs.sort_by_key(|a| {
            let name = a.as_str();
            let rsa = name.starts_with("rsa-") || name == "ssh-rsa";
            !known.iter().any(|k| k == name || (rsa && k == "ssh-rsa"))
        });
        config.preferred.key = Cow::Owned(algs);
    }
    let question = Arc::new(Mutex::new(None));
    let handler = Client {
        host: addr.host.clone(),
        port: addr.port,
        question: question.clone(),
    };
    let target = (addr.host.clone(), addr.port);
    let connected = run(client::connect(Arc::new(config), target, handler));
    let mut ssh = match connected {
        Ok(h) => h,
        Err(e) => {
            if let Some(q) = question.lock().unwrap().take() {
                return Err(RemoteError::HostKey(Box::new(q)));
            }
            return Err(e);
        }
    };
    authenticate(&mut ssh, login)?;
    let sftp = run(async {
        let channel = ssh.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        Ok::<_, RemoteError>(SftpSession::new(channel.into_stream()).await?)
    })?;
    sftp.set_timeout(TIMEOUT.as_secs());
    Ok(Session { ssh, sftp })
}

fn authenticate(ssh: &mut Handle<Client>, login: &Login) -> RemoteResult<()> {
    let user = if login.addr.user.is_empty() {
        whoami()
    } else {
        login.addr.user.clone()
    };
    let password = login.password.as_deref();
    let rsa_hash = run(ssh.best_supported_rsa_hash()).ok().flatten().flatten();
    match &login.auth {
        AuthMethod::KeyFile(path) => {
            if try_key_file(ssh, &user, path, password, rsa_hash, true)? {
                return Ok(());
            }
            return Err(RemoteError::NeedPassword {
                rejected: password.is_some(),
            });
        }
        AuthMethod::Auto => {
            if try_agent(ssh, &user, rsa_hash) {
                return Ok(());
            }
            for key in default_keys() {
                if try_key_file(ssh, &user, &key, password, rsa_hash, false)? {
                    return Ok(());
                }
            }
        }
        AuthMethod::Password => {}
    }
    let Some(password) = password else {
        return Err(RemoteError::NeedPassword { rejected: false });
    };
    let result = run(ssh.authenticate_password(user.as_str(), password))?;
    if result.success() {
        return Ok(());
    }
    // Many servers only take passwords through keyboard-interactive.
    if let AuthResult::Failure {
        remaining_methods, ..
    } = &result
    {
        if remaining_methods.contains(&MethodKind::KeyboardInteractive)
            && keyboard_interactive(ssh, &user, password)?
        {
            return Ok(());
        }
    }
    Err(RemoteError::NeedPassword { rejected: true })
}

fn keyboard_interactive(
    ssh: &mut Handle<Client>,
    user: &str,
    password: &str,
) -> RemoteResult<bool> {
    let mut reply = run(ssh.authenticate_keyboard_interactive_start(user, None))?;
    // A few rounds at most (some servers send an empty request first).
    for _ in 0..4 {
        match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| password.to_owned()).collect();
                reply = run(ssh.authenticate_keyboard_interactive_respond(answers))?;
            }
        }
    }
    Ok(false)
}

/// Keys offered by a running agent: OpenSSH's (Windows service) or Pageant.
fn try_agent(ssh: &mut Handle<Client>, user: &str, rsa_hash: Option<HashAlg>) -> bool {
    #[cfg(windows)]
    {
        use russh::keys::agent::client::AgentClient;
        let openssh = run(async {
            let mut agent = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await?;
            let ids = agent.request_identities().await?;
            Ok::<_, keys::Error>((agent.dynamic(), ids))
        });
        let pageant = || {
            run(async {
                let mut agent = AgentClient::connect_pageant().await?;
                let ids = agent.request_identities().await?;
                Ok::<_, keys::Error>((agent.dynamic(), ids))
            })
        };
        for found in [openssh.ok(), pageant().ok()].into_iter().flatten() {
            let (mut agent, ids) = found;
            for id in ids {
                let key = id.public_key().into_owned();
                let hash = if key.algorithm().is_rsa() {
                    rsa_hash
                } else {
                    None
                };
                let r = run(async {
                    ssh.authenticate_publickey_with(user, key, hash, &mut agent)
                        .await
                        .map_err(RemoteError::other)
                });
                if r.is_ok_and(|r| r.success()) {
                    return true;
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (ssh, user, rsa_hash);
    false
}

/// `Ok(false)` if the key was refused or can't be used (an encrypted key
/// without passphrase is skipped unless it is the chosen key).
fn try_key_file(
    ssh: &mut Handle<Client>,
    user: &str,
    path: &Path,
    passphrase: Option<&str>,
    rsa_hash: Option<HashAlg>,
    chosen: bool,
) -> RemoteResult<bool> {
    let key = match keys::load_secret_key(path, None) {
        Ok(k) => k,
        Err(keys::Error::KeyIsEncrypted) => match passphrase {
            Some(p) => match keys::load_secret_key(path, Some(p)) {
                Ok(k) => k,
                Err(_) if chosen => return Err(RemoteError::NeedPassword { rejected: true }),
                Err(_) => return Ok(false),
            },
            None if chosen => return Err(RemoteError::NeedPassword { rejected: false }),
            None => return Ok(false),
        },
        Err(e) if chosen => {
            return Err(RemoteError::Other(format!("{}: {e}", path.display())));
        }
        Err(_) => return Ok(false),
    };
    let hash = if key.algorithm().is_rsa() {
        rsa_hash
    } else {
        None
    };
    let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
    Ok(run(ssh.authenticate_publickey(user, key))?.success())
}

/// The usual private keys in `%USERPROFILE%\.ssh`.
fn default_keys() -> Vec<PathBuf> {
    let Some(dir) = dirs::home_dir().map(|h| h.join(".ssh")) else {
        return Vec::new();
    };
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|n| dir.join(n))
        .filter(|p| p.is_file())
        .collect()
}

/// The Windows user name, for logins without one.
fn whoami() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}

fn to_entry(name: String, attrs: &FileAttributes) -> RemoteEntry {
    RemoteEntry {
        name,
        is_dir: attrs.is_dir(),
        is_symlink: attrs.is_symlink(),
        size: attrs.size.unwrap_or(0),
        modified: attrs
            .mtime
            .map(|t| UNIX_EPOCH + Duration::from_secs(u64::from(t))),
        permissions: attrs.permissions.map(|p| p & 0o7777),
    }
}

impl RemoteFs for SftpFs {
    fn home(&self) -> RemoteResult<String> {
        self.with(|s| run(s.canonicalize(".")))
    }

    fn list(&self, dir: &str) -> RemoteResult<Vec<RemoteEntry>> {
        self.with(|s| {
            let items = run(s.read_dir(dir))?;
            let mut entries = Vec::new();
            for item in items {
                let name = item.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let mut e = to_entry(name, &item.metadata());
                if e.is_symlink {
                    // Where the link points decides whether it opens as a folder.
                    if let Ok(target) = run(s.metadata(item.path())) {
                        e.is_dir = target.is_dir();
                        e.size = if e.is_dir {
                            0
                        } else {
                            target.size.unwrap_or(0)
                        };
                    }
                }
                entries.push(e);
            }
            Ok(entries)
        })
    }

    fn stat(&self, path: &str) -> RemoteResult<Option<RemoteEntry>> {
        self.with(|s| match run(s.metadata(path)) {
            Ok(attrs) => Ok(Some(to_entry(
                super::url::file_name(path).to_owned(),
                &attrs,
            ))),
            Err(RemoteError::NotFound) => Ok(None),
            Err(e) => Err(e),
        })
    }

    fn read_to(&self, path: &str, out: &mut dyn Write) -> RemoteResult<u64> {
        self.read_from(path, 0, out)
    }

    fn read_from(&self, path: &str, offset: u64, out: &mut dyn Write) -> RemoteResult<u64> {
        let s = self.session()?;
        let mut file = run(s.sftp.open(path))?;
        if offset > 0 {
            run(async { file.seek(std::io::SeekFrom::Start(offset)).await })?;
        }
        let mut buf = vec![0u8; 256 * 1024];
        let mut total = 0u64;
        let result = loop {
            let n = match run(file.read(&mut buf)) {
                Ok(n) => n,
                Err(e) => break Err(e),
            };
            if n == 0 {
                break Ok(total);
            }
            if let Err(e) = out.write_all(&buf[..n]) {
                break Err(e.into());
            }
            total += n as u64;
        };
        let _ = run(file.shutdown());
        result
    }

    fn write_from(&self, path: &str, src: &mut dyn Read, _size: u64) -> RemoteResult<()> {
        self.write_at(path, None, src)
    }

    fn can_append(&self) -> bool {
        true
    }

    fn append_from(
        &self,
        path: &str,
        offset: u64,
        src: &mut dyn Read,
        _size: u64,
    ) -> RemoteResult<()> {
        self.write_at(path, Some(offset), src)
    }

    fn mkdir(&self, path: &str) -> RemoteResult<()> {
        self.with(|s| run(s.create_dir(path)))
    }

    fn rename(&self, from: &str, to: &str) -> RemoteResult<()> {
        self.with(|s| run(s.rename(from, to)))
    }

    fn remove_file(&self, path: &str) -> RemoteResult<()> {
        self.with(|s| run(s.remove_file(path)))
    }

    fn remove_dir(&self, path: &str) -> RemoteResult<()> {
        self.with(|s| run(s.remove_dir(path)))
    }

    fn set_modified(&self, path: &str, time: SystemTime) -> RemoteResult<()> {
        let secs = time
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs().min(u64::from(u32::MAX)) as u32)
            .unwrap_or(0);
        let mut attrs = FileAttributes::empty();
        attrs.mtime = Some(secs);
        attrs.atime = Some(secs);
        self.with(|s| run(s.set_metadata(path, attrs.clone())))
    }

    fn parallel(&self) -> usize {
        4
    }

    fn close(&self) {
        if let Some(s) = self.session.lock().unwrap().take() {
            let _ = run(s.sftp.close());
            let _ = run(s.ssh.disconnect(russh::Disconnect::ByApplication, "", "en"));
        }
    }
}

impl SftpFs {
    /// Write `src` to `path`: a new file, or (`offset`) the rest of one,
    /// keeping its first `offset` bytes.
    fn write_at(&self, path: &str, offset: Option<u64>, src: &mut dyn Read) -> RemoteResult<()> {
        let s = self.session()?;
        let flags = match offset {
            None => OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            Some(_) => OpenFlags::WRITE,
        };
        let mut file = run(s.sftp.open_with_flags(path, flags))?;
        if let Some(offset) = offset {
            run(async { file.seek(std::io::SeekFrom::Start(offset)).await })?;
        }
        let mut buf = vec![0u8; 256 * 1024];
        let result = loop {
            let n = match src.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => break Err(e.into()),
            };
            if let Err(e) = run(file.write_all(&buf[..n])) {
                break Err(e);
            }
        };
        let closed = run(file.shutdown());
        result.and(closed)
    }
}

impl Drop for SftpFs {
    fn drop(&mut self) {
        // The SSH tasks live on the runtime: drop them in its context.
        let _guard = RUNTIME.enter();
        self.session.lock().unwrap().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lists a real server: `OX_SFTP_URL=sftp://user@host/path` (password
    /// in `OX_SFTP_PASSWORD`, or keys/agent). The first connection must stop
    /// at the unknown host key; it is then trusted (in a temporary file).
    #[test]
    #[ignore]
    fn lists_a_real_server() {
        let tmp = tempfile::tempdir().unwrap();
        *known_hosts::TEST_FILE.lock().unwrap() = Some(tmp.path().join("known_hosts"));
        let addr = super::super::RemoteAddr::parse(&std::env::var("OX_SFTP_URL").unwrap()).unwrap();
        let login = Login {
            password: std::env::var("OX_SFTP_PASSWORD").ok(),
            auth: AuthMethod::Auto,
            addr,
        };
        let fs = match SftpFs::connect(login.clone()) {
            Err(RemoteError::HostKey(q)) => {
                println!("{} {} {}", q.host, q.key_type, q.fingerprint);
                q.trust().unwrap();
                SftpFs::connect(login).unwrap()
            }
            Ok(_) => panic!("connected without asking about the host key"),
            Err(e) => panic!("{e}"),
        };
        let home = fs.home().unwrap();
        assert!(home.starts_with('/'));
        let entries = fs.list(&home).unwrap();
        println!("{home}: {} entries", entries.len());
        if let Some(file) = entries.iter().find(|e| !e.is_dir && e.size > 0) {
            let path = super::super::url::join_path(&home, &file.name);
            let head = super::super::read_head(&fs, &path, 100).unwrap();
            assert!(!head.is_empty());
        }
        fs.close();
    }
}
