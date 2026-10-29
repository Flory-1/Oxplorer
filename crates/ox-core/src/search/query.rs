//! Parsing a query string and matching names and metadata against it.

use std::cell::RefCell;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::entry::FileKind;

#[derive(Debug, Clone)]
enum Term {
    /// Substring, lower-cased unless the query is case-sensitive.
    Contains(String),
    Wildcard(Vec<char>),
    Regex(regex::Regex),
}

impl PartialEq for Term {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Term::Contains(a), Term::Contains(b)) => a == b,
            (Term::Wildcard(a), Term::Wildcard(b)) => a == b,
            (Term::Regex(a), Term::Regex(b)) => a.as_str() == b.as_str(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFilter {
    Files,
    Folders,
}

/// Text to find inside files, compiled once per query.
#[derive(Debug, Clone)]
struct ContentPattern {
    text: String,
    regex: regex::bytes::Regex,
}

impl PartialEq for ContentPattern {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

/// A parsed search query.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    include: Vec<Term>,
    exclude: Vec<Term>,
    extensions: Vec<String>,
    kind: Option<KindFilter>,
    kinds: Vec<FileKind>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    modified_within: Option<Duration>,
    content: Option<ContentPattern>,
    case_sensitive: bool,
}

/// File kinds for `kind:` values.
fn parse_kinds(value: &str) -> Vec<FileKind> {
    let mut out = Vec::new();
    for v in value
        .split([',', ';', '|'])
        .map(|v| v.trim().to_ascii_lowercase())
    {
        let kinds: &[FileKind] = match v.as_str() {
            "image" | "images" | "picture" | "pictures" | "photo" | "photos" => &[FileKind::Image],
            "video" | "videos" | "movie" | "movies" => &[FileKind::Video],
            "audio" | "music" | "sound" => &[FileKind::Audio],
            "doc" | "docs" | "document" | "documents" => &[
                FileKind::Document,
                FileKind::Pdf,
                FileKind::Text,
                FileKind::Spreadsheet,
                FileKind::Presentation,
            ],
            "pdf" => &[FileKind::Pdf],
            "spreadsheet" | "sheet" => &[FileKind::Spreadsheet],
            "presentation" | "slides" => &[FileKind::Presentation],
            "archive" | "archives" | "zip" => &[FileKind::Archive],
            "code" | "source" => &[FileKind::Code],
            "text" => &[FileKind::Text],
            "app" | "apps" | "program" | "exe" | "executable" => &[FileKind::Executable],
            "font" | "fonts" => &[FileKind::Font],
            _ => &[],
        };
        out.extend_from_slice(kinds);
    }
    out
}

fn regex_term(pattern: &str, case_sensitive: bool) -> Option<Term> {
    let flags = if case_sensitive { "" } else { "(?i)" };
    regex::Regex::new(&format!("{flags}{pattern}"))
        .ok()
        .map(Term::Regex)
}

impl Query {
    pub fn parse(input: &str, case_sensitive: bool) -> Self {
        let mut q = Query {
            case_sensitive,
            ..Default::default()
        };
        for token in tokenize(input) {
            let (negate, token) = match token.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_owned()),
                _ => (false, token),
            };
            let mut term = None;
            if let Some((key, value)) = token.split_once(':') {
                let value = value.trim_matches('"');
                match key.to_ascii_lowercase().as_str() {
                    "ext" => {
                        q.extensions.extend(
                            value
                                .split([',', ';', '|'])
                                .map(|e| e.trim().trim_start_matches('.').to_lowercase())
                                .filter(|e| !e.is_empty()),
                        );
                        continue;
                    }
                    "type" | "kind" => {
                        match value.to_ascii_lowercase().as_str() {
                            "folder" | "folders" | "dir" | "directory" => {
                                q.kind = Some(KindFilter::Folders)
                            }
                            "file" | "files" => q.kind = Some(KindFilter::Files),
                            other => q.kinds.extend(parse_kinds(other)),
                        }
                        continue;
                    }
                    "size" => {
                        if let Some(rest) = value.strip_prefix('>') {
                            q.min_size = parse_size(rest);
                        } else if let Some(rest) = value.strip_prefix('<') {
                            q.max_size = parse_size(rest);
                        } else if let Some(n) = parse_size(value) {
                            q.min_size = Some(n);
                        }
                        continue;
                    }
                    "modified" | "date" => {
                        q.modified_within = parse_age(value);
                        continue;
                    }
                    "content" | "contains" => {
                        let flags = if case_sensitive { "" } else { "(?i)" };
                        if let Ok(regex) =
                            regex::bytes::Regex::new(&format!("{flags}{}", regex::escape(value)))
                        {
                            q.content = Some(ContentPattern {
                                text: value.to_owned(),
                                regex,
                            });
                        }
                        continue;
                    }
                    "regex" | "re" => term = regex_term(value, case_sensitive),
                    _ => {} // Not a filter, e.g. "C:" — treat as a normal term.
                }
            }
            if term.is_none() && token.len() > 2 && token.starts_with('/') && token.ends_with('/') {
                term = regex_term(&token[1..token.len() - 1], case_sensitive);
            }
            let term = term.unwrap_or_else(|| make_term(&token, case_sensitive));
            if negate {
                q.exclude.push(term);
            } else {
                q.include.push(term);
            }
        }
        q
    }

    /// True if there is nothing to search for.
    pub fn is_empty(&self) -> bool {
        self.include.is_empty()
            && self.exclude.is_empty()
            && self.extensions.is_empty()
            && self.kind.is_none()
            && self.kinds.is_empty()
            && self.min_size.is_none()
            && self.max_size.is_none()
            && self.modified_within.is_none()
            && self.content.is_none()
    }

    /// The query looks inside files, so it can't be answered from names alone.
    pub fn needs_content(&self) -> bool {
        self.content.is_some()
    }

    /// Does a file name match the name-based parts of the query?
    pub fn matches_name(&self, name: &str) -> bool {
        // ASCII names (almost all of them) are compared without allocating.
        let ascii_ci = !self.case_sensitive && name.is_ascii();
        let lowered;
        let folded = if self.case_sensitive || ascii_ci {
            name
        } else {
            lowered = name.to_lowercase();
            lowered.as_str()
        };
        self.include
            .iter()
            .all(|t| term_matches(t, name, folded, ascii_ci))
            && !self
                .exclude
                .iter()
                .any(|t| term_matches(t, name, folded, ascii_ci))
    }

    /// Everything except the `content:` part, from already known metadata.
    pub fn matches_meta(
        &self,
        name: &str,
        is_dir: bool,
        size: u64,
        modified: Option<SystemTime>,
    ) -> bool {
        self.matches_type(name, is_dir)
            && self.matches_name(name)
            && self.matches_size_and_age(is_dir, size, modified)
    }

    fn matches_type(&self, name: &str, is_dir: bool) -> bool {
        match self.kind {
            Some(KindFilter::Files) if is_dir => return false,
            Some(KindFilter::Folders) if !is_dir => return false,
            _ => {}
        }
        if self.extensions.is_empty() && self.kinds.is_empty() {
            return true;
        }
        if is_dir {
            return false;
        }
        let ext = Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        (self.extensions.is_empty() || self.extensions.contains(&ext))
            && (self.kinds.is_empty() || self.kinds.contains(&FileKind::from_extension(&ext)))
    }

    pub(super) fn needs_meta(&self) -> bool {
        self.min_size.is_some() || self.max_size.is_some() || self.modified_within.is_some()
    }

    fn matches_size_and_age(&self, is_dir: bool, size: u64, modified: Option<SystemTime>) -> bool {
        if self.min_size.is_some_and(|min| is_dir || size < min) {
            return false;
        }
        if self.max_size.is_some_and(|max| is_dir || size > max) {
            return false;
        }
        if let Some(within) = self.modified_within {
            let fresh = modified
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age <= within);
            if !fresh {
                return false;
            }
        }
        true
    }

    /// Does the file contain the `content:` text? (`true` if there is none.)
    pub fn matches_content(&self, path: &Path) -> bool {
        match &self.content {
            Some(c) => file_contains(path, &c.regex),
            None => true,
        }
    }

    pub(super) fn matches(
        &self,
        name: &str,
        is_dir: bool,
        meta: impl Fn() -> Option<fs::Metadata>,
        path: &Path,
    ) -> bool {
        if !self.matches_type(name, is_dir) || !self.matches_name(name) {
            return false;
        }
        if self.needs_meta() {
            let Some(m) = meta() else { return false };
            if !self.matches_size_and_age(is_dir, m.len(), m.modified().ok()) {
                return false;
            }
        }
        if self.content.is_some() && (is_dir || !self.matches_content(path)) {
            return false;
        }
        true
    }
}

pub(super) fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn make_term(token: &str, case_sensitive: bool) -> Term {
    let unquoted = token.trim_matches('"');
    let text = if case_sensitive {
        unquoted.to_owned()
    } else {
        unquoted.to_lowercase()
    };
    if !token.starts_with('"') && (text.contains('*') || text.contains('?')) {
        Term::Wildcard(text.chars().collect())
    } else {
        Term::Contains(text)
    }
}

/// `folded` is the lower-cased name for case-insensitive non-ASCII names,
/// otherwise the name itself; `ascii_ci` asks for ASCII case folding on the fly.
fn term_matches(term: &Term, original: &str, folded: &str, ascii_ci: bool) -> bool {
    match term {
        Term::Contains(s) if ascii_ci => contains_ascii_ci(folded, s),
        Term::Contains(s) => folded.contains(s.as_str()),
        Term::Wildcard(p) if ascii_ci => {
            let n: Vec<char> = folded
                .bytes()
                .map(|b| b.to_ascii_lowercase() as char)
                .collect();
            wildcard(p, &n)
        }
        Term::Wildcard(p) => {
            let n: Vec<char> = folded.chars().collect();
            wildcard(p, &n)
        }
        // The regex carries its own (?i) flag.
        Term::Regex(r) => r.is_match(original),
    }
}

/// Substring search ignoring ASCII case; `needle` is already lower-case.
fn contains_ascii_ci(hay: &str, needle: &str) -> bool {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() {
        return true;
    }
    if n.len() > h.len() {
        return false;
    }
    let first = n[0];
    (0..=h.len() - n.len()).any(|i| {
        h[i].to_ascii_lowercase() == first
            && h[i + 1..i + n.len()]
                .iter()
                .zip(&n[1..])
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
    })
}

/// Iterative `*`/`?` matcher with backtracking (linear in practice).
pub(crate) fn wildcard(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = n;
            p += 1;
        } else if let Some(s) = star {
            p = s + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let idx = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let num: f64 = num.parse().ok()?;
    let mult: f64 = match unit.trim() {
        "" | "b" => 1.0,
        "k" | "kb" => 1024.0,
        "m" | "mb" => 1024.0 * 1024.0,
        "g" | "gb" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" => 1024.0f64.powi(4),
        _ => return None,
    };
    Some((num * mult) as u64)
}

fn parse_age(s: &str) -> Option<Duration> {
    let s = s.trim().to_ascii_lowercase();
    match s.as_str() {
        "today" => return Some(Duration::from_secs(24 * 3600)),
        "week" => return Some(Duration::from_secs(7 * 24 * 3600)),
        "month" => return Some(Duration::from_secs(31 * 24 * 3600)),
        "year" => return Some(Duration::from_secs(365 * 24 * 3600)),
        _ => {}
    }
    let s = s.trim_start_matches('<');
    let idx = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let n: u64 = num.parse().ok()?;
    let secs = match unit {
        "m" | "min" => 60,
        "h" => 3600,
        "" | "d" => 24 * 3600,
        "w" => 7 * 24 * 3600,
        "y" => 365 * 24 * 3600,
        _ => return None,
    };
    Some(Duration::from_secs(n * secs))
}

/// Max file size scanned by `content:` searches.
const CONTENT_LIMIT: u64 = 32 * 1024 * 1024;

thread_local! {
    /// Reused read buffer: content searches touch thousands of files.
    static CONTENT_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn file_contains(path: &Path, pattern: &regex::bytes::Regex) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    // Online-only cloud files aren't read: that would download them all.
    let skip = |m: fs::Metadata| m.len() > CONTENT_LIMIT || crate::cloud::online(&m);
    if file.metadata().map(skip).unwrap_or(true) {
        return false;
    }
    CONTENT_BUF.with(|buf| {
        let mut data = buf.borrow_mut();
        data.clear();
        if file.take(CONTENT_LIMIT).read_to_end(&mut data).is_err() {
            return false;
        }
        // Skip binary files (a NUL byte in the first 8 KB).
        if data.iter().take(8192).any(|&b| b == 0) {
            return false;
        }
        pattern.is_match(&data)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query() {
        let q = Query::parse(
            r#"report "big file" -tmp ext:PDF,.docx size:>1mb type:file modified:7d"#,
            false,
        );
        assert_eq!(
            q.include,
            vec![
                Term::Contains("report".into()),
                Term::Contains("big file".into())
            ]
        );
        assert_eq!(q.exclude, vec![Term::Contains("tmp".into())]);
        assert_eq!(q.extensions, vec!["pdf", "docx"]);
        assert_eq!(q.min_size, Some(1024 * 1024));
        assert_eq!(q.kind, Some(KindFilter::Files));
        assert_eq!(q.modified_within, Some(Duration::from_secs(7 * 86400)));
        assert!(!q.is_empty());
        assert!(Query::parse("   ", false).is_empty());
    }

    #[test]
    fn name_matching() {
        let q = Query::parse("*.RS", false);
        assert!(q.matches_name("main.rs"));
        assert!(!q.matches_name("main.rs.bak"));
        let q = Query::parse("img_?? -raw", false);
        assert!(q.matches_name("IMG_01"));
        assert!(!q.matches_name("img_001"));
        let q = Query::parse("Read", true);
        assert!(q.matches_name("Readme"));
        assert!(!q.matches_name("readme"));
    }

    #[test]
    fn wildcards() {
        let w = |p: &str, n: &str| {
            wildcard(
                &p.chars().collect::<Vec<_>>(),
                &n.chars().collect::<Vec<_>>(),
            )
        };
        assert!(w("*", ""));
        assert!(w("a*b*c", "aXXbYYc"));
        assert!(!w("a*b*c", "aXXbYY"));
        assert!(w("*.tar.*", "x.tar.gz"));
    }

    #[test]
    fn sizes_and_ages() {
        assert_eq!(parse_size("10kb"), Some(10 * 1024));
        assert_eq!(
            parse_size("1.5 GB"),
            Some((1.5 * 1024.0 * 1024.0 * 1024.0) as u64)
        );
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_age("24h"), Some(Duration::from_secs(86400)));
        assert_eq!(parse_age("2w"), Some(Duration::from_secs(14 * 86400)));
    }

    #[test]
    fn regex_and_kind_terms() {
        let q = Query::parse(r"/^img_\d{4}\.jpe?g$/", false);
        assert!(q.matches_name("IMG_2041.JPG"));
        assert!(!q.matches_name("IMG_204.jpg"));
        let q = Query::parse(r"regex:^a.c$", true);
        assert!(q.matches_name("abc") && !q.matches_name("ABC"));
        // An invalid regex falls back to a plain substring.
        assert!(Query::parse("/[/", false).matches_name("a/[/b"));

        let q = Query::parse("kind:image,video", false);
        assert!(q.matches_meta("cat.PNG", false, 1, None));
        assert!(q.matches_meta("clip.mkv", false, 1, None));
        assert!(!q.matches_meta("notes.txt", false, 1, None));
        assert!(!q.matches_meta("Pictures", true, 0, None));
        assert!(Query::parse("type:docs", false).matches_meta("a.pdf", false, 1, None));
    }

    #[test]
    fn case_folding() {
        // Non-ASCII names are folded with full Unicode rules.
        let q = Query::parse("ÜBER", false);
        assert!(q.matches_name("über-uns.txt"));
        assert!(Query::parse("straße", false).matches_name("STRAßE.md"));
        assert!(contains_ascii_ci("Annual Report.PDF", "report.pdf"));
        assert!(!contains_ascii_ci("rep", "report"));
        assert!(contains_ascii_ci("x", ""));
    }

    #[test]
    fn meta_filters() {
        let q = Query::parse("size:>1kb modified:1d", false);
        let now = SystemTime::now();
        assert!(q.matches_meta("a", false, 2048, Some(now)));
        assert!(!q.matches_meta("a", false, 10, Some(now)));
        assert!(!q.matches_meta("a", false, 2048, Some(now - Duration::from_secs(3 * 86400))));
        assert!(!q.matches_meta("a", false, 2048, None));
        assert!(Query::parse("content:x", false).needs_content());
    }
}
