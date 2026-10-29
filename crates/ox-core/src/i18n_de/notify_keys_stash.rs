//! German translations for notifications when transfers are done, custom
//! commands on the Shortcuts page, and Git stashes and progress (continued
//! from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Notifications when transfers are done
    ("Copy finished", "Kopieren abgeschlossen"),
    ("Move finished", "Verschieben abgeschlossen"),
    ("{v1} finished", "Abgeschlossen: {v1}"),
    ("{v1} couldn't be copied", "Nicht kopiert: {v1}"),
    ("{v1} couldn't be moved", "Nicht verschoben: {v1}"),
    ("{v1} to {v2}", "{v1} nach {v2}"),
    ("{v1} to several folders", "{v1} in mehrere Ordner"),
    ("Notifications when transfers are done", "Benachrichtigungen, wenn Übertragungen fertig sind"),
    ("A Windows notification when copies and moves finish while Oxplorer is in the background. Click it to bring Oxplorer back.", "Eine Windows-Benachrichtigung, wenn Kopier- und Verschiebevorgänge fertig sind, während Oxplorer im Hintergrund ist. Ein Klick darauf holt Oxplorer wieder nach vorn."),
    // Custom commands on the Shortcuts page
    ("The same shortcuts as in Settings → Custom commands. When a built-in command has the same keys, the built-in one goes first.", "Dieselben Tastenkombinationen wie unter Einstellungen → Eigene Befehle. Hat ein eingebauter Befehl dieselben Tasten, geht der eingebaute vor."),
    // Git: progress
    ("Stop git (what it already did stays done).", "Git anhalten (was schon erledigt ist, bleibt erledigt)."),
    // Git: stashes
    ("Stashes", "Beiseitegelegtes (Stashes)"),
    ("Message (optional)", "Nachricht (optional)"),
    ("📦 Stash changes", "📦 Änderungen beiseitelegen"),
    ("Put all local changes aside, untracked files too (git stash). Apply or pop them later.", "Legt alle lokalen Änderungen beiseite, auch nicht versionierte Dateien (git stash). Später mit Übernehmen oder Pop zurückholen."),
    ("There are no local changes to stash.", "Es gibt keine lokalen Änderungen zum Beiseitelegen."),
    ("Stashing…", "Wird beiseitegelegt …"),
    ("Applying {stash}…", "{stash} wird übernommen …"),
    ("Dropping {stash}…", "{stash} wird verworfen …"),
    ("Pop", "Pop"),
    ("Bring the changes back and remove the stash (git stash pop).", "Holt die Änderungen zurück und entfernt den Stash (git stash pop)."),
    ("Bring the changes back and keep the stash (git stash apply).", "Holt die Änderungen zurück und behält den Stash (git stash apply)."),
    ("Drop: throw the stash away", "Verwerfen: den Stash wegwerfen"),
    ("On {branch}", "Auf {branch}"),
    ("Drop {stash} “{message}”? Its changes are gone then; this can't be undone.", "{stash} „{message}“ verwerfen? Seine Änderungen sind dann weg; das lässt sich nicht rückgängig machen."),
    ("🗑 Drop", "🗑 Verwerfen"),
    ("The list of stashes changed in the meantime. Look at it again and try once more.", "Die Liste der Stashes hat sich inzwischen geändert. Sehen Sie sie sich noch einmal an und versuchen Sie es erneut."),
];
