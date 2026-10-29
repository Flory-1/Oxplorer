//! FTP folder listings: `MLSD` lines (RFC 3659) and the `LIST` output of
//! Unix (`ls -l`) and Windows/IIS (DOS style) servers.

use std::time::SystemTime;

use chrono::NaiveDateTime;
use suppaftp::list::{ListParser, PosixPexQuery};

use super::RemoteEntry;

/// One `MLSD` line; `None` for the folder itself, its parent and lines
/// that can't be read.
pub(super) fn parse_mlsd(line: &str) -> Option<RemoteEntry> {
    let line = line.trim_end_matches(['\r', '\n']);
    // Facts end with "; ", the rest is the name (which may contain both).
    let (facts, name) = match line.strip_prefix(' ') {
        Some(name) => ("", name),
        None => {
            let end = line.find("; ")?;
            (&line[..=end], &line[end + 2..])
        }
    };
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    let mut entry = RemoteEntry::file(name, 0, None);
    for fact in facts.split(';') {
        let Some((key, value)) = fact.split_once('=') else {
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            "type" => match value.to_ascii_lowercase().as_str() {
                "cdir" | "pdir" => return None,
                "dir" => entry.is_dir = true,
                "file" => {}
                // `OS.unix=slink:/target`, `OS.unix=symlink`, …
                t if t.contains("link") => entry.is_symlink = true,
                _ => {}
            },
            "size" | "sizd" => entry.size = value.parse().unwrap_or(0),
            "modify" => entry.modified = parse_mlsd_time(value),
            "unix.mode" => entry.permissions = u32::from_str_radix(value, 8).ok(),
            _ => {}
        }
    }
    if entry.is_dir {
        entry.size = 0;
    }
    Some(entry)
}

/// `YYYYMMDDHHMMSS[.sss]`, in UTC.
fn parse_mlsd_time(value: &str) -> Option<SystemTime> {
    let main = value.split('.').next()?;
    let t = NaiveDateTime::parse_from_str(main, "%Y%m%d%H%M%S").ok()?;
    Some(t.and_utc().into())
}

/// One `LIST` line (Unix or DOS style); `None` for `total …`, `.`/`..`
/// and lines that can't be read.
pub(super) fn parse_list(line: &str) -> Option<RemoteEntry> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() || line.starts_with("total ") {
        return None;
    }
    let file = ListParser::parse_posix(line)
        .or_else(|_| ListParser::parse_dos(line))
        .ok()?;
    let name = file.name();
    if name == "." || name == ".." || name.is_empty() {
        return None;
    }
    let is_dir = file.is_directory();
    Some(RemoteEntry {
        name: name.to_owned(),
        is_dir,
        is_symlink: file.is_symlink(),
        size: if is_dir { 0 } else { file.size() as u64 },
        modified: Some(file.modified()).filter(|m| *m != SystemTime::UNIX_EPOCH),
        // Only the owner's write right matters here (read-only or not).
        permissions: file.uid().map(|_| {
            if file.can_write(PosixPexQuery::Owner) {
                0o644
            } else {
                0o444
            }
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Utc};

    #[test]
    fn mlsd_lines() {
        let e = parse_mlsd(
            "type=file;size=1024;modify=20240131120000;UNIX.mode=0644; report v2; final.pdf",
        )
        .unwrap();
        assert_eq!(e.name, "report v2; final.pdf");
        assert_eq!(e.size, 1024);
        assert!(!e.is_dir);
        assert_eq!(e.permissions, Some(0o644));
        let when = Utc.with_ymd_and_hms(2024, 1, 31, 12, 0, 0).unwrap();
        assert_eq!(e.modified, Some(SystemTime::from(when)));

        let d =
            parse_mlsd("Type=dir;Sizd=4096;Modify=20200101000000.123;perm=flcdmpe; pub").unwrap();
        assert!(d.is_dir);
        assert_eq!(d.size, 0);
        assert_eq!(d.name, "pub");

        let l = parse_mlsd("type=OS.unix=slink:/srv/data;modify=20200101000000; data").unwrap();
        assert!(l.is_symlink);

        assert_eq!(parse_mlsd("type=cdir;modify=20200101000000; ."), None);
        assert_eq!(parse_mlsd("type=pdir;modify=20200101000000; .."), None);
        assert_eq!(parse_mlsd("garbage"), None);
        // No facts at all: just the name after a space.
        assert_eq!(parse_mlsd(" plain.txt").unwrap().name, "plain.txt");
    }

    #[test]
    fn unix_list_lines() {
        assert_eq!(parse_list("total 12"), None);
        let f = parse_list("-rw-r--r--    1 1000     1000         5120 Mar 14  2021 my file.txt")
            .unwrap();
        assert_eq!(f.name, "my file.txt");
        assert_eq!(f.size, 5120);
        assert!(!f.is_dir);
        let year = chrono::DateTime::<Utc>::from(f.modified.unwrap()).year();
        assert_eq!(year, 2021);

        let d = parse_list("drwxr-xr-x    2 ftp      ftp          4096 Jan  5 10:30 incoming\r")
            .unwrap();
        assert!(d.is_dir);
        assert_eq!(d.name, "incoming");
        assert_eq!(d.size, 0);

        let l = parse_list(
            "lrwxrwxrwx    1 0        0              11 Feb  1  2022 latest -> release-2.0",
        )
        .unwrap();
        assert!(l.is_symlink);
        assert_eq!(l.name, "latest");

        assert_eq!(parse_list("drwxr-xr-x 2 0 0 4096 Jan 5 10:30 ."), None);
        let ro = parse_list("-r--r--r--    1 1000     1000         1 Mar 14  2021 locked").unwrap();
        assert_eq!(ro.permissions, Some(0o444));
    }

    #[test]
    fn dos_list_lines() {
        let f = parse_list("04-27-21  03:12PM                 2048 Budget 2021.xlsx").unwrap();
        assert_eq!(f.name, "Budget 2021.xlsx");
        assert_eq!(f.size, 2048);
        assert!(!f.is_dir);
        let d = parse_list("10-05-20  09:41AM       <DIR>          Old Projects").unwrap();
        assert!(d.is_dir);
        assert_eq!(d.name, "Old Projects");
        assert_eq!(parse_list("this is not a listing"), None);
    }
}
