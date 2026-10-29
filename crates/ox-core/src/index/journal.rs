//! Windows only: reading the NTFS change journal, directly (as
//! administrator) or through the index service, to follow changes live and
//! to catch up on them after a restart (`catchup`).

use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Sender;
use windows_sys::Win32::Foundation::{
    ERROR_JOURNAL_DELETE_IN_PROGRESS, ERROR_JOURNAL_ENTRY_DELETED, ERROR_JOURNAL_NOT_ACTIVE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileIdType, GetFinalPathNameByHandleW, OpenFileById, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0,
};

use super::catchup::{start_at, JournalSource, ReadError};
use super::checkpoint::{Checkpoint, JournalState};
use super::helper::client::{Client, ClientError};
use super::helper::proto::{ErrorCode, JournalInfo};
use super::helper::{self};
use super::mft::{drive_letter, ioctl, on_drive_volume, open_volume, wide, Buf, Handle, SHARE_ALL};
use super::patch::Signal;
use super::usn::{next_value, parse_records, prefixes, signals_for, Dirs, UsnRecord};
use super::volume::{first, Route};

/// The journal's id and its first and next USN.
pub(super) fn query_journal(vol: &Handle) -> Option<JournalInfo> {
    let mut buf = Buf::new(256);
    let n = ioctl::<()>(vol, FSCTL_QUERY_USN_JOURNAL, None, buf.get()).ok()?;
    if n < size_of::<USN_JOURNAL_DATA_V0>() {
        return None;
    }
    // SAFETY: the buffer holds at least one USN_JOURNAL_DATA_V0 (checked above).
    let j: USN_JOURNAL_DATA_V0 = unsafe { std::ptr::read_unaligned(buf.get().as_ptr().cast()) };
    Some(JournalInfo {
        journal: j.UsnJournalID,
        first: j.FirstUsn,
        next: j.NextUsn,
    })
}

/// One read of the journal from `start` (up to 64 KB of records), checked
/// against the current journal first.
pub(super) fn read_chunk(
    vol: &Handle,
    journal: u64,
    start: i64,
) -> Result<(i64, Vec<UsnRecord>), ReadError> {
    let info = query_journal(vol).ok_or(ReadError::Failed)?;
    if info.journal != journal || start < info.first || start > info.next {
        return Err(ReadError::Gone);
    }
    let input = READ_USN_JOURNAL_DATA_V0 {
        StartUsn: start,
        ReasonMask: u32::MAX,
        ReturnOnlyOnClose: 0,
        Timeout: 0,
        BytesToWaitFor: 0,
        UsnJournalID: journal,
    };
    let mut buf = Buf::new(64 * 1024);
    let out = buf.get();
    match ioctl(vol, FSCTL_READ_USN_JOURNAL, Some(&input), out) {
        Ok(n) if n >= 8 => {
            let next = next_value(out).map_or(start, |v| v as i64);
            Ok((next.max(start), parse_records(&out[8..n])))
        }
        Ok(_) => Ok((start, Vec::new())),
        Err(
            ERROR_JOURNAL_ENTRY_DELETED
            | ERROR_JOURNAL_DELETE_IN_PROGRESS
            | ERROR_JOURNAL_NOT_ACTIVE,
        ) => Err(ReadError::Gone),
        Err(_) => Err(ReadError::Failed),
    }
}

/// The current path of a file reference. `hint` is any handle on the volume
/// (the volume itself, or its root folder).
fn path_of_id(hint: &Handle, frn: u64) -> Option<PathBuf> {
    let desc = FILE_ID_DESCRIPTOR {
        dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32,
        Type: FileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: frn as i64 },
    };
    // SAFETY: `hint` is open and `desc` is a valid descriptor.
    let h = unsafe {
        OpenFileById(
            hint.0,
            &desc,
            0,
            SHARE_ALL,
            std::ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let h = Handle(h);
    let mut buf = vec![0u16; 1024];
    loop {
        // SAFETY: `buf` has room for `buf.len()` characters.
        let n = unsafe { GetFinalPathNameByHandleW(h.0, buf.as_mut_ptr(), buf.len() as u32, 0) }
            as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            let s = String::from_utf16_lossy(&buf[..n]);
            let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
            return (!s.starts_with(r"UNC\")).then(|| PathBuf::from(s));
        }
        buf.resize(n + 1, 0);
    }
}

/// The root folder of a drive, opened without any access (enough to look
/// up file references on it).
fn open_root(letter: char) -> Option<Handle> {
    let root = wide(format!("{letter}:\\"));
    // SAFETY: `root` is NUL-terminated; no access is requested.
    let h = unsafe {
        CreateFileW(
            root.as_ptr(),
            0,
            SHARE_ALL,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE).then_some(Handle(h))
}

/// A drive's change journal.
pub(super) enum Journal {
    Direct(Handle),
    Service {
        letter: char,
        client: Client<File>,
        /// For looking up paths (as the user: folders they can't open are skipped).
        root: Handle,
    },
}

/// The change journal of drive `letter`, through the first route that works.
pub(super) fn open(letter: char) -> Option<Journal> {
    let (j, route) = first(helper::enabled(), |route| {
        let mut j = match route {
            Route::Service => Journal::Service {
                letter,
                client: helper::connect()?,
                root: open_root(letter)?,
            },
            Route::Direct => Journal::Direct(open_volume(letter)?),
        };
        j.info().map(|_| j)
    })?;
    log::debug!("index: reading the change journal of {letter}: ({route:?})");
    Some(j)
}

impl JournalSource for Journal {
    fn info(&mut self) -> Option<JournalInfo> {
        match self {
            Journal::Direct(vol) => query_journal(vol),
            Journal::Service { letter, client, .. } => client.query_journal(*letter).ok(),
        }
    }

    fn read(&mut self, journal: u64, start: i64) -> Result<(i64, Vec<UsnRecord>), ReadError> {
        match self {
            Journal::Direct(vol) => read_chunk(vol, journal, start),
            Journal::Service { letter, client, .. } => {
                match client.read_journal(*letter, journal, start) {
                    Ok((next, records)) => Ok((
                        next,
                        records
                            .into_iter()
                            .map(|r| UsnRecord {
                                frn: r.frn,
                                parent: r.parent,
                                usn: r.usn,
                                reason: r.reason,
                                attrs: r.attrs,
                                name: r.name,
                            })
                            .collect(),
                    )),
                    Err(ClientError::Service(ErrorCode::JournalGone)) => Err(ReadError::Gone),
                    Err(_) => Err(ReadError::Failed),
                }
            }
        }
    }

    fn path_of_id(&mut self, frn: u64) -> Option<PathBuf> {
        match self {
            Journal::Direct(vol) => path_of_id(vol, frn),
            Journal::Service { root, .. } => path_of_id(root, frn),
        }
    }
}

/// The roots on drive letters (not on volumes mounted in folders), by
/// drive, and the rest.
pub(super) fn group_by_drive(roots: &[PathBuf]) -> (Vec<(char, Vec<PathBuf>)>, Vec<PathBuf>) {
    let mut by_drive: Vec<(char, Vec<PathBuf>)> = Vec::new();
    let mut rest = Vec::new();
    for root in roots {
        match drive_letter(root).filter(|&l| on_drive_volume(root, l)) {
            Some(l) => match by_drive.iter_mut().find(|(d, _)| *d == l) {
                Some((_, v)) => v.push(root.clone()),
                None => by_drive.push((l, vec![root.clone()])),
            },
            None => rest.push(root.clone()),
        }
    }
    (by_drive, rest)
}

/// Where the journals of the drives holding `roots` are now (taken before
/// a build: the changes after it are caught up on next time).
pub(super) fn positions(roots: &[PathBuf]) -> Vec<Checkpoint> {
    group_by_drive(roots)
        .0
        .into_iter()
        .filter_map(|(letter, _)| {
            let info = open(letter)?.info()?;
            Some(Checkpoint {
                drive: letter,
                journal: info.journal,
                usn: info.next,
            })
        })
        .collect()
}

/// Follow the change journals of the drives holding `roots`, sending what
/// changes to `tx` until `stop` is set; each drive starts at its checkpoint
/// in `state` when that is still good. Returns how many drives are followed
/// and the roots that need a file watcher instead.
pub(super) fn follow_journals(
    roots: &[PathBuf],
    tx: &Sender<Signal>,
    stop: &Arc<AtomicBool>,
    state: &JournalState,
) -> (usize, Vec<PathBuf>) {
    let (by_drive, mut rest) = group_by_drive(roots);
    let mut followed = 0;
    for (letter, roots) in by_drive {
        let opened = open(letter).and_then(|mut j| j.info().map(|i| (j, i)));
        let Some((journal, info)) = opened else {
            rest.extend(roots);
            continue;
        };
        let start = start_at(&info, state.get(letter).as_ref());
        let (tx, stop) = (tx.clone(), stop.clone());
        let spawned = std::thread::Builder::new()
            .name("index-journal".into())
            .spawn(move || {
                let at = Position {
                    letter,
                    id: info.journal,
                    start,
                };
                read_journal(journal, at, &roots, &tx, &stop)
            });
        if spawned.is_ok() {
            followed += 1;
        }
    }
    (followed, rest)
}

struct Position {
    letter: char,
    id: u64,
    start: i64,
}

fn read_journal(
    mut journal: Journal,
    mut at: Position,
    roots: &[PathBuf],
    tx: &Sender<Signal>,
    stop: &AtomicBool,
) {
    let prefixes = prefixes(roots);
    let mut dirs = Dirs::default();
    while !stop.load(Ordering::Relaxed) {
        let mut busy = false;
        match journal.read(at.id, at.start) {
            Ok((next, records)) => {
                let signals = signals_for(
                    &records,
                    &mut dirs,
                    &mut |frn| journal.path_of_id(frn),
                    &prefixes,
                );
                for s in signals {
                    if tx.send(s).is_err() {
                        return;
                    }
                }
                if next != at.start {
                    at.start = next;
                    let cp = Checkpoint {
                        drive: at.letter,
                        journal: at.id,
                        usn: next,
                    };
                    if tx.send(Signal::Checkpoint(cp)).is_err() {
                        return;
                    }
                }
                // Many records: more are probably waiting.
                busy = records.len() >= 200;
            }
            Err(ReadError::Gone) => {
                // Changes were lost (the journal wrapped or was recreated).
                if tx.send(Signal::Rescan).is_err() {
                    return;
                }
                match journal.info() {
                    Some(i) => (at.id, at.start) = (i.journal, i.next),
                    None => return,
                }
                dirs = Dirs::default();
            }
            Err(ReadError::Failed) => {
                // The service went away (stopped, restarted): connect again.
                if matches!(journal, Journal::Service { .. }) {
                    std::thread::sleep(Duration::from_secs(2));
                    if let Some(j) = open(at.letter) {
                        journal = j;
                    }
                }
            }
        }
        if !busy {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}
