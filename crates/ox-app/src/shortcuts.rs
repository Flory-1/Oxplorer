//! Keyboard shortcuts: the commands that have one, their default keys, and
//! the user's own keys from the settings (`[shortcuts]` in settings.toml).

use std::collections::BTreeMap;
use std::sync::RwLock;

use egui::{InputState, Key, KeyboardShortcut, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    ToggleSplit,
    SwitchPane,
    CompareFolders,
    Back,
    Forward,
    Up,
    Refresh,
    EditAddress,
    Search,
    Palette,
    Help,
    TogglePreview,
    Settings,
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    CopyPath,
    NewFolder,
    Delete,
    DeletePermanently,
    Rename,
    Properties,
    ShellMenu,
    ToggleTerminal,
    OpenInEditor,
}

pub struct CommandInfo {
    pub cmd: Command,
    /// Name in settings.toml.
    pub id: &'static str,
    pub label: &'static str,
    pub defaults: &'static [&'static str],
}

const fn info(
    cmd: Command,
    id: &'static str,
    label: &'static str,
    defaults: &'static [&'static str],
) -> CommandInfo {
    CommandInfo {
        cmd,
        id,
        label,
        defaults,
    }
}

/// Every command with a shortcut, in the order the settings page lists them.
pub const COMMANDS: &[CommandInfo] = &[
    info(Command::NewTab, "new_tab", "New tab", &["Ctrl+T"]),
    info(Command::CloseTab, "close_tab", "Close tab", &["Ctrl+W"]),
    info(Command::NextTab, "next_tab", "Next tab", &["Ctrl+Tab"]),
    info(
        Command::PrevTab,
        "prev_tab",
        "Previous tab",
        &["Ctrl+Shift+Tab"],
    ),
    info(
        Command::ToggleSplit,
        "split_view",
        "Split view",
        &["Ctrl+Backslash"],
    ),
    info(Command::SwitchPane, "switch_pane", "Switch pane", &["F6"]),
    info(
        Command::CompareFolders,
        "compare_folders",
        "Compare folders",
        &["Ctrl+Shift+D"],
    ),
    info(Command::Back, "back", "Back", &["Alt+Left"]),
    info(Command::Forward, "forward", "Forward", &["Alt+Right"]),
    info(Command::Up, "up", "Up one folder", &["Alt+Up", "Backspace"]),
    info(Command::Refresh, "refresh", "Refresh", &["F5", "Ctrl+R"]),
    info(
        Command::EditAddress,
        "edit_address",
        "Edit address",
        &["Ctrl+L", "Alt+D"],
    ),
    info(Command::Search, "search", "Search", &["Ctrl+F", "Ctrl+E"]),
    info(
        Command::Palette,
        "command_palette",
        "Command palette",
        &["Ctrl+Shift+P"],
    ),
    info(Command::Help, "help", "Help", &["F1"]),
    info(
        Command::TogglePreview,
        "preview_pane",
        "Preview pane",
        &["Alt+P"],
    ),
    info(Command::Settings, "settings", "Settings", &["Ctrl+Comma"]),
    info(Command::SelectAll, "select_all", "Select all", &["Ctrl+A"]),
    info(Command::Copy, "copy", "Copy", &["Ctrl+C"]),
    info(Command::Cut, "cut", "Cut", &["Ctrl+X"]),
    info(Command::Paste, "paste", "Paste", &["Ctrl+V"]),
    info(Command::Undo, "undo", "Undo", &["Ctrl+Z"]),
    info(Command::Redo, "redo", "Redo", &["Ctrl+Y", "Ctrl+Shift+Z"]),
    info(
        Command::CopyPath,
        "copy_path",
        "Copy path",
        &["Ctrl+Shift+C"],
    ),
    info(
        Command::NewFolder,
        "new_folder",
        "New folder",
        &["Ctrl+Shift+N"],
    ),
    info(Command::Delete, "delete", "Delete", &["Delete"]),
    info(
        Command::DeletePermanently,
        "delete_permanently",
        "Delete permanently",
        &["Shift+Delete"],
    ),
    info(Command::Rename, "rename", "Rename", &["F2"]),
    info(
        Command::Properties,
        "properties",
        "Properties",
        &["Alt+Enter"],
    ),
    info(
        Command::ShellMenu,
        "shell_menu",
        "Windows context menu",
        &["Shift+F10"],
    ),
    info(
        Command::ToggleTerminal,
        "terminal",
        "Terminal panel",
        &["Ctrl+Backtick"],
    ),
    info(
        Command::OpenInEditor,
        "open_in_editor",
        "Open in editor",
        &["Ctrl+Shift+E"],
    ),
];

pub fn command_info(cmd: Command) -> &'static CommandInfo {
    COMMANDS
        .iter()
        .find(|c| c.cmd == cmd)
        .expect("every command is listed")
}

/// Parse "Ctrl+Shift+P" (key names as in [`Key::name`], case-insensitive
/// modifiers).
pub fn parse(text: &str) -> Option<KeyboardShortcut> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_lowercase().as_str() {
            "ctrl" | "cmd" | "control" => mods |= Modifiers::COMMAND,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" => mods |= Modifiers::ALT,
            "" => return None,
            _ => key = Some(Key::from_name(part).filter(|k| !is_modifier(*k))?),
        }
    }
    Some(KeyboardShortcut::new(mods, key?))
}

/// Ctrl, Shift, Alt and the Windows key on their own are not shortcuts.
pub fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::ControlLeft
            | Key::ControlRight
            | Key::AltLeft
            | Key::AltRight
            | Key::SuperLeft
            | Key::SuperRight
    )
}

/// The form stored in settings.toml ("Ctrl+Shift+P").
pub fn to_text(sc: &KeyboardShortcut) -> String {
    let mut s = String::new();
    let m = sc.modifiers;
    if m.command || m.ctrl {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    s.push_str(sc.logical_key.name());
    s
}

/// The form shown to people ("Ctrl+,", "Alt+Left"; in German "Strg+,",
/// "Alt+Links", like the key caps on German keyboards).
pub fn display(sc: &KeyboardShortcut) -> String {
    let german = ox_core::i18n::is_german();
    let m = sc.modifiers;
    let mut s = String::new();
    if m.command || m.ctrl {
        s.push_str(if german { "Strg+" } else { "Ctrl+" });
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift {
        s.push_str(if german { "Umschalt+" } else { "Shift+" });
    }
    let key = match (sc.logical_key, german) {
        (Key::ArrowLeft, false) => "Left",
        (Key::ArrowRight, false) => "Right",
        (Key::ArrowUp, false) => "Up",
        (Key::ArrowDown, false) => "Down",
        (Key::Delete, false) => "Del",
        (Key::ArrowLeft, true) => "Links",
        (Key::ArrowRight, true) => "Rechts",
        (Key::ArrowUp, true) => "Oben",
        (Key::ArrowDown, true) => "Unten",
        (Key::Delete, true) => "Entf",
        (Key::Enter, true) => "Eingabe",
        (Key::Home, true) => "Pos1",
        (Key::End, true) => "Ende",
        (Key::Insert, true) => "Einfg",
        (Key::Backspace, true) => "Rücktaste",
        (Key::Space, true) => "Leertaste",
        (Key::PageUp, true) => "Bild↑",
        (Key::PageDown, true) => "Bild↓",
        (k, _) => k.symbol_or_name(),
    };
    s.push_str(key);
    s
}

/// Shortcuts in effect: the defaults with the user's changes applied.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    keys: Vec<(Command, Vec<KeyboardShortcut>)>,
}

impl Keymap {
    /// `overrides`: command id → shortcut text ("" = no shortcut).
    pub fn new(overrides: &BTreeMap<String, String>) -> Self {
        let keys = COMMANDS
            .iter()
            .map(|c| {
                let keys = match overrides.get(c.id) {
                    Some(text) => parse(text).into_iter().collect(),
                    None => c.defaults.iter().filter_map(|d| parse(d)).collect(),
                };
                (c.cmd, keys)
            })
            .collect();
        Self { keys }
    }

    pub fn keys(&self, cmd: Command) -> &[KeyboardShortcut] {
        self.keys
            .iter()
            .find(|(c, _)| *c == cmd)
            .map(|(_, k)| k.as_slice())
            .unwrap_or(&[])
    }

    /// The first shortcut, for menus and tooltips ("" when there is none).
    pub fn label(&self, cmd: Command) -> String {
        self.keys(cmd).first().map(display).unwrap_or_default()
    }

    /// Commands whose shortcut was pressed, considering only `available`
    /// ones. More specific shortcuts are checked first, so Ctrl+Shift+Tab
    /// doesn't also count as Ctrl+Tab.
    pub fn pressed(
        &self,
        input: &mut InputState,
        available: impl Fn(Command) -> bool,
    ) -> Vec<Command> {
        let mut all: Vec<(Command, KeyboardShortcut)> = self
            .keys
            .iter()
            .filter(|(c, _)| available(*c))
            .flat_map(|(c, keys)| keys.iter().map(move |k| (*c, *k)))
            .collect();
        all.sort_by_key(|(_, k)| {
            std::cmp::Reverse(u8::from(k.modifiers.shift) + u8::from(k.modifiers.alt))
        });
        let mut out = Vec::new();
        for (cmd, key) in all {
            if input.consume_shortcut(&key) && !out.contains(&cmd) {
                out.push(cmd);
            }
        }
        out
    }

    /// Another command that uses `key` too.
    pub fn conflict(&self, cmd: Command, key: &KeyboardShortcut) -> Option<Command> {
        self.keys
            .iter()
            .find(|(c, keys)| *c != cmd && keys.contains(key))
            .map(|(c, _)| *c)
    }
}

static CURRENT: RwLock<Option<Keymap>> = RwLock::new(None);

/// Make `keymap` the one menus and the palette show.
pub fn set_current(keymap: &Keymap) {
    *CURRENT.write().unwrap() = Some(keymap.clone());
}

/// Shortcut text for menus ("" when the command has none).
pub fn label(cmd: Command) -> String {
    CURRENT
        .read()
        .unwrap()
        .as_ref()
        .map(|k| k.label(cmd))
        .unwrap_or_default()
}

/// "Copy (Ctrl+C)" for tooltips, or just "Copy" without a shortcut.
pub fn hint(text: &str, cmd: Command) -> String {
    match label(cmd) {
        k if k.is_empty() => text.to_owned(),
        k => format!("{text} ({k})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_and_round_trip() {
        for c in COMMANDS {
            for d in c.defaults {
                let sc = parse(d).unwrap_or_else(|| panic!("{d}"));
                assert_eq!(&to_text(&sc), d);
            }
        }
        assert_eq!(display(&parse("Ctrl+Comma").unwrap()), "Ctrl+,");
        assert_eq!(display(&parse("Alt+Left").unwrap()), "Alt+Left");
        assert!(parse("Ctrl+").is_none());
        assert!(parse("Ctrl+Nope").is_none());
    }

    #[test]
    fn overrides_replace_defaults() {
        let mut o = BTreeMap::new();
        o.insert("refresh".to_owned(), "Ctrl+Shift+R".to_owned());
        o.insert("rename".to_owned(), String::new());
        let k = Keymap::new(&o);
        assert_eq!(k.label(Command::Refresh), "Ctrl+Shift+R");
        assert_eq!(k.keys(Command::Refresh).len(), 1);
        assert!(k.keys(Command::Rename).is_empty());
        assert_eq!(k.label(Command::Up), "Alt+Up");
        let copy = parse("Ctrl+C").unwrap();
        assert_eq!(k.conflict(Command::Refresh, &copy), Some(Command::Copy));
        assert_eq!(k.conflict(Command::Copy, &copy), None);
    }
}
