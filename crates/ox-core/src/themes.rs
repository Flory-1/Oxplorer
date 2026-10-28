//! Colour schemes for the window. Built-in ones plus the user's own, stored
//! as small TOML files in `<config>/themes` (colours as `"#rrggbb"`), which
//! makes them easy to share: export one, send the file, "Use as theme".

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::settings::save_toml;

/// An sRGB colour, written as `"#rrggbb"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    pub fn hex(self) -> String {
        let [r, g, b] = self.0;
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
        Some(Rgb([byte(0)?, byte(2)?, byte(4)?]))
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("not a colour: {s}")))
    }
}

/// A colour scheme.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorTheme {
    pub name: String,
    /// Built on egui's dark (or light) look.
    pub dark: bool,
    /// Sidebar, header and other panels.
    pub panel: Rgb,
    /// Menus, dialogs and pop-ups.
    pub window: Rgb,
    /// The file list and text fields.
    pub background: Rgb,
    /// Stripes, cells and other faint areas.
    pub faint: Rgb,
    /// Text; buttons and lines are derived from it and the panel colour.
    /// Without it egui's own text and widget colours are kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Rgb>,
    /// Suggested accent colour (taken over when the theme is picked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<Rgb>,
    /// Built into Oxplorer (not a file).
    #[serde(skip)]
    pub builtin: bool,
}

const fn rgb(v: u32) -> Rgb {
    Rgb([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn builtin(
    name: &str,
    dark: bool,
    [panel, window, background, faint]: [u32; 4],
    text: Option<u32>,
    accent: Option<u32>,
) -> ColorTheme {
    ColorTheme {
        name: name.to_owned(),
        dark,
        panel: rgb(panel),
        window: rgb(window),
        background: rgb(background),
        faint: rgb(faint),
        text: text.map(rgb),
        accent: accent.map(rgb),
        builtin: true,
    }
}

pub const DEFAULT_DARK: &str = "Dark";
pub const DEFAULT_LIGHT: &str = "Light";

/// The themes that come with Oxplorer.
pub fn builtins() -> Vec<ColorTheme> {
    vec![
        builtin(
            DEFAULT_DARK,
            true,
            [0x202124, 0x2a2b2f, 0x17181a, 0x26272b],
            None,
            None,
        ),
        builtin(
            DEFAULT_LIGHT,
            false,
            [0xf3f4f6, 0xffffff, 0xffffff, 0xeceef1],
            None,
            None,
        ),
        builtin(
            "Nord",
            true,
            [0x2e3440, 0x3b4252, 0x272c36, 0x343b48],
            Some(0xd8dee9),
            Some(0x88c0d0),
        ),
        builtin(
            "Dracula",
            true,
            [0x282a36, 0x343746, 0x21222c, 0x2f3140],
            Some(0xf8f8f2),
            Some(0xbd93f9),
        ),
        builtin(
            "Solarized Light",
            false,
            [0xeee8d5, 0xfdf6e3, 0xfdf6e3, 0xe6dfcb],
            Some(0x586e75),
            Some(0x268bd2),
        ),
        builtin(
            "High contrast",
            true,
            [0x000000, 0x0a0a0a, 0x000000, 0x141414],
            Some(0xffffff),
            Some(0xffd400),
        ),
    ]
}

pub fn dir() -> PathBuf {
    crate::config_dir().join("themes")
}

/// A theme file's name for `name` ("Night Owl" → "Night Owl.toml").
fn file_for(dir: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
        .collect();
    dir.join(format!("{}.toml", safe.trim()))
}

/// Read a theme file.
pub fn read(path: &Path) -> io::Result<ColorTheme> {
    let text = fs::read_to_string(path)?;
    let mut theme: ColorTheme = toml::from_str(&text).map_err(io::Error::other)?;
    if theme.name.trim().is_empty() {
        theme.name = crate::entry::display_name(path)
            .trim_end_matches(".toml")
            .to_owned();
    }
    Ok(theme)
}

/// Built-in themes, then the user's (a user theme with a built-in's name
/// replaces it).
pub fn load_all() -> Vec<ColorTheme> {
    load_from(&dir())
}

fn load_from(dir: &Path) -> Vec<ColorTheme> {
    let mut all = builtins();
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    for theme in files.iter().filter_map(|p| read(p).ok()) {
        all.retain(|t| t.name != theme.name);
        all.push(theme);
    }
    all
}

/// Save `theme` into the themes folder; returns the file.
pub fn save(theme: &ColorTheme) -> io::Result<PathBuf> {
    save_to(&dir(), theme)
}

fn save_to(dir: &Path, theme: &ColorTheme) -> io::Result<PathBuf> {
    let path = file_for(dir, &theme.name);
    save_toml(&path, theme)?;
    Ok(path)
}

/// Copy a theme file into the themes folder; returns the theme.
pub fn install(path: &Path) -> io::Result<ColorTheme> {
    let theme = read(path)?;
    save(&theme)?;
    Ok(theme)
}

/// Delete a user theme.
pub fn remove(name: &str) -> io::Result<()> {
    fs::remove_file(file_for(&dir(), name))
}

/// `name`, or the first free "name 2", "name 3", … among `taken`.
pub fn free_name(name: &str, taken: &[ColorTheme]) -> String {
    if !taken.iter().any(|t| t.name == name) {
        return name.to_owned();
    }
    (2..)
        .map(|n| format!("{name} {n}"))
        .find(|candidate| !taken.iter().any(|t| &t.name == candidate))
        .expect("there is always a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_hex_strings() {
        assert_eq!(Rgb([0x4c, 0x8d, 0xf6]).hex(), "#4c8df6");
        assert_eq!(Rgb::parse("#4C8DF6"), Some(Rgb([0x4c, 0x8d, 0xf6])));
        assert_eq!(Rgb::parse("4c8df6"), Some(Rgb([0x4c, 0x8d, 0xf6])));
        assert!(Rgb::parse("#4c8df").is_none());
        assert!(Rgb::parse("#zzzzzz").is_none());
    }

    #[test]
    fn themes_round_trip_and_user_themes_override_builtins() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mine = builtins().remove(2);
        mine.builtin = false;
        mine.name = "My Nord".into();
        mine.panel = Rgb([1, 2, 3]);
        let path = save_to(tmp.path(), &mine).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("panel = \"#010203\""), "{text}");
        assert_eq!(read(&path).unwrap(), mine);

        let mut dark = builtins().remove(0);
        dark.builtin = false;
        dark.faint = Rgb([9, 9, 9]);
        save_to(tmp.path(), &dark).unwrap();
        let all = load_from(tmp.path());
        assert_eq!(all.iter().filter(|t| t.name == DEFAULT_DARK).count(), 1);
        let d = all.iter().find(|t| t.name == DEFAULT_DARK).unwrap();
        assert!(!d.builtin && d.faint == Rgb([9, 9, 9]));
        assert!(all.iter().any(|t| t.name == "My Nord"));

        // Broken files are skipped.
        fs::write(tmp.path().join("broken.toml"), "name = 3").unwrap();
        assert_eq!(load_from(tmp.path()).len(), all.len());
    }

    #[test]
    fn free_names_and_file_names() {
        let all = builtins();
        assert_eq!(free_name("Nord", &all), "Nord 2");
        assert_eq!(free_name("Mine", &all), "Mine");
        let f = file_for(Path::new("/t"), "A/B: C?");
        assert_eq!(f, Path::new("/t/A_B_ C_.toml"));
    }
}
