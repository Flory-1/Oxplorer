//! SQLite databases in the preview: their tables and views, and the first
//! rows of one. Opened strictly read-only and immutable: nothing is written
//! (no journal), nothing is locked, so a program using the database isn't
//! disturbed.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

use crate::datatable::{Column, Table};

/// How a database file starts.
const MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// Time for counting the rows of all tables, and for reading rows.
const COUNT_TIME: Duration = Duration::from_secs(2);
const READ_TIME: Duration = Duration::from_secs(5);
/// Longer text values are cut.
const MAX_CELL_CHARS: usize = 1000;

/// A table or view.
#[derive(Clone, Debug, PartialEq)]
pub struct DbObject {
    pub name: String,
    pub view: bool,
    /// `None` for views, or if counting took too long.
    pub rows: Option<u64>,
    /// Name and declared type of each column.
    pub columns: Vec<(String, String)>,
}

/// A database file by its extension and first bytes.
pub fn is_database(path: &Path) -> bool {
    use std::io::Read;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(
        ext.as_str(),
        "sqlite" | "sqlite3" | "db" | "db3" | "s3db" | "sl3"
    ) {
        return false;
    }
    let mut head = [0u8; 16];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok_and(|()| &head == MAGIC)
}

/// The `file:` URI that opens `path` read-only and immutable.
fn uri(path: &Path) -> String {
    let p = path.to_string_lossy();
    let p = p.strip_prefix(r"\\?\UNC\").map_or_else(
        || p.strip_prefix(r"\\?\").unwrap_or(&p).to_owned(),
        |unc| format!(r"\\{unc}"),
    );
    let mut out = String::from("file:");
    if p.starts_with(r"\\") {
        out.push_str("//"); // UNC: file:////server/share/…
    } else if !p.starts_with('/') {
        out.push('/');
    }
    for c in p.chars() {
        match c {
            '\\' => out.push('/'),
            '?' | '#' | '%' => out.push_str(&format!("%{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push_str("?mode=ro&immutable=1");
    out
}

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(uri(path), flags)
}

/// Stop what the connection runs after `time`.
fn deadline(conn: &mut Connection, time: Duration) -> rusqlite::Result<()> {
    let until = Instant::now() + time;
    conn.progress_handler(10_000, Some(move || Instant::now() > until))
}

fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The tables and views of the database (SQLite's own left out), by name.
pub fn objects(path: &Path) -> Result<Vec<DbObject>, String> {
    let mut conn = open(path).map_err(|e| e.to_string())?;
    let mut list: Vec<DbObject> = {
        let mut stmt = conn
            .prepare(
                "SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') \
                 AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name COLLATE NOCASE",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map([], |r| {
            Ok(DbObject {
                name: r.get(0)?,
                view: r.get::<_, String>(1)? == "view",
                rows: None,
                columns: Vec::new(),
            })
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<_>>())
        .map_err(|e| e.to_string())?
    };
    for o in &mut list {
        if let Ok(mut stmt) = conn.prepare("SELECT name, type FROM pragma_table_info(?1)") {
            o.columns = stmt
                .query_map([&o.name], |r| Ok((r.get(0)?, r.get(1)?)))
                .and_then(|rows| rows.collect())
                .unwrap_or_default();
        }
    }
    deadline(&mut conn, COUNT_TIME).map_err(|e| e.to_string())?;
    for o in list.iter_mut().filter(|o| !o.view) {
        let sql = format!("SELECT count(*) FROM {}", quoted(&o.name));
        o.rows = conn
            .query_row(&sql, [], |r| r.get::<_, i64>(0))
            .ok()
            .map(|n| n as u64);
    }
    Ok(list)
}

/// The first `limit` rows of `object`.
pub fn rows(path: &Path, object: &DbObject, limit: usize) -> Result<Table, String> {
    let mut conn = open(path).map_err(|e| e.to_string())?;
    deadline(&mut conn, READ_TIME).map_err(|e| e.to_string())?;
    let sql = format!("SELECT * FROM {} LIMIT ?1", quoted(&object.name));
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let names: Vec<String> = stmt.column_names().map(str::to_owned).collect();
    let mut table = Table {
        columns: names
            .iter()
            .map(|n| Column {
                name: n.clone(),
                decl: object
                    .columns
                    .iter()
                    .find(|(c, _)| c == n)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_default(),
                numeric: true,
            })
            .collect(),
        ..Default::default()
    };
    let mut any = vec![false; names.len()];
    let mut rows = stmt.query([limit as i64 + 1]).map_err(|e| e.to_string())?;
    loop {
        let row = match rows.next() {
            Ok(Some(row)) => row,
            Ok(None) => break,
            // Interrupted: keep what was read.
            Err(_) if !table.rows.is_empty() => {
                table.truncated = true;
                break;
            }
            Err(e) => return Err(e.to_string()),
        };
        if table.rows.len() == limit {
            table.truncated = true;
            break;
        }
        let mut cells = Vec::with_capacity(names.len());
        for (c, (col, any)) in table.columns.iter_mut().zip(&mut any).enumerate() {
            let value = row.get_ref(c).map_err(|e| e.to_string())?;
            let number = matches!(value, ValueRef::Integer(_) | ValueRef::Real(_));
            if !matches!(value, ValueRef::Null) {
                *any = true;
                col.numeric &= number;
            }
            cells.push(cell(value));
        }
        table.rows.push(cells);
    }
    for (col, any) in table.columns.iter_mut().zip(any) {
        col.numeric &= any;
    }
    Ok(table)
}

/// A value as shown in a cell; `None` for NULL.
fn cell(value: ValueRef) -> Option<String> {
    Some(match value {
        ValueRef::Null => return None,
        ValueRef::Integer(n) => n.to_string(),
        ValueRef::Real(f) => format!("{f:?}"),
        ValueRef::Text(t) => {
            let text = String::from_utf8_lossy(t);
            match text.char_indices().nth(MAX_CELL_CHARS) {
                Some((at, _)) => format!("{}…", &text[..at]),
                None => text.into_owned(),
            }
        }
        ValueRef::Blob(b) => format!("BLOB ({})", crate::format::size(b.len() as u64)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("test.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, score REAL, photo BLOB);
             INSERT INTO people (name, score, photo) VALUES ('Ann', 1.5, x'0102'), ('Bob', NULL, NULL);
             CREATE TABLE \"odd \"\"name\"\"\" (x);
             INSERT INTO \"odd \"\"name\"\"\" VALUES (1), (2), (3);
             CREATE VIEW names AS SELECT name FROM people;",
        )
        .unwrap();
        path
    }

    #[test]
    fn uris() {
        assert_eq!(
            uri(Path::new(r"C:\Data\a b#1?.db")),
            "file:/C:/Data/a b%231%3F.db?mode=ro&immutable=1"
        );
        assert_eq!(
            uri(Path::new(r"\\server\share\x.db")),
            "file:////server/share/x.db?mode=ro&immutable=1"
        );
        assert_eq!(
            uri(Path::new(r"\\?\C:\x%.db")),
            "file:/C:/x%25.db?mode=ro&immutable=1"
        );
        assert_eq!(
            uri(Path::new("/home/x.db")),
            "file:/home/x.db?mode=ro&immutable=1"
        );
    }

    #[test]
    fn lists_tables_and_views() {
        let dir = tempfile::tempdir().unwrap();
        let path = sample(dir.path());
        assert!(is_database(&path));
        let list = objects(&path).unwrap();
        let names: Vec<_> = list.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["names", "odd \"name\"", "people"]);
        assert!(list[0].view);
        assert_eq!(list[0].rows, None);
        assert_eq!(list[1].rows, Some(3));
        assert_eq!(list[2].rows, Some(2));
        assert_eq!(
            list[2].columns,
            [
                ("id".to_owned(), "INTEGER".to_owned()),
                ("name".to_owned(), "TEXT".to_owned()),
                ("score".to_owned(), "REAL".to_owned()),
                ("photo".to_owned(), "BLOB".to_owned()),
            ]
        );
    }

    #[test]
    fn reads_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = sample(dir.path());
        let list = objects(&path).unwrap();
        let people = rows(&path, &list[2], 10).unwrap();
        assert!(!people.truncated);
        assert_eq!(people.columns[1].name, "name");
        assert_eq!(people.columns[2].decl, "REAL");
        assert!(people.columns[0].numeric);
        assert!(!people.columns[1].numeric);
        assert!(people.columns[2].numeric);
        assert_eq!(
            people.rows[0],
            [
                Some("1".to_owned()),
                Some("Ann".to_owned()),
                Some("1.5".to_owned()),
                Some("BLOB (2 B)".to_owned())
            ]
        );
        assert_eq!(people.rows[1][2], None);
        let odd = rows(&path, &list[1], 2).unwrap();
        assert_eq!(odd.rows.len(), 2);
        assert!(odd.truncated);
        let view = rows(&path, &list[0], 10).unwrap();
        assert_eq!(view.rows.len(), 2);
    }

    #[test]
    fn leaves_the_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = sample(dir.path());
        let before: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        let conn = open(&path).unwrap();
        assert!(conn.execute("DELETE FROM people", []).is_err());
        drop(conn);
        objects(&path).unwrap();
        let after: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(before.len(), after.len(), "no journal or WAL file");
        // Not a database.
        let other = dir.path().join("other.db");
        std::fs::write(&other, b"not a database at all").unwrap();
        assert!(!is_database(&other));
    }
}
