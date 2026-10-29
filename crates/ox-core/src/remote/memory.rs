//! An in-memory remote file system for tests.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

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

    fn check_fail(&self, path: &str) -> RemoteResult<()> {
        if let Some(bad) = self.fail_on.lock().unwrap().as_deref() {
            if path.contains(bad) {
                self.failures.fetch_add(1, Ordering::Relaxed);
                return Err(RemoteError::Other("simulated failure".into()));
            }
        }
        Ok(())
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
        let path = normalize_path(path);
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .get(&path)
            .map(|n| Self::entry(&path, n)))
    }

    fn read_to(&self, path: &str, out: &mut dyn Write) -> RemoteResult<u64> {
        self.check_fail(path)?;
        let data = self.contents(path).ok_or(RemoteError::NotFound)?;
        // In small blocks, like a network would.
        for chunk in data.chunks(4) {
            out.write_all(chunk)?;
        }
        Ok(data.len() as u64)
    }

    fn write_from(&self, path: &str, src: &mut dyn Read, _size: u64) -> RemoteResult<()> {
        self.check_fail(path)?;
        let parent = parent_path(path).unwrap_or_else(|| "/".into());
        if parent != "/" && !matches!(self.nodes.lock().unwrap().get(&parent), Some(Node::Dir)) {
            return Err(RemoteError::NotFound);
        }
        let mut data = Vec::new();
        src.read_to_end(&mut data)?;
        self.nodes
            .lock()
            .unwrap()
            .insert(path.to_owned(), Node::File(data, None));
        Ok(())
    }

    fn mkdir(&self, path: &str) -> RemoteResult<()> {
        let mut nodes = self.nodes.lock().unwrap();
        if nodes.contains_key(path) {
            return Err(RemoteError::Other("exists".into()));
        }
        nodes.insert(path.to_owned(), Node::Dir);
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> RemoteResult<()> {
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
        match self.nodes.lock().unwrap().remove(path) {
            Some(_) => Ok(()),
            None => Err(RemoteError::NotFound),
        }
    }

    fn remove_dir(&self, path: &str) -> RemoteResult<()> {
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
