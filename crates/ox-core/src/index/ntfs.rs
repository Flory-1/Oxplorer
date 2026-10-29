//! Reading sizes and dates from raw NTFS file records. The master file table
//! listing (`usn`) only has names and parents; sizes and dates come from
//! reading the `$MFT` file itself, record by record.

const ATTR_STANDARD_INFORMATION: u32 = 0x10;
const ATTR_DATA: u32 = 0x80;
const ATTR_END: u32 = 0xFFFF_FFFF;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// Undo the update sequence protection of a record read from disk: the last
/// two bytes of every sector hold a check value, and their real contents are
/// kept in the record header. Returns false for torn or damaged records.
pub(super) fn apply_fixup(rec: &mut [u8]) -> bool {
    let (Some(off), Some(count)) = (u16_at(rec, 4), u16_at(rec, 6)) else {
        return false;
    };
    let (off, count) = (off as usize, count as usize);
    if count < 2 || off + count * 2 > rec.len() || !rec.len().is_multiple_of(count - 1) {
        return false;
    }
    let stride = rec.len() / (count - 1);
    if stride < 2 {
        return false;
    }
    let check = [rec[off], rec[off + 1]];
    for i in 1..count {
        let end = i * stride;
        if rec[end - 2..end] != check {
            return false;
        }
        rec[end - 2] = rec[off + 2 * i];
        rec[end - 1] = rec[off + 2 * i + 1];
    }
    true
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct FileRecord {
    /// For extension records: the record number of the base record.
    pub base: Option<u64>,
    /// Last write time (FILETIME).
    pub modified: Option<u64>,
    /// Size of the unnamed data stream.
    pub size: Option<u64>,
}

/// The attributes of a record: (type, bytes).
fn attributes(rec: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    let used = u32_at(rec, 0x18).map_or(0, |u| (u as usize).min(rec.len()));
    let mut at = match u16_at(rec, 0x14) {
        Some(first) if first >= 0x28 => first as usize,
        _ => usize::MAX,
    };
    std::iter::from_fn(move || {
        let ty = u32_at(rec, at)?;
        if ty == ATTR_END {
            return None;
        }
        let len = u32_at(rec, at + 4)? as usize;
        if len < 0x18 || at + len > used {
            return None;
        }
        let a = &rec[at..at + len];
        at += len;
        Some((ty, a))
    })
}

/// Size and date from a (fixed-up) file record; `None` if it isn't a file
/// record in use.
pub(super) fn parse_file_record(rec: &[u8]) -> Option<FileRecord> {
    if rec.get(0..4)? != b"FILE" || u16_at(rec, 0x16)? & 1 == 0 {
        return None;
    }
    let base = u64_at(rec, 0x20)? & 0xFFFF_FFFF_FFFF;
    let mut out = FileRecord {
        base: (base != 0).then_some(base),
        ..Default::default()
    };
    for (ty, a) in attributes(rec) {
        let resident = a[8] == 0;
        let unnamed = a[9] == 0;
        match ty {
            ATTR_STANDARD_INFORMATION if resident => {
                let value = u16_at(a, 0x14)? as usize;
                out.modified = u64_at(a, value + 8);
            }
            ATTR_DATA if unnamed && resident => out.size = u32_at(a, 0x10).map(u64::from),
            // Only the first piece of a split stream carries its size.
            ATTR_DATA if unnamed && u64_at(a, 0x10) == Some(0) => out.size = u64_at(a, 0x30),
            _ => {}
        }
    }
    Some(out)
}

/// Where the unnamed data stream lies on disk: (first cluster, cluster count)
/// per piece. Used on record 0 to find the `$MFT` itself.
pub(super) fn data_runs(rec: &[u8]) -> Option<Vec<(u64, u64)>> {
    let (_, a) = attributes(rec).find(|(ty, a)| *ty == ATTR_DATA && a[8] == 1 && a[9] == 0)?;
    let off = u16_at(a, 0x20)? as usize;
    parse_runs(a.get(off..)?)
}

fn le_uint(b: &[u8]) -> u64 {
    b.iter().rev().fold(0, |v, &x| v << 8 | x as u64)
}

fn le_int(b: &[u8]) -> i64 {
    let v = le_uint(b);
    let bits = b.len() * 8;
    if bits < 64 && v & (1 << (bits - 1)) != 0 {
        (v | (!0u64 << bits)) as i64
    } else {
        v as i64
    }
}

/// Decode a run list: each run is a header byte (sizes of the two fields),
/// the run length and the offset from the previous run's first cluster.
pub(super) fn parse_runs(b: &[u8]) -> Option<Vec<(u64, u64)>> {
    let mut runs = Vec::new();
    let mut at = 0;
    let mut lcn: i64 = 0;
    loop {
        let h = *b.get(at)?;
        if h == 0 {
            break;
        }
        let (len_size, off_size) = ((h & 0xF) as usize, (h >> 4) as usize);
        // Sparse runs (no offset) don't occur in the $MFT.
        if len_size == 0 || len_size > 8 || off_size == 0 || off_size > 8 {
            return None;
        }
        let len = le_uint(b.get(at + 1..at + 1 + len_size)?);
        let delta = le_int(b.get(at + 1 + len_size..at + 1 + len_size + off_size)?);
        lcn = lcn.checked_add(delta).filter(|l| *l >= 0)?;
        runs.push((lcn as u64, len));
        at += 1 + len_size + off_size;
    }
    Some(runs)
}

/// FILETIME (100 ns steps since 1601) to seconds since 1970, 0 if earlier.
pub(super) fn filetime_to_unix(ft: u64) -> u32 {
    const SECS_1601_TO_1970: u64 = 11_644_473_600;
    (ft / 10_000_000)
        .checked_sub(SECS_1601_TO_1970)
        .map_or(0, |s| s.min(u32::MAX as u64) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODIFIED: u64 = 133_000_000_000_000_000; // 2022-06-17

    /// A 1024-byte record with a standard information and a data attribute,
    /// protected like on disk (two sectors, check value 0xABCD).
    fn record(in_use: bool, base: u64, resident_data: bool) -> Vec<u8> {
        let mut r = vec![0u8; 1024];
        r[0..4].copy_from_slice(b"FILE");
        r[4..6].copy_from_slice(&0x30u16.to_le_bytes());
        r[6..8].copy_from_slice(&3u16.to_le_bytes());
        r[0x14..0x16].copy_from_slice(&0x38u16.to_le_bytes());
        r[0x16..0x18].copy_from_slice(&(in_use as u16).to_le_bytes());
        r[0x20..0x28].copy_from_slice(&(base | 3 << 48).to_le_bytes());
        let mut at = 0x38;
        // $STANDARD_INFORMATION, resident, value at 0x18.
        let si = &mut r[at..at + 0x60];
        si[0..4].copy_from_slice(&0x10u32.to_le_bytes());
        si[4..8].copy_from_slice(&0x60u32.to_le_bytes());
        si[0x10..0x14].copy_from_slice(&0x48u32.to_le_bytes());
        si[0x14..0x16].copy_from_slice(&0x18u16.to_le_bytes());
        si[0x18 + 8..0x18 + 16].copy_from_slice(&MODIFIED.to_le_bytes());
        at += 0x60;
        // A named data stream (alternate stream) that must be ignored.
        let ads = &mut r[at..at + 0x20];
        ads[0..4].copy_from_slice(&0x80u32.to_le_bytes());
        ads[4..8].copy_from_slice(&0x20u32.to_le_bytes());
        ads[9] = 3;
        ads[0x10..0x14].copy_from_slice(&999u32.to_le_bytes());
        at += 0x20;
        let data = &mut r[at..at + 0x50];
        data[0..4].copy_from_slice(&0x80u32.to_le_bytes());
        data[4..8].copy_from_slice(&0x50u32.to_le_bytes());
        if resident_data {
            data[0x10..0x14].copy_from_slice(&42u32.to_le_bytes());
        } else {
            data[8] = 1;
            data[0x20..0x22].copy_from_slice(&0x40u16.to_le_bytes());
            data[0x30..0x38].copy_from_slice(&123_456u64.to_le_bytes());
            // 16 clusters at 256, then 8 clusters 16 before that.
            data[0x40..0x46].copy_from_slice(&[0x21, 0x10, 0x00, 0x01, 0x11, 0x08]);
            data[0x46] = 0xF0;
        }
        at += 0x50;
        r[at..at + 4].copy_from_slice(&ATTR_END.to_le_bytes());
        r[0x18..0x1C].copy_from_slice(&(at as u32 + 8).to_le_bytes());
        // Protect: move each sector's last two bytes into the header.
        r[0x30..0x32].copy_from_slice(&[0xCD, 0xAB]);
        for i in 1..3 {
            let end = i * 512;
            r[0x30 + 2 * i] = r[end - 2];
            r[0x30 + 2 * i + 1] = r[end - 1];
            r[end - 2..end].copy_from_slice(&[0xCD, 0xAB]);
        }
        r
    }

    #[test]
    fn reads_size_and_date_from_a_record() {
        let mut r = record(true, 0, false);
        r[1022] = 0x55; // the stashed original isn't restored yet
        assert!(!apply_fixup(&mut r.clone()), "torn sector");

        let mut r = record(true, 0, false);
        assert!(apply_fixup(&mut r));
        assert_eq!(
            parse_file_record(&r),
            Some(FileRecord {
                base: None,
                modified: Some(MODIFIED),
                size: Some(123_456),
            })
        );
        assert_eq!(data_runs(&r), Some(vec![(256, 16), (240, 8)]));
        assert_eq!(filetime_to_unix(MODIFIED), 1_655_526_400);
        assert_eq!(filetime_to_unix(0), 0);

        let mut small = record(true, 0, true);
        assert!(apply_fixup(&mut small));
        assert_eq!(parse_file_record(&small).unwrap().size, Some(42));
        assert_eq!(data_runs(&small), None, "resident: no runs");

        let mut ext = record(true, 77, false);
        assert!(apply_fixup(&mut ext));
        assert_eq!(parse_file_record(&ext).unwrap().base, Some(77));

        let mut free = record(false, 0, false);
        assert!(apply_fixup(&mut free));
        assert_eq!(parse_file_record(&free), None);
        assert_eq!(parse_file_record(&[0u8; 1024]), None);
    }

    #[test]
    fn decodes_run_lists() {
        // Negative offsets are sign-extended; a run with no offset is rejected.
        assert_eq!(
            parse_runs(&[0x31, 0x04, 0x00, 0x00, 0x10, 0x11, 0x02, 0x80, 0x00]),
            Some(vec![(0x10_0000, 4), (0x10_0000 - 128, 2)])
        );
        assert_eq!(parse_runs(&[0x01, 0x04, 0x00]), None);
        assert_eq!(
            parse_runs(&[0x11, 0x04, 0xFF, 0x00]),
            None,
            "before cluster 0"
        );
        assert_eq!(parse_runs(&[0x11, 0x04]), None, "truncated");
        assert_eq!(parse_runs(&[0x00]), Some(vec![]));
    }
}
