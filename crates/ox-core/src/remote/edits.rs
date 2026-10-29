//! Remote files opened with their app: the downloaded copies, noticing
//! when one was saved, and uploading it again (after checking that the
//! server's file didn't change meanwhile).
//!
//! Each copy gets a folder of its own in the temp folder
//! (`Oxplorer remote/edit/<hash of the address>/<name>`), so files with
//! the same name from different folders or servers don't collide.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::{url, RemoteAddr, RemoteEntry, RemoteError, RemoteFs};

/// How long a saved file must stay unchanged before it is offered for
/// uploading (editors may write it in several steps).
pub const SETTLE: Duration = Duration::from_millis(800);

/// Copies older than this are removed (at start-up).
pub const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The temp folder of remote copies (previews and opened files).
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join("Oxplorer remote")
}

/// Where a remote file opened with its app is downloaded.
pub fn copy_path(addr: &RemoteAddr) -> PathBuf {
    let hash = xxhash_rust::xxh3::xxh3_64(addr.to_url().as_bytes());
    temp_root()
        .join("edit")
        .join(format!("{hash:016x}"))
        .join(super::sanitize(&addr.name()))
}

/// Size and date of a file (here or on the server).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Stamp {
    pub fn of_local(path: &Path) -> Option<Self> {
        let m = std::fs::metadata(path).ok()?;
        Some(Self {
            size: m.len(),
            modified: m.modified().ok(),
        })
    }

    pub fn of_entry(e: &RemoteEntry) -> Self {
        Self {
            size: e.size,
            modified: e.modified,
        }
    }

    /// The same version: same size, and the same date where both have one
    /// (servers round dates differently, so whole seconds count).
    pub fn same_as(&self, other: &Stamp) -> bool {
        let secs = |t: SystemTime| {
            t.duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .ok()
        };
        self.size == other.size
            && match (self.modified, other.modified) {
                (Some(a), Some(b)) => secs(a) == secs(b),
                _ => true,
            }
    }
}

/// An opened remote file.
#[derive(Debug, Clone)]
pub struct EditedFile {
    pub local: PathBuf,
    pub addr: RemoteAddr,
    /// The server's file as it was downloaded (or last uploaded).
    pub remote: Stamp,
    /// The copy as it was downloaded, uploaded or last looked at.
    pub seen: Option<Stamp>,
    /// Upload changes without asking (this session).
    pub always: bool,
    /// An upload is running.
    pub busy: bool,
    /// A change seen, and since when it stayed as it is.
    settling: Option<(Stamp, Instant)>,
}

impl EditedFile {
    /// The copy has changes that aren't on the server.
    pub fn has_changes(&self) -> bool {
        Stamp::of_local(&self.local).is_some_and(|now| Some(now) != self.seen)
    }
}

/// The opened remote files of this session.
#[derive(Debug, Default)]
pub struct EditedFiles {
    files: Vec<EditedFile>,
}

impl EditedFiles {
    /// Follow a downloaded copy; `remote`: the server's file it came from.
    pub fn track(&mut self, local: PathBuf, addr: RemoteAddr, remote: Stamp) {
        self.files.retain(|f| f.local != local);
        self.files.push(EditedFile {
            seen: Stamp::of_local(&local),
            local,
            addr,
            remote,
            always: false,
            busy: false,
            settling: None,
        });
    }

    pub fn get(&self, local: &Path) -> Option<&EditedFile> {
        self.files.iter().find(|f| f.local == local)
    }

    pub fn get_mut(&mut self, local: &Path) -> Option<&mut EditedFile> {
        self.files.iter_mut().find(|f| f.local == local)
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// A change was seen that hasn't settled yet (look again soon).
    pub fn settling(&self) -> bool {
        self.files.iter().any(|f| f.settling.is_some())
    }

    /// Copies that were saved and then stayed unchanged for [`SETTLE`]
    /// (call now and then; `now` is the current time).
    pub fn saved(&mut self, now: Instant) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for f in self.files.iter_mut().filter(|f| !f.busy) {
            let current = Stamp::of_local(&f.local);
            match current {
                Some(c) if Some(c) != f.seen => match f.settling {
                    Some((s, since)) if s == c => {
                        if now.duration_since(since) >= SETTLE {
                            out.push(f.local.clone());
                        }
                    }
                    _ => f.settling = Some((c, now)),
                },
                _ => f.settling = None,
            }
        }
        out
    }

    /// The current state of the copy is known (ignored or uploaded): only
    /// later changes count.
    pub fn mark_seen(&mut self, local: &Path) {
        if let Some(f) = self.get_mut(local) {
            f.seen = Stamp::of_local(&f.local);
            f.settling = None;
        }
    }

    /// An upload finished: the server now has `uploaded` (the copy's state
    /// that was sent) as `remote`.
    pub fn uploaded(&mut self, local: &Path, uploaded: Option<Stamp>, remote: Stamp) {
        if let Some(f) = self.get_mut(local) {
            f.busy = false;
            f.seen = uploaded;
            f.remote = remote;
            f.settling = None;
        }
    }

    /// Remove the copies without changes that still need uploading (when
    /// the app closes). Copies with changes stay until [`sweep`] removes
    /// them after a day.
    pub fn clean_up(&mut self) {
        for f in self.files.drain(..) {
            if f.has_changes() {
                continue;
            }
            if let Some(dir) = f.local.parent().filter(|d| d.starts_with(temp_root())) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// Why an edited copy wasn't uploaded.
#[derive(Debug, Clone, PartialEq)]
pub enum UploadError {
    /// The server's file changed since it was downloaded (`None`: it is
    /// gone). Upload with `force` to replace it anyway.
    Conflict(Option<RemoteEntry>),
    Failed(String),
}

impl From<RemoteError> for UploadError {
    fn from(e: RemoteError) -> Self {
        Self::Failed(e.to_string())
    }
}

/// Upload an edited copy to its server. Unless `force`, the server's file
/// must still be `expected` (the version that was downloaded). Returns the
/// state of the copy that was sent and of the server's new file.
pub fn upload(
    fs: &dyn RemoteFs,
    local: &Path,
    addr: &RemoteAddr,
    expected: Stamp,
    force: bool,
) -> Result<(Option<Stamp>, Stamp), UploadError> {
    let now = fs.stat(&addr.path)?;
    if !force {
        match &now {
            Some(e) if Stamp::of_entry(e).same_as(&expected) => {}
            other => return Err(UploadError::Conflict(other.clone())),
        }
    }
    let sent = Stamp::of_local(local);
    let size = sent.map_or(0, |s| s.size);
    let mut file = std::fs::File::open(local).map_err(|e| UploadError::Failed(e.to_string()))?;
    if fs.upload_via_temp() {
        let parent = url::parent_path(&addr.path).unwrap_or_else(|| "/".into());
        let tmp = url::join_path(&parent, &format!(".{}.oxpart", addr.name()));
        if let Err(e) = fs.write_from(&tmp, &mut file, size) {
            let _ = fs.remove_file(&tmp);
            return Err(e.into());
        }
        if now.is_some() {
            fs.remove_file(&addr.path)?;
        }
        fs.rename(&tmp, &addr.path)?;
    } else {
        fs.write_from(&addr.path, &mut file, size)?;
    }
    let remote = fs
        .stat(&addr.path)?
        .map(|e| Stamp::of_entry(&e))
        .unwrap_or(Stamp {
            size,
            modified: None,
        });
    Ok((sent, remote))
}

/// Remove what is older than `max_age` in the temp folder of remote
/// copies (left over from earlier sessions).
pub fn sweep(root: &Path, max_age: Duration) {
    let Ok(now) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
        return;
    };
    let old = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .is_some_and(|t| now.saturating_sub(t) > max_age)
    };
    let Ok(groups) = std::fs::read_dir(root) else {
        return;
    };
    // `edit/<hash>/…`, `preview/<n>/…`, older `<host>/…`: each folder
    // below the first level holds one copy.
    for group in groups.flatten().map(|e| e.path()) {
        let Ok(items) = std::fs::read_dir(&group) else {
            if old(&group) {
                let _ = std::fs::remove_file(&group);
            }
            continue;
        };
        for item in items.flatten().map(|e| e.path()) {
            let newest_inside = std::fs::read_dir(&item)
                .map(|d| d.flatten().all(|e| old(&e.path())))
                .unwrap_or(true);
            if old(&item) && newest_inside {
                let _ = if item.is_dir() {
                    std::fs::remove_dir_all(&item)
                } else {
                    std::fs::remove_file(&item)
                };
            }
        }
        let _ = std::fs::remove_dir(&group); // only when empty
    }
}

#[cfg(test)]
mod tests {
    use super::super::memory::MemoryFs;
    use super::*;

    fn addr() -> RemoteAddr {
        RemoteAddr::parse("sftp://me@h/srv/notes.txt").unwrap()
    }

    #[test]
    fn copies_get_a_folder_of_their_own() {
        let a = copy_path(&addr());
        let b = copy_path(&RemoteAddr::parse("sftp://me@h/other/notes.txt").unwrap());
        assert_ne!(a, b);
        assert_eq!(a.file_name(), b.file_name());
        assert!(a.starts_with(temp_root().join("edit")));
    }

    #[test]
    fn saved_copies_are_offered_once_they_settle() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("notes.txt");
        std::fs::write(&local, b"v1").unwrap();
        let mut files = EditedFiles::default();
        let remote = Stamp {
            size: 2,
            modified: None,
        };
        files.track(local.clone(), addr(), remote);
        let t0 = Instant::now();
        assert!(files.saved(t0).is_empty());
        std::fs::write(&local, b"v2 longer").unwrap();
        // Seen, but not settled yet.
        assert!(files.saved(t0).is_empty());
        assert!(files.saved(t0 + SETTLE / 2).is_empty());
        assert_eq!(files.saved(t0 + SETTLE), std::slice::from_ref(&local));
        assert!(files.get(&local).unwrap().has_changes());
        // Ignored: not offered again until the next change.
        files.mark_seen(&local);
        assert!(files.saved(t0 + SETTLE * 3).is_empty());
        assert!(!files.get(&local).unwrap().has_changes());
    }

    #[test]
    fn uploads_check_for_changes_on_the_server() {
        let fs = MemoryFs::default();
        fs.add_file("/srv/notes.txt", b"v1");
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("notes.txt");
        std::fs::write(&local, b"edited").unwrap();
        let expected = Stamp::of_entry(&fs.stat("/srv/notes.txt").unwrap().unwrap());

        // Someone else changed it meanwhile: asked first.
        let later = SystemTime::UNIX_EPOCH + Duration::from_secs(5000);
        fs.change_file("/srv/notes.txt", b"theirs", later);
        match upload(&fs, &local, &addr(), expected, false) {
            Err(UploadError::Conflict(Some(e))) => assert_eq!(e.size, 6),
            other => panic!("{other:?}"),
        }
        assert_eq!(fs.contents("/srv/notes.txt").unwrap(), b"theirs");

        // Replaced when the user says so.
        let (sent, remote) = upload(&fs, &local, &addr(), expected, true).unwrap();
        assert_eq!(fs.contents("/srv/notes.txt").unwrap(), b"edited");
        assert_eq!(sent.unwrap().size, 6);
        assert_eq!(remote.size, 6);
        assert!(fs.paths().iter().all(|p| !p.ends_with(".oxpart")));

        // Unchanged since the last upload: no question.
        std::fs::write(&local, b"edited again").unwrap();
        upload(&fs, &local, &addr(), remote, false).unwrap();
        assert_eq!(fs.contents("/srv/notes.txt").unwrap(), b"edited again");

        // Deleted on the server: a conflict too.
        fs.remove_file("/srv/notes.txt").unwrap();
        assert_eq!(
            upload(&fs, &local, &addr(), remote, false),
            Err(UploadError::Conflict(None))
        );
    }

    #[test]
    fn old_copies_are_swept() {
        let root = tempfile::tempdir().unwrap();
        let copy = root.path().join("edit").join("abc").join("a.txt");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, b"x").unwrap();
        sweep(root.path(), MAX_AGE);
        assert!(copy.exists(), "new copies stay");
        sweep(root.path(), Duration::ZERO);
        std::thread::sleep(Duration::from_millis(20));
        sweep(root.path(), Duration::ZERO);
        assert!(!root.path().join("edit").exists());
    }
}
