//! Bulk rename: build new names from a pattern, preview them, then apply
//! them safely.
//!
//! Pattern tokens: `{name}` (name without extension), `{ext}` (extension),
//! `{n}` / `{n:3}` (counter, optionally zero-padded), `{date}` (modified,
//! `YYYY-MM-DD`), `{parent}` (folder name). After the pattern, an optional
//! find & replace (plain or regex) and a case change are applied.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::fsops::{self, split_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Case {
    #[default]
    Keep,
    Lower,
    Upper,
    /// First letter of each word upper-case.
    Title,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenameRule {
    pub pattern: String,
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub case: Case,
    /// First counter value and increment for `{n}`.
    pub start: u32,
    pub step: u32,
    /// The pattern only builds the name; the original extension is kept.
    pub keep_extension: bool,
}

impl Default for RenameRule {
    fn default() -> Self {
        Self {
            pattern: "{name}".into(),
            find: String::new(),
            replace: String::new(),
            regex: false,
            case: Case::Keep,
            start: 1,
            step: 1,
            keep_extension: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenameItem {
    pub path: PathBuf,
    pub is_dir: bool,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub from: PathBuf,
    pub to: PathBuf,
    /// Why this item can't be renamed (the whole rename is blocked then).
    pub error: Option<String>,
}

impl Planned {
    pub fn changes(&self) -> bool {
        self.from != self.to
    }
}

fn expand(pattern: &str, stem: &str, ext: &str, n: u32, item: &RenameItem) -> String {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let token = &rest[open + 1..open + close];
        let (key, arg) = token.split_once(':').unwrap_or((token, ""));
        match key {
            "name" => out.push_str(stem),
            "ext" => out.push_str(ext),
            "n" => {
                let width: usize = arg.parse().unwrap_or(0);
                out.push_str(&format!("{n:0width$}"));
            }
            "date" => {
                if let Some(t) = item.modified {
                    let fmt = if arg.is_empty() { "%Y-%m-%d" } else { arg };
                    out.push_str(&crate::format::time(t, fmt));
                }
            }
            "parent" => {
                if let Some(p) = item.path.parent() {
                    out.push_str(&crate::entry::display_name(p));
                }
            }
            // Unknown tokens are kept literally.
            _ => out.push_str(&rest[open..open + close + 1]),
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

fn change_case(s: &str, case: Case) -> String {
    match case {
        Case::Keep => s.to_owned(),
        Case::Lower => s.to_lowercase(),
        Case::Upper => s.to_uppercase(),
        Case::Title => {
            let mut out = String::with_capacity(s.len());
            let mut start_of_word = true;
            for c in s.chars() {
                if start_of_word {
                    out.extend(c.to_uppercase());
                } else {
                    out.extend(c.to_lowercase());
                }
                start_of_word = c.is_whitespace() || c == '-' || c == '_' || c == '.';
            }
            out
        }
    }
}

fn key(path: &Path) -> String {
    // Windows and macOS file systems are case-insensitive.
    if cfg!(any(windows, target_os = "macos")) {
        path.to_string_lossy().to_lowercase()
    } else {
        path.to_string_lossy().into_owned()
    }
}

/// Compute new names for `items` (in the given order, which drives `{n}`).
pub fn plan(items: &[RenameItem], rule: &RenameRule) -> Vec<Planned> {
    let finder = if rule.regex && !rule.find.is_empty() {
        match regex::Regex::new(&rule.find) {
            Ok(r) => Some(Ok(r)),
            Err(e) => Some(Err(format!("Invalid regular expression: {e}"))),
        }
    } else {
        None
    };

    let mut out: Vec<Planned> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let name = crate::entry::display_name(&item.path);
            let (stem, dot_ext) = if item.is_dir {
                (name.as_str(), "")
            } else {
                split_name(&name)
            };
            let ext = dot_ext.trim_start_matches('.');
            let n = rule
                .start
                .saturating_add(rule.step.saturating_mul(i as u32));
            let mut new = expand(&rule.pattern, stem, ext, n, item);
            let mut error = None;
            if !rule.find.is_empty() {
                match &finder {
                    Some(Ok(re)) => new = re.replace_all(&new, rule.replace.as_str()).into_owned(),
                    Some(Err(e)) => error = Some(e.clone()),
                    None => new = new.replace(&rule.find, &rule.replace),
                }
            }
            // Trim before the extension goes back on ("Holiday .jpg" → "Holiday.jpg").
            let mut new = change_case(&new, rule.case).trim().to_owned();
            if rule.keep_extension && !dot_ext.is_empty() {
                new.push_str(dot_ext);
            }
            if error.is_none() {
                error = fsops::validate_name(&new).err();
            }
            let parent = item.path.parent().unwrap_or(Path::new(""));
            Planned {
                from: item.path.clone(),
                to: parent.join(&new),
                error,
            }
        })
        .collect();

    // Two items must not end up with the same name …
    let mut seen: HashMap<String, usize> = HashMap::new();
    for p in &out {
        *seen.entry(key(&p.to)).or_default() += 1;
    }
    // … and nothing may overwrite a file that isn't renamed away.
    let leaving: HashSet<String> = out
        .iter()
        .filter(|p| p.changes())
        .map(|p| key(&p.from))
        .collect();
    for p in &mut out {
        if p.error.is_some() || !p.changes() {
            continue;
        }
        let k = key(&p.to);
        if seen[&k] > 1 {
            p.error = Some("Another item would get the same name.".into());
        } else if fsops::exists(&p.to) && !leaving.contains(&k) && k != key(&p.from) {
            p.error = Some("An item with this name already exists.".into());
        }
    }
    out
}

/// Apply a plan. Uses temporary names first so swaps (a↔b) work; if a step
/// fails, completed renames are undone. Returns the number of renamed items.
pub fn apply(plan: &[Planned]) -> Result<usize, String> {
    if let Some(p) = plan.iter().find(|p| p.error.is_some()) {
        return Err(format!(
            "{}: {}",
            crate::entry::display_name(&p.from),
            p.error.as_deref().unwrap_or_default()
        ));
    }
    let todo: Vec<&Planned> = plan.iter().filter(|p| p.changes()).collect();
    let pid = std::process::id();
    let temps: Vec<PathBuf> = todo
        .iter()
        .enumerate()
        .map(|(i, p)| {
            p.from
                .parent()
                .unwrap_or(Path::new(""))
                .join(format!(".oxplorer-rename-{pid}-{i}"))
        })
        .collect();

    // Phase 1: everything to a temporary name.
    for (i, p) in todo.iter().enumerate() {
        if let Err(e) = fs::rename(&p.from, &temps[i]) {
            for j in (0..i).rev() {
                let _ = fs::rename(&temps[j], &todo[j].from);
            }
            return Err(format!("{}: {e}", p.from.display()));
        }
    }
    // Phase 2: temporary name to the final name.
    for (i, p) in todo.iter().enumerate() {
        if let Err(e) = fs::rename(&temps[i], &p.to) {
            for j in (0..i).rev() {
                let _ = fs::rename(&todo[j].to, &temps[j]);
            }
            for j in 0..todo.len() {
                let _ = fs::rename(&temps[j], &todo[j].from);
            }
            return Err(format!("{}: {e}", p.to.display()));
        }
    }
    Ok(todo.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(dir: &Path, names: &[&str]) -> Vec<RenameItem> {
        names
            .iter()
            .map(|n| {
                let path = dir.join(n);
                fs::write(&path, n).unwrap();
                RenameItem {
                    path,
                    is_dir: false,
                    modified: None,
                }
            })
            .collect()
    }

    fn new_names(plan: &[Planned]) -> Vec<String> {
        plan.iter()
            .map(|p| crate::entry::display_name(&p.to))
            .collect()
    }

    #[test]
    fn pattern_tokens_and_counter() {
        let dir = tempfile::tempdir().unwrap();
        let it = items(dir.path(), &["IMG_001.JPG", "IMG_002.JPG", "notes"]);
        let rule = RenameRule {
            pattern: "Holiday {n:3} ({name})".into(),
            start: 9,
            step: 1,
            ..Default::default()
        };
        assert_eq!(
            new_names(&plan(&it, &rule)),
            vec![
                "Holiday 009 (IMG_001).JPG",
                "Holiday 010 (IMG_002).JPG",
                "Holiday 011 (notes)"
            ]
        );
        let rule = RenameRule {
            pattern: "{name}.{ext}.bak {unknown}".into(),
            keep_extension: false,
            ..Default::default()
        };
        assert_eq!(
            new_names(&plan(&it[..1], &rule)),
            vec!["IMG_001.JPG.bak {unknown}"]
        );
    }

    #[test]
    fn find_replace_and_case() {
        let dir = tempfile::tempdir().unwrap();
        let it = items(dir.path(), &["my_holiday_photo.jpeg"]);
        let rule = RenameRule {
            find: "_".into(),
            replace: " ".into(),
            case: Case::Title,
            ..Default::default()
        };
        assert_eq!(new_names(&plan(&it, &rule)), vec!["My Holiday Photo.jpeg"]);
        let rule = RenameRule {
            find: r"(\w+)_(\w+)_(\w+)".into(),
            replace: "$3-$1".into(),
            regex: true,
            case: Case::Upper,
            ..Default::default()
        };
        assert_eq!(new_names(&plan(&it, &rule)), vec!["PHOTO-MY.jpeg"]);
        let rule = RenameRule {
            find: "(".into(),
            regex: true,
            ..Default::default()
        };
        assert!(plan(&it, &rule)[0]
            .error
            .as_deref()
            .unwrap()
            .contains("Invalid regular"));
    }

    #[test]
    fn detects_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let it = items(dir.path(), &["a.txt", "b.txt"]);
        fs::write(dir.path().join("taken.txt"), "").unwrap();
        // Both would become "same.txt".
        let rule = RenameRule {
            pattern: "same".into(),
            ..Default::default()
        };
        assert!(plan(&it, &rule).iter().all(|p| p.error.is_some()));
        // An existing, unrelated file.
        let rule = RenameRule {
            pattern: "taken".into(),
            ..Default::default()
        };
        assert!(plan(&it[..1], &rule)[0]
            .error
            .as_deref()
            .unwrap()
            .contains("already exists"));
        // Invalid characters.
        let rule = RenameRule {
            pattern: "a:b".into(),
            ..Default::default()
        };
        assert!(plan(&it[..1], &rule)[0].error.is_some());
        assert!(apply(&plan(&it[..1], &rule)).is_err());
    }

    #[test]
    fn applies_swaps_safely() {
        let dir = tempfile::tempdir().unwrap();
        let it = items(dir.path(), &["a.txt", "b.txt", "c.txt"]);
        // a -> b, b -> a (a swap) and c stays.
        let swap = vec![
            Planned {
                from: it[0].path.clone(),
                to: it[1].path.clone(),
                error: None,
            },
            Planned {
                from: it[1].path.clone(),
                to: it[0].path.clone(),
                error: None,
            },
            Planned {
                from: it[2].path.clone(),
                to: it[2].path.clone(),
                error: None,
            },
        ];
        assert_eq!(apply(&swap).unwrap(), 2);
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "b.txt"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("b.txt")).unwrap(),
            "a.txt"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("c.txt")).unwrap(),
            "c.txt"
        );

        // The planner accepts a swap too (targets are being renamed away).
        let rule = RenameRule {
            pattern: "{n}".into(),
            ..Default::default()
        };
        let p = plan(&it, &rule);
        assert!(p.iter().all(|x| x.error.is_none()), "{p:?}");
        assert_eq!(apply(&p).unwrap(), 3);
        assert!(dir.path().join("1.txt").exists() && dir.path().join("3.txt").exists());
    }
}
