//! Checksums a server computes for its files (FTP `HASH`/`XSHA256`/…,
//! WebDAV `OC-Checksum`), and the same hashes of local files, so a
//! transfer can be verified without downloading the file again.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// The hash functions servers offer (best first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HashKind {
    Sha256,
    Sha1,
    Md5,
}

impl HashKind {
    /// From a name like `SHA-256`, `sha256`, `SHA1` or `MD5`.
    pub fn parse(name: &str) -> Option<Self> {
        let n: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_uppercase();
        match n.as_str() {
            "SHA256" => Some(Self::Sha256),
            "SHA1" => Some(Self::Sha1),
            "MD5" => Some(Self::Md5),
            _ => None,
        }
    }

    /// The name FTP's `HASH` command uses.
    pub fn ftp_name(self) -> &'static str {
        match self {
            Self::Sha256 => "SHA-256",
            Self::Sha1 => "SHA-1",
            Self::Md5 => "MD5",
        }
    }

    fn hex_len(self) -> usize {
        match self {
            Self::Sha256 => 64,
            Self::Sha1 => 40,
            Self::Md5 => 32,
        }
    }
}

/// A file's checksum: the hash function and the hash in lower-case hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    pub kind: HashKind,
    pub hex: String,
}

impl Checksum {
    /// `None` unless `hex` is a hash of the right length.
    pub fn new(kind: HashKind, hex: &str) -> Option<Self> {
        let hex = hex.trim().to_ascii_lowercase();
        (hex.len() == kind.hex_len() && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then_some(Self { kind, hex })
    }

    /// The checksum of `data` (tests and small files).
    pub fn of_bytes(kind: HashKind, data: &[u8]) -> Self {
        let mut h = Hasher::new(kind);
        h.update(data);
        h.finish()
    }
}

enum Inner {
    Sha256(Sha256),
    Sha1(Sha1),
    Md5(Md5),
}

/// Computes a [`Checksum`] piece by piece.
pub struct Hasher {
    kind: HashKind,
    inner: Inner,
}

impl Hasher {
    pub fn new(kind: HashKind) -> Self {
        let inner = match kind {
            HashKind::Sha256 => Inner::Sha256(Sha256::new()),
            HashKind::Sha1 => Inner::Sha1(Sha1::new()),
            HashKind::Md5 => Inner::Md5(Md5::new()),
        };
        Self { kind, inner }
    }

    pub fn update(&mut self, data: &[u8]) {
        match &mut self.inner {
            Inner::Sha256(h) => h.update(data),
            Inner::Sha1(h) => h.update(data),
            Inner::Md5(h) => h.update(data),
        }
    }

    pub fn finish(self) -> Checksum {
        let bytes: Vec<u8> = match self.inner {
            Inner::Sha256(h) => h.finalize().to_vec(),
            Inner::Sha1(h) => h.finalize().to_vec(),
            Inner::Md5(h) => h.finalize().to_vec(),
        };
        Checksum {
            kind: self.kind,
            hex: bytes.iter().map(|b| format!("{b:02x}")).collect(),
        }
    }
}

/// The checksum of a local file. `go` returns `false` to stop (cancel).
pub fn hash_file(path: &Path, kind: HashKind, go: &dyn Fn() -> bool) -> io::Result<Checksum> {
    let mut file = File::open(path)?;
    let mut hasher = Hasher::new(kind);
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buf[..n]);
        if !go() {
            return Err(io::Error::other(crate::i18n::t("Cancelled")));
        }
    }
    Ok(hasher.finish())
}

/// The best checksum of a WebDAV `OC-Checksum` header (ownCloud/Nextcloud:
/// `SHA1:ab12… MD5:cd34… ADLER32:…`).
pub fn parse_oc_checksum(header: &str) -> Option<Checksum> {
    header
        .split([' ', ','])
        .filter_map(|part| {
            let (name, hex) = part.split_once(':')?;
            Checksum::new(HashKind::parse(name)?, hex)
        })
        .min_by_key(|c| c.kind)
}

/// The hash functions an FTP server lists in `FEAT` (`HASH SHA-256*;MD5`,
/// or the older `XSHA256`, `XSHA1`, `XMD5` commands), best first, with
/// whether the old command is used.
pub fn ftp_hashes<'a>(
    features: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> Vec<(HashKind, bool)> {
    let mut out = Vec::new();
    for (name, value) in features {
        match name.to_ascii_uppercase().as_str() {
            "HASH" => {
                for alg in value.unwrap_or("").split(';') {
                    if let Some(k) = HashKind::parse(alg.trim_end_matches('*')) {
                        out.push((k, false));
                    }
                }
            }
            "XSHA256" => out.push((HashKind::Sha256, true)),
            "XSHA1" => out.push((HashKind::Sha1, true)),
            "XMD5" => out.push((HashKind::Md5, true)),
            _ => {}
        }
    }
    // The `HASH` command first: it is the standard one.
    out.sort_by_key(|(k, old)| (*k, *old));
    out.dedup_by_key(|(k, _)| *k);
    out
}

/// The hash in an FTP reply: `213 SHA-256 0-1234 ab12… name` (`HASH`) or
/// `250 ab12…` / `213 ab12…` (`XSHA256` and friends).
pub fn parse_ftp_reply(kind: HashKind, text: &str) -> Option<Checksum> {
    text.split_whitespace()
        .find_map(|word| Checksum::new(kind, word))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    const HELLO_MD5: &str = "5d41402abc4b2a76b9719d911017c592";
    const HELLO_SHA1: &str = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";

    #[test]
    fn hashes_bytes_and_files() {
        assert_eq!(
            Checksum::of_bytes(HashKind::Sha256, b"hello").hex,
            HELLO_SHA256
        );
        assert_eq!(Checksum::of_bytes(HashKind::Md5, b"hello").hex, HELLO_MD5);
        assert_eq!(Checksum::of_bytes(HashKind::Sha1, b"hello").hex, HELLO_SHA1);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"hello").unwrap();
        let c = hash_file(&p, HashKind::Sha256, &|| true).unwrap();
        assert_eq!(c.hex, HELLO_SHA256);
        assert!(hash_file(&p, HashKind::Md5, &|| false).is_err());
    }

    #[test]
    fn reads_server_answers() {
        let c = parse_oc_checksum(&format!(
            "MD5:{HELLO_MD5} SHA1:{HELLO_SHA1} ADLER32:062c0215"
        ))
        .unwrap();
        assert_eq!(c.kind, HashKind::Sha1);
        assert_eq!(parse_oc_checksum("ADLER32:062c0215"), None);
        let reply = format!("213 SHA-256 0-5 {} hello.txt", HELLO_SHA256.to_uppercase());
        assert_eq!(
            parse_ftp_reply(HashKind::Sha256, &reply).unwrap().hex,
            HELLO_SHA256
        );
        assert_eq!(parse_ftp_reply(HashKind::Md5, "250 nope"), None);
        let feats = [
            ("HASH", Some("SHA-1;SHA-256*;MD5;CRC32")),
            ("XMD5", None),
            ("SIZE", None),
        ];
        assert_eq!(
            ftp_hashes(feats.into_iter()),
            [
                (HashKind::Sha256, false),
                (HashKind::Sha1, false),
                (HashKind::Md5, false)
            ]
        );
        assert_eq!(
            ftp_hashes([("XSHA1", None)].into_iter()),
            [(HashKind::Sha1, true)]
        );
    }
}
