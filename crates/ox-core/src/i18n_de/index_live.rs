//! German translations for live index updates and the NTFS master file
//! table reader (continued from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    ("Live updates", "Live-Aktualisierung"),
    ("Apply file changes to the index as they happen, between refreshes.", "Dateiänderungen sofort in den Index übernehmen, nicht erst bei der nächsten Aktualisierung."),
    ("Live updates on", "Live-Aktualisierung an"),
    ("Live updates on (NTFS change journal)", "Live-Aktualisierung an (NTFS-Änderungsjournal)"),
    (" · {v1} applied", " · {v1} übernommen"),
    (" · read from the NTFS master file table", " · aus der NTFS-Masterdateitabelle gelesen"),
    ("Run Oxplorer as administrator to read NTFS drives directly from the master file table (much faster) and follow changes through the change journal.", "Starten Sie Oxplorer als Administrator, um NTFS-Laufwerke direkt aus der Masterdateitabelle zu lesen (viel schneller) und Änderungen über das Änderungsjournal zu verfolgen."),
];
