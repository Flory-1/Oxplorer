//! Keyboard shortcuts: the commands that have one, their default keys, and
//! the user's own keys from the settings (`[shortcuts]` in settings.toml).
//!
//! Custom commands (Settings → Custom commands) can have a shortcut too.
//! It is stored with the command (`shortcut` in its `[[commands]]` entry),
//! which keeps it when the command is renamed or moved; the [`Keymap`]
//! holds both kinds, so a clash between any two is found the same way.

use std::collections::BTreeMap;
use std::sync::RwLock;

use egui::{InputState, Key, KeyboardShortcut, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    NewTab,
    NewWindow,
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
    FlatView,
    Settings,
    SelectAll,
    SelectPattern,
    DeselectPattern,
    InvertSelection,
    SelectSameType,
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
    QuickLook,
    PlayerPlayPause,
    PlayerNext,
    PlayerPrevious,
}

/// What a shortcut runs: a built-in command or a custom command (by its
/// place in the list of custom commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    Builtin(Command),
    Custom(usize),
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
    info(Command::NewWindow, "new_window", "New window", &["Ctrl+N"]),
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
    info(
        Command::FlatView,
        "flat_view",
        "Flat view",
        &["Ctrl+Shift+F"],
    ),
    info(Command::Settings, "settings", "Settings", &["Ctrl+Comma"]),
    info(Command::SelectAll, "select_all", "Select all", &["Ctrl+A"]),
    info(
        Command::SelectPattern,
        "select_pattern",
        "Select by pattern",
        &["Ctrl+Shift+A"],
    ),
    info(
        Command::DeselectPattern,
        "deselect_pattern",
        "Deselect by pattern",
        &[],
    ),
    info(
        Command::InvertSelection,
        "invert_selection",
        "Invert selection",
        &["Ctrl+Shift+I"],
    ),
    info(
        Command::SelectSameType,
        "select_same_type",
        "Select all of the same type",
        &[],
    ),
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
    info(Command::QuickLook, "quick_look", "Quick Look", &["Space"]),
    info(
        Command::PlayerPlayPause,
        "player_play_pause",
        "Player: play / pause",
        &["Ctrl+Space"],
    ),
    info(
        Command::PlayerNext,
        "player_next",
        "Player: next file",
        &["Ctrl+Right"],
    ),
    info(
        Command::PlayerPrevious,
        "player_previous",
        "Player: previous file",
        &["Ctrl+Left"],
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

/// A custom command's shortcut as stored ("" or keys that don't parse:
/// none).
pub fn parse_custom(text: &str) -> Option<KeyboardShortcut> {
    match text.trim() {
        "" => None,
        s => parse(s),
    }
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

/// While a new shortcut is recorded: the keys pressed this frame, if any
/// (`Some(None)`: Esc, recording is cancelled).
pub fn recorded(input: &InputState) -> Option<Option<KeyboardShortcut>> {
    let (key, m) = input.events.iter().find_map(|e| match e {
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } if !is_modifier(*key) => Some((*key, *modifiers)),
        _ => None,
    })?;
    if key == Key::Escape && m.is_none() {
        return Some(None);
    }
    let mut mods = Modifiers::NONE;
    if m.ctrl || m.command {
        mods |= Modifiers::COMMAND;
    }
    if m.alt {
        mods |= Modifiers::ALT;
    }
    if m.shift {
        mods |= Modifiers::SHIFT;
    }
    Some(Some(KeyboardShortcut::new(mods, key)))
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

/// Shortcuts in effect: the defaults with the user's changes applied, and
/// the custom commands' ones.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    keys: Vec<(Command, Vec<KeyboardShortcut>)>,
    /// The custom commands' shortcuts, by their place in the list.
    custom: Vec<Option<KeyboardShortcut>>,
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
        Self {
            keys,
            custom: Vec::new(),
        }
    }

    /// With the shortcuts of the custom commands, in their order (as
    /// stored: "" for none).
    pub fn with_custom<'a>(mut self, keys: impl IntoIterator<Item = &'a str>) -> Self {
        self.custom = keys.into_iter().map(parse_custom).collect();
        self
    }

    /// The keymap of `settings`: `[shortcuts]` and the custom commands.
    pub fn from_settings(settings: &ox_core::Settings) -> Self {
        let custom = settings.commands.0.iter().map(|c| c.shortcut.as_str());
        Self::new(&settings.shortcuts).with_custom(custom)
    }

    /// The shortcut of custom command `i`.
    pub fn custom_key(&self, i: usize) -> Option<KeyboardShortcut> {
        self.custom.get(i).copied().flatten()
    }

    /// Every shortcut with what it runs, built-in commands first.
    fn all(&self) -> impl Iterator<Item = (Binding, &KeyboardShortcut)> {
        let builtin = self
            .keys
            .iter()
            .flat_map(|(c, keys)| keys.iter().map(move |k| (Binding::Builtin(*c), k)));
        let custom = self
            .custom
            .iter()
            .enumerate()
            .filter_map(|(i, k)| Some((Binding::Custom(i), k.as_ref()?)));
        builtin.chain(custom)
    }

    /// The custom commands a key runs: those whose shortcut no built-in
    /// command has (built-in ones go first; the settings warn about it).
    pub fn custom_keys(&self) -> Vec<(usize, KeyboardShortcut)> {
        self.custom
            .iter()
            .enumerate()
            .filter_map(|(i, k)| Some((i, (*k)?)))
            .filter(|(_, k)| !self.keys.iter().any(|(_, keys)| keys.contains(k)))
            .collect()
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

    /// Another command, built-in or custom, that uses `key` too (built-in
    /// ones first, as they win).
    pub fn conflict(&self, of: Binding, key: &KeyboardShortcut) -> Option<Binding> {
        self.all()
            .find(|(b, k)| *b != of && *k == key)
            .map(|(b, _)| b)
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
        use Binding::Builtin;
        assert_eq!(
            k.conflict(Builtin(Command::Refresh), &copy),
            Some(Builtin(Command::Copy))
        );
        assert_eq!(k.conflict(Builtin(Command::Copy), &copy), None);
    }

    #[test]
    fn custom_commands_clash_like_built_in_ones() {
        use Binding::{Builtin, Custom};
        let mut o = BTreeMap::new();
        o.insert("refresh".to_owned(), "Ctrl+Alt+H".to_owned());
        // Custom commands: one on a built-in's keys, two on the same keys,
        // one without and one with keys that don't parse.
        let k = Keymap::new(&o).with_custom(["Ctrl+Alt+H", "F9", " F9 ", "", "Ctrl+Nope"]);
        let h = parse("Ctrl+Alt+H").unwrap();
        let f9 = parse("F9").unwrap();
        assert_eq!(k.custom_key(0), Some(h));
        assert_eq!(k.custom_key(2), Some(f9));
        assert_eq!(
            (k.custom_key(3), k.custom_key(4), k.custom_key(9)),
            (None, None, None)
        );
        // Custom and built-in, both ways.
        assert_eq!(k.conflict(Custom(0), &h), Some(Builtin(Command::Refresh)));
        assert_eq!(k.conflict(Builtin(Command::Refresh), &h), Some(Custom(0)));
        // Two custom ones.
        assert_eq!(k.conflict(Custom(1), &f9), Some(Custom(2)));
        assert_eq!(k.conflict(Custom(2), &f9), Some(Custom(1)));
        // Built-in against built-in still works, and nothing clashes with
        // itself alone.
        let ctrl_c = parse("Ctrl+C").unwrap();
        assert_eq!(k.conflict(Custom(3), &ctrl_c), Some(Builtin(Command::Copy)));
        assert_eq!(k.conflict(Builtin(Command::Copy), &ctrl_c), None);
        // Keys a built-in command has don't run the custom one.
        let runs: Vec<usize> = k.custom_keys().into_iter().map(|(i, _)| i).collect();
        assert_eq!(runs, [1, 2]);
    }

    #[test]
    fn old_settings_keep_their_custom_shortcuts() {
        // A settings.toml from before custom commands were on the
        // Shortcuts page: the shortcut is still read from the command.
        let s: ox_core::Settings = toml::from_str(
            r#"
            [shortcuts]
            copy = "Ctrl+Alt+C"

            [[commands]]
            name = "Hash"
            command = "certutil -hashfile {path}"
            shortcut = "Ctrl+Alt+H"

            [[commands]]
            name = "Old copy"
            command = "x"
            shortcut = "Ctrl+Alt+C"
            "#,
        )
        .unwrap();
        let k = Keymap::from_settings(&s);
        assert_eq!(k.custom_key(0), parse("Ctrl+Alt+H"));
        assert_eq!(k.label(Command::Copy), "Ctrl+Alt+C");
        let c = parse("Ctrl+Alt+C").unwrap();
        assert_eq!(
            k.conflict(Binding::Custom(1), &c),
            Some(Binding::Builtin(Command::Copy))
        );
        assert_eq!(k.custom_keys(), vec![(0, parse("Ctrl+Alt+H").unwrap())]);
    }
}
