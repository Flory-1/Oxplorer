//! FTP and FTPS (explicit TLS) through `suppaftp`. A command connection
//! can do one thing at a time, so connections are pooled: each operation
//! takes an idle one (or opens a new one) and puts it back when done.
//! Passive mode, binary transfers, `MLSD` with a `LIST` fallback, UTF-8.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use suppaftp::types::{Features, FileType};
use suppaftp::{FtpError, Mode, RustlsConnector, RustlsFtpStream, Status};

use super::checksum::{self, Checksum, HashKind};
use super::url::{file_name, parent_path};
use super::{
    ftp_list, tls, Login, Protocol, RemoteEntry, RemoteError, RemoteFs, RemoteResult, TIMEOUT,
};

type Conn = RustlsFtpStream;

/// Idle connections kept per server.
const MAX_IDLE: usize = 4;

const MLSD_UNKNOWN: u8 = 0;
const MLSD_YES: u8 = 1;
const MLSD_NO: u8 = 2;

pub(super) struct FtpFs {
    login: Login,
    idle: Mutex<Vec<Conn>>,
    mlsd: AtomicU8,
    /// What the server lists in `FEAT` (asked once).
    feat: OnceLock<Features>,
}

impl FtpFs {
    pub fn connect(login: Login) -> RemoteResult<Self> {
        let fs = Self {
            login,
            idle: Mutex::new(Vec::new()),
            mlsd: AtomicU8::new(MLSD_UNKNOWN),
            feat: OnceLock::new(),
        };
        // Log in once now, so wrong passwords show up right away.
        let conn = fs.open()?;
        fs.put_back(conn);
        Ok(fs)
    }

    fn open(&self) -> RemoteResult<Conn> {
        let addr = &self.login.addr;
        let socket = (addr.host.as_str(), addr.port)
            .to_socket_addrs()
            .map_err(RemoteError::other)?
            .next()
            .ok_or_else(|| RemoteError::other(format!("{}: no address", addr.host)))?;
        let stream = TcpStream::connect_timeout(&socket, TIMEOUT).map_err(RemoteError::other)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        let mut conn = Conn::connect_with_stream(stream)
            .map_err(ftp_error)?
            .passive_stream_builder(|addr| {
                let s = TcpStream::connect_timeout(&addr, TIMEOUT)
                    .map_err(FtpError::ConnectionError)?;
                let _ = s.set_read_timeout(Some(TIMEOUT));
                let _ = s.set_write_timeout(Some(TIMEOUT));
                Ok(s)
            });
        if addr.protocol == Protocol::Ftps {
            let config = tls::client_config().map_err(RemoteError::Other)?;
            conn = conn
                .into_secure(RustlsConnector::from(config), &addr.host)
                .map_err(ftp_error)?;
        }
        // Servers behind NAT often announce their private address.
        conn.set_passive_nat_workaround(true);
        conn.set_mode(Mode::Passive);
        let (user, password) = if addr.user.is_empty() {
            ("anonymous".to_owned(), "anonymous@".to_owned())
        } else {
            match &self.login.password {
                Some(pw) => (addr.user.clone(), pw.clone()),
                None => return Err(RemoteError::NeedPassword { rejected: false }),
            }
        };
        if let Err(e) = conn.login(user.as_str(), password.as_str()) {
            return Err(match e {
                FtpError::UnexpectedResponse(r) if r.status == Status::NotLoggedIn => {
                    RemoteError::NeedPassword {
                        rejected: !addr.user.is_empty(),
                    }
                }
                e => ftp_error(e),
            });
        }
        conn.transfer_type(FileType::Binary).map_err(ftp_error)?;
        // Most servers use UTF-8 anyway; some need to be asked.
        let _ = conn.opts("UTF8", Some("ON"));
        Ok(conn)
    }

    /// An idle connection that still works, or a new one.
    fn take(&self) -> RemoteResult<Conn> {
        loop {
            let idle = self.idle.lock().unwrap().pop();
            match idle {
                Some(mut conn) => {
                    if conn.noop().is_ok() {
                        return Ok(conn);
                    }
                }
                None => return self.open(),
            }
        }
    }

    fn put_back(&self, conn: Conn) {
        let mut idle = self.idle.lock().unwrap();
        if idle.len() < MAX_IDLE {
            idle.push(conn);
        }
    }

    /// Run `f` on a connection; it goes back to the pool unless the
    /// connection itself failed.
    fn with<T>(&self, f: impl FnOnce(&mut Conn) -> Result<T, FtpError>) -> RemoteResult<T> {
        let mut conn = self.take()?;
        match f(&mut conn) {
            Ok(v) => {
                self.put_back(conn);
                Ok(v)
            }
            Err(e) => {
                if matches!(e, FtpError::UnexpectedResponse(_)) {
                    self.put_back(conn);
                }
                Err(ftp_error(e))
            }
        }
    }

    /// The server's features (none if it doesn't answer `FEAT`).
    fn features(&self) -> &Features {
        self.feat
            .get_or_init(|| self.with(|c| c.feat()).unwrap_or_default())
    }

    /// `STOR` (or `REST offset` + `STOR`: the rest of a file).
    fn store(&self, path: &str, offset: Option<u64>, src: &mut dyn Read) -> RemoteResult<()> {
        let mut conn = self.take()?;
        if let Some(offset) = offset {
            let _ = conn.resume_transfer(offset as usize);
        }
        let mut stream = match conn.put_with_stream(path) {
            Ok(s) => s,
            Err(e) => {
                self.put_back(conn);
                return Err(ftp_error(e));
            }
        };
        io::copy(src, &mut stream)?;
        stream.finish().map_err(ftp_error)?;
        self.put_back(conn);
        Ok(())
    }

    fn list_lines(&self, conn: &mut Conn, dir: &str) -> Result<Vec<RemoteEntry>, FtpError> {
        // `CWD` first: paths with spaces are safe that way on every server.
        conn.cwd(if dir.is_empty() { "/" } else { dir })?;
        if self.mlsd.load(Ordering::Relaxed) != MLSD_NO {
            match conn.mlsd(None) {
                Ok(lines) => {
                    self.mlsd.store(MLSD_YES, Ordering::Relaxed);
                    return Ok(lines
                        .iter()
                        .filter_map(|l| ftp_list::parse_mlsd(l))
                        .collect());
                }
                Err(FtpError::UnexpectedResponse(r)) if is_unsupported(r.status) => {
                    self.mlsd.store(MLSD_NO, Ordering::Relaxed);
                }
                Err(e) => return Err(e),
            }
        }
        let lines = conn.list(None)?;
        Ok(lines
            .iter()
            .filter_map(|l| ftp_list::parse_list(l))
            .collect())
    }
}

fn is_unsupported(status: Status) -> bool {
    matches!(
        status,
        Status::BadCommand
            | Status::BadArguments
            | Status::NotImplemented
            | Status::NotImplementedParameter
            | Status::CommandNotImplemented
    )
}

fn ftp_error(e: FtpError) -> RemoteError {
    match e {
        FtpError::UnexpectedResponse(r) => {
            let text = String::from_utf8_lossy(&r.body).trim().to_owned();
            if r.status == Status::FileUnavailable && text.to_lowercase().contains("no such") {
                RemoteError::NotFound
            } else if text.is_empty() {
                RemoteError::Other(format!("FTP {}", r.status.code()))
            } else {
                RemoteError::Other(text)
            }
        }
        FtpError::ConnectionError(e) => RemoteError::from(e),
        e => RemoteError::other(e),
    }
}

impl RemoteFs for FtpFs {
    fn home(&self) -> RemoteResult<String> {
        self.with(|c| c.pwd())
            .map(|p| super::url::normalize_path(&p))
            .map(|p| if p.is_empty() { "/".into() } else { p })
    }

    fn list(&self, dir: &str) -> RemoteResult<Vec<RemoteEntry>> {
        let mut conn = self.take()?;
        let result = self.list_lines(&mut conn, dir);
        let mut entries = match result {
            Ok(e) => e,
            Err(e) => {
                if matches!(e, FtpError::UnexpectedResponse(_)) {
                    self.put_back(conn);
                }
                return Err(ftp_error(e));
            }
        };
        // Links: a folder if we can change into it.
        for e in entries.iter_mut().filter(|e| e.is_symlink) {
            e.is_dir = conn.cwd(super::url::join_path(dir, &e.name)).is_ok();
        }
        self.put_back(conn);
        Ok(entries)
    }

    fn stat(&self, path: &str) -> RemoteResult<Option<RemoteEntry>> {
        let parent = parent_path(path).unwrap_or_else(|| "/".into());
        let name = file_name(path);
        if name.is_empty() {
            return Ok(Some(RemoteEntry::dir("", None)));
        }
        match self.list(&parent) {
            Ok(entries) => Ok(entries.into_iter().find(|e| e.name == name)),
            Err(RemoteError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn read_to(&self, path: &str, out: &mut dyn Write) -> RemoteResult<u64> {
        self.read_from(path, 0, out)
    }

    fn read_from(&self, path: &str, offset: u64, out: &mut dyn Write) -> RemoteResult<u64> {
        if offset > 0 && !self.can_append() {
            // No `REST STREAM`: from the start, skipping what is there.
            return super::read_skipping(&|out| self.read_from(path, 0, out), offset, out);
        }
        let mut conn = self.take()?;
        // `REST` right before `RETR` (sent by suppaftp with the transfer).
        let _ = conn.resume_transfer(offset as usize);
        let mut stream = match conn.retr_as_stream(path) {
            Ok(s) => s,
            Err(e) => {
                self.put_back(conn);
                return Err(ftp_error(e));
            }
        };
        let mut buf = vec![0u8; 256 * 1024];
        let mut total = 0u64;
        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // The connection is in an unknown state: dropped, not pooled.
                Err(e) => return Err(e.into()),
            };
            out.write_all(&buf[..n])?;
            total += n as u64;
        }
        stream.finish().map_err(ftp_error)?;
        self.put_back(conn);
        Ok(total)
    }

    fn write_from(&self, path: &str, src: &mut dyn Read, _size: u64) -> RemoteResult<()> {
        self.store(path, None, src)
    }

    fn can_append(&self) -> bool {
        // `REST STREAM` (RFC 3659) is what `REST` + `STOR` needs.
        self.features()
            .get("REST")
            .is_some_and(|v| v.as_deref().unwrap_or("").contains("STREAM"))
    }

    fn append_from(
        &self,
        path: &str,
        offset: u64,
        src: &mut dyn Read,
        _size: u64,
    ) -> RemoteResult<()> {
        self.store(path, Some(offset), src)
    }

    fn checksum(&self, path: &str) -> RemoteResult<Option<Checksum>> {
        let Some((kind, old)) = checksum::ftp_hashes(
            self.features()
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_deref())),
        )
        .first()
        .copied() else {
            return Ok(None);
        };
        let reply = self.with(|c| {
            if old {
                let cmd = match kind {
                    HashKind::Sha256 => "XSHA256",
                    HashKind::Sha1 => "XSHA1",
                    HashKind::Md5 => "XMD5",
                };
                c.custom_command(
                    format!("{cmd} {path}"),
                    &[Status::File, Status::RequestedFileActionOk],
                )
            } else {
                // The default (marked `*` in `FEAT`) may be another one.
                let _ = c.custom_command(
                    format!("OPTS HASH {}", kind.ftp_name()),
                    &[Status::CommandOk],
                );
                c.custom_command(format!("HASH {path}"), &[Status::File])
            }
        });
        match reply {
            Ok(r) => Ok(checksum::parse_ftp_reply(
                kind,
                &String::from_utf8_lossy(&r.body),
            )),
            // A server that lists the command but refuses it for this file.
            Err(RemoteError::Other(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn mkdir(&self, path: &str) -> RemoteResult<()> {
        self.with(|c| c.mkdir(path))
    }

    fn rename(&self, from: &str, to: &str) -> RemoteResult<()> {
        self.with(|c| c.rename(from, to))
    }

    fn remove_file(&self, path: &str) -> RemoteResult<()> {
        self.with(|c| c.rm(path))
    }

    fn remove_dir(&self, path: &str) -> RemoteResult<()> {
        self.with(|c| c.rmdir(path))
    }

    fn set_modified(&self, path: &str, time: SystemTime) -> RemoteResult<()> {
        // `MFMT` (not on every server; a missing date is no error).
        let stamp = chrono::DateTime::<chrono::Utc>::from(time).format("%Y%m%d%H%M%S");
        let _ = self.with(|c| c.custom_command(format!("MFMT {stamp} {path}"), &[Status::File]));
        Ok(())
    }

    fn close(&self) {
        for mut conn in self.idle.lock().unwrap().drain(..) {
            let _ = conn.quit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lists a real server: `OX_FTP_URL=ftps://user@host/path`, password
    /// in `OX_FTP_PASSWORD`.
    #[test]
    #[ignore]
    fn lists_a_real_server() {
        let addr = super::super::RemoteAddr::parse(&std::env::var("OX_FTP_URL").unwrap()).unwrap();
        let fs = FtpFs::connect(Login {
            password: std::env::var("OX_FTP_PASSWORD").ok(),
            auth: Default::default(),
            addr,
        })
        .unwrap();
        let home = fs.home().unwrap();
        let entries = fs.list(&home).unwrap();
        println!("{home}: {} entries", entries.len());
        assert!(!entries.is_empty());
        if let Some(file) = entries.iter().find(|e| !e.is_dir && e.size > 0) {
            let path = super::super::url::join_path(&home, &file.name);
            let head = super::super::read_head(&fs, &path, 100).unwrap();
            assert!(!head.is_empty());
            // The connection still works after a stopped download.
            assert!(fs.stat(&path).unwrap().is_some());
        }
        fs.close();
    }
}
