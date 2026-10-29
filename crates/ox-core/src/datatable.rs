//! Tables for the preview's Table tab: CSV/TSV files (delimiter, quoting
//! and header row guessed) and the rows of SQLite tables.

use crate::textinfo::{self, Encoding};

/// How many rows are kept.
pub const MAX_ROWS: usize = 10_000;
/// How much of a CSV file is read.
pub const MAX_CSV_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub name: String,
    /// Declared type (SQLite), empty for CSV.
    pub decl: String,
    /// Holds numbers only (shown right-aligned).
    pub numeric: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub columns: Vec<Column>,
    /// The cells of each row; `None` is SQL NULL. Rows may be shorter
    /// than the column list.
    pub rows: Vec<Vec<Option<String>>>,
    /// There are more rows than these.
    pub truncated: bool,
}

/// What was guessed about a CSV file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CsvFormat {
    pub delimiter: u8,
    pub quote: u8,
    /// The first row names the columns.
    pub header: bool,
}

impl CsvFormat {
    /// The delimiter's name (translated).
    pub fn delimiter_label(&self) -> &'static str {
        use crate::i18n::t;
        match self.delimiter {
            b',' => t("Comma"),
            b';' => t("Semicolon"),
            b'\t' => t("Tab"),
            b'|' => t("Vertical bar"),
            _ => "?",
        }
    }
}

/// Delimiters that are guessed.
const DELIMITERS: [u8; 4] = *b",;\t|";

/// Text of a CSV file from its `bytes` (encoding detected). `None` if it's
/// binary.
pub fn decode_csv(bytes: &[u8]) -> Option<String> {
    let enc = textinfo::encoding(bytes);
    let utf16 = matches!(enc, Encoding::Utf16Le | Encoding::Utf16Be);
    if !utf16 && bytes.iter().take(8192).any(|&b| b == 0) {
        return None;
    }
    Some(textinfo::decode(bytes, enc))
}

/// Parse CSV `text`. `tabs`: the file is a .tsv/.tab (tabs preferred).
/// `cut`: `text` is only the beginning of the file (its last, perhaps
/// incomplete, record is dropped).
pub fn parse_csv(text: &str, tabs: bool, cut: bool) -> (Table, CsvFormat) {
    let delimiter = sniff_delimiter(text, tabs);
    let quote = sniff_quote(text, delimiter);
    let (mut records, more) = records(text, delimiter, quote, MAX_ROWS + 1);
    let mut truncated = more || records.len() > MAX_ROWS;
    if cut && !more && records.len() <= MAX_ROWS {
        // The last record may be cut in the middle.
        records.pop();
        truncated = true;
    }
    let header = has_header(&records, delimiter);
    let names = if header && !records.is_empty() {
        records.remove(0)
    } else {
        Vec::new()
    };
    records.truncate(MAX_ROWS);
    let width = records
        .iter()
        .map(Vec::len)
        .chain([names.len()])
        .max()
        .unwrap_or(0);
    let columns = (0..width)
        .map(|c| Column {
            name: names.get(c).cloned().unwrap_or_else(|| column_letters(c)),
            decl: String::new(),
            numeric: numeric_column(&records, c, delimiter),
        })
        .collect();
    let rows = records
        .into_iter()
        .map(|r| r.into_iter().map(Some).collect())
        .collect();
    let table = Table {
        columns,
        rows,
        truncated,
    };
    (
        table,
        CsvFormat {
            delimiter,
            quote,
            header,
        },
    )
}

/// Spreadsheet-style column names: A … Z, AA, AB …
pub fn column_letters(mut n: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII")
}

/// The delimiter that splits the first lines most evenly.
pub fn sniff_delimiter(text: &str, tabs: bool) -> u8 {
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(50)
        .collect();
    let mut best = (b',', 0.0f32, 0usize);
    for d in DELIMITERS {
        let counts: Vec<usize> = lines.iter().map(|l| count_outside_quotes(l, d)).collect();
        // The count most lines share, and how many do.
        let mut mode = (0usize, 0usize);
        for &c in counts.iter().filter(|&&c| c > 0) {
            let n = counts.iter().filter(|&&x| x == c).count();
            if (n, c) > (mode.1, mode.0) {
                mode = (c, n);
            }
        }
        if mode.0 == 0 {
            continue;
        }
        let share = mode.1 as f32 / lines.len() as f32;
        if tabs && d == b'\t' {
            return d;
        }
        if (share, mode.0) > (best.1, best.2) {
            best = (d, share, mode.0);
        }
    }
    best.0
}

fn count_outside_quotes(line: &str, d: u8) -> usize {
    let mut quoted = false;
    line.bytes()
        .filter(|&b| {
            if b == b'"' {
                quoted = !quoted;
            }
            b == d && !quoted
        })
        .count()
}

/// `"` unless fields are rather quoted with `'`.
pub fn sniff_quote(text: &str, delimiter: u8) -> u8 {
    let mut counts = [0usize; 2];
    for line in text.lines().take(50) {
        for field in line.split(delimiter as char) {
            let f = field.trim().as_bytes();
            for (i, &q) in b"\"'".iter().enumerate() {
                if f.len() >= 2 && f[0] == q && f[f.len() - 1] == q {
                    counts[i] += 1;
                }
            }
        }
    }
    if counts[1] > counts[0] {
        b'\''
    } else {
        b'"'
    }
}

/// Split `text` into records (quoted fields may hold delimiters, line
/// breaks and doubled quotes). Stops after `max`; says if there was more.
pub fn records(text: &str, delimiter: u8, quote: u8, max: usize) -> (Vec<Vec<String>>, bool) {
    let (d, q) = (delimiter as char, quote as char);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    // At the start of a field (where a quote opens a quoted field).
    let mut start = true;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == q {
                if chars.peek() == Some(&q) {
                    field.push(q);
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            _ if c == q && start => {
                quoted = true;
                start = false;
            }
            _ if c == d => {
                row.push(std::mem::take(&mut field));
                start = true;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                start = true;
                // Blank lines are skipped.
                if !(row.len() == 1 && row[0].is_empty()) {
                    rows.push(std::mem::take(&mut row));
                    if rows.len() >= max {
                        return (rows, chars.peek().is_some());
                    }
                }
                row.clear();
            }
            _ => {
                field.push(c);
                start = false;
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    (rows, false)
}

/// Whether `s` is a number (`-1.5`, `1e3`, `12%`; with a decimal comma
/// when the delimiter isn't a comma).
pub fn is_number(s: &str, delimiter: u8) -> bool {
    let s = s.trim();
    let s = s.strip_suffix('%').unwrap_or(s).trim_end();
    if s.is_empty()
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+-.,eE".contains(&b))
        || !s.bytes().any(|b| b.is_ascii_digit())
    {
        return false;
    }
    let s = if delimiter != b',' && !s.contains('.') {
        s.replacen(',', ".", 1)
    } else {
        s.to_owned()
    };
    s.parse::<f64>().is_ok()
}

/// Column `c` has numbers in every non-empty data cell (and has some).
fn numeric_column(rows: &[Vec<String>], c: usize, delimiter: u8) -> bool {
    let mut any = false;
    for cell in rows.iter().filter_map(|r| r.get(c)) {
        if cell.trim().is_empty() {
            continue;
        }
        if !is_number(cell, delimiter) {
            return false;
        }
        any = true;
    }
    any
}

/// Guess whether the first record names the columns: there are more
/// records, and its cells are filled, distinct and not numbers.
fn has_header(records: &[Vec<String>], delimiter: u8) -> bool {
    let Some(first) = records.first() else {
        return false;
    };
    if records.len() < 2 || first.iter().any(|c| c.trim().is_empty()) {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    if !first.iter().all(|c| seen.insert(c.trim())) {
        return false;
    }
    if first.iter().any(|c| is_number(c, delimiter)) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters() {
        assert_eq!(column_letters(0), "A");
        assert_eq!(column_letters(25), "Z");
        assert_eq!(column_letters(26), "AA");
        assert_eq!(column_letters(27), "AB");
        assert_eq!(column_letters(26 * 27), "AAA");
    }

    #[test]
    fn delimiters_are_guessed() {
        assert_eq!(sniff_delimiter("a,b,c\n1,2,3\n", false), b',');
        assert_eq!(sniff_delimiter("a;b;c\n1,5;2;3\n4;5,5;6\n", false), b';');
        assert_eq!(sniff_delimiter("a\tb\n1\t2\n", false), b'\t');
        assert_eq!(sniff_delimiter("a|b|c\nx|y|z\n", false), b'|');
        // Commas inside quotes don't count.
        assert_eq!(
            sniff_delimiter("name;note\n\"a, b, c\";x\n\"d, e\";y\n", false),
            b';'
        );
        // A .tsv with tabs stays tab-separated.
        assert_eq!(sniff_delimiter("a,b\tc\n", true), b'\t');
        // One column.
        assert_eq!(sniff_delimiter("one\ntwo\n", false), b',');
    }

    #[test]
    fn quoted_fields() {
        let (rows, more) = records(
            "a,\"b, c\",\"say \"\"hi\"\"\"\r\n\"multi\nline\",2,3\n\n",
            b',',
            b'"',
            10,
        );
        assert!(!more);
        assert_eq!(
            rows,
            vec![
                vec!["a", "b, c", "say \"hi\""],
                vec!["multi\nline", "2", "3"]
            ]
        );
        assert_eq!(sniff_quote("'a';'b'\n'c';'d'\n", b';'), b'\'');
        assert_eq!(sniff_quote("\"a\";'b'\n", b';'), b'"');
        let (rows, _) = records("'x;y';z\n", b';', b'\'', 10);
        assert_eq!(rows, vec![vec!["x;y", "z"]]);
    }

    #[test]
    fn records_stop_at_the_limit() {
        let (rows, more) = records("1\n2\n3\n", b',', b'"', 2);
        assert_eq!(rows.len(), 2);
        assert!(more);
        let (rows, more) = records("1\n2\n", b',', b'"', 2);
        assert_eq!(rows.len(), 2);
        assert!(!more);
    }

    #[test]
    fn numbers() {
        assert!(is_number("12", b','));
        assert!(is_number(" -1.5e3 ", b','));
        assert!(is_number("50%", b','));
        assert!(is_number("1,5", b';'));
        assert!(!is_number("1,5", b','));
        assert!(!is_number("inf", b','));
        assert!(!is_number("NaN", b','));
        assert!(!is_number("12 kg", b','));
        assert!(!is_number("", b','));
        assert!(!is_number("-", b','));
    }

    #[test]
    fn header_and_columns() {
        let (t, f) = parse_csv("name;price\nTea;1,5\nCake;\nPie;3\n", false, false);
        assert_eq!(f.delimiter, b';');
        assert!(f.header);
        assert_eq!(t.columns.len(), 2);
        assert_eq!(t.columns[0].name, "name");
        assert!(!t.columns[0].numeric);
        assert!(t.columns[1].numeric);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(
            t.rows[1],
            vec![Some("Cake".to_owned()), Some(String::new())]
        );
        assert!(!t.truncated);

        // Numbers in the first row: no header.
        let (t, f) = parse_csv("1,2,3\n4,5,6,7\n", false, false);
        assert!(!f.header);
        let names: Vec<_> = t.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "C", "D"]);
        assert_eq!(t.rows.len(), 2);

        // Repeated names: no header.
        let (_, f) = parse_csv("a,a\nb,c\n", false, false);
        assert!(!f.header);
    }

    #[test]
    fn cut_files_drop_their_last_record() {
        let (t, _) = parse_csv("a,b\n1,2\n3,", false, true);
        assert_eq!(t.rows.len(), 1);
        assert!(t.truncated);
        let many: String = (0..MAX_ROWS + 5).map(|i| format!("{i}\n")).collect();
        let (t, _) = parse_csv(&many, false, false);
        assert_eq!(t.rows.len(), MAX_ROWS);
        assert!(t.truncated);
    }

    #[test]
    fn binary_isnt_csv() {
        assert_eq!(decode_csv(b"a,b\0c"), None);
        assert_eq!(decode_csv(b"\xEF\xBB\xBFa,b").as_deref(), Some("a,b"));
    }
}
