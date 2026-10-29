//! Windows only: what the index service reads, straight from the volumes.
//! Only drive letters of fixed NTFS volumes are ever opened (checked again
//! for every request), only `FSCTL_ENUM_USN_DATA`, the `$MFT` records'
//! sizes and dates and the change journal are read, and no handle outlives
//! the request.

use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use super::proto::{EnumRecord, ErrorCode, JournalInfo, JournalRecord};
use super::server::Volumes;
use crate::index::catchup::ReadError;
use crate::index::journal::{query_journal, read_chunk};
use crate::index::mft::{enumerate_with, fixed_ntfs_drives, open_volume, read_metadata, Handle};
use crate::index::tree::record_number;

/// The volumes of this computer.
#[derive(Default)]
pub struct LocalVolumes {
    /// One enumeration at a time (each reads a whole master file table).
    enumerating: Mutex<()>,
}

impl LocalVolumes {
    pub fn new() -> Self {
        Self::default()
    }
}

fn open(drive: u8) -> Result<Handle, ErrorCode> {
    // Checked by `serve` already; checked again so nothing else is ever opened.
    if !fixed_ntfs_drives().contains(&drive) {
        return Err(ErrorCode::NotServed);
    }
    open_volume(drive as char).ok_or(ErrorCode::Unreadable)
}

impl Volumes for LocalVolumes {
    fn served(&self) -> Vec<u8> {
        fixed_ntfs_drives()
    }

    fn enumerate(
        &self,
        drive: u8,
        sink: &mut dyn FnMut(Vec<EnumRecord>) -> io::Result<()>,
    ) -> Result<u64, ErrorCode> {
        let _one = self.enumerating.lock().unwrap_or_else(|e| e.into_inner());
        let vol = open(drive)?;
        let never = AtomicBool::new(false);
        let meta = read_metadata(&vol, &never).ok_or(ErrorCode::Unreadable)?;
        let mut total = 0u64;
        let listed = enumerate_with(&vol, &never, |records| {
            let out: Vec<EnumRecord> = records
                .into_iter()
                .map(|r| {
                    let (size, mtime) = meta
                        .get(record_number(r.frn) as usize)
                        .copied()
                        .unwrap_or_default();
                    EnumRecord {
                        frn: r.frn,
                        parent: r.parent,
                        attrs: r.attrs,
                        size,
                        mtime,
                        name: r.name,
                    }
                })
                .collect();
            total += out.len() as u64;
            sink(out).is_ok()
        });
        if listed {
            Ok(total)
        } else {
            Err(ErrorCode::Unreadable)
        }
    }

    fn query_journal(&self, drive: u8) -> Result<JournalInfo, ErrorCode> {
        query_journal(&open(drive)?).ok_or(ErrorCode::Unreadable)
    }

    fn read_journal(
        &self,
        drive: u8,
        journal: u64,
        start: i64,
    ) -> Result<(i64, Vec<JournalRecord>), ErrorCode> {
        let vol = open(drive)?;
        let (next, records) = read_chunk(&vol, journal, start).map_err(|e| match e {
            ReadError::Gone => ErrorCode::JournalGone,
            ReadError::Failed => ErrorCode::Unreadable,
        })?;
        let records = records
            .into_iter()
            .map(|r| JournalRecord {
                frn: r.frn,
                parent: r.parent,
                usn: r.usn,
                reason: r.reason,
                attrs: r.attrs,
                name: r.name,
            })
            .collect();
        Ok((next, records))
    }
}
