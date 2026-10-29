//! HTTP authentication for WebDAV: Basic, and Digest (MD5 or SHA-256,
//! `qop=auth`) when the server asks for it.

use std::sync::atomic::{AtomicU32, Ordering};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use md5::Md5;
use sha2::{Digest as _, Sha256};

pub(super) fn basic(user: &str, password: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("{user}:{password}")))
}

/// A server's `WWW-Authenticate: Digest …` challenge.
#[derive(Debug)]
pub(super) struct DigestChallenge {
    realm: String,
    nonce: String,
    opaque: Option<String>,
    qop_auth: bool,
    sha256: bool,
    count: AtomicU32,
}

impl DigestChallenge {
    /// The Digest challenge among the `WWW-Authenticate` headers, if any
    /// (and if its algorithm is one we know).
    pub fn parse(header: &str) -> Option<Self> {
        let rest = header.trim().strip_prefix("Digest")?.trim_start();
        let params = params(rest);
        let get = |k: &str| {
            params
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(k))
                .map(|(_, v)| v.clone())
        };
        let algorithm = get("algorithm").unwrap_or_else(|| "MD5".into());
        let sha256 = match algorithm.to_ascii_uppercase().as_str() {
            "MD5" => false,
            "SHA-256" => true,
            _ => return None,
        };
        Some(Self {
            realm: get("realm").unwrap_or_default(),
            nonce: get("nonce")?,
            opaque: get("opaque"),
            qop_auth: get("qop").is_some_and(|q| q.split(',').any(|v| v.trim() == "auth")),
            sha256,
            count: AtomicU32::new(0),
        })
    }

    fn hash(&self, text: &str) -> String {
        if self.sha256 {
            hex(&Sha256::digest(text.as_bytes()))
        } else {
            hex(&Md5::digest(text.as_bytes()))
        }
    }

    /// The `Authorization` header for one request.
    pub fn authorization(&self, user: &str, password: &str, method: &str, uri: &str) -> String {
        let nc = self.count.fetch_add(1, Ordering::Relaxed) + 1;
        let cnonce = format!("{:016x}", rand_u64());
        self.authorization_with(user, password, method, uri, nc, &cnonce)
    }

    fn authorization_with(
        &self,
        user: &str,
        password: &str,
        method: &str,
        uri: &str,
        nc: u32,
        cnonce: &str,
    ) -> String {
        let ha1 = self.hash(&format!("{user}:{}:{password}", self.realm));
        let ha2 = self.hash(&format!("{method}:{uri}"));
        let nc = format!("{nc:08x}");
        let response = if self.qop_auth {
            self.hash(&format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", self.nonce))
        } else {
            self.hash(&format!("{ha1}:{}:{ha2}", self.nonce))
        };
        let mut header = format!(
            "Digest username=\"{user}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\", response=\"{response}\", algorithm={}",
            self.realm,
            self.nonce,
            if self.sha256 { "SHA-256" } else { "MD5" }
        );
        if self.qop_auth {
            header.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
        }
        if let Some(opaque) = &self.opaque {
            header.push_str(&format!(", opaque=\"{opaque}\""));
        }
        header
    }
}

/// `key=value, key="quoted, value"` pairs.
fn params(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim().trim_start_matches(',').trim().to_owned();
        rest = &rest[eq + 1..];
        let value;
        if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            value = quoted[..end].to_owned();
            rest = quoted.get(end + 1..).unwrap_or("");
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            value = rest[..end].trim().to_owned();
            rest = &rest[end..];
        }
        out.push((key, value));
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A random-enough client nonce (no crypto needed: it only has to differ).
fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_header() {
        assert_eq!(
            basic("Aladdin", "open sesame"),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    #[test]
    fn digest_matches_rfc_2617() {
        let challenge = DigestChallenge::parse(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        )
        .unwrap();
        let header = challenge.authorization_with(
            "Mufasa",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            1,
            "0a4f113b",
        );
        assert!(
            header.contains(r#"response="6629fae49393a05397450978507c4ef1""#),
            "{header}"
        );
        assert!(header.contains("nc=00000001"));
        assert!(header.contains(r#"opaque="5ccc069c403ebaf9f0171e9517f40e41""#));
    }

    #[test]
    fn unknown_challenges_are_ignored() {
        assert!(DigestChallenge::parse(r#"Basic realm="x""#).is_none());
        assert!(
            DigestChallenge::parse(r#"Digest realm="x", nonce="n", algorithm=SHA-512-256"#)
                .is_none()
        );
        assert!(
            DigestChallenge::parse(r#"Digest realm="x", nonce="n", algorithm=SHA-256"#).is_some()
        );
    }
}
