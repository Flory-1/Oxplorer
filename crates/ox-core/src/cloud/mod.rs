//! Cloud files (OneDrive, Dropbox, … — any provider that uses Windows'
//! Cloud Files API): whether an item is only online, on this device, always
//! kept here, syncing, or has a sync problem; and *Always keep on this
//! device* / *Free up space*.
//!
//! The cheap part comes from the file attributes the directory listing
//! already has ([`from_attributes`]). Inside a sync root, [`refine`] looks at
//! the placeholder state of a whole folder in one more enumeration, which
//! tells "on this device" and "sync pending" apart.

use std::path::{Path, PathBuf};

use crate::entry::FileEntry;
use crate::i18n::t;

#[cfg(windows)]
mod win;

/// Where an item in a cloud folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloudStatus {
    /// Only a placeholder is here; opening it downloads it.
    OnlineOnly,
    /// Downloaded; the provider may free the space again.
    Local,
    /// *Always keep on this device*.
    AlwaysKeep,
    /// Waiting to be uploaded or downloaded, or being transferred.
    Syncing,
    /// The provider reports a problem with it.
    Error,
}

impl CloudStatus {
    /// Short name, as in Explorer's Status column.
    pub fn label(self) -> &'static str {
        match self {
            Self::OnlineOnly => t("Available when online"),
            Self::Local => t("Available on this device"),
            Self::AlwaysKeep => t("Always available on this device"),
            Self::Syncing => t("Sync pending"),
            Self::Error => t("Sync problem"),
        }
    }

    /// One sentence for the tooltip.
    pub fn description(self) -> &'static str {
        match self {
            Self::OnlineOnly => t(
                "Online only: takes no space on this device and is downloaded when you open it.",
            ),
            Self::Local => {
                t("On this device: downloaded. \"Free up space\" makes it online only again.")
            }
            Self::AlwaysKeep => t("Always kept on this device, even when space runs low."),
            Self::Syncing => t("Changes are waiting to be uploaded or downloaded."),
            Self::Error => t("The cloud provider can't sync it. Its app has the details."),
        }
    }

    /// Order for sorting by status: online only first, problems last.
    pub fn rank(self) -> u8 {
        match self {
            Self::OnlineOnly => 1,
            Self::Local => 2,
            Self::AlwaysKeep => 3,
            Self::Syncing => 4,
            Self::Error => 5,
        }
    }
}

/// Sort key of an entry's status (items outside cloud folders first).
pub fn sort_rank(status: Option<CloudStatus>) -> u8 {
    status.map_or(0, CloudStatus::rank)
}

// File attributes (winnt.h).
pub const OFFLINE: u32 = 0x1000;
pub const RECALL_ON_OPEN: u32 = 0x4_0000;
pub const PINNED: u32 = 0x8_0000;
pub const UNPINNED: u32 = 0x10_0000;
pub const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

// Placeholder state bits (`CF_PLACEHOLDER_STATE`).
pub const PH_PLACEHOLDER: u32 = 0x1;
pub const PH_SYNC_ROOT: u32 = 0x2;
pub const PH_IN_SYNC: u32 = 0x8;
pub const PH_PARTIAL: u32 = 0x10;
/// `CF_PLACEHOLDER_STATE_INVALID`.
pub const PH_INVALID: u32 = u32::MAX;

// `SYNC_TRANSFER_STATUS` bits of the shell's System.SyncTransferStatus.
pub const STS_HAS_ERROR: u32 = 0x10;
pub const STS_EXCLUDED: u32 = 0x100;

/// Is an item with these attributes only online (its data isn't here)?
pub fn is_online_attrs(attrs: u32, is_dir: bool) -> bool {
    if is_dir {
        attrs & (RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS | UNPINNED) != 0 && attrs & PINNED == 0
    } else {
        attrs & (RECALL_ON_DATA_ACCESS | OFFLINE) != 0
    }
}

/// Is the data of the item behind `meta` only online? Reading it would
/// download it.
pub fn online(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        is_online_attrs(meta.file_attributes(), meta.is_dir())
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// Status from the attributes alone (no placeholder state): online only and
/// always keep are visible there, "on this device" is not.
pub fn from_attributes(attrs: u32, is_dir: bool) -> Option<CloudStatus> {
    classify(attrs, is_dir, None)
}

/// Status of an item from its attributes and, inside a sync root, its
/// placeholder state (`CF_PLACEHOLDER_STATE` bits).
pub fn classify(attrs: u32, is_dir: bool, placeholder: Option<u32>) -> Option<CloudStatus> {
    let online = is_online_attrs(attrs, is_dir)
        || (!is_dir && placeholder.is_some_and(|s| s != PH_INVALID && s & PH_PARTIAL != 0));
    let pinned = attrs & PINNED != 0;
    match placeholder {
        Some(state) if state != PH_INVALID => {
            let placeholder = state & (PH_PLACEHOLDER | PH_SYNC_ROOT) != 0;
            if !placeholder || state & PH_IN_SYNC == 0 || (pinned && online) {
                Some(CloudStatus::Syncing)
            } else if pinned {
                Some(CloudStatus::AlwaysKeep)
            } else if online {
                Some(CloudStatus::OnlineOnly)
            } else {
                Some(CloudStatus::Local)
            }
        }
        _ => match (online, pinned) {
            // Asked to keep it, still downloading.
            (true, true) if !is_dir => Some(CloudStatus::Syncing),
            (true, _) => Some(CloudStatus::OnlineOnly),
            (false, true) => Some(CloudStatus::AlwaysKeep),
            (false, false) => None,
        },
    }
}

/// What the shell's System.SyncTransferStatus adds to a pending item: a
/// problem, or that the provider leaves it out on purpose.
pub fn with_transfer_status(status: Option<CloudStatus>, sts: u32) -> Option<CloudStatus> {
    if sts & STS_EXCLUDED != 0 {
        None
    } else if sts & STS_HAS_ERROR != 0 {
        Some(CloudStatus::Error)
    } else {
        status
    }
}

/// Files the providers never upload (so they'd look pending forever).
pub fn never_synced(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(lower.as_str(), "desktop.ini" | "thumbs.db" | ".ds_store")
        || lower.starts_with("~$")
        || lower.starts_with(".~lock.")
        || lower.ends_with(".tmp")
}

/// The cloud provider of the sync root `path` is in ("OneDrive"), or `None`
/// outside sync roots and on other platforms.
pub fn provider(path: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        win::provider(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// Fill in the cloud status of the entries listed from `dir` if it is in a
/// sync root, with the placeholder state of every item (one more directory
/// enumeration) and, for pending items, the provider's error state. Returns
/// the provider's name. Call it off the UI thread.
pub fn refine(dir: &Path, entries: &mut [FileEntry]) -> Option<String> {
    #[cfg(windows)]
    {
        let provider = win::provider(dir)?;
        let states = win::placeholder_states(dir)?;
        let mut pending = Vec::new();
        for (i, e) in entries.iter_mut().enumerate() {
            if e.path.parent() != Some(dir) {
                continue;
            }
            let Some(&(attrs, state)) = states.get(&e.name.to_lowercase()) else {
                continue;
            };
            e.cloud = if never_synced(&e.name) {
                None
            } else {
                classify(attrs, e.is_dir, Some(state))
            };
            if e.cloud == Some(CloudStatus::Syncing) {
                pending.push(i);
            }
        }
        // Ask the shell about a few pending items only: it is slow-ish.
        for i in pending.into_iter().take(MAX_TRANSFER_QUERIES) {
            if let Some(sts) = win::transfer_status(&entries[i].path) {
                entries[i].cloud = with_transfer_status(entries[i].cloud, sts);
            }
        }
        Some(provider)
    }
    #[cfg(not(windows))]
    {
        let _ = (dir, entries);
        None
    }
}

#[cfg(windows)]
const MAX_TRANSFER_QUERIES: usize = 64;

/// Status and provider of a single item (for the Info tab and Properties).
pub fn status_of(path: &Path) -> Option<(CloudStatus, String)> {
    #[cfg(windows)]
    {
        let provider = win::provider(path)?;
        let (attrs, state) = win::placeholder_state(path)?;
        let name = crate::entry::display_name(path);
        let is_dir = attrs & 0x10 != 0;
        let mut status = classify(attrs, is_dir, Some(state)).filter(|_| !never_synced(&name));
        if status == Some(CloudStatus::Syncing) {
            if let Some(sts) = win::transfer_status(path) {
                status = with_transfer_status(status, sts);
            }
        }
        Some((status?, provider))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// Where the user wants an item's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    /// *Always keep on this device*.
    Always,
    /// Neither: the provider decides (unticking *Always keep*).
    Default,
    /// *Free up space*: online only.
    Online,
}

/// Pin or unpin these items, folders with everything in them. Blocks while
/// the provider takes note, so call it off the UI thread. Returns the items
/// that failed, with the reason.
pub fn set_pin(paths: &[PathBuf], pin: Pin) -> Vec<(PathBuf, String)> {
    #[cfg(windows)]
    {
        paths
            .iter()
            .filter_map(|p| win::set_pin(p, pin).err().map(|e| (p.clone(), e)))
            .collect()
    }
    #[cfg(not(windows))]
    {
        let _ = pin;
        paths
            .iter()
            .map(|p| (p.clone(), t("Cloud files need Windows.").to_owned()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::CloudStatus::*;
    use super::*;

    const IN_SYNC: u32 = PH_PLACEHOLDER | PH_IN_SYNC;

    #[test]
    fn attributes_alone() {
        assert_eq!(from_attributes(0x20, false), None);
        assert_eq!(
            from_attributes(RECALL_ON_DATA_ACCESS, false),
            Some(OnlineOnly)
        );
        assert_eq!(from_attributes(OFFLINE | UNPINNED, false), Some(OnlineOnly));
        assert_eq!(from_attributes(PINNED, false), Some(AlwaysKeep));
        assert_eq!(
            from_attributes(PINNED | RECALL_ON_DATA_ACCESS, false),
            Some(Syncing)
        );
        // Folders: not populated yet, or freed up.
        assert_eq!(
            from_attributes(0x10 | RECALL_ON_OPEN, true),
            Some(OnlineOnly)
        );
        assert_eq!(from_attributes(0x10 | UNPINNED, true), Some(OnlineOnly));
        assert_eq!(from_attributes(0x10 | PINNED, true), Some(AlwaysKeep));
        assert_eq!(from_attributes(0x10, true), None);
    }

    #[test]
    fn with_placeholder_state() {
        let local = 0x20 | 0x400;
        assert_eq!(classify(local, false, Some(IN_SYNC)), Some(Local));
        assert_eq!(
            classify(local | PINNED, false, Some(IN_SYNC)),
            Some(AlwaysKeep)
        );
        assert_eq!(
            classify(
                local | RECALL_ON_DATA_ACCESS,
                false,
                Some(IN_SYNC | PH_PARTIAL)
            ),
            Some(OnlineOnly)
        );
        // Partial data without the recall attribute is still online only.
        assert_eq!(
            classify(local, false, Some(IN_SYNC | PH_PARTIAL)),
            Some(OnlineOnly)
        );
        // Not in sync, or not a placeholder yet (a new file): pending.
        assert_eq!(classify(local, false, Some(PH_PLACEHOLDER)), Some(Syncing));
        assert_eq!(classify(0x20, false, Some(0)), Some(Syncing));
        // Pinned but not downloaded yet.
        assert_eq!(
            classify(local | PINNED | RECALL_ON_DATA_ACCESS, false, Some(IN_SYNC)),
            Some(Syncing)
        );
        // The sync root itself and folders.
        assert_eq!(
            classify(0x10, true, Some(PH_SYNC_ROOT | PH_IN_SYNC)),
            Some(Local)
        );
        assert_eq!(
            classify(0x10, true, Some(IN_SYNC | PH_PARTIAL)),
            Some(Local)
        );
        // An invalid state falls back to the attributes.
        assert_eq!(classify(PINNED, false, Some(PH_INVALID)), Some(AlwaysKeep));
    }

    #[test]
    fn transfer_status_and_ignored_names() {
        assert_eq!(
            with_transfer_status(Some(Syncing), STS_HAS_ERROR),
            Some(Error)
        );
        assert_eq!(
            with_transfer_status(Some(Syncing), STS_EXCLUDED | 0x10),
            None
        );
        assert_eq!(with_transfer_status(Some(Syncing), 0x4), Some(Syncing));
        assert!(never_synced("desktop.ini"));
        assert!(never_synced("~$Report.docx"));
        assert!(never_synced("Upload.TMP"));
        assert!(!never_synced("Report.docx"));
    }

    #[test]
    fn labels_and_order() {
        let all = [OnlineOnly, Local, AlwaysKeep, Syncing, Error];
        for s in all {
            assert!(!s.label().is_empty() && !s.description().is_empty());
        }
        let mut ranks: Vec<u8> = all.iter().map(|s| s.rank()).collect();
        ranks.dedup();
        assert_eq!(ranks.len(), all.len());
        assert_eq!(sort_rank(None), 0);
    }

    #[test]
    fn plain_folders_are_not_cloud_folders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let mut entries = crate::entry::list_dir(dir.path(), true).unwrap();
        assert_eq!(entries[0].cloud, None);
        assert_eq!(refine(dir.path(), &mut entries), None);
        assert_eq!(provider(dir.path()), None);
        assert_eq!(status_of(&dir.path().join("a.txt")), None);
        assert_eq!(entries[0].cloud, None);
    }

    /// Read-only look at the real OneDrive folder, when there is one: the
    /// calls must not fail or change anything. Not required to pass anything
    /// else (OneDrive may not be set up).
    #[test]
    fn onedrive_folder_can_be_read() {
        let Some(root) = std::env::var_os("OneDrive").map(PathBuf::from) else {
            return;
        };
        let Ok(mut entries) = crate::entry::list_dir(&root, true) else {
            return;
        };
        let _ = refine(&root, &mut entries);
        let _ = status_of(&root);
    }
}
