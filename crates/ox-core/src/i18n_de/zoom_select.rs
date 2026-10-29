//! German translations for grid zoom, the selection helpers and Quick Look
//! (continued from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Grid zoom
    ("Icon size (Ctrl+mouse wheel)", "Symbolgröße (Strg+Mausrad)"),
    ("Larger icons", "Größere Symbole"),
    ("Smaller icons", "Kleinere Symbole"),
    // Selection helpers
    ("☑ Select", "☑ Auswählen"),
    ("Select", "Auswählen"),
    ("Deselect", "Abwählen"),
    ("Select by pattern", "Nach Muster auswählen"),
    ("Deselect by pattern", "Nach Muster abwählen"),
    ("Select by pattern…", "Nach Muster auswählen …"),
    ("Deselect by pattern…", "Nach Muster abwählen …"),
    ("Invert selection", "Auswahl umkehren"),
    (
        "Select all of the same type",
        "Alle vom selben Typ auswählen",
    ),
    (
        "* stands for any characters, ? for one. Separate several patterns with ;",
        "* steht für beliebige Zeichen, ? für eines. Mehrere Muster mit ; trennen",
    ),
    ("Matching: {v1}", "Passend: {v1}"),
    (
        "No items match \"{v1}\".",
        "Keine Elemente passen zu „{v1}“.",
    ),
    (
        "Select an item first.",
        "Wählen Sie zuerst ein Element aus.",
    ),
    // Quick Look
    ("Quick Look", "Schnellansicht"),
    ("👁 Quick Look", "👁 Schnellansicht"),
    ("Close (Space or Esc)", "Schließen (Leertaste oder Esc)"),
    (
        "Arrow keys: the previous or next item",
        "Pfeiltasten: voriges oder nächstes Element",
    ),
    ("📂 Contents", "📂 Inhalt"),
    (
        "Only the first {v1} items are shown.",
        "Nur die ersten {v1} Elemente werden angezeigt.",
    ),
];
