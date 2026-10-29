//! An in-memory remote file system for tests.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use super::checksum::{Checksum, HashKind};
use super::url::{file_name, normalize_path, parent_path};
use super::{RemoteEntry, RemoteError, RemoteFs, RemoteResult};

#[derive(Debug, Clone)]
enum Node {
    Dir,
    File(Vec<u8>, Option<SystemTime>),
}

#[derive(Default)]
pub struct MemoryFs {
    nodes: Mutex<BTreeMap<String, Node>>,
    /// Fail uploads and downloads of paths containing this text.
    pub fail_on: Mutex<Option<String>>,
    /// How often a failing path was tried.
    pub failures: AtomicUsize,
    /// The connection drops once a download or upload got this far
    /// (bytes of the file; `0`: never).
    pub drop_after: AtomicU64,
    /// Nothing works (the server is unreachable).
    pub offline: AtomicBool,
    /// The server stays unreachable after the connection dropped.
    pub stay_down: AtomicBool,
    /// Uploads can't be continued (like WebDAV).
    pub no_append: AtomicBool,
    /// The server offers SHA-256 checksums; `bad_checksums`: wrong ones.
    pub checksums: AtomicBool,
    pub bad_checksums: AtomicBool,
    /// Offsets downloads and uploads were continued from.
    pub resumed_at: Mutex<Vec<u64>>,
    /// A pause after each block of a download (a slow network).
    pub delay: Mutex<Option<Duration>>,
    /// Downloads started.
    pub reads: AtomicUsize,
}

impl MemoryFs {
    pub fn add_dir(&self, path: &str) {
        let path = normalize_path(path);
        let mut nodes = self.nodes.lock().unwrap();
        let mut p = Some(path);
        while let Some(dir) = p {
            if dir == "/" {
                break;
            }
            p = parent_path(&dir);
            nodes.insert(dir, Node::Dir);
        }
    }

    pub fn add_file(&self, path: &str, data: &[u8]) {
        let path = normalize_path(path);
        if let Some(parent) = parent_path(&path) {
            self.add_dir(&parent);
        }
        self.nodes.lock().unwrap().insert(
            path,
            Node::File(data.to_vec(), Some(SystemTime::UNIX_EPOCH)),
        );
    }

    /// Change a file on the server (a newer version, with this date).
    pub fn change_file(&self, path: &str, data: &[u8], modified: SystemTime) {
        self.nodes.lock().unwrap().insert(
            normalize_path(path),
            Node::File(data.to_vec(), Some(modified)),
        );
    }

    pub fn contents(&self, path: &str) -> Option<Vec<u8>> {
        match self.nodes.lock().unwrap().get(path) {
            Some(Node::File(d, _)) => Some(d.clone()),
            _ => None,
        }
    }

    pub fn modified(&self, path: &str) -> Option<SystemTime> {
        match self.nodes.lock().unwrap().get(path) {
            Some(Node::File(_, m)) => *m,
            _ => None,
        }
    }

    pub fn paths(&self) -> Vec<String> {
        self.nodes.lock().unwrap().keys().cloned().collect()
    }

    fn online(&self) -> RemoteResult<()> {
        if self.offline.load(Ordering::Relaxed) {
            return Err(RemoteError::Other("the server isn't reachable".into()));
        }
        Ok(())
    }

    /// A transfer got to byte `at` of the file: the connection drops (once)
    /// past `drop_after`.
    fn moved(&self, at: u64) -> RemoteResult<()> {
        let limit = self.drop_after.load(Ordering::Relaxed);
        if limit > 0 && at >= limit {
            self.drop_after.store(0, Ordering::Relaxed);
            if self.stay_down.load(Ordering::Relaxed) {
                self.offline.store(true, Ordering::Relaxed);
            }
            return Err(RemoteError::Other("connection lost".into()));
        }
        Ok(())
    }

    fn check_fail(&self, path: &str) -> RemoteResult<()> {
        self.online()?;
        if let Some(bad) = self.fail_on.lock().unwrap().as_deref() {
            if path.contains(bad) {
                self.failures.fetch_add(1, Ordering::Relaxed);
                return Err(RemoteError::Other("simulated failure".into()));
            }
        }
        Ok(())
    }

    /// Write `data` and then what `src` holds to `path`. What arrived stays
    /// when the connection drops (a partial file, as on a real server).
    fn store(&self, path: &str, mut data: Vec<u8>, src: &mut dyn Read) -> RemoteResult<()> {
        self.check_fail(path)?;
        let parent = parent_path(path).unwrap_or_else(|| "/".into());
        if parent != "/" && !matches!(self.nodes.lock().unwrap().get(&parent), Some(Node::Dir)) {
            return Err(RemoteError::NotFound);
        }
        let mut buf = [0u8; 4];
        let result = loop {
            if let Err(e) = self.moved(data.len() as u64) {
                break Err(e);
            }
            match src.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => data.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => break Err(e.into()),
            }
        };
        self.nodes
            .lock()
            .unwrap()
            .insert(path.to_owned(), Node::File(data, None));
        result
    }

    fn entry(path: &str, node: &Node) -> RemoteEntry {
        match node {
            Node::Dir => RemoteEntry::dir(file_name(path), None),
            Node::File(d, m) => RemoteEntry::file(file_name(path), d.len() as u64, *m),
        }
    }
}

impl RemoteFs for MemoryFs {
    fn home(&self) -> RemoteResult<String> {
        Ok("/".into())
    }

    fn list(&self, dir: &str) -> RemoteResult<Vec<RemoteEntry>> {
        self.online()?;
        let dir = normalize_path(dir);
        let nodes = self.nodes.lock().unwrap();
        if dir != "/" && !matches!(nodes.get(&dir), Some(Node::Dir)) {
            return Err(RemoteError::NotFound);
        }
        Ok(nodes
            .iter()
            .filter(|(p, _)| parent_path(p).as_deref() == Some(dir.as_str()))
            .map(|(p, n)| Self::entry(p, n))
            .collect())
    }

    fn stat(&self, path: &str) -> RemoteResult<Option<RemoteEntry>> {
        self.online()?;
        let path = normalize_path(path);
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .get(&path)
            .map(|n| Self::entry(&path, n)))
    }

    fn read_to(&self, path: &str, out: &mut dyn Write) -> RemoteResult<u64> {
        self.read_from(path, 0, out)
    }

    fn read_from(&self, path: &str, offset: u64, out: &mut dyn Write) -> RemoteResult<u64> {
        self.check_fail(path)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let data = self.contents(path).ok_or(RemoteError::NotFound)?;
        if offset > 0 {
            self.resumed_at.lock().unwrap().push(offset);
        }
        let start = (offset as usize).min(data.len());
        let delay = *self.delay.lock().unwrap();
        let mut at = start as u64;
        // In small blocks, like a network would.
        for chunk in data[start..].chunks(4) {
            self.moved(at)?;
            out.write_all(chunk)?;
            at += chunk.len() as u64;
            if let Some(d) = delay {
                std::thread::sleep(d);
            }
        }
        Ok((data.len() - start) as u64)
    }

    fn write_from(&self, path: &str, src: &mut dyn Read, _size: u64) -> RemoteResult<()> {
        self.store(path, Vec::new(), src)
    }

    fn can_append(&self) -> bool {
        !self.no_append.load(Ordering::Relaxed)
    }

    fn append_from(
        &self,
        path: &str,
        offset: u64,
        src: &mut dyn Read,
        _size: u64,
    ) -> RemoteResult<()> {
        self.online()?;
        let mut kept = self.contents(path).ok_or(RemoteError::NotFound)?;
        if (kept.len() as u64) < offset {
            return Err(RemoteError::Other("offset beyond the end".into()));
        }
        kept.truncate(offset as usize);
        self.resumed_at.lock().unwrap().push(offset);
        self.store(path, kept, src)
    }

    fn checksum(&self, path: &str) -> RemoteResult<Option<Checksum>> {
        self.online()?;
        if !self.checksums.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let mut data = self.contents(path).ok_or(RemoteError::NotFound)?;
        if self.bad_checksums.load(Ordering::Relaxed) {
            data.push(0);
        }
        Ok(Some(Checksum::of_bytes(HashKind::Sha256, &data)))
    }

    fn mkdir(&self, path: &str) -> RemoteResult<()> {
        self.online()?;
        let mut nodes = self.nodes.lock().unwrap();
        if nodes.contains_key(path) {
            return Err(RemoteError::Other("exists".into()));
        }
        nodes.insert(path.to_owned(), Node::Dir);
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> RemoteResult<()> {
        self.online()?;
        let mut nodes = self.nodes.lock().unwrap();
        if nodes.contains_key(to) {
            return Err(RemoteError::Other("target exists".into()));
        }
        let moved: Vec<(String, Node)> = nodes
            .iter()
            .filter(|(p, _)| *p == from || p.starts_with(&format!("{from}/")))
            .map(|(p, n)| (p.clone(), n.clone()))
            .collect();
        if moved.is_empty() {
            return Err(RemoteError::NotFound);
        }
        for (p, n) in moved {
            nodes.remove(&p);
            nodes.insert(format!("{to}{}", &p[from.len()..]), n);
        }
        Ok(())
    }

    fn remove_file(&self, path: &str) -> RemoteResult<()> {
        self.online()?;
        match self.nodes.lock().unwrap().remove(path) {
            Some(_) => Ok(()),
            None => Err(RemoteError::NotFound),
        }
    }

    fn remove_dir(&self, path: &str) -> RemoteResult<()> {
        self.online()?;
        let mut nodes = self.nodes.lock().unwrap();
        let prefix = format!("{path}/");
        if nodes.keys().any(|p| p.starts_with(&prefix)) {
            return Err(RemoteError::Other("not empty".into()));
        }
        nodes.remove(path).map(|_| ()).ok_or(RemoteError::NotFound)
    }

    fn set_modified(&self, path: &str, time: SystemTime) -> RemoteResult<()> {
        if let Some(Node::File(_, m)) = self.nodes.lock().unwrap().get_mut(path) {
            *m = Some(time);
        }
        Ok(())
    }
}
