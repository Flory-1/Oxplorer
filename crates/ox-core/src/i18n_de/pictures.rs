//! German translations for pictures: SVG, Windows' codecs, animations and
//! EXIF details (continued from `i18n_de.rs`).

pub const TABLE: &[(&str, &str)] = &[
    // SVG drawings
    ("Can't read this drawing: {e}", "Diese Zeichnung kann nicht gelesen werden: {e}"),
    ("The drawing is empty.", "Die Zeichnung ist leer."),
    // Windows' codecs
    ("Windows has no codec for this format.", "Windows hat keinen Codec für dieses Format."),
    ("Windows has no codec for this format. Install {app} from the Microsoft Store.", "Windows hat keinen Codec für dieses Format. Installieren Sie {app} aus dem Microsoft Store."),
    ("This format can only be shown on Windows.", "Dieses Format kann nur unter Windows angezeigt werden."),
    // Animations
    ("Frames", "Einzelbilder"),
    ("{v1} ({v2} s per loop)", "{v1} ({v2} s pro Durchlauf)"),
    // EXIF details
    ("Exposure time", "Belichtungszeit"),
    ("Aperture", "Blende"),
    ("ISO", "ISO"),
    ("Focal length", "Brennweite"),
    ("{v1} mm (35 mm equivalent: {v2} mm)", "{v1} mm (entspricht {v2} mm Kleinbild)"),
    ("Flash", "Blitz"),
    ("Fired", "Ausgelöst"),
    ("Did not fire", "Nicht ausgelöst"),
    ("No flash", "Kein Blitz"),
    ("Altitude", "Höhe"),
    ("🌐 Open in map", "🌐 Auf der Karte zeigen"),
    ("Shows the place on OpenStreetMap in the browser", "Zeigt den Ort auf OpenStreetMap im Browser"),
];
