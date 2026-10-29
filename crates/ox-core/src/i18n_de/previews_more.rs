//! German translations for lossless JPEG rotation, data views inside
//! archives and comparing two text files (continued from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Lossless JPEG rotation
    (
        "Re-encode JPEGs instead (for programs that ignore the orientation)",
        "JPEGs stattdessen neu kodieren (für Programme, die die Ausrichtung ignorieren)",
    ),
    (
        "Rotating and flipping a JPEG only changes its EXIF orientation: lossless, and every viewer that honours the tag shows it turned. On: the picture is turned and saved again at quality 95.",
        "Drehen und Spiegeln eines JPEGs ändert nur seine EXIF-Ausrichtung: verlustfrei, und jedes Programm, das sie beachtet, zeigt das Bild gedreht. Ein: Das Bild wird gedreht und mit Qualität 95 neu gespeichert.",
    ),
    // Data views and the hex view inside archives
    (
        "The database is too large to show from inside the archive.",
        "Die Datenbank ist zu groß, um sie aus dem Archiv heraus anzuzeigen.",
    ),
    ("Can't read this item.", "Dieses Element kann nicht gelesen werden."),
    (
        "Only the first {shown} of {total} are shown.",
        "Nur die ersten {shown} von {total} werden angezeigt.",
    ),
    // Comparing two text files
    ("⇄ Compare these two files", "⇄ Diese zwei Dateien vergleichen"),
    ("Compare these two files", "Diese zwei Dateien vergleichen"),
    ("Compare files", "Dateien vergleichen"),
    ("Swap the two files", "Die zwei Dateien tauschen"),
    ("The files are identical.", "Die Dateien sind identisch."),
    (
        "Too large to compare line by line (more than {size} or {lines} lines).",
        "Zu groß für einen zeilenweisen Vergleich (mehr als {size} oder {lines} Zeilen).",
    ),
    (
        "The bytes differ from offset {offset} on.",
        "Die Bytes unterscheiden sich ab Offset {offset}.",
    ),
    ("Ignore whitespace", "Leerraum ignorieren"),
    (
        "Encodings differ: {old} → {new}",
        "Kodierungen unterscheiden sich: {old} → {new}",
    ),
    (
        "Line endings differ: {old} → {new} (not shown as changes)",
        "Zeilenenden unterscheiden sich: {old} → {new} (nicht als Änderungen gezeigt)",
    ),
];
