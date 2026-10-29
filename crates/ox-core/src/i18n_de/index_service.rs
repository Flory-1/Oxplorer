//! German translations for the Oxplorer index service (continued from
//! `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    ("Use the Oxplorer index service", "Oxplorer-Indexdienst verwenden"),
    ("Reads NTFS drives from the master file table and follows the change journal without running Oxplorer as administrator. The service is installed by the installer (task \"Fast index service\", needs admin rights). It shows you the names of all files on the drives, also in folders you can't open.", "Liest NTFS-Laufwerke aus der Masterdateitabelle und verfolgt das Änderungsjournal, ohne Oxplorer als Administrator zu starten. Der Dienst wird vom Installationsprogramm eingerichtet (Aufgabe „Schneller Indexdienst“, braucht Administratorrechte). Er zeigt Ihnen die Namen aller Dateien auf den Laufwerken, auch in Ordnern, die Sie nicht öffnen können."),
    ("Index service", "Indexdienst"),
    ("Checking…", "Wird geprüft …"),
    ("Running · reads {v1}", "Läuft · liest {v1}"),
    ("Installed · starts when needed", "Installiert · startet bei Bedarf"),
    ("Installed for another user", "Für einen anderen Benutzer installiert"),
    ("Installed, but it doesn't answer", "Installiert, antwortet aber nicht"),
    ("Not installed: run the installer for all users and choose \"Fast index service\"", "Nicht installiert: Installationsprogramm für alle Benutzer ausführen und „Schneller Indexdienst“ wählen"),
];
