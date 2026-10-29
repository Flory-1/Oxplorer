//! Suggestions under the search box: earlier searches that match what is
//! typed, and completions of the query syntax (`ki` → `kind:`, `kind:im` →
//! `kind:image`) for the word being typed.

use crate::i18n::t;
use crate::tags::TagColor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    /// An earlier search.
    History,
    /// A filter name (`kind:`).
    Filter,
    /// A value for a filter (`kind:image`).
    Value,
    /// A folder or a file completing a typed path ([`crate::path_complete`]).
    Folder,
    File,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    /// The whole text of the search box once it's accepted.
    pub text: String,
    /// What the list shows.
    pub label: String,
    /// A short explanation (translated), or "".
    pub detail: &'static str,
}

/// A filter of the query syntax (see [`crate::search`]).
struct Filter {
    key: &'static str,
    /// Other names the query parser takes for it.
    aliases: &'static [&'static str],
    /// English; translated when shown.
    detail: &'static str,
    values: &'static [&'static str],
    /// Takes a comma-separated list (`ext:pdf,docx`).
    list: bool,
}

const FILTERS: &[Filter] = &[
    Filter {
        key: "ext",
        aliases: &[],
        detail: "Extension, e.g. ext:pdf,docx",
        values: &[
            "pdf", "docx", "xlsx", "pptx", "txt", "jpg", "png", "mp4", "mp3", "zip", "exe",
        ],
        list: true,
    },
    Filter {
        key: "type",
        aliases: &[],
        detail: "Only files or only folders",
        values: &["file", "folder"],
        list: false,
    },
    Filter {
        key: "kind",
        aliases: &[],
        detail: "Kind of file: image, video, document, …",
        values: &[
            "image",
            "video",
            "audio",
            "document",
            "pdf",
            "spreadsheet",
            "presentation",
            "archive",
            "code",
            "text",
            "app",
            "font",
        ],
        list: true,
    },
    Filter {
        key: "size",
        aliases: &[],
        detail: "Size, e.g. size:>10mb",
        values: &[">1mb", ">10mb", ">100mb", ">1gb", "<100kb", "<1mb"],
        list: false,
    },
    Filter {
        key: "modified",
        aliases: &["date"],
        detail: "Changed within, e.g. modified:7d",
        values: &["today", "24h", "7d", "week", "month", "year"],
        list: false,
    },
    Filter {
        key: "content",
        aliases: &["contains"],
        detail: "Text inside files",
        values: &[],
        list: false,
    },
    Filter {
        key: "tag",
        aliases: &[],
        detail: "Has a colour tag",
        // The tag colours, see `values`.
        values: &[],
        list: false,
    },
    Filter {
        key: "regex",
        aliases: &["re"],
        detail: "Regular expression on the name",
        values: &[],
        list: false,
    },
];

fn values(f: &Filter) -> Vec<String> {
    if f.key == "tag" {
        return TagColor::ALL
            .iter()
            .map(|c| c.name().to_lowercase())
            .collect();
    }
    f.values.iter().map(|v| v.to_string()).collect()
}

/// Where the last word of `text` starts (a byte index): after the last
/// space outside quotes. `None` while a quote is open.
fn last_word_start(text: &str) -> Option<usize> {
    let mut start = 0;
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            start = i + c.len_utf8();
        }
    }
    (!quoted).then_some(start)
}

/// Completions of the query syntax for the word at the end of `text`.
pub fn completions(text: &str) -> Vec<Suggestion> {
    let Some(start) = last_word_start(text) else {
        return Vec::new();
    };
    let (head, word) = text.split_at(start);
    // Filters can't be negated (`-ext:…`), so `-…` is a name.
    if word.is_empty() || word.starts_with('-') {
        return Vec::new();
    }
    let Some((key, value)) = word.split_once(':') else {
        let typed = word.to_lowercase();
        return FILTERS
            .iter()
            .filter(|f| f.key.starts_with(&typed))
            .map(|f| Suggestion {
                kind: SuggestionKind::Filter,
                text: format!("{head}{}:", f.key),
                label: format!("{}:", f.key),
                detail: t(f.detail),
            })
            .collect();
    };
    let lower = key.to_lowercase();
    let Some(f) = FILTERS
        .iter()
        .find(|f| f.key == lower || f.aliases.contains(&lower.as_str()))
    else {
        return Vec::new();
    };
    // In a list only the last value is completed; the ones before it aren't offered again.
    let (done, partial) = match value.rfind(',') {
        Some(i) if f.list => value.split_at(i + 1),
        _ => ("", value),
    };
    let taken: Vec<String> = done.split(',').map(|v| v.trim().to_lowercase()).collect();
    let partial = partial.to_lowercase();
    values(f)
        .into_iter()
        .filter(|v| v.starts_with(&partial) && *v != partial && !taken.contains(v))
        .map(|v| Suggestion {
            kind: SuggestionKind::Value,
            text: format!("{head}{key}:{done}{v}"),
            label: format!("{key}:{done}{v}"),
            detail: "",
        })
        .collect()
}

/// Earlier searches that fit `text`, at most `max`: all of them (newest
/// first) for an empty box, else those starting with it, then fuzzy matches
/// (from two characters on), best first. The text itself isn't offered.
pub fn history_matches<'a>(text: &str, history: &'a [String], max: usize) -> Vec<&'a String> {
    let typed = text.trim().to_lowercase();
    if typed.is_empty() {
        return history.iter().take(max).collect();
    }
    let mut out: Vec<&String> = history
        .iter()
        .filter(|e| {
            let e = e.to_lowercase();
            e.starts_with(&typed) && e != typed
        })
        .take(max)
        .collect();
    if typed.chars().count() >= 2 {
        let mut fuzzy: Vec<(i32, &String)> = history
            .iter()
            .filter(|e| !out.contains(e) && e.to_lowercase() != typed)
            .filter_map(|e| crate::fuzzy::score(&typed, e).map(|s| (s, e)))
            .collect();
        // Stable: equally good ones stay newest first.
        fuzzy.sort_by_key(|&(s, _)| std::cmp::Reverse(s));
        out.extend(fuzzy.into_iter().map(|(_, e)| e));
    }
    out.truncate(max);
    out
}

/// What to offer under the search box for `text`, at most `max`: while a
/// filter is being typed its completions come first, then earlier searches.
pub fn suggest(text: &str, history: &[String], max: usize) -> Vec<Suggestion> {
    let mut out = completions(text);
    out.extend(
        history_matches(text, history, max)
            .into_iter()
            .map(|e| Suggestion {
                kind: SuggestionKind::History,
                text: e.clone(),
                label: e.clone(),
                detail: "",
            }),
    );
    out.truncate(max);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &[Suggestion]) -> Vec<&str> {
        s.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn completes_filter_names() {
        assert_eq!(texts(&completions("report ki")), ["report kind:"]);
        assert_eq!(texts(&completions("t")), ["type:", "tag:"]);
        assert_eq!(texts(&completions("MOD")), ["modified:"]);
        assert_eq!(completions("ki")[0].kind, SuggestionKind::Filter);
        // Nothing for an empty word, a negated one or an unknown filter.
        assert!(completions("").is_empty());
        assert!(completions("report ").is_empty());
        assert!(completions("-ki").is_empty());
        assert!(completions("xyz").is_empty());
        assert!(completions("C:").is_empty());
    }

    #[test]
    fn completes_values() {
        assert_eq!(texts(&completions("kind:im")), ["kind:image"]);
        assert_eq!(texts(&completions("type:")), ["type:file", "type:folder"]);
        assert_eq!(
            texts(&completions("a size:>1")),
            [
                "a size:>1mb",
                "a size:>10mb",
                "a size:>100mb",
                "a size:>1gb"
            ]
        );
        // Aliases and the case typed are kept.
        assert_eq!(texts(&completions("Date:to")), ["Date:today"]);
        // A complete value isn't offered again.
        assert!(completions("kind:image").is_empty());
        assert!(completions("content:").is_empty());
    }

    #[test]
    fn completes_the_last_value_of_a_list() {
        let s = completions("ext:pdf,d");
        assert_eq!(texts(&s), ["ext:pdf,docx"]);
        assert_eq!(s[0].kind, SuggestionKind::Value);
        // Values already in the list aren't offered.
        let s = completions("kind:image,");
        assert!(!texts(&s).contains(&"kind:image,image"));
        assert!(texts(&s).contains(&"kind:image,video"));
        // `type:` takes one value only.
        assert!(completions("type:file,").is_empty());
    }

    #[test]
    fn completes_tag_colours() {
        assert_eq!(texts(&completions("tag:r")), ["tag:red"]);
        assert_eq!(completions("tag:").len(), TagColor::ALL.len());
    }

    #[test]
    fn no_completions_inside_quotes() {
        assert!(completions("content:\"kind:im").is_empty());
        assert_eq!(texts(&completions("\"a b\" ki")), ["\"a b\" kind:"]);
    }

    #[test]
    fn history_by_prefix_then_fuzzy() {
        let h: Vec<String> = ["report 2024", "budget", "kind:image", "Report", "rpt old"]
            .map(String::from)
            .to_vec();
        // Empty box: all, newest first.
        assert_eq!(history_matches("", &h, 3), [&h[0], &h[1], &h[2]]);
        // Prefix (any case) first; the text itself isn't offered.
        let m = history_matches("report", &h, 10);
        assert_eq!(m, [&h[0]]);
        let m = history_matches("rep", &h, 10);
        assert_eq!(m[..2], [&h[0], &h[3]]);
        // Fuzzy only from two characters on.
        assert!(history_matches("rp", &h, 10).contains(&&h[4]));
        assert!(history_matches("rpt", &h, 10).contains(&&h[0]));
        assert_eq!(history_matches("x", &h, 10), Vec::<&String>::new());
    }

    #[test]
    fn suggest_puts_completions_first() {
        let h: Vec<String> = ["kind:video", "kind:image big"].map(String::from).to_vec();
        let s = suggest("kind:i", &h, 10);
        assert_eq!(s[0].text, "kind:image");
        assert_eq!(s[0].kind, SuggestionKind::Value);
        assert_eq!(s[1].text, "kind:image big");
        assert_eq!(s[1].kind, SuggestionKind::History);
        assert_eq!(suggest("kind:", &h, 3).len(), 3);
    }

    #[test]
    fn filter_explanations_are_translated() {
        for f in FILTERS {
            assert!(
                crate::i18n::german(f.detail).is_some(),
                "no German text for {:?}",
                f.detail
            );
        }
    }
}
