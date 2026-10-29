//! German translations for finding duplicates and empty folders (continued
//! from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Pages and address bar
    ("Duplicates: {name}", "Duplikate: {name}"),
    ("Empty folders: {name}", "Leere Ordner: {name}"),
    (
        "{icon}  Duplicates in {place}",
        "{icon}  Duplikate in {place}",
    ),
    (
        "{icon}  Empty folders in {place}",
        "{icon}  Leere Ordner in {place}",
    ),
    ("Duplicates", "Duplikate"),
    ("Empty folders", "Leere Ordner"),
    // Menus and palette
    ("✨ Clean up", "✨ Aufräumen"),
    ("📋 Find duplicates", "📋 Duplikate finden"),
    ("📂 Find empty folders", "📂 Leere Ordner finden"),
    (
        "Find duplicates in this folder",
        "Duplikate in diesem Ordner finden",
    ),
    (
        "Find empty folders in this folder",
        "Leere Ordner in diesem Ordner finden",
    ),
    // Progress and summary
    (
        "Looking through {files} in {folders}…",
        "{files} in {folders} werden durchsucht …",
    ),
    (
        "Looking through {folders}…",
        "{folders} werden durchsucht …",
    ),
    (
        "Comparing {files} of the same size… {bytes} read",
        "{files} gleicher Größe werden verglichen … {bytes} gelesen",
    ),
    ("The scan was stopped.", "Die Suche wurde abgebrochen."),
    ("No duplicates found.", "Keine Duplikate gefunden."),
    ("No empty folders found.", "Keine leeren Ordner gefunden."),
    ("Duplicate groups: {n}", "Duplikatgruppen: {n}"),
    ("Wasted space: {size}", "Verschwendeter Platz: {size}"),
    ("Empty folders: {n}", "Leere Ordner: {n}"),
    ("(took {time})", "(Dauer: {time})"),
    ("🔄 Scan again", "🔄 Erneut scannen"),
    (
        "⚠ {items} couldn't be read",
        "⚠ {items} konnten nicht gelesen werden",
    ),
    (
        "☁ {files} only in the cloud were left out (reading them would download them)",
        "☁ {files} nur in der Cloud wurden ausgelassen (Lesen würde sie herunterladen)",
    ),
    // The list
    (
        "Select all but the newest",
        "Alle außer der neuesten auswählen",
    ),
    (
        "Select all but the oldest",
        "Alle außer der ältesten auswählen",
    ),
    ("Select none", "Nichts auswählen"),
    ("{size} wasted", "{size} verschwendet"),
    ("⚠ Every copy is selected", "⚠ Alle Kopien sind ausgewählt"),
    ("Open file location", "Dateipfad öffnen"),
    ("🗑 Delete selected ({what})", "🗑 Auswahl löschen ({what})"),
    // Before deleting
    (
        "⚠ For {files} every copy is selected: none of them would be left.",
        "⚠ Bei {files} sind alle Kopien ausgewählt: Davon bliebe keine übrig.",
    ),
    (
        "Delete every copy of these anyway",
        "Trotzdem alle Kopien davon löschen",
    ),
];
