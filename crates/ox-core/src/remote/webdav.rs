//! WebDAV over http/https through `ureq` (rustls, certificates checked
//! against the Windows store): `PROPFIND` (depth 1) to list, `GET`, `PUT`,
//! `MKCOL`, `MOVE` and `DELETE`. Works with Nextcloud/ownCloud-style
//! servers when the address includes their WebDAV path
//! (`/remote.php/dav/files/<user>`).

use std::io::{self, Read, Write};
use std::sync::Mutex;

use ureq::http::{self, Response};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Body, SendBody};

use super::checksum::{parse_oc_checksum, Checksum};
use super::url::normalize_path;
use super::webdav_auth::{basic, DigestChallenge};
use super::webdav_xml::{parse_multistatus, PROPFIND_BODY};
use super::{Login, Protocol, RemoteEntry, RemoteError, RemoteFs, RemoteResult, TIMEOUT};

/// Folder listings bigger than this are refused (a broken server).
const MAX_LISTING: u64 = 64 * 1024 * 1024;

pub(super) struct DavFs {
    login: Login,
    agent: Agent,
    /// `scheme://host:port`
    base: String,
    /// Set once the server asked for Digest authentication.
    digest: Mutex<Option<DigestChallenge>>,
}

impl DavFs {
    pub fn connect(login: Login) -> RemoteResult<Self> {
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .allow_non_standard_methods(true)
            .timeout_connect(Some(TIMEOUT))
            .timeout_recv_response(Some(TIMEOUT))
            .timeout_per_read(Some(TIMEOUT))
            .max_redirects(5)
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build();
        let addr = &login.addr;
        let scheme = if addr.protocol == Protocol::WebDavs {
            "https"
        } else {
            "http"
        };
        let host = if addr.host.contains(':') {
            format!("[{}]", addr.host)
        } else {
            addr.host.clone()
        };
        let fs = Self {
            base: format!("{scheme}://{host}:{}", addr.port),
            agent: config.into(),
            login,
            digest: Mutex::new(None),
        };
        // Check the login (and learn the authentication scheme) right away.
        let start = fs.home()?;
        match fs.propfind(&start, "0") {
            Ok(_) | Err(RemoteError::NotFound) => Ok(fs),
            Err(e @ RemoteError::NeedPassword { .. }) => Err(e),
            Err(RemoteError::Other(msg)) if msg.starts_with("HTTP 40") => Ok(fs),
            Err(e) => Err(e),
        }
    }

    fn url(&self, path: &str, dir: bool) -> String {
        let mut url = self.base.clone();
        url.push_str(&encode_path(path));
        if dir && !url.ends_with('/') {
            url.push('/');
        }
        url
    }

    fn auth_header(&self, method: &str, url: &str) -> Option<String> {
        let user = &self.login.addr.user;
        let password = self.login.password.as_deref()?;
        let digest = self.digest.lock().unwrap();
        Some(match digest.as_ref() {
            Some(d) => {
                let uri = url.strip_prefix(&self.base).unwrap_or(url);
                d.authorization(user, password, method, uri)
            }
            None => basic(user, password),
        })
    }

    /// Send a request whose body can be sent again (after a Digest challenge).
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<&str>,
    ) -> RemoteResult<Response<Body>> {
        for attempt in 0..2 {
            let mut req = http::Request::builder().method(method).uri(url);
            for (k, v) in headers {
                req = req.header(*k, v.as_str());
            }
            if let Some(auth) = self.auth_header(method, url) {
                req = req.header("Authorization", auth);
            }
            let resp = match body {
                Some(text) => self
                    .agent
                    .run(req.body(text.to_owned()).map_err(RemoteError::other)?),
                None => self.agent.run(req.body(()).map_err(RemoteError::other)?),
            }
            .map_err(net_error)?;
            if resp.status() == 401 && attempt == 0 && self.learn_digest(&resp) {
                continue;
            }
            return self.check(resp);
        }
        unreachable!()
    }

    /// Switch to Digest if the server asks for it. Returns whether to try again.
    fn learn_digest(&self, resp: &Response<Body>) -> bool {
        if self.login.password.is_none() {
            return false;
        }
        let challenge = resp
            .headers()
            .get_all("www-authenticate")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(DigestChallenge::parse);
        match challenge {
            Some(c) => {
                *self.digest.lock().unwrap() = Some(c);
                true
            }
            None => false,
        }
    }

    fn check(&self, resp: Response<Body>) -> RemoteResult<Response<Body>> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        Err(match status.as_u16() {
            401 => RemoteError::NeedPassword {
                rejected: self.login.password.is_some(),
            },
            404 => RemoteError::NotFound,
            403 => RemoteError::Other(format!("HTTP 403: {}", crate::i18n::t("Access denied."))),
            code => RemoteError::Other(format!(
                "HTTP {code} {}",
                status.canonical_reason().unwrap_or("")
            )),
        })
    }

    fn propfind(&self, path: &str, depth: &str) -> RemoteResult<Vec<super::webdav_xml::DavItem>> {
        let url = self.url(path, depth == "1");
        let headers = [
            ("Depth", depth.to_owned()),
            ("Content-Type", "application/xml; charset=utf-8".to_owned()),
        ];
        let resp = self.send("PROPFIND", &url, &headers, Some(PROPFIND_BODY))?;
        let text = resp
            .into_body()
            .with_config()
            .limit(MAX_LISTING)
            .read_to_string()
            .map_err(net_error)?;
        parse_multistatus(&text).map_err(RemoteError::Other)
    }
}

/// Percent-encode each part of a path (keeps the slashes).
fn encode_path(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            b'!' | b'$' | b'\'' | b'(' | b')' | b'*' | b',' | b';' | b'=' | b':' | b'@' => {
                out.push(b as char)
            }
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn net_error(e: ureq::Error) -> RemoteError {
    match e {
        ureq::Error::Io(e) => RemoteError::from(e),
        e => RemoteError::other(e),
    }
}

impl RemoteFs for DavFs {
    fn home(&self) -> RemoteResult<String> {
        Ok(match self.login.addr.path.as_str() {
            "" => "/".to_owned(),
            p => p.to_owned(),
        })
    }

    fn list(&self, dir: &str) -> RemoteResult<Vec<RemoteEntry>> {
        let own = normalize_path(dir);
        let own = if own.is_empty() { "/".to_owned() } else { own };
        Ok(self
            .propfind(&own, "1")?
            .into_iter()
            // The folder itself is part of the answer.
            .filter(|item| item.path != own && !item.entry.name.is_empty())
            .map(|item| item.entry)
            .collect())
    }

    fn stat(&self, path: &str) -> RemoteResult<Option<RemoteEntry>> {
        match self.propfind(path, "0") {
            Ok(items) => Ok(items.into_iter().next().map(|i| i.entry)),
            Err(RemoteError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn read_to(&self, path: &str, out: &mut dyn Write) -> RemoteResult<u64> {
        self.read_from(path, 0, out)
    }

    fn read_from(&self, path: &str, offset: u64, out: &mut dyn Write) -> RemoteResult<u64> {
        let range = [("Range", format!("bytes={offset}-"))];
        let headers: &[(&str, String)] = if offset > 0 { &range } else { &[] };
        let resp = self.send("GET", &self.url(path, false), headers, None)?;
        // `206`: the server sent the rest. `200`: it ignores ranges (no
        // `Accept-Ranges`): the whole file, of which the start is skipped.
        let mut skip = if offset > 0 && resp.status() != 206 {
            offset
        } else {
            0
        };
        let mut reader = resp.into_body().into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut total = 0u64;
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            let from = (skip.min(n as u64)) as usize;
            skip -= from as u64;
            out.write_all(&buf[from..n])?;
            total += (n - from) as u64;
        }
        Ok(total)
    }

    fn checksum(&self, path: &str) -> RemoteResult<Option<Checksum>> {
        // ownCloud/Nextcloud tell the checksum they keep for a file.
        let resp = self.send("HEAD", &self.url(path, false), &[], None)?;
        Ok(resp
            .headers()
            .get_all("oc-checksum")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(parse_oc_checksum))
    }

    fn write_from(&self, path: &str, src: &mut dyn Read, size: u64) -> RemoteResult<()> {
        self.put(path, src, size)
    }

    fn mkdir(&self, path: &str) -> RemoteResult<()> {
        self.send("MKCOL", &self.url(path, true), &[], None)
            .map(|_| ())
    }

    fn rename(&self, from: &str, to: &str) -> RemoteResult<()> {
        let is_dir = self.stat(from)?.is_some_and(|e| e.is_dir);
        let headers = [
            ("Destination", self.url(to, is_dir)),
            ("Overwrite", "F".to_owned()),
        ];
        self.send("MOVE", &self.url(from, is_dir), &headers, None)
            .map(|_| ())
    }

    fn remove_file(&self, path: &str) -> RemoteResult<()> {
        self.send("DELETE", &self.url(path, false), &[], None)
            .map(|_| ())
    }

    fn remove_dir(&self, path: &str) -> RemoteResult<()> {
        self.send("DELETE", &self.url(path, true), &[], None)
            .map(|_| ())
    }

    fn upload_via_temp(&self) -> bool {
        // A PUT only replaces the file once it is complete.
        false
    }

    fn parallel(&self) -> usize {
        4
    }
}

impl DavFs {
    fn put(&self, path: &str, src: &mut dyn Read, size: u64) -> RemoteResult<()> {
        let url = self.url(path, false);
        let mut req = http::Request::builder()
            .method("PUT")
            .uri(&url)
            .header("Content-Length", size.to_string())
            .header("Content-Type", "application/octet-stream");
        if let Some(auth) = self.auth_header("PUT", &url) {
            req = req.header("Authorization", auth);
        }
        let req = req
            .body(SendBody::from_reader(src))
            .map_err(RemoteError::other)?;
        let resp = self.agent.run(req).map_err(net_error)?;
        self.check(resp).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_encoded_per_part() {
        assert_eq!(encode_path("/a b/ä&c.txt"), "/a%20b/%C3%A4%26c.txt");
        assert_eq!(encode_path("/x/#?%"), "/x/%23%3F%25");
    }

    /// Lists a real server: `OX_WEBDAV_URL=webdavs://user@host/path`,
    /// `OX_WEBDAV_PASSWORD=…`.
    #[test]
    #[ignore]
    fn lists_a_real_server() {
        let addr =
            super::super::RemoteAddr::parse(&std::env::var("OX_WEBDAV_URL").unwrap()).unwrap();
        let fs = DavFs::connect(Login {
            password: std::env::var("OX_WEBDAV_PASSWORD").ok(),
            auth: Default::default(),
            addr: addr.clone(),
        })
        .unwrap();
        let entries = fs.list(&fs.home().unwrap()).unwrap();
        assert!(!entries.is_empty());
    }
}
