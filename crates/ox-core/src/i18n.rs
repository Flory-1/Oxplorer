//! Translations. The English text is the key: `t("Cut")` gives "Ausschneiden"
//! when the interface is German and "Cut" otherwise, so code reads the same
//! in both languages. Missing translations fall back to English.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::LazyLock;

use crate::settings::Language;

const ENGLISH: u8 = 0;
const GERMAN: u8 = 1;

static CURRENT: AtomicU8 = AtomicU8::new(ENGLISH);

/// Whether Windows (or the desktop) is set to German.
fn system_is_german() -> bool {
    sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("de"))
}

/// Switch the interface language (takes effect on the next frame).
pub fn set_language(language: Language) {
    let german = match language {
        Language::System => system_is_german(),
        Language::English => false,
        Language::German => true,
    };
    CURRENT.store(if german { GERMAN } else { ENGLISH }, Ordering::Relaxed);
}

pub fn is_german() -> bool {
    CURRENT.load(Ordering::Relaxed) == GERMAN
}

static DE: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| crate::i18n_de::entries().copied().collect());

/// The text in the current language.
pub fn t(english: &'static str) -> &'static str {
    t_in(english, is_german())
}

fn t_in(english: &'static str, german: bool) -> &'static str {
    if german {
        DE.get(english).copied().unwrap_or(english)
    } else {
        english
    }
}

/// Like [`t`] for text that isn't fixed in the code (e.g. labels the user
/// can change): translated if it is a known text, else as it is.
pub fn t_any(english: &str) -> &str {
    if is_german() {
        DE.get(english).copied().unwrap_or(english)
    } else {
        english
    }
}

/// German translation of `english`, if there is one (for tests and tools).
pub fn german(english: &str) -> Option<&'static str> {
    DE.get(english).copied()
}

/// Fill `{name}` placeholders in `template` (see [`trf!`](crate::trf)).
pub fn fill(template: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// Translate a template and fill in its `{name}` placeholders:
/// `trf!("{n} selected", n = 3)` → "3 ausgewählt".
#[macro_export]
macro_rules! trf {
    ($template:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::t($template),
            &[$((stringify!($name), &$value as &dyn std::fmt::Display)),*],
        )
    };
}

/// "1 file", "3 files" — in the current language.
pub fn count(n: impl Into<u64>, noun: &str) -> String {
    count_in(n.into(), noun, is_german())
}

fn count_in(n: u64, noun: &str, german: bool) -> String {
    if !german {
        return crate::format::count(n, noun);
    }
    let (one, many) = match noun {
        "item" => ("Element", "Elemente"),
        "file" => ("Datei", "Dateien"),
        "folder" => ("Ordner", "Ordner"),
        "link" => ("Verknüpfung", "Verknüpfungen"),
        "result" => ("Ergebnis", "Ergebnisse"),
        "change" => ("Änderung", "Änderungen"),
        "character" => ("Zeichen", "Zeichen"),
        "error" => ("Fehler", "Fehler"),
        "transfer" => ("Übertragung", "Übertragungen"),
        "tab" => ("Tab", "Tabs"),
        "failed file" => ("fehlgeschlagene Datei", "fehlgeschlagene Dateien"),
        "picture" => ("Bild", "Bilder"),
        _ => return crate::format::count(n, noun),
    };
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    // These don't switch the global language: other tests run in parallel.
    #[test]
    fn translating() {
        assert_eq!(t_in("Cut", true), "Ausschneiden");
        assert_eq!(t_in("Cut", false), "Cut");
        assert_eq!(
            t_in("No such text in the table", true),
            "No such text in the table"
        );
        assert_eq!(count_in(3, "file", true), "3 Dateien");
        assert_eq!(count_in(1, "item", true), "1 Element");
        assert_eq!(count_in(3, "file", false), "3 files");
        let n: &dyn std::fmt::Display = &3;
        assert_eq!(
            fill(t_in("{n} selected", true), &[("n", n)]),
            "3 ausgewählt"
        );
    }

    #[test]
    fn table_has_no_duplicates_or_blanks() {
        let mut seen = std::collections::HashSet::new();
        for (en, de) in crate::i18n_de::entries() {
            assert!(seen.insert(en), "duplicate: {en}");
            assert!(!de.trim().is_empty(), "empty translation for {en}");
        }
    }
}
