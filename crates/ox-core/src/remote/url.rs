//! Addresses of remote places: `sftp://user@host:22/path`, `ftp://…`,
//! `ftps://…` (explicit TLS), `webdav://…` (http) and `webdavs://…` (https).
//!
//! Remote items are shown with their address as their path (a `PathBuf`
//! holding the URL), so the file view, the selection and the transfer
//! engine can carry them like local paths; [`RemoteAddr::from_path`] turns
//! such a path back into an address.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Protocol {
    Sftp,
    Ftp,
    /// FTP with explicit TLS (`AUTH TLS`).
    Ftps,
    /// WebDAV over http.
    WebDav,
    /// WebDAV over https.
    WebDavs,
}

impl Protocol {
    pub const ALL: [Protocol; 5] = [
        Self::Sftp,
        Self::Ftp,
        Self::Ftps,
        Self::WebDav,
        Self::WebDavs,
    ];

    pub fn scheme(self) -> &'static str {
        match self {
            Self::Sftp => "sftp",
            Self::Ftp => "ftp",
            Self::Ftps => "ftps",
            Self::WebDav => "webdav",
            Self::WebDavs => "webdavs",
        }
    }

    /// The scheme of the address, also the web ones (`https://` = WebDAV over https).
    pub fn from_scheme(scheme: &str) -> Option<Self> {
        Some(match scheme.to_ascii_lowercase().as_str() {
            "sftp" | "ssh" => Self::Sftp,
            "ftp" => Self::Ftp,
            "ftps" | "ftpes" => Self::Ftps,
            "webdav" | "dav" | "http" => Self::WebDav,
            "webdavs" | "davs" | "https" => Self::WebDavs,
            _ => return None,
        })
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Sftp => 22,
            Self::Ftp | Self::Ftps => 21,
            Self::WebDav => 80,
            Self::WebDavs => 443,
        }
    }

    /// Name shown in the interface (not translated).
    pub fn label(self) -> &'static str {
        match self {
            Self::Sftp => "SFTP",
            Self::Ftp => "FTP",
            Self::Ftps => "FTPS",
            Self::WebDav => "WebDAV (http)",
            Self::WebDavs => "WebDAV (https)",
        }
    }

    pub fn is_webdav(self) -> bool {
        matches!(self, Self::WebDav | Self::WebDavs)
    }

    /// The connection is encrypted.
    pub fn is_secure(self) -> bool {
        !matches!(self, Self::Ftp | Self::WebDav)
    }
}

/// A folder or file on a server. `path` is absolute (`/home/me`), `/` for
/// the server's root, or empty for the folder a login starts in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RemoteAddr {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub path: String,
}

impl RemoteAddr {
    pub fn new(protocol: Protocol, host: &str, user: &str, path: &str) -> Self {
        Self {
            protocol,
            host: host.trim().to_owned(),
            port: protocol.default_port(),
            user: user.trim().to_owned(),
            path: normalize_path(path),
        }
    }

    /// Parse an address such as `sftp://me@example.com:2222/home/me`.
    /// A password in the address (`user:secret@`) is left out (see
    /// [`password_in`]).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (scheme, rest) = text.split_once("://")?;
        let protocol = Protocol::from_scheme(scheme)?;
        let end = rest.find(['/', '\\']).unwrap_or(rest.len());
        let (authority, path) = rest.split_at(end);
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
            None => (None, authority),
        };
        let user = userinfo
            .map(|u| decode(u.split_once(':').map_or(u, |(name, _)| name)))
            .unwrap_or_default();
        let (host, port) = split_host_port(hostport)?;
        if host.is_empty() {
            return None;
        }
        Some(Self {
            protocol,
            host: host.to_owned(),
            port: port.unwrap_or(protocol.default_port()),
            user,
            path: normalize_path(path),
        })
    }

    /// The address of a remote item's path (see the module docs).
    pub fn from_path(path: &Path) -> Option<Self> {
        let text = path.to_str()?;
        if !is_remote_text(text) {
            return None;
        }
        Self::parse(text)
    }

    /// `sftp://me@host:2222/path` (the port only when it isn't the default).
    pub fn to_url(&self) -> String {
        let mut url = self.server_url();
        url.push_str(&self.path);
        url
    }

    /// The path the file view uses for this item.
    pub fn to_path(&self) -> PathBuf {
        PathBuf::from(self.to_url())
    }

    /// The address without a path: `sftp://me@host`.
    pub fn server_url(&self) -> String {
        let mut url = format!("{}://", self.protocol.scheme());
        if !self.user.is_empty() {
            url.push_str(&encode_user(&self.user));
            url.push('@');
        }
        if self.host.contains(':') {
            url.push_str(&format!("[{}]", self.host));
        } else {
            url.push_str(&self.host);
        }
        if self.port != self.protocol.default_port() {
            url.push_str(&format!(":{}", self.port));
        }
        url
    }

    /// One server login (protocol, user, host and port): connections are
    /// shared by all addresses with the same key.
    pub fn server_key(&self) -> String {
        format!(
            "{}://{}@{}:{}",
            self.protocol.scheme(),
            encode_user(&self.user),
            self.host.to_lowercase(),
            self.port
        )
    }

    /// The name of the saved password in the Windows Credential Manager.
    pub fn credential_target(&self) -> String {
        format!("Oxplorer:{}", self.server_url())
    }

    pub fn same_server(&self, other: &RemoteAddr) -> bool {
        self.server_key() == other.server_key()
    }

    pub fn with_path(&self, path: &str) -> Self {
        Self {
            path: normalize_path(path),
            ..self.clone()
        }
    }

    pub fn join(&self, name: &str) -> Self {
        self.with_path(&join_path(&self.path, name))
    }

    /// The folder above, or `None` at the top.
    pub fn parent(&self) -> Option<Self> {
        parent_path(&self.path).map(|p| self.with_path(&p))
    }

    /// The last part of the path, or the host at the top.
    pub fn name(&self) -> String {
        match file_name(&self.path) {
            "" => self.host.clone(),
            name => name.to_owned(),
        }
    }

    /// `name on host`-style title for tabs and the sidebar.
    pub fn title(&self) -> String {
        match file_name(&self.path) {
            "" if self.user.is_empty() => self.host.clone(),
            "" => format!("{}@{}", self.user, self.host),
            name => name.to_owned(),
        }
    }
}

/// The password typed into an address (`ftp://me:secret@host`), if any.
pub fn password_in(text: &str) -> Option<String> {
    let (_, rest) = text.trim().split_once("://")?;
    let authority = &rest[..rest.find(['/', '\\']).unwrap_or(rest.len())];
    let userinfo = &authority[..authority.rfind('@')?];
    userinfo.split_once(':').map(|(_, pw)| decode(pw))
}

/// The text starts like a remote address (`sftp://`, `ftp://`, …). Web
/// addresses (`https://`) only count where WebDAV is asked for.
pub fn is_remote_text(text: &str) -> bool {
    text.split_once("://").is_some_and(|(scheme, _)| {
        !matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "ssh"
        ) && Protocol::from_scheme(scheme).is_some()
    })
}

/// The path is a remote item's (see the module docs).
pub fn is_remote_path(path: &Path) -> bool {
    path.to_str().is_some_and(is_remote_text)
}

fn split_host_port(hostport: &str) -> Option<(&str, Option<u16>)> {
    if let Some(v6) = hostport.strip_prefix('[') {
        let (host, rest) = v6.split_once(']')?;
        let port = match rest.strip_prefix(':') {
            Some(p) => Some(p.parse().ok()?),
            None if rest.is_empty() => None,
            None => return None,
        };
        return Some((host, port));
    }
    match hostport.rsplit_once(':') {
        Some((host, "")) => Some((host, None)),
        Some((host, port)) => Some((host, Some(port.parse().ok()?))),
        None => Some((hostport, None)),
    }
}

/// `/a/b` for any spelling (`a\b`, `/a//b/`, `/a/./c/../b`); empty stays empty.
pub fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// `dir/name` with exactly one slash between.
pub fn join_path(dir: &str, name: &str) -> String {
    let name = name.trim_matches(['/', '\\']);
    if dir.is_empty() || dir == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", dir.trim_end_matches('/'))
    }
}

/// The folder above `path`, `None` at the top.
pub fn parent_path(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let i = trimmed.rfind('/')?;
    Some(if i == 0 {
        "/".to_owned()
    } else {
        trimmed[..i].to_owned()
    })
}

/// The last part of a remote path ("" at the top).
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or("")
}

fn encode_user(user: &str) -> String {
    let mut out = String::new();
    for c in user.chars() {
        match c {
            '@' | ':' | '/' | '\\' | '%' | '?' | '#' | ' ' => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Undo `%XX` escapes (invalid ones are kept as they are).
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(b) = hex {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_addresses() {
        let a = RemoteAddr::parse("sftp://me@example.com:2222/home/me/").unwrap();
        assert_eq!(a.protocol, Protocol::Sftp);
        assert_eq!(a.host, "example.com");
        assert_eq!(a.port, 2222);
        assert_eq!(a.user, "me");
        assert_eq!(a.path, "/home/me");
        assert_eq!(a.to_url(), "sftp://me@example.com:2222/home/me");
    }

    #[test]
    fn default_ports_and_paths() {
        let a = RemoteAddr::parse("FTP://ftp.example.org").unwrap();
        assert_eq!(a.protocol, Protocol::Ftp);
        assert_eq!(a.port, 21);
        assert_eq!(a.user, "");
        assert_eq!(a.path, "");
        assert_eq!(a.to_url(), "ftp://ftp.example.org");
        let root = RemoteAddr::parse("ftps://h/").unwrap();
        assert_eq!(root.path, "/");
        assert_eq!(root.to_url(), "ftps://h/");
        assert_eq!(root.port, 21);
        let dav = RemoteAddr::parse("https://cloud.example.com/remote.php/dav/files/me").unwrap();
        assert_eq!(dav.protocol, Protocol::WebDavs);
        assert_eq!(dav.port, 443);
        assert_eq!(
            dav.to_url(),
            "webdavs://cloud.example.com/remote.php/dav/files/me"
        );
        assert_eq!(RemoteAddr::parse("webdav://h:8080/x").unwrap().port, 8080);
    }

    #[test]
    fn passwords_users_and_ipv6() {
        let text = "ftp://some%40one:se%3Acret@[::1]:2121/pub";
        let a = RemoteAddr::parse(text).unwrap();
        assert_eq!(a.user, "some@one");
        assert_eq!(a.host, "::1");
        assert_eq!(a.port, 2121);
        assert_eq!(password_in(text).as_deref(), Some("se:cret"));
        assert_eq!(password_in("ftp://me@h/"), None);
        assert_eq!(a.to_url(), "ftp://some%40one@[::1]:2121/pub");
        assert_eq!(RemoteAddr::parse(&a.to_url()).unwrap(), a);
    }

    #[test]
    fn rejects_bad_addresses() {
        assert!(RemoteAddr::parse("C:\\Users").is_none());
        assert!(RemoteAddr::parse("gopher://h/").is_none());
        assert!(RemoteAddr::parse("sftp://").is_none());
        assert!(RemoteAddr::parse("sftp://h:99999/").is_none());
        assert!(RemoteAddr::parse("sftp://[::1/").is_none());
    }

    #[test]
    fn paths_join_and_go_up() {
        let a = RemoteAddr::parse("sftp://h/a/b").unwrap();
        assert_eq!(a.join("c d.txt").path, "/a/b/c d.txt");
        assert_eq!(a.parent().unwrap().path, "/a");
        assert_eq!(a.parent().unwrap().parent().unwrap().path, "/");
        assert!(a.with_path("/").parent().is_none());
        assert!(a.with_path("").parent().is_none());
        assert_eq!(a.name(), "b");
        assert_eq!(a.with_path("/").name(), "h");
        assert_eq!(normalize_path("a\\b//./c/../d/"), "/a/b/d");
        assert_eq!(join_path("/", "x"), "/x");
        assert_eq!(join_path("", "x"), "/x");
        assert_eq!(file_name("/a/b/"), "b");
    }

    #[test]
    fn remote_paths_round_trip_through_pathbuf() {
        let a = RemoteAddr::parse("webdavs://me@h/dav/My Files").unwrap();
        let p = a.join("x.txt").to_path();
        assert!(is_remote_path(&p));
        assert_eq!(
            RemoteAddr::from_path(&p).unwrap().path,
            "/dav/My Files/x.txt"
        );
        // Windows joins with a backslash; that is read back as a slash.
        let joined = a.to_path().join("y");
        assert_eq!(
            RemoteAddr::from_path(&joined).unwrap().path,
            "/dav/My Files/y"
        );
        assert!(!is_remote_path(Path::new(r"C:\x")));
        assert!(!is_remote_path(Path::new("https://example.com/")));
    }

    #[test]
    fn server_keys_and_credential_targets() {
        let a = RemoteAddr::parse("sftp://Me@Example.com/x").unwrap();
        let b = RemoteAddr::parse("sftp://Me@example.com:22/y").unwrap();
        assert!(a.same_server(&b));
        assert_eq!(a.server_key(), "sftp://Me@example.com:22");
        assert_eq!(a.credential_target(), "Oxplorer:sftp://Me@Example.com");
        let c = RemoteAddr::parse("ftps://u%3Ax@h:990").unwrap();
        assert_eq!(c.credential_target(), "Oxplorer:ftps://u%3Ax@h:990");
        let mut other = a.clone();
        other.user = "other".into();
        assert!(!a.same_server(&other));
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(decode("a%20b%zz%4"), "a b%zz%4");
        assert_eq!(decode("%C3%A4"), "ä");
    }
}
