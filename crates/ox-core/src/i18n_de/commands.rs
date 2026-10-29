//! German translations for the custom commands (continued from
//! `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Settings page
    ("⚡ Custom commands", "⚡ Eigene Befehle"),
    (
        "Your own commands in the right-click menu and the command palette. In the command line, the placeholders become the selected items; paths are put in quotes where needed. A line with |, &, > or a command of cmd (echo, start, …) runs through cmd /C, other programs are started directly.",
        "Eigene Befehle im Kontextmenü und in der Befehlspalette. In der Befehlszeile werden die Platzhalter durch die ausgewählten Elemente ersetzt; Pfade kommen bei Bedarf in Anführungszeichen. Eine Zeile mit |, &, > oder einem Befehl von cmd (echo, start, …) läuft über cmd /C, andere Programme werden direkt gestartet.",
    ),
    ("Duplicate", "Duplizieren"),
    ("Edit", "Bearbeiten"),
    ("{name} (copy)", "{name} (Kopie)"),
    ("➕ Add command", "➕ Befehl hinzufügen"),
    ("New command", "Neuer Befehl"),
    ("Examples to add", "Beispiele zum Hinzufügen"),
    ("files", "Dateien"),
    ("folders", "Ordner"),
    ("folder background", "Ordnerhintergrund"),
    ("Example", "Beispiel"),
    ("Icon", "Symbol"),
    ("Command line", "Befehlszeile"),
    (
        "Also in the environment: OX_PATH, OX_PATHS (one per line) and OX_FOLDER, e.g. $env:OX_PATH in PowerShell.",
        "Auch als Umgebungsvariablen: OX_PATH, OX_PATHS (einer pro Zeile) und OX_FOLDER, z. B. $env:OX_PATH in PowerShell.",
    ),
    ("Working folder", "Arbeitsordner"),
    ("the current folder", "der aktuelle Ordner"),
    ("For", "Für"),
    ("Files", "Dateien"),
    ("Folder background", "Ordnerhintergrund"),
    ("File types", "Dateitypen"),
    ("all files", "alle Dateien"),
    (
        "Extensions (jpg, png) or kinds: {kinds}",
        "Endungen (jpg, png) oder Arten: {kinds}",
    ),
    ("Selection", "Auswahl"),
    ("Window", "Fenster"),
    (
        "The console closes when the command ends; add & pause to keep it open.",
        "Die Konsole schließt sich, wenn der Befehl endet; mit & pause am Ende bleibt sie offen.",
    ),
    ("Ask first", "Vorher fragen"),
    ("Ask before running it", "Vor dem Ausführen nachfragen"),
    ("Shortcut", "Tastenkombination"),
    (
        "⚠ Already the shortcut of “{other}”, which goes first. Change it in Settings → Shortcuts or pick other keys.",
        "⚠ Schon die Tastenkombination von „{other}“, die Vorrang hat. Ändern Sie sie unter Einstellungen → Tastenkombinationen oder wählen Sie andere Tasten.",
    ),
    (
        "⚠ Also the shortcut of “{other}”",
        "⚠ Auch die Tastenkombination von „{other}“",
    ),
    (
        "For example items (select items in a folder tab to see them here):",
        "Für Beispiel-Elemente (wählen Sie Elemente in einem Ordner-Tab aus, um sie hier zu sehen):",
    ),
    (
        "For the selection of the folder tab:",
        "Für die Auswahl im Ordner-Tab:",
    ),
    ("No command line yet", "Noch keine Befehlszeile"),
    ("… and {n} more runs", "… und {n} weitere Aufrufe"),
    ("Runs through cmd /C.", "Läuft über cmd /C."),
    ("Starts {exe} directly.", "Startet {exe} direkt."),
    ("In {dir}", "In {dir}"),
    // Placeholders
    (
        "The first selected item, its full path",
        "Das erste ausgewählte Element, mit vollem Pfad",
    ),
    (
        "All selected items, each in quotes if needed",
        "Alle ausgewählten Elemente, jedes bei Bedarf in Anführungszeichen",
    ),
    ("The name of the first item", "Der Name des ersten Elements"),
    ("The name without its extension", "Der Name ohne Endung"),
    ("The extension, without the dot", "Die Endung, ohne Punkt"),
    (
        "The folder the first item is in",
        "Der Ordner, in dem das erste Element liegt",
    ),
    ("The current folder", "Der aktuelle Ordner"),
    ("How many items", "Wie viele Elemente"),
    // Options and examples
    ("One item only", "Nur ein Element"),
    (
        "Several: once for each item",
        "Mehrere: einmal für jedes Element",
    ),
    (
        "Several: once with all items",
        "Mehrere: einmal mit allen Elementen",
    ),
    ("Run hidden", "Unsichtbar ausführen"),
    ("Show a console window", "Ein Konsolenfenster zeigen"),
    ("Wait and show the output", "Warten und die Ausgabe zeigen"),
    ("Open in VS Code", "In VS Code öffnen"),
    (
        "Copy file hash to clipboard",
        "Prüfsumme in die Zwischenablage kopieren",
    ),
    ("Convert to MP3 with ffmpeg", "Mit ffmpeg in MP3 umwandeln"),
    ("Recent Git commits", "Letzte Git-Commits"),
    // Running
    ("⚡ Commands", "⚡ Befehle"),
    ("Run “{name}”?", "„{name}“ ausführen?"),
    (
        "It runs {n} times, once for each item:",
        "Er läuft {n}-mal, einmal für jedes Element:",
    ),
    ("Run", "Ausführen"),
    ("running…", "läuft …"),
    ("done", "fertig"),
    ("failed", "fehlgeschlagen"),
    ("No output", "Keine Ausgabe"),
    ("“{name}” failed: {e}", "„{name}“ ist fehlgeschlagen: {e}"),
    ("“{name}” is done", "„{name}“ ist fertig"),
    (
        "“{name}” isn't for what is selected",
        "„{name}“ passt nicht zur Auswahl",
    ),
    ("Program not found: {p}", "Programm nicht gefunden: {p}"),
    (
        "The folder doesn't exist: {d}",
        "Der Ordner existiert nicht: {d}",
    ),
    ("Exit code {c}", "Exit-Code {c}"),
    ("It was ended", "Er wurde beendet"),
];
