//! German translations for the text index and the byte-by-byte compare of
//! duplicates (continued from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Search status
    (" · from the text index", " · aus dem Textindex"),
    // Settings → Search
    ("Text index", "Textindex"),
    ("Keeps the words of documents so content: searches in these folders don't have to read every file. Files the index doesn't know yet are still read.", "Merkt sich die Wörter von Dokumenten, damit content:-Suchen in diesen Ordnern nicht jede Datei lesen müssen. Dateien, die der Index noch nicht kennt, werden weiterhin gelesen."),
    ("Index the text of documents", "Text von Dokumenten indizieren"),
    ("Text files, code, PDF, Word, Excel, PowerPoint and OpenDocument files. Read in the background, paused while files are copied; online-only cloud files are left out.", "Textdateien, Code, PDF-, Word-, Excel-, PowerPoint- und OpenDocument-Dateien. Wird im Hintergrund gelesen und pausiert, während Dateien kopiert werden; reine Online-Dateien aus der Cloud werden ausgelassen."),
    ("One per line. Empty = Documents, Desktop and your code folders.", "Einer pro Zeile. Leer = Dokumente, Desktop und Ihre Code-Ordner."),
    ("Largest file", "Größte Datei"),
    ("Larger files are read by each search, as without the index.", "Größere Dateien liest jede Suche selbst, wie ohne Index."),
    ("Reading documents… {files} looked at", "Dokumente werden gelesen … {files} angesehen"),
    ("⏸ Paused while files are copied", "⏸ Pausiert, während Dateien kopiert werden"),
    ("Couldn't save the text index: {e}", "Der Textindex konnte nicht gespeichert werden: {e}"),
    ("✔ {files} with text ({size}) · {words} words · {memory} in memory", "✔ {files} mit Text ({size}) · {words} Wörter · {memory} im Speicher"),
    (" · {size} on disk", " · {size} auf dem Datenträger"),
    ("Checked {age} ago, took {time}", "Vor {age} geprüft, Dauer {time}"),
    // Duplicates
    ("Compare contents byte by byte", "Inhalte Byte für Byte vergleichen"),
    ("After the hashes match, read the files again and compare every byte. Slower; the hash alone is practically certain.", "Wenn die Prüfsummen gleich sind, die Dateien noch einmal lesen und jedes Byte vergleichen. Langsamer; die Prüfsumme allein ist praktisch sicher."),
];
