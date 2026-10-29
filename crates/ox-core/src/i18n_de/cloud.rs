//! German translations for cloud files (OneDrive & co.), continued from
//! `i18n_de.rs`.

pub const TABLE: &[(&str, &str)] = &[
    // Status
    ("Available when online", "Online verfügbar"),
    ("Available on this device", "Auf diesem Gerät verfügbar"),
    ("Always available on this device", "Immer auf diesem Gerät verfügbar"),
    ("Sync pending", "Synchronisierung ausstehend"),
    ("Sync problem", "Synchronisierungsproblem"),
    ("Online only: takes no space on this device and is downloaded when you open it.", "Nur online: belegt keinen Speicherplatz auf diesem Gerät und wird beim Öffnen heruntergeladen."),
    ("On this device: downloaded. \"Free up space\" makes it online only again.", "Auf diesem Gerät: heruntergeladen. Mit „Speicherplatz freigeben“ ist es wieder nur online verfügbar."),
    ("Always kept on this device, even when space runs low.", "Wird immer auf diesem Gerät behalten, auch wenn der Speicherplatz knapp wird."),
    ("Changes are waiting to be uploaded or downloaded.", "Änderungen warten darauf, hoch- oder heruntergeladen zu werden."),
    ("The cloud provider can't sync it. Its app has the details.", "Der Cloud-Anbieter kann es nicht synchronisieren. Details finden Sie in seiner App."),
    ("Cloud files need Windows.", "Clouddateien erfordern Windows."),
    // Menus and the command palette
    ("📌 Always keep on this device", "📌 Immer auf diesem Gerät beibehalten"),
    ("☁ Free up space", "☁ Speicherplatz freigeben"),
    ("Keep it online only; it is downloaded again when you open it", "Nur online behalten; beim Öffnen wird es wieder heruntergeladen"),
    ("Always keep selection on this device", "Auswahl immer auf diesem Gerät beibehalten"),
    ("Free up space used by the selection", "Von der Auswahl belegten Speicherplatz freigeben"),
    ("{v1} items couldn't be changed: {e}", "{v1} Elemente konnten nicht geändert werden: {e}"),
];
