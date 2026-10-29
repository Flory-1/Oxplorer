//! SSH host key checks against `known_hosts` files: the user's
//! (`%USERPROFILE%\.ssh\known_hosts`, shared with OpenSSH) and Oxplorer's
//! own, where keys the user trusts in Oxplorer are added.
//!
//! A key is trusted when a line for the host has exactly this key. A host
//! with saved keys of the same type, none of them this one, has *changed*
//! its key: that may be an attack and is never accepted without asking.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyStatus {
    Trusted,
    /// No key of this type is saved for the host.
    Unknown,
    /// Keys of this type are saved for the host, but not this one.
    Changed,
    /// The key is marked `@revoked`.
    Revoked,
}

impl HostKeyStatus {
    /// Rank for combining files: a revoked key always wins, then a trusted one.
    fn rank(self) -> u8 {
        match self {
            Self::Revoked => 3,
            Self::Trusted => 2,
            Self::Changed => 1,
            Self::Unknown => 0,
        }
    }
}

/// Oxplorer's own file (in its config folder).
pub fn oxplorer_file() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = TEST_FILE.lock().unwrap().clone() {
        return p;
    }
    crate::config_dir().join("known_hosts")
}

/// Tests against real servers keep the keys they trust out of the user's file.
#[cfg(test)]
pub(crate) static TEST_FILE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// OpenSSH's file of the user.
pub fn user_file() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts"))
}

fn files() -> Vec<String> {
    let mut paths: Vec<PathBuf> = user_file().into_iter().collect();
    paths.push(oxplorer_file());
    paths
        .into_iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .collect()
}

/// The host as `known_hosts` names it: `host`, or `[host]:port` when the
/// port isn't 22.
pub fn host_name(host: &str, port: u16) -> String {
    let host = host.to_lowercase();
    if port == 22 {
        host
    } else {
        format!("[{host}]:{port}")
    }
}

/// Check a key (`key_type` like `ssh-ed25519`, `key` its base64 blob)
/// against all `known_hosts` files.
pub fn check(host: &str, port: u16, key_type: &str, key: &str) -> HostKeyStatus {
    files()
        .iter()
        .map(|text| check_in(text, host, port, key_type, key))
        .max_by_key(|s| s.rank())
        .unwrap_or(HostKeyStatus::Unknown)
}

/// Key types saved for the host, so the connection can ask for one of them first.
pub fn known_types(host: &str, port: u16) -> Vec<String> {
    let name = host_name(host, port);
    let mut types = Vec::new();
    for text in files() {
        for line in lines(&text) {
            if line.marker.is_none() && line.matches(&name) && !types.contains(&line.key_type) {
                types.push(line.key_type.clone());
            }
        }
    }
    types
}

/// Check a key against one file's text.
pub fn check_in(text: &str, host: &str, port: u16, key_type: &str, key: &str) -> HostKeyStatus {
    let name = host_name(host, port);
    let mut status = HostKeyStatus::Unknown;
    for line in lines(text) {
        if line.marker.as_deref() == Some("@revoked") {
            if line.key == key {
                return HostKeyStatus::Revoked;
            }
            continue;
        }
        if line.marker.is_some() || !line.matches(&name) {
            continue;
        }
        if line.key_type == key_type && line.key == key {
            status = HostKeyStatus::Trusted;
        } else if same_kind(&line.key_type, key_type) && status != HostKeyStatus::Trusted {
            status = HostKeyStatus::Changed;
        }
    }
    status
}

/// RSA keys are the same key with any of their signature names.
fn same_kind(a: &str, b: &str) -> bool {
    let rsa = |t: &str| matches!(t, "ssh-rsa" | "rsa-sha2-256" | "rsa-sha2-512");
    a == b || (rsa(a) && rsa(b))
}

/// Save a key the user trusts in Oxplorer's file, replacing the keys of
/// the same type it had for the host.
pub fn trust(host: &str, port: u16, key_type: &str, key: &str) -> io::Result<()> {
    let path = oxplorer_file();
    let text = fs::read_to_string(&path).unwrap_or_default();
    let updated = with_trusted(&text, host, port, key_type, key);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = fs::File::create(&path)?;
    file.write_all(updated.as_bytes())
}

/// `text` with the key added and older keys of its type for the host removed.
pub fn with_trusted(text: &str, host: &str, port: u16, key_type: &str, key: &str) -> String {
    let name = host_name(host, port);
    let mut out = String::new();
    for raw in text.lines() {
        let replaced = parse_line(raw).is_some_and(|line| {
            line.marker.is_none()
                && same_kind(&line.key_type, key_type)
                && line.hosts.len() == 1
                && line.matches(&name)
        });
        if !replaced && !raw.trim().is_empty() {
            out.push_str(raw);
            out.push('\n');
        }
    }
    out.push_str(&format!("{name} {key_type} {key}\n"));
    out
}

/// `SHA256:…` as OpenSSH shows it, from the base64 key blob.
pub fn fingerprint(key: &str) -> String {
    match STANDARD.decode(key.trim()) {
        Ok(blob) => format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(&blob))),
        Err(_) => "?".to_owned(),
    }
}

struct Line {
    marker: Option<String>,
    hosts: Vec<String>,
    key_type: String,
    key: String,
}

impl Line {
    /// Does a host pattern match? A negated pattern (`!host`) that matches wins.
    fn matches(&self, name: &str) -> bool {
        let mut hit = false;
        for pattern in &self.hosts {
            if let Some(neg) = pattern.strip_prefix('!') {
                if pattern_matches(neg, name) {
                    return false;
                }
            } else if pattern_matches(pattern, name) {
                hit = true;
            }
        }
        hit
    }
}

fn lines(text: &str) -> impl Iterator<Item = Line> + '_ {
    text.lines().filter_map(parse_line)
}

fn parse_line(raw: &str) -> Option<Line> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') {
        return None;
    }
    let mut fields = raw.split_whitespace();
    let mut first = fields.next()?;
    let marker = if first.starts_with('@') {
        let m = first.to_owned();
        first = fields.next()?;
        Some(m)
    } else {
        None
    };
    Some(Line {
        marker,
        hosts: first.split(',').map(str::to_owned).collect(),
        key_type: fields.next()?.to_owned(),
        key: fields.next()?.to_owned(),
    })
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    if let Some(hashed) = pattern.strip_prefix("|1|") {
        return hashed_matches(hashed, name);
    }
    glob(&pattern.to_lowercase(), name)
}

/// `|1|salt|hash`: HMAC-SHA1 of the name with the salt as key.
fn hashed_matches(hashed: &str, name: &str) -> bool {
    let Some((salt, hash)) = hashed.split_once('|') else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (STANDARD.decode(salt), STANDARD.decode(hash)) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<sha1::Sha1>::new_from_slice(&salt) else {
        return false;
    };
    mac.update(name.as_bytes());
    mac.verify_slice(&hash).is_ok()
}

/// `*` (any run) and `?` (one character), as in OpenSSH host patterns.
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
    const ED2: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIBbH6oTrd/hpk+1n2JmL3jNcTPxAqVbtZTfb7Z1g3qWA";
    const RSA: &str = "AAAAB3NzaC1yc2EAAAADAQABAAABAQC7";

    #[test]
    fn plain_hosts_ports_and_lists() {
        let text = format!(
            "# comment\n\nexample.com,192.0.2.1 ssh-ed25519 {ED}\n[example.com]:2222 ssh-ed25519 {ED2} me@pc\n"
        );
        let check = |host, port, key| check_in(&text, host, port, "ssh-ed25519", key);
        assert_eq!(check("example.com", 22, ED), HostKeyStatus::Trusted);
        assert_eq!(check("EXAMPLE.com", 22, ED), HostKeyStatus::Trusted);
        assert_eq!(check("192.0.2.1", 22, ED), HostKeyStatus::Trusted);
        assert_eq!(check("example.com", 2222, ED2), HostKeyStatus::Trusted);
        // Another key than the saved one: changed.
        assert_eq!(check("example.com", 22, ED2), HostKeyStatus::Changed);
        assert_eq!(check("example.com", 2222, ED), HostKeyStatus::Changed);
        assert_eq!(check("other.com", 22, ED), HostKeyStatus::Unknown);
        // Only a key of another type is saved: unknown, not changed.
        assert_eq!(
            check_in(&text, "example.com", 22, "ssh-rsa", RSA),
            HostKeyStatus::Unknown
        );
    }

    #[test]
    fn hashed_hosts() {
        // `ssh-keygen -H` style entry for "example.com" (salt and HMAC-SHA1).
        let salt = [7u8; 20];
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&salt).unwrap();
        mac.update(b"example.com");
        let hash = mac.finalize().into_bytes();
        let text = format!(
            "|1|{}|{} ssh-ed25519 {ED}\n",
            STANDARD.encode(salt),
            STANDARD.encode(hash)
        );
        assert_eq!(
            check_in(&text, "example.com", 22, "ssh-ed25519", ED),
            HostKeyStatus::Trusted
        );
        assert_eq!(
            check_in(&text, "example.org", 22, "ssh-ed25519", ED),
            HostKeyStatus::Unknown
        );
    }

    #[test]
    fn patterns_negation_markers() {
        let text = format!(
            "*.lan,!bad.lan ssh-ed25519 {ED}\n@cert-authority *.corp ssh-ed25519 {ED2}\n@revoked * ssh-ed25519 {ED2}\n"
        );
        let check = |host, key| check_in(&text, host, 22, "ssh-ed25519", key);
        assert_eq!(check("nas.lan", ED), HostKeyStatus::Trusted);
        assert_eq!(check("bad.lan", ED), HostKeyStatus::Unknown);
        assert_eq!(check("x.corp", ED2), HostKeyStatus::Revoked);
        assert!(glob("a?c*", "abcdef"));
        assert!(!glob("a?c", "ac"));
    }

    #[test]
    fn rsa_signature_names_are_one_kind() {
        let text = format!("h ssh-rsa {RSA}\n");
        assert_eq!(
            check_in(&text, "h", 22, "ssh-rsa", "AAAAother"),
            HostKeyStatus::Changed
        );
        assert_eq!(
            check_in(&text, "h", 22, "rsa-sha2-512", "AAAAother"),
            HostKeyStatus::Changed
        );
    }

    #[test]
    fn trusting_replaces_the_old_key_of_the_host() {
        let text =
            format!("h ssh-ed25519 {ED}\nother ssh-ed25519 {ED}\n[h]:2222 ssh-ed25519 {ED}\n");
        let updated = with_trusted(&text, "h", 22, "ssh-ed25519", ED2);
        assert_eq!(
            check_in(&updated, "h", 22, "ssh-ed25519", ED2),
            HostKeyStatus::Trusted
        );
        assert_eq!(
            check_in(&updated, "h", 22, "ssh-ed25519", ED),
            HostKeyStatus::Changed
        );
        assert_eq!(
            check_in(&updated, "other", 22, "ssh-ed25519", ED),
            HostKeyStatus::Trusted
        );
        assert_eq!(
            check_in(&updated, "h", 2222, "ssh-ed25519", ED),
            HostKeyStatus::Trusted
        );
        assert_eq!(updated.lines().count(), 3);
    }

    #[test]
    fn fingerprints_like_openssh() {
        let fp = fingerprint(ED);
        assert!(fp.starts_with("SHA256:"), "{fp}");
        assert!(!fp.ends_with('='));
        assert_eq!(fp.len(), "SHA256:".len() + 43);
        assert_eq!(fingerprint("not base64!"), "?");
    }
}
