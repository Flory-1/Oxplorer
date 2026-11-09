//! Phones and cameras on a USB cable (MTP). They have no drive letter:
//! Windows shows them under This PC through the Shell, and that is how they
//! are read here (see `mtp/win.rs`). For now they are read-only: browse,
//! open, preview and copy files off the device.
//!
//! A device's address is `mtp://<key>/<path>`: the key is made from the
//! device's Shell parsing name (long, with characters an address can't
//! hold), and [`devices`] remembers which device each key stands for. The
//! path is made of the names shown in Explorer (`/Internal storage/DCIM`);
//! [`MtpFs`] finds the Shell items behind them.

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use super::url::{normalize_path, Protocol, RemoteAddr};
use super::{RemoteEntry, RemoteError, RemoteFs, RemoteResult};

#[cfg(windows)]
mod win;

/// A connected phone or camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The host of its addresses.
    pub key: String,
    /// The name Windows shows (`Pixel 8`, `Galaxy S24`).
    pub name: String,
    /// The Shell parsing name of the device.
    parsing: String,
}

impl Device {
    /// Its top: the storages (`Internal storage`, `SD card`).
    pub fn addr(&self) -> RemoteAddr {
        RemoteAddr {
            protocol: Protocol::Mtp,
            host: self.key.clone(),
            port: Protocol::Mtp.default_port(),
            user: String::new(),
            path: "/".into(),
        }
    }
}

/// The devices seen so far, by key: addresses of devices unplugged since
/// keep their names.
static KNOWN: LazyLock<Mutex<HashMap<String, Device>>> = LazyLock::new(Default::default);

/// The key of a device: a hash of its Shell parsing name (FNV-1a, so it
/// stays the same across runs and versions).
pub fn device_key(parsing: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in parsing.to_lowercase().bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn remember(found: Vec<(String, String)>) -> Vec<Device> {
    let devices: Vec<Device> = found
        .into_iter()
        .map(|(parsing, name)| Device {
            key: device_key(&parsing),
            name,
            parsing,
        })
        .collect();
    let mut known = KNOWN.lock().unwrap();
    for d in &devices {
        known.insert(d.key.clone(), d.clone());
    }
    devices
}

/// The phones and cameras connected now. Blocks (it asks the Shell), so
/// call it on a background thread.
pub fn devices() -> Vec<Device> {
    #[cfg(windows)]
    {
        remember(win::devices())
    }
    #[cfg(not(windows))]
    {
        remember(Vec::new())
    }
}

/// The name of the device with this key, if it was seen.
pub fn device_name(key: &str) -> Option<String> {
    KNOWN.lock().unwrap().get(key).map(|d| d.name.clone())
}

/// An item in a device folder, as the Shell shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellChild {
    /// The file name (with its extension).
    pub name: String,
    /// What finds the item again ([`ShellTree::children`], [`ShellTree::read`]).
    pub parsing: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// What [`MtpFs`] needs from the Shell (a fake one in the tests).
pub trait ShellTree: Send + Sync {
    fn children(&self, parsing: &str) -> io::Result<Vec<ShellChild>>;
    /// Copy the file's content into `out`; returns the bytes written.
    fn read(&self, parsing: &str, out: &mut dyn Write) -> io::Result<u64>;
}

/// A connected device, read through the Shell.
pub struct MtpFs<T: ShellTree> {
    tree: T,
    /// The device's parsing name (the folder `/`).
    root: String,
    /// Parsing names of the items listed so far, by path.
    found: Mutex<HashMap<String, ShellChild>>,
}

/// Open the device of `addr`.
pub(crate) fn connect(addr: &RemoteAddr) -> RemoteResult<Arc<dyn RemoteFs>> {
    #[cfg(windows)]
    {
        let device = find(addr)?;
        Ok(Arc::new(MtpFs::new(win::Shell, device.parsing)))
    }
    #[cfg(not(windows))]
    {
        let _ = addr;
        Err(RemoteError::Other(
            crate::i18n::t("Phones can only be opened on Windows.").into(),
        ))
    }
}

/// The connected device an address is on.
#[cfg(windows)]
fn find(addr: &RemoteAddr) -> RemoteResult<Device> {
    let known = KNOWN.lock().unwrap().get(&addr.host).cloned();
    if let Some(d) = known {
        return Ok(d);
    }
    // Not seen in this run yet (a tab of the last session): look now.
    devices()
        .into_iter()
        .find(|d| d.key == addr.host)
        .ok_or_else(not_connected)
}

fn not_connected() -> RemoteError {
    RemoteError::Other(crate::i18n::t("The phone isn't connected (or is locked).").into())
}

fn read_only() -> RemoteError {
    RemoteError::Other(crate::i18n::t("Phones are read-only in Oxplorer for now.").into())
}

impl<T: ShellTree> MtpFs<T> {
    pub fn new(tree: T, root: String) -> Self {
        Self {
            tree,
            root,
            found: Mutex::new(HashMap::new()),
        }
    }

    /// The Shell item at `path`, looked up folder by folder (the folders
    /// on the way are listed once and remembered).
    fn resolve(&self, path: &str) -> RemoteResult<Option<ShellChild>> {
        let path = normalize_path(path);
        if path == "/" || path.is_empty() {
            return Ok(None);
        }
        if let Some(c) = self.found.lock().unwrap().get(&path) {
            return Ok(Some(c.clone()));
        }
        let mut dir = String::from("/");
        let mut parsing = self.root.clone();
        let mut item = None;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let children = self.list_children(&dir, &parsing)?;
            let child = children
                .into_iter()
                .find(|c| c.name == part)
                .ok_or(RemoteError::NotFound)?;
            dir = super::url::join_path(&dir, part);
            parsing = child.parsing.clone();
            item = Some(child);
        }
        Ok(item)
    }

    /// The parsing name of the folder at `path`.
    fn folder(&self, path: &str) -> RemoteResult<String> {
        match self.resolve(path)? {
            None => Ok(self.root.clone()),
            Some(c) if c.is_dir => Ok(c.parsing),
            Some(_) => Err(RemoteError::Other(format!("{path}: not a folder"))),
        }
    }

    /// List the folder `dir` (parsing name `parsing`), remembering its items.
    fn list_children(&self, dir: &str, parsing: &str) -> RemoteResult<Vec<ShellChild>> {
        let children = self.tree.children(parsing).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                not_connected()
            } else {
                RemoteError::from(e)
            }
        })?;
        let mut found = self.found.lock().unwrap();
        for c in &children {
            found.insert(super::url::join_path(dir, &c.name), c.clone());
        }
        Ok(children)
    }
}

impl<T: ShellTree> RemoteFs for MtpFs<T> {
    fn home(&self) -> RemoteResult<String> {
        Ok("/".into())
    }

    fn list(&self, dir: &str) -> RemoteResult<Vec<RemoteEntry>> {
        let dir = normalize_path(dir);
        let parsing = self.folder(&dir)?;
        let children = self.list_children(&dir, &parsing)?;
        Ok(children
            .into_iter()
            .map(|c| RemoteEntry {
                is_dir: c.is_dir,
                ..RemoteEntry::file(&c.name, c.size, c.modified)
            })
            .collect())
    }

    fn stat(&self, path: &str) -> RemoteResult<Option<RemoteEntry>> {
        match self.resolve(path) {
            Ok(Some(c)) => Ok(Some(RemoteEntry {
                is_dir: c.is_dir,
                ..RemoteEntry::file(&c.name, c.size, c.modified)
            })),
            Ok(None) => Ok(Some(RemoteEntry::dir("", None))),
            Err(RemoteError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn read_to(&self, path: &str, out: &mut dyn Write) -> RemoteResult<u64> {
        let item = self.resolve(path)?.ok_or(RemoteError::NotFound)?;
        if item.is_dir {
            return Err(RemoteError::Other(format!("{path}: is a folder")));
        }
        Ok(self.tree.read(&item.parsing, out)?)
    }

    fn write_from(&self, _: &str, _: &mut dyn io::Read, _: u64) -> RemoteResult<()> {
        Err(read_only())
    }

    fn mkdir(&self, _: &str) -> RemoteResult<()> {
        Err(read_only())
    }

    fn rename(&self, _: &str, _: &str) -> RemoteResult<()> {
        Err(read_only())
    }

    fn remove_file(&self, _: &str) -> RemoteResult<()> {
        Err(read_only())
    }

    fn remove_dir(&self, _: &str) -> RemoteResult<()> {
        Err(read_only())
    }

    fn upload_via_temp(&self) -> bool {
        false
    }

    /// A phone answers one request at a time.
    fn parallel(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device with `/Internal storage/DCIM/a.jpg` and `/Internal storage/notes.txt`.
    struct Fake;

    fn child(name: &str, parsing: &str, is_dir: bool, size: u64) -> ShellChild {
        ShellChild {
            name: name.into(),
            parsing: parsing.into(),
            is_dir,
            size,
            modified: None,
        }
    }

    impl ShellTree for Fake {
        fn children(&self, parsing: &str) -> io::Result<Vec<ShellChild>> {
            Ok(match parsing {
                "dev" => vec![child("Internal storage", "dev\\s1", true, 0)],
                "dev\\s1" => vec![
                    child("DCIM", "dev\\s1\\d", true, 0),
                    child("notes.txt", "dev\\s1\\n", false, 5),
                ],
                "dev\\s1\\d" => vec![child("a.jpg", "dev\\s1\\d\\a", false, 3)],
                "gone" => return Err(io::ErrorKind::NotFound.into()),
                _ => Vec::new(),
            })
        }

        fn read(&self, parsing: &str, out: &mut dyn Write) -> io::Result<u64> {
            let data: &[u8] = match parsing {
                "dev\\s1\\n" => b"hello",
                "dev\\s1\\d\\a" => b"jpg",
                _ => return Err(io::ErrorKind::NotFound.into()),
            };
            out.write_all(data)?;
            Ok(data.len() as u64)
        }
    }

    #[test]
    fn keys_are_stable_and_ignore_case() {
        let key = device_key(r"::{20D04FE0}\\?\usb#vid_18d1&pid_4ee1#abc");
        assert_eq!(key.len(), 16);
        assert_eq!(
            key,
            device_key(r"::{20d04fe0}\\?\USB#VID_18D1&PID_4EE1#ABC")
        );
        assert_ne!(
            key,
            device_key(r"::{20D04FE0}\\?\usb#vid_18d1&pid_4ee1#abd")
        );
        let d = Device {
            key: key.clone(),
            name: "Pixel".into(),
            parsing: String::new(),
        };
        let addr = d.addr();
        assert_eq!(addr.to_url(), format!("mtp://{key}/"));
        assert_eq!(RemoteAddr::parse(&addr.to_url()), Some(addr.clone()));
        assert!(addr.is_phone());
        remember(vec![("x".into(), "Pixel 8".into())]);
        assert_eq!(device_name(&device_key("x")).as_deref(), Some("Pixel 8"));
    }

    #[test]
    fn lists_and_reads_by_path() {
        let fs = MtpFs::new(Fake, "dev".into());
        assert_eq!(fs.home().unwrap(), "/");
        let top = fs.list("/").unwrap();
        assert_eq!(top.len(), 1);
        assert!(top[0].is_dir);
        let names: Vec<_> = fs
            .list("/Internal storage")
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir, e.size))
            .collect();
        assert_eq!(
            names,
            vec![("DCIM".into(), true, 0), ("notes.txt".into(), false, 5)]
        );
        // Found without listing the folders on the way first.
        let fresh = MtpFs::new(Fake, "dev".into());
        let mut out = Vec::new();
        assert_eq!(
            fresh
                .read_to("/Internal storage/DCIM/a.jpg", &mut out)
                .unwrap(),
            3
        );
        assert_eq!(out, b"jpg");
        assert_eq!(
            fresh
                .stat("/Internal storage/notes.txt")
                .unwrap()
                .unwrap()
                .size,
            5
        );
        assert_eq!(fresh.stat("/Internal storage/nope").unwrap(), None);
        assert_eq!(
            fresh.list("/Internal storage/nope"),
            Err(RemoteError::NotFound)
        );
    }

    #[test]
    fn writing_is_refused_and_unplugged_devices_say_so() {
        let fs = MtpFs::new(Fake, "dev".into());
        assert!(fs.mkdir("/Internal storage/new").is_err());
        assert!(fs.remove_file("/Internal storage/notes.txt").is_err());
        assert!(fs
            .write_from("/Internal storage/x", &mut io::empty(), 0)
            .is_err());
        let gone = MtpFs::new(Fake, "gone".into());
        assert_eq!(gone.list("/"), Err(not_connected()));
    }
}
