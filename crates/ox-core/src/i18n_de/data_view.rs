//! German translations for data files and the hex view in the preview
//! (continued from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Tabs
    ("▦ Table", "▦ Tabelle"),
    ("🗃 Tree", "🗃 Baum"),
    ("🔐 Hex", "🔐 Hex"),
    // Info tab
    ("Rows", "Zeilen"),
    ("Delimiter", "Trennzeichen"),
    ("Comma", "Komma"),
    ("Semicolon", "Semikolon"),
    ("Tab", "Tabulator"),
    ("Vertical bar", "Senkrechter Strich"),
    ("Header row", "Kopfzeile"),
    ("Yes", "Ja"),
    ("No", "Nein"),
    ("Nodes", "Knoten"),
    ("Error", "Fehler"),
    ("Tables", "Tabellen"),
    ("Views", "Sichten"),
    ("Shown", "Angezeigt"),
    (
        "Line {line}, column {column}: {message}",
        "Zeile {line}, Spalte {column}: {message}",
    ),
    // Table and tree
    (
        "⚠ This isn't valid {format}.",
        "⚠ Das ist kein gültiges {format}.",
    ),
    (
        "The element <{name}> isn't closed.",
        "Das Element <{name}> wird nicht geschlossen.",
    ),
    (
        "The database has no tables.",
        "Die Datenbank hat keine Tabellen.",
    ),
    ("view", "Sicht"),
    ("There are no rows.", "Es gibt keine Zeilen."),
    (
        "Only the first {v1} rows are shown.",
        "Nur die ersten {v1} Zeilen werden angezeigt.",
    ),
    ("📋 Copy value", "📋 Wert kopieren"),
    ("📋 Copy row", "📋 Zeile kopieren"),
    ("📋 Copy path", "📋 Pfad kopieren"),
    ("Copied to the clipboard", "In die Zwischenablage kopiert"),
    ("The file is empty.", "Die Datei ist leer."),
    ("⏷ Expand all", "⏷ Alle aufklappen"),
    ("⏶ Collapse all", "⏶ Alle zuklappen"),
    (
        "Only the first {v1} nodes are shown.",
        "Nur die ersten {v1} Knoten werden angezeigt.",
    ),
    // Hex view
    ("Go to offset", "Gehe zu Offset"),
    (
        "Decimal, or hex with 0x (e.g. 0x1F00)",
        "Dezimal oder hexadezimal mit 0x (z. B. 0x1F00)",
    ),
    ("Not in this file", "Nicht in dieser Datei"),
    ("📋 Copy as hex", "📋 Als Hex kopieren"),
    ("📋 Copy as text", "📋 Als Text kopieren"),
    ("📋 Copy offset", "📋 Offset kopieren"),
    (
        "Only the first {v1} were copied.",
        "Nur die ersten {v1} wurden kopiert.",
    ),
];
