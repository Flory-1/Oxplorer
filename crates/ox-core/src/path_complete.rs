//! Completing a path while it is typed (the *New link* dialog and the
//! address bar): the folders and files of the typed parent folder whose
//! names start with what follows the last separator, and the names of
//! environment variables after a `%`.

use std::path::{Path, PathBuf};

use crate::natural::natural_cmp;

/// `%USERPROFILE%\Desktop` → `C:\Users\me\Desktop`, and `~` → home on all
/// platforms. Unknown variables stay as typed.
pub fn expand(text: &str) -> String {
    expand_with(text, &|name| std::env::var(name).ok())
}

fn expand_with(text: &str, var: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = text.to_owned();
    if let Some(rest) = out.strip_prefix('~') {
        if rest.is_empty() || rest.starts_with(['\\', '/']) {
            out = format!("{}{rest}", crate::places::home_dir().display());
        }
    }
    let mut from = 0;
    while let Some(start) = out[from..].find('%').map(|i| i + from) {
        let Some(len) = out[start + 1..].find('%') else {
            break;
        };
        let name = &out[start + 1..start + 1 + len];
        match var(name) {
            Some(v) => {
                out.replace_range(start..start + len + 2, &v);
                from = start + v.len();
            }
            None => from = start + len + 2,
        }
    }
    out
}

/// One way to go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The whole text once it's taken (folders end with a separator, so
    /// that their contents come next).
    pub text: String,
    /// The name shown in the list.
    pub name: String,
    pub is_dir: bool,
}

impl Completion {
    /// For the list under a text field (shared with the search box's).
    pub fn into_suggestion(self) -> crate::search_history::Suggestion {
        use crate::search_history::{Suggestion, SuggestionKind};
        Suggestion {
            kind: if self.is_dir {
                SuggestionKind::Folder
            } else {
                SuggestionKind::File
            },
            text: self.text,
            label: self.name,
            detail: "",
        }
    }
}

/// Up to `max` ways to complete `text`: entries of the folder before the
/// last separator whose names start with the rest (ignoring case), folders
/// first. A relative parent is taken inside `base`. `%NA` lists variables
/// whose value is a folder. Network paths (`\\server\…`) aren't listed,
/// so that a slow server doesn't hold up typing.
pub fn complete(text: &str, base: Option<&Path>, max: usize) -> Vec<Completion> {
    complete_with(
        text,
        base,
        max,
        &|name| std::env::var(name).ok(),
        &env_names,
    )
}

fn env_names() -> Vec<(String, String)> {
    std::env::vars().collect()
}

type Vars<'a> = &'a dyn Fn() -> Vec<(String, String)>;

fn complete_with(
    text: &str,
    base: Option<&Path>,
    max: usize,
    var: &dyn Fn(&str) -> Option<String>,
    vars: Vars<'_>,
) -> Vec<Completion> {
    let text = text.trim_start().trim_start_matches('"');
    if text.starts_with(r"\\") || text.starts_with("//") || max == 0 {
        return Vec::new();
    }
    let Some(cut) = text.rfind(['\\', '/']) else {
        return complete_var(text, max, vars);
    };
    let (parent_text, prefix) = (&text[..=cut], &text[cut + 1..]);
    let sep = &text[cut..=cut];
    let expanded = expand_with(parent_text, var);
    let mut parent = PathBuf::from(&expanded);
    if parent.is_relative() && !expanded.starts_with(['\\', '/']) {
        match base {
            Some(b) => parent = b.join(parent),
            None => return Vec::new(),
        }
    }
    let Ok(items) = std::fs::read_dir(&parent) else {
        return Vec::new();
    };
    let lower = prefix.to_lowercase();
    let mut found: Vec<(bool, String)> = items
        .flatten()
        .filter_map(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            if !name.to_lowercase().starts_with(&lower) {
                return None;
            }
            // Through links: a link to a folder completes like a folder.
            let is_dir = std::fs::metadata(item.path()).is_ok_and(|m| m.is_dir());
            Some((is_dir, name))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| natural_cmp(&a.1, &b.1)));
    found.truncate(max);
    found
        .into_iter()
        .map(|(is_dir, name)| Completion {
            text: format!("{parent_text}{name}{}", if is_dir { sep } else { "" }),
            name,
            is_dir,
        })
        .collect()
}

/// `%US` → `%USERPROFILE%\` and the other variables that name a folder.
fn complete_var(text: &str, max: usize, vars: Vars<'_>) -> Vec<Completion> {
    let Some(prefix) = text.strip_prefix('%') else {
        return Vec::new();
    };
    if prefix.contains('%') {
        return Vec::new();
    }
    let lower = prefix.to_lowercase();
    let mut found: Vec<String> = vars()
        .into_iter()
        .filter(|(name, value)| {
            name.to_lowercase().starts_with(&lower) && Path::new(value).is_dir()
        })
        .map(|(name, _)| name)
        .collect();
    found.sort_by(|a, b| natural_cmp(a, b));
    found.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    found.truncate(max);
    found
        .into_iter()
        .map(|name| Completion {
            text: format!("%{name}%{}", std::path::MAIN_SEPARATOR),
            name: format!("%{name}%"),
            is_dir: true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(c: &[Completion]) -> Vec<&str> {
        c.iter().map(|c| c.name.as_str()).collect()
    }

    fn texts(c: &[Completion]) -> Vec<&str> {
        c.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn expands_variables_and_home() {
        let var = |n: &str| (n.eq_ignore_ascii_case("ROOT")).then(|| "C:\\Data".to_owned());
        assert_eq!(expand_with(r"%ROOT%\x", &var), r"C:\Data\x");
        assert_eq!(expand_with(r"%NOPE%\%ROOT%", &var), r"%NOPE%\C:\Data");
        assert_eq!(expand_with("100% sure", &var), "100% sure");
        let home = crate::places::home_dir().display().to_string();
        assert_eq!(expand_with("~", &var), home);
        assert_eq!(expand_with(r"~\a", &var), format!(r"{home}\a"));
        // Not a home folder: a name that starts with ~.
        assert_eq!(expand_with("~draft", &var), "~draft");
    }

    #[test]
    fn lists_matching_entries_folders_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("Docs")).unwrap();
        fs::create_dir(root.join("downloads")).unwrap();
        fs::write(root.join("diary.txt"), "x").unwrap();
        fs::write(root.join("other.txt"), "x").unwrap();
        let none = |_: &str| None;
        let no_vars = || Vec::new();
        let typed = format!("{}{}d", root.display(), std::path::MAIN_SEPARATOR);
        let c = complete_with(&typed, None, 20, &none, &no_vars);
        assert_eq!(names(&c), ["Docs", "downloads", "diary.txt"]);
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(c[0].text, format!("{}{sep}Docs{sep}", root.display()));
        assert_eq!(c[2].text, format!("{}{sep}diary.txt", root.display()));
        assert!(c[0].is_dir && !c[2].is_dir);
        // At most `max`, and the separator the user typed.
        let typed = format!("{}/", root.display());
        let c = complete_with(&typed, None, 2, &none, &no_vars);
        assert_eq!(names(&c), ["Docs", "downloads"]);
        assert_eq!(c[0].text, format!("{}/Docs/", root.display()));
        // Nothing matches; the parent doesn't exist.
        let typed = format!("{}{sep}zz", root.display());
        assert!(complete_with(&typed, None, 20, &none, &no_vars).is_empty());
        let typed = format!("{}{sep}missing{sep}", root.display());
        assert!(complete_with(&typed, None, 20, &none, &no_vars).is_empty());
    }

    #[test]
    fn relative_parents_variables_and_network_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("sub").join("inner")).unwrap();
        let none = |_: &str| None;
        let no_vars = || Vec::new();
        // Relative to the base folder (without a base: nothing).
        let c = complete_with("sub/i", Some(root), 20, &none, &no_vars);
        assert_eq!(texts(&c), ["sub/inner/"]);
        assert!(complete_with("sub/i", None, 20, &none, &no_vars).is_empty());
        // Inside a variable's folder, keeping the variable in the text.
        let dir = root.display().to_string();
        let var = |n: &str| (n == "PLACE").then(|| dir.clone());
        let c = complete_with(r"%PLACE%\s", None, 20, &var, &no_vars);
        assert_eq!(texts(&c), [r"%PLACE%\sub\"]);
        // Variable names that name folders.
        let vars = || {
            vec![
                ("PLACE".to_owned(), dir.clone()),
                ("PATHEXT".to_owned(), ".EXE".to_owned()),
                ("Other".to_owned(), dir.clone()),
            ]
        };
        let c = complete_with("%pl", None, 20, &none, &vars);
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(texts(&c), [format!("%PLACE%{sep}")]);
        assert!(complete_with("%pa", None, 20, &none, &vars).is_empty());
        assert!(complete_with("plain", None, 20, &none, &vars).is_empty());
        assert!(complete_with(r"\\server\sh", None, 20, &none, &vars).is_empty());
    }
}
