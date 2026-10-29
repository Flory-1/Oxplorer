//! German translations for the disk usage view (continued from
//! `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Opening it
    ("Disk usage: {name}", "Speicherbelegung: {name}"),
    ("🖴  Disk usage of {path}", "🖴  Speicherbelegung von {path}"),
    ("🖴 Disk usage", "🖴 Speicherbelegung"),
    (
        "What takes up the space in here, as a map of rectangles",
        "Was hier den Platz belegt, als Karte aus Rechtecken",
    ),
    (
        "Show disk usage of this folder",
        "Speicherbelegung dieses Ordners anzeigen",
    ),
    ("Scan disk usage again", "Speicherbelegung neu einlesen"),
    (
        "Stop disk usage scan",
        "Einlesen der Speicherbelegung anhalten",
    ),
    // The view
    ("Not counted yet…", "Noch nicht gezählt …"),
    ("Counting…", "Wird gezählt …"),
    (
        "There is nothing in this folder that takes up space.",
        "In diesem Ordner belegt nichts Platz.",
    ),
    (
        "Scanning… {files}, {size}",
        "Wird eingelesen … {files}, {size}",
    ),
    (
        "{files}, {size}, counted in {time}",
        "{files}, {size}, gezählt in {time}",
    ),
    (
        "Stopped: {files}, {size} so far",
        "Angehalten: bisher {files}, {size}",
    ),
    ("{folders} couldn't be read", "{folders} nicht lesbar"),
    (
        "Their sizes are missing (no permission, or in use).",
        "Ihre Größen fehlen (keine Berechtigung oder in Benutzung).",
    ),
    ("Other ({n})", "Sonstige ({n})"),
    ("Smaller files ({n})", "Kleinere Dateien ({n})"),
    (
        "{size} ({share} of {folder})",
        "{size} ({share} von {folder})",
    ),
    ("Still counting…", "Wird noch gezählt …"),
    (
        "{items}, each too small to show",
        "{items}, jeweils zu klein zum Anzeigen",
    ),
    ("🖴 Zoom in", "🖴 Hineinzoomen"),
    ("📂 Open folder", "📂 Ordner öffnen"),
    ("Largest items", "Größte Elemente"),
];
