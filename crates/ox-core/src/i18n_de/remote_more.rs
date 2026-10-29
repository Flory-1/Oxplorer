//! German translations for resuming and verifying server transfers,
//! uploading edited remote files and thumbnails in remote folders
//! (continued from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // Transfers
    (" · server transfer", " · Serverübertragung"),
    ("✔ {v1}: size checked only", "✔ {v1}: nur Größe geprüft"),
    ("The original changed during the transfer. Try again.", "Das Original hat sich während der Übertragung geändert. Versuchen Sie es erneut."),
    // Uploading edited files
    ("Opened a downloaded copy. When you save it, Oxplorer offers to upload the changes.", "Eine heruntergeladene Kopie wurde geöffnet. Wenn Sie sie speichern, bietet Oxplorer an, die Änderungen hochzuladen."),
    ("Upload the changes to {server}?", "Die Änderungen auf {server} hochladen?"),
    ("\"{name}\" was saved.", "„{name}“ wurde gespeichert."),
    ("Upload", "Hochladen"),
    ("Ignore", "Ignorieren"),
    ("Always for this file", "Immer für diese Datei"),
    ("Upload this file's changes without asking until Oxplorer is closed.", "Änderungen dieser Datei ohne Nachfrage hochladen, bis Oxplorer geschlossen wird."),
    ("\"{name}\" changed on the server after you opened it.", "„{name}“ wurde auf dem Server geändert, nachdem Sie die Datei geöffnet haben."),
    ("Replace it with your version?", "Durch Ihre Version ersetzen?"),
    ("Uploaded \"{name}\" to {server}.", "„{name}“ wurde auf {server} hochgeladen."),
    ("Couldn't upload \"{name}\": {e}", "„{name}“ konnte nicht hochgeladen werden: {e}"),
    // Thumbnails
    ("Thumbnails in remote folders", "Miniaturansichten in Remotespeicherorten"),
    ("Download pictures up to 8 MB from servers in the background to show their thumbnails.", "Bilder bis 8 MB im Hintergrund von Servern herunterladen, um ihre Miniaturansichten zu zeigen."),
];
