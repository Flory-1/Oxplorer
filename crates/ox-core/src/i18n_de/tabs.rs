//! German translations for pinned tabs, named sessions and windows
//! (continued from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Pinned tabs
    ("📌 Pin tab", "📌 Tab anheften"),
    ("📍 Unpin tab", "📍 Tab lösen"),
    ("Pin tab", "Tab anheften"),
    ("Unpin tab", "Tab lösen"),
    // Named sessions
    ("Right-click for sessions", "Rechtsklick für Sitzungen"),
    (
        "💾 Save tabs as session…",
        "💾 Tabs als Sitzung speichern …",
    ),
    ("Save tabs as session…", "Tabs als Sitzung speichern …"),
    ("Save tabs as session", "Tabs als Sitzung speichern"),
    (
        "No saved sessions yet.",
        "Noch keine gespeicherten Sitzungen.",
    ),
    (
        "📂 Open instead of the open tabs",
        "📂 Statt der geöffneten Tabs öffnen",
    ),
    (
        "➕ Add to the open tabs",
        "➕ Zu den geöffneten Tabs hinzufügen",
    ),
    (
        "💾 Save the open tabs here",
        "💾 Geöffnete Tabs hierin speichern",
    ),
    ("✏ Rename…", "✏ Umbenennen …"),
    ("Rename session", "Sitzung umbenennen"),
    ("Enter a name.", "Geben Sie einen Namen ein."),
    (
        "A session with this name already exists.",
        "Es gibt bereits eine Sitzung mit diesem Namen.",
    ),
    (
        "A session with this name already exists; saving replaces it.",
        "Es gibt bereits eine Sitzung mit diesem Namen; Speichern ersetzt sie.",
    ),
    (
        "This session doesn't exist any more.",
        "Diese Sitzung gibt es nicht mehr.",
    ),
    (
        "Couldn't save the sessions: {e}",
        "Die Sitzungen konnten nicht gespeichert werden: {e}",
    ),
    (
        "Saved the tabs as \"{name}\".",
        "Die Tabs wurden als „{name}“ gespeichert.",
    ),
    (
        "Deleted the session \"{name}\".",
        "Die Sitzung „{name}“ wurde gelöscht.",
    ),
    ("Open session: {name}", "Sitzung öffnen: {name}"),
    (
        "Add session to the open tabs: {name}",
        "Sitzung zu den geöffneten Tabs hinzufügen: {name}",
    ),
    // More windows
    ("New window", "Neues Fenster"),
    ("🗔 New window", "🗔 Neues Fenster"),
    ("🗔 Move to new window", "🗔 In neues Fenster verschieben"),
    ("Move tab to new window", "Tab in neues Fenster verschieben"),
    (
        "🗔 Release to open in a new window",
        "🗔 Loslassen, um es in einem neuen Fenster zu öffnen",
    ),
    (
        "Couldn't open a new window: {e}",
        "Ein neues Fenster konnte nicht geöffnet werden: {e}",
    ),
];
