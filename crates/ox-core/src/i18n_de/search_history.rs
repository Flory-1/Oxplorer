//! German translations for the search history and the suggestions under the
//! search box (continued from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Suggestions
    ("Remove from history", "Aus dem Verlauf entfernen"),
    ("Clear search history", "Suchverlauf löschen"),
    ("Search history cleared", "Suchverlauf gelöscht"),
    // Query syntax completions
    (
        "Extension, e.g. ext:pdf,docx",
        "Dateiendung, z. B. ext:pdf,docx",
    ),
    ("Only files or only folders", "Nur Dateien oder nur Ordner"),
    (
        "Kind of file: image, video, document, …",
        "Dateiart: image, video, document, …",
    ),
    ("Size, e.g. size:>10mb", "Größe, z. B. size:>10mb"),
    (
        "Changed within, e.g. modified:7d",
        "Geändert innerhalb, z. B. modified:7d",
    ),
    ("Text inside files", "Text in Dateien"),
    ("Has a colour tag", "Hat einen Farb-Tag"),
    (
        "Regular expression on the name",
        "Regulärer Ausdruck auf den Namen",
    ),
    // Settings
    ("Search history", "Suchverlauf"),
    ("🗑 Clear", "🗑 Löschen"),
    ("Remember searches", "Suchen merken"),
    (
        "Suggest earlier searches under the search box. Kept only on this computer.",
        "Frühere Suchen unter dem Suchfeld vorschlagen. Nur auf diesem Computer gespeichert.",
    ),
    ("Nothing remembered yet", "Noch nichts gemerkt"),
];
