//! Parsing the records returned by `FSCTL_ENUM_USN_DATA` (listing the master
//! file table) and `FSCTL_READ_USN_JOURNAL` (the NTFS change journal).
//! Both return a buffer that starts with an 8-byte value (the next file
//! reference or USN to ask for) followed by `USN_RECORD_V2`/`V3` records.

use std::collections::HashMap;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

use super::patch::Signal;
use super::tree::ATTR_DIRECTORY;

pub(super) const REASON_FILE_DELETE: u32 = 0x0000_0200;
pub(super) const REASON_RENAME_OLD_NAME: u32 = 0x0000_1000;
pub(super) const REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct UsnRecord {
    pub frn: u64,
    pub parent: u64,
    pub usn: i64,
    pub reason: u32,
    pub attrs: u32,
    pub name: String,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// The 8-byte value at the start of an output buffer.
pub(super) fn next_value(buf: &[u8]) -> Option<u64> {
    u64_at(buf, 0)
}

/// The records in `buf` (the output after the leading 8 bytes). Stops at the
/// first malformed record.
pub(super) fn parse_records(buf: &[u8]) -> Vec<UsnRecord> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(len) = u32_at(buf, at) {
        let len = len as usize;
        if len < 8 || at + len > buf.len() {
            break;
        }
        if let Some(r) = parse_record(&buf[at..at + len]) {
            out.push(r);
        }
        at += len;
    }
    out
}

fn parse_record(r: &[u8]) -> Option<UsnRecord> {
    // (frn, parent, usn, reason, attrs, name length, name offset) by version.
    let (frn, parent, usn, reason, attrs, name_len, name_off) = match u16_at(r, 4)? {
        2 => (
            u64_at(r, 8)?,
            u64_at(r, 16)?,
            u64_at(r, 24)?,
            u32_at(r, 40)?,
            u32_at(r, 52)?,
            u16_at(r, 56)?,
            u16_at(r, 58)?,
        ),
        // 128-bit file ids; on NTFS they fit in the low 64 bits.
        3 => (
            u64_at(r, 8)?,
            u64_at(r, 24)?,
            u64_at(r, 40)?,
            u32_at(r, 56)?,
            u32_at(r, 68)?,
            u16_at(r, 72)?,
            u16_at(r, 74)?,
        ),
        _ => return None,
    };
    let start = name_off as usize;
    let bytes = r.get(start..start + name_len as usize)?;
    let wide: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    Some(UsnRecord {
        frn,
        parent,
        usn: usn as i64,
        reason,
        attrs,
        name: String::from_utf16_lossy(&wide),
    })
}

/// The roots as the journal's paths spell them (long names, no `\\?\`,
/// ending in a separator), each with the root as given.
pub(super) fn prefixes(roots: &[PathBuf]) -> Vec<(String, PathBuf)> {
    roots
        .iter()
        .map(|r| {
            // Roots may be written differently (short 8.3 names, other case).
            let long = std::fs::canonicalize(r).unwrap_or_else(|_| r.clone());
            let long = long.to_string_lossy();
            let mut s = long.strip_prefix(r"\\?\").unwrap_or(&long).to_owned();
            if !s.ends_with(MAIN_SEPARATOR) {
                s.push(MAIN_SEPARATOR);
            }
            (s, r.clone())
        })
        .collect()
}

/// `path` below one of the roots, written the way that root is.
pub(super) fn map_to_root(path: &Path, prefixes: &[(String, PathBuf)]) -> Option<PathBuf> {
    let s = path.to_string_lossy();
    prefixes.iter().find_map(|(long, root)| {
        let n = long.len();
        (s.len() > n && s.is_char_boundary(n) && s[..n].eq_ignore_ascii_case(long))
            .then(|| root.join(&s[n..]))
    })
}

/// Folder paths by file reference, so busy folders are looked up once.
#[derive(Default)]
pub(super) struct Dirs(HashMap<u64, Option<PathBuf>>);

/// What journal records mean for the index: the path of each (its folder's
/// current path, from `lookup`, and its name) below one of the roots.
/// Records whose folder is gone are skipped: the folder's own record covers
/// them.
pub(super) fn signals_for(
    records: &[UsnRecord],
    dirs: &mut Dirs,
    lookup: &mut dyn FnMut(u64) -> Option<PathBuf>,
    prefixes: &[(String, PathBuf)],
) -> Vec<Signal> {
    let mut out = Vec::new();
    if dirs.0.len() > 10_000 {
        dirs.0.clear();
    }
    for r in records {
        let dir = match dirs.0.get(&r.parent) {
            Some(d) => d.clone(),
            None => {
                let d = lookup(r.parent);
                dirs.0.insert(r.parent, d.clone());
                d
            }
        };
        if r.attrs & ATTR_DIRECTORY != 0
            && r.reason & (REASON_RENAME_OLD_NAME | REASON_RENAME_NEW_NAME | REASON_FILE_DELETE)
                != 0
        {
            dirs.0.clear(); // paths below it changed
        }
        let Some(dir) = dir else { continue };
        let Some(path) = map_to_root(&dir.join(&r.name), prefixes) else {
            continue;
        };
        out.push(if r.reason & REASON_RENAME_OLD_NAME != 0 {
            Signal::RenamedFrom(path)
        } else if r.reason & REASON_RENAME_NEW_NAME != 0 {
            Signal::RenamedTo(path)
        } else {
            Signal::Changed(path)
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn maps_journal_paths_onto_roots() {
        let short = PathBuf::from(r"C:\Users\RUNNER~1\Temp");
        let drive = PathBuf::from(r"D:\");
        let prefixes = [
            (r"C:\Users\runneradmin\Temp\".to_owned(), short),
            (r"D:\".to_owned(), drive),
        ];
        assert_eq!(
            map_to_root(Path::new(r"c:\users\RunnerAdmin\Temp\a\b.txt"), &prefixes),
            Some(PathBuf::from(r"C:\Users\RUNNER~1\Temp\a\b.txt"))
        );
        assert_eq!(
            map_to_root(Path::new(r"D:\x.txt"), &prefixes),
            Some(PathBuf::from(r"D:\x.txt"))
        );
        assert_eq!(map_to_root(Path::new(r"C:\Windows\x"), &prefixes), None);
        assert_eq!(
            map_to_root(Path::new(r"C:\Users\runneradmin\Temp"), &prefixes),
            None
        );
    }

    #[test]
    fn records_become_signals_below_the_roots() {
        let root = std::env::temp_dir().join("ox-usn-test-root");
        let prefixes = vec![(format!("{}{MAIN_SEPARATOR}", root.display()), root.clone())];
        let rec = |frn, parent, reason, attrs, name: &str| UsnRecord {
            frn,
            parent,
            usn: 0,
            reason,
            attrs,
            name: name.into(),
        };
        let records = [
            rec(10, 1, 0x100, 0, "new.txt"),
            rec(11, 2, REASON_RENAME_OLD_NAME, ATTR_DIRECTORY, "old"),
            rec(11, 2, REASON_RENAME_NEW_NAME, ATTR_DIRECTORY, "renamed"),
            rec(12, 3, 0x100, 0, "outside.txt"),
            rec(13, 4, REASON_FILE_DELETE, 0, "child-of-a-gone-folder.txt"),
            rec(14, 1, 0x100, 0, "again.txt"),
        ];
        let mut looked_up = Vec::new();
        let mut lookup = |frn: u64| {
            looked_up.push(frn);
            match frn {
                1 => Some(root.clone()),
                2 => Some(root.join("sub")),
                3 => Some(std::env::temp_dir().join("elsewhere")),
                _ => None,
            }
        };
        let signals = signals_for(&records, &mut Dirs::default(), &mut lookup, &prefixes);
        assert_eq!(
            signals,
            vec![
                Signal::Changed(root.join("new.txt")),
                Signal::RenamedFrom(root.join("sub").join("old")),
                Signal::RenamedTo(root.join("sub").join("renamed")),
                Signal::Changed(root.join("again.txt")),
            ]
        );
        // Folders are looked up again after a folder was renamed.
        assert_eq!(looked_up, vec![1, 2, 2, 3, 4, 1]);
    }

    /// A `USN_RECORD_V2` as Windows lays it out (8-byte aligned).
    fn record_v2(frn: u64, parent: u64, reason: u32, attrs: u32, name: &str) -> Vec<u8> {
        let wide: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let len = (60 + wide.len()).next_multiple_of(8);
        let mut r = vec![0u8; len];
        r[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        r[4..6].copy_from_slice(&2u16.to_le_bytes());
        r[8..16].copy_from_slice(&frn.to_le_bytes());
        r[16..24].copy_from_slice(&parent.to_le_bytes());
        r[24..32].copy_from_slice(&77i64.to_le_bytes());
        r[40..44].copy_from_slice(&reason.to_le_bytes());
        r[52..56].copy_from_slice(&attrs.to_le_bytes());
        r[56..58].copy_from_slice(&(wide.len() as u16).to_le_bytes());
        r[58..60].copy_from_slice(&60u16.to_le_bytes());
        r[60..60 + wide.len()].copy_from_slice(&wide);
        r
    }

    fn record_v3(frn: u64, parent: u64, reason: u32, name: &str) -> Vec<u8> {
        let wide: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let len = (76 + wide.len()).next_multiple_of(8);
        let mut r = vec![0u8; len];
        r[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        r[4..6].copy_from_slice(&3u16.to_le_bytes());
        r[8..16].copy_from_slice(&frn.to_le_bytes());
        r[24..32].copy_from_slice(&parent.to_le_bytes());
        r[40..48].copy_from_slice(&99i64.to_le_bytes());
        r[56..60].copy_from_slice(&reason.to_le_bytes());
        r[68..72].copy_from_slice(&0x10u32.to_le_bytes());
        r[72..74].copy_from_slice(&(wide.len() as u16).to_le_bytes());
        r[74..76].copy_from_slice(&76u16.to_le_bytes());
        r[76..76 + wide.len()].copy_from_slice(&wide);
        r
    }

    #[test]
    fn parses_v2_and_v3_records() {
        let mut buf = 1234u64.to_le_bytes().to_vec();
        buf.extend(record_v2(
            0x0003_0000_0000_0040,
            5,
            REASON_RENAME_OLD_NAME,
            0x2,
            "Résumé.txt",
        ));
        buf.extend(record_v3(41, 0x40, REASON_RENAME_NEW_NAME, "新しい"));
        assert_eq!(next_value(&buf), Some(1234));
        let records = parse_records(&buf[8..]);
        assert_eq!(
            records,
            vec![
                UsnRecord {
                    frn: 0x0003_0000_0000_0040,
                    parent: 5,
                    usn: 77,
                    reason: REASON_RENAME_OLD_NAME,
                    attrs: 0x2,
                    name: "Résumé.txt".into(),
                },
                UsnRecord {
                    frn: 41,
                    parent: 0x40,
                    usn: 99,
                    reason: REASON_RENAME_NEW_NAME,
                    attrs: 0x10,
                    name: "新しい".into(),
                },
            ]
        );
    }

    #[test]
    fn stops_at_malformed_records() {
        let mut buf = record_v2(1, 5, 0, 0, "ok");
        let mut bad = record_v2(2, 5, 0, 0, "too long");
        bad[0..4].copy_from_slice(&4096u32.to_le_bytes());
        buf.extend(bad);
        assert_eq!(parse_records(&buf).len(), 1);

        // A name pointing past the record is skipped, the rest still read.
        let mut buf = record_v2(1, 5, 0, 0, "x");
        buf[58..60].copy_from_slice(&500u16.to_le_bytes());
        buf.extend(record_v2(2, 5, 0, 0, "y"));
        let records = parse_records(&buf);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "y");

        // Unknown versions and empty input.
        let mut v4 = record_v2(1, 5, 0, 0, "x");
        v4[4] = 4;
        assert!(parse_records(&v4).is_empty());
        assert!(parse_records(&[]).is_empty());
        assert!(parse_records(&[0; 8]).is_empty());
    }
}
