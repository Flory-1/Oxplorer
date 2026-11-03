//! Custom commands (Settings → Custom commands): command lines with
//! placeholders for the selected items, offered in the right-click menu and
//! the command palette, optionally with a shortcut.
//!
//! Placeholders (see [`line::expand`]): `{path}` the first item, `{paths}` or
//! `{selection}` all of them, `{name}`, `{stem}`, `{ext}`, `{parent}` (the
//! folder of the first item), `{folder}` (the current folder) and `{n}` (the
//! number of items). Paths are quoted where needed. The same values are in
//! the environment as `OX_PATH`, `OX_PATHS` (one per line) and `OX_FOLDER`,
//! for scripts that can't take them on the command line safely.
//!
//! Whether a line runs through `cmd /C` or is started directly is decided by
//! [`run::launch`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::entry::FileKind;

pub mod line;
pub mod run;

/// How many items the command takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Many {
    /// Only offered when exactly one item is selected.
    #[default]
    One,
    /// Runs once for each item, one after another.
    Each,
    /// Runs once with all of them (`{paths}`).
    All,
}

impl Many {
    pub const ALL: [Self; 3] = [Self::One, Self::Each, Self::All];

    pub fn label(self) -> &'static str {
        match self {
            Self::One => "One item only",
            Self::Each => "Several: once for each item",
            Self::All => "Several: once with all items",
        }
    }
}

/// What the user sees of the running command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Window {
    /// No window; a message only if it fails.
    #[default]
    Hidden,
    /// In a console window of its own.
    Console,
    /// Hidden; what it prints is shown in a window when it's done.
    Output,
}

impl Window {
    pub const ALL: [Self; 3] = [Self::Hidden, Self::Console, Self::Output];

    pub fn label(self) -> &'static str {
        match self {
            Self::Hidden => "Run hidden",
            Self::Console => "Show a console window",
            Self::Output => "Wait and show the output",
        }
    }
}

/// Where in the right-click menu a command is listed: under one of its
/// entries. The menu of a folder's background, which lacks some of them,
/// uses the nearest place it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MenuPlace {
    /// First in the menu.
    Top,
    /// Under Open, Open with and Quick Look (background: under New).
    Open,
    /// Under Cut, Copy, Copy to and Move to (background: under Paste).
    Copy,
    /// Under Rename and Delete (background: under Select).
    Delete,
    /// Under Tags (background: under Select).
    Tags,
    /// With the tools, after Compress to ZIP.
    #[default]
    Tools,
    /// Under Copy path (background: with the tools).
    CopyPath,
    /// Last, under Properties.
    Bottom,
}

impl MenuPlace {
    pub const ALL: [Self; 8] = [
        Self::Top,
        Self::Open,
        Self::Copy,
        Self::Delete,
        Self::Tags,
        Self::Tools,
        Self::CopyPath,
        Self::Bottom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "At the top",
            Self::Open => "Under “Open”",
            Self::Copy => "Under “Copy”",
            Self::Delete => "Under “Delete”",
            Self::Tags => "Under “Tags”",
            Self::Tools => "Under “Compress to ZIP” (with the tools)",
            Self::CopyPath => "Under “Copy path”",
            Self::Bottom => "Under “Properties” (at the bottom)",
        }
    }

    /// Where the place is in the menu of a folder's background.
    pub fn on_background(self) -> Self {
        match self {
            Self::Tags => Self::Delete,
            Self::CopyPath => Self::Tools,
            p => p,
        }
    }
}

/// One custom command, as stored in `[[commands]]` in settings.toml.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomCommand {
    /// Name in the menus and the palette.
    pub name: String,
    /// An icon symbol (one of the app's icons); empty = the default one.
    pub icon: String,
    /// The command line, with placeholders.
    pub command: String,
    /// Folder it runs in (placeholders allowed); empty = the current folder.
    pub working_dir: String,
    /// Offered for files, for folders and on a folder's background.
    pub files: bool,
    pub folders: bool,
    pub background: bool,
    /// Only for files of these types: extensions and kinds ("jpg, png",
    /// "image, video"); empty = all files.
    pub types: String,
    pub many: Many,
    pub window: Window,
    /// Ask before running it.
    pub confirm: bool,
    /// Keys that run it ("Ctrl+Alt+H"); empty = none.
    pub shortcut: String,
    /// Where it is in the right-click menu.
    pub menu_place: MenuPlace,
    /// Listed in a submenu of this name there (commands with the same name
    /// share it); empty = in the menu itself.
    pub submenu: String,
}

impl Default for CustomCommand {
    fn default() -> Self {
        Self {
            name: String::new(),
            icon: String::new(),
            command: String::new(),
            working_dir: String::new(),
            files: true,
            folders: false,
            background: false,
            types: String::new(),
            many: Many::One,
            window: Window::Hidden,
            confirm: false,
            shortcut: String::new(),
            menu_place: MenuPlace::Tools,
            submenu: String::new(),
        }
    }
}

/// How the commands at one place of a menu are listed.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuGroup<T> {
    /// In the menu itself.
    Entry(T),
    /// In a submenu with this name.
    Submenu(String, Vec<T>),
}

/// Up to this many commands without a submenu of their own are listed with
/// the tools; more go to a "Commands" submenu (named by the caller).
pub const TOOLS_TOP_LEVEL: usize = 3;

/// Groups the commands at one `place` (`commands` are in their order, with
/// something to tell them apart): submenus where the first of them is.
/// Commands with the tools and without a submenu of their own go to one
/// named `overflow` when there are more than [`TOOLS_TOP_LEVEL`].
pub fn menu_groups<'a, T: Clone>(
    commands: impl IntoIterator<Item = (&'a CustomCommand, T)>,
    place: MenuPlace,
    overflow: &str,
) -> Vec<MenuGroup<T>> {
    let mut groups: Vec<MenuGroup<T>> = Vec::new();
    let mut loose = 0;
    for (c, t) in commands {
        if c.menu_place != place {
            continue;
        }
        let name = c.submenu.trim();
        if name.is_empty() {
            loose += 1;
            groups.push(MenuGroup::Entry(t));
            continue;
        }
        let shared = groups.iter_mut().find_map(|g| match g {
            MenuGroup::Submenu(n, list) if n.eq_ignore_ascii_case(name) => Some(list),
            _ => None,
        });
        match shared {
            Some(list) => list.push(t),
            None => groups.push(MenuGroup::Submenu(name.to_owned(), vec![t])),
        }
    }
    if place == MenuPlace::Tools && loose > TOOLS_TOP_LEVEL {
        let mut rest = Vec::new();
        let mut first = None;
        let mut kept = Vec::new();
        for (i, g) in groups.into_iter().enumerate() {
            match g {
                MenuGroup::Entry(t) => {
                    first.get_or_insert(i);
                    rest.push(t);
                }
                g => kept.push((i, g)),
            }
        }
        let at = kept
            .iter()
            .position(|(i, _)| Some(*i) > first)
            .unwrap_or(kept.len());
        let mut groups: Vec<MenuGroup<T>> = kept.into_iter().map(|(_, g)| g).collect();
        groups.insert(at, MenuGroup::Submenu(overflow.to_owned(), rest));
        return groups;
    }
    groups
}

/// The user's custom commands, in the order they are listed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct CustomCommands(pub Vec<CustomCommand>);

/// Kinds that can stand in the type list, by the word used there.
pub const KINDS: &[(&str, FileKind)] = &[
    ("image", FileKind::Image),
    ("video", FileKind::Video),
    ("audio", FileKind::Audio),
    ("document", FileKind::Document),
    ("spreadsheet", FileKind::Spreadsheet),
    ("presentation", FileKind::Presentation),
    ("pdf", FileKind::Pdf),
    ("archive", FileKind::Archive),
    ("code", FileKind::Code),
    ("text", FileKind::Text),
    ("executable", FileKind::Executable),
    ("font", FileKind::Font),
];

/// The parsed type list of a command.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypeFilter {
    pub extensions: Vec<String>,
    pub kinds: Vec<FileKind>,
}

impl TypeFilter {
    /// "jpg, .png *.gif; image" → extensions jpg, png, gif and the kind Image.
    pub fn parse(text: &str) -> Self {
        let mut f = Self::default();
        for word in text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
            let word = word.trim().trim_start_matches('*').trim_start_matches('.');
            if word.is_empty() {
                continue;
            }
            let word = word.to_lowercase();
            match KINDS.iter().find(|(w, _)| *w == word) {
                Some((_, kind)) => f.kinds.push(*kind),
                None => f.extensions.push(word),
            }
        }
        f
    }

    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty() && self.kinds.is_empty()
    }

    /// Whether a file called `path` is of one of the types (all are if
    /// none are given).
    pub fn matches(&self, path: &Path) -> bool {
        if self.is_empty() {
            return true;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        self.extensions.contains(&ext)
            || (!ext.is_empty() && self.kinds.contains(&FileKind::from_extension(&ext)))
    }
}

/// Where a command would run: on items (path, is a folder) or on the
/// background of a folder.
#[derive(Debug, Clone, Copy)]
pub enum Target<'a> {
    Items(&'a [(PathBuf, bool)]),
    Background,
}

/// One run of a command: the expanded line and where it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    /// The command line with the placeholders filled in.
    pub line: String,
    /// The template needs the shell (pipes, redirection, built-in commands).
    pub shell_syntax: bool,
    /// Working folder.
    pub dir: PathBuf,
    /// The items this run is for.
    pub items: Vec<PathBuf>,
    /// `OX_PATH`, `OX_PATHS`, `OX_FOLDER`.
    pub env: Vec<(&'static str, String)>,
}

impl CustomCommand {
    /// Whether the command is offered for `target`.
    pub fn applies(&self, target: Target<'_>) -> bool {
        if self.command.trim().is_empty() {
            return false;
        }
        let items = match target {
            Target::Background => return self.background,
            Target::Items(items) => items,
        };
        if items.is_empty() || (self.many == Many::One && items.len() != 1) {
            return false;
        }
        let types = TypeFilter::parse(&self.types);
        items.iter().all(|(path, is_dir)| {
            if *is_dir {
                self.folders
            } else {
                self.files && types.matches(path)
            }
        })
    }

    /// The runs for `items` (none: the folder itself, for the background)
    /// in the current folder `folder`.
    pub fn invocations(&self, items: &[PathBuf], folder: Option<&Path>) -> Vec<Invocation> {
        let items: Vec<PathBuf> = if items.is_empty() {
            folder.map(Path::to_path_buf).into_iter().collect()
        } else {
            items.to_vec()
        };
        let Some(first) = items.first() else {
            return Vec::new();
        };
        let folder = folder
            .map(Path::to_path_buf)
            .unwrap_or_else(|| line::parent_of(first));
        let groups: Vec<Vec<PathBuf>> = match self.many {
            Many::Each => items.iter().map(|p| vec![p.clone()]).collect(),
            Many::One => vec![vec![first.clone()]],
            Many::All => vec![items.clone()],
        };
        let shell_syntax = line::needs_shell(&self.command);
        groups
            .into_iter()
            .map(|items| {
                let values = line::Values {
                    items: &items,
                    folder: &folder,
                };
                let dir = match self.working_dir.trim() {
                    "" => folder.clone(),
                    d => folder.join(line::expand_plain(d, &values)),
                };
                let env = vec![
                    ("OX_PATH", items[0].to_string_lossy().into_owned()),
                    (
                        "OX_PATHS",
                        items
                            .iter()
                            .map(|p| p.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    ("OX_FOLDER", folder.to_string_lossy().into_owned()),
                ];
                Invocation {
                    line: line::expand(&self.command, &values),
                    shell_syntax,
                    dir,
                    items,
                    env,
                }
            })
            .collect()
    }
}

/// Ready-made commands to add with one click (names in the current language).
pub fn examples() -> Vec<CustomCommand> {
    use crate::i18n::t;
    vec![
        CustomCommand {
            name: t("Open in VS Code").into(),
            icon: "📝".into(),
            command: "code {paths}".into(),
            files: true,
            folders: true,
            background: true,
            many: Many::All,
            ..Default::default()
        },
        CustomCommand {
            name: t("Copy file hash to clipboard").into(),
            icon: "🔐".into(),
            command: "powershell -NoProfile -Command \"(Get-FileHash -LiteralPath $env:OX_PATH).Hash | Set-Clipboard\"".into(),
            ..Default::default()
        },
        CustomCommand {
            name: t("Convert to MP3 with ffmpeg").into(),
            icon: "🎵".into(),
            command: "ffmpeg -hide_banner -i {path} {stem}.mp3".into(),
            working_dir: "{parent}".into(),
            types: "video, audio".into(),
            many: Many::Each,
            window: Window::Output,
            confirm: true,
            ..Default::default()
        },
        CustomCommand {
            name: t("Recent Git commits").into(),
            icon: "🌿".into(),
            command: "git log --oneline -20".into(),
            files: false,
            background: true,
            window: Window::Output,
            ..Default::default()
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(list: &[(&str, bool)]) -> Vec<(PathBuf, bool)> {
        list.iter().map(|(p, d)| (PathBuf::from(p), *d)).collect()
    }

    #[test]
    fn type_lists() {
        let f = TypeFilter::parse(" JPG, .png;*.gif  image ");
        assert_eq!(f.extensions, ["jpg", "png", "gif"]);
        assert_eq!(f.kinds, [FileKind::Image]);
        assert!(f.matches(Path::new("a.Jpg")));
        assert!(f.matches(Path::new("b.webp")), "an image by kind");
        assert!(!f.matches(Path::new("c.txt")));
        assert!(!f.matches(Path::new("noext")));
        assert!(TypeFilter::parse("").matches(Path::new("noext")));
        let none = TypeFilter::parse("");
        assert!(none.is_empty());
    }

    #[test]
    fn what_a_command_applies_to() {
        let mut c = CustomCommand {
            command: "x {path}".into(),
            types: "mp4, audio".into(),
            ..Default::default()
        };
        let one = items(&[("/a/clip.mp4", false)]);
        let two = items(&[("/a/clip.mp4", false), ("/a/song.flac", false)]);
        let dir = items(&[("/a/sub", true)]);
        let other = items(&[("/a/notes.txt", false)]);
        assert!(c.applies(Target::Items(&one)));
        assert!(!c.applies(Target::Items(&two)), "one item only");
        assert!(!c.applies(Target::Items(&dir)));
        assert!(!c.applies(Target::Items(&other)));
        assert!(!c.applies(Target::Background));
        assert!(!c.applies(Target::Items(&[])));
        c.many = Many::Each;
        assert!(c.applies(Target::Items(&two)));
        c.folders = true;
        c.background = true;
        let mixed = items(&[("/a/clip.mp4", false), ("/a/sub", true)]);
        assert!(c.applies(Target::Items(&mixed)), "types are for files only");
        assert!(c.applies(Target::Background));
        c.files = false;
        assert!(!c.applies(Target::Items(&mixed)));
        assert!(c.applies(Target::Items(&dir)));
        c.command = "  ".into();
        assert!(!c.applies(Target::Background), "nothing to run");
    }

    #[test]
    fn runs_per_item_or_all_at_once() {
        let mut c = CustomCommand {
            command: "tool {paths} --n {n}".into(),
            many: Many::All,
            ..Default::default()
        };
        let list = [PathBuf::from("/x/a.txt"), PathBuf::from("/x/b c.txt")];
        let runs = c.invocations(&list, Some(Path::new("/x")));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].line, "tool /x/a.txt \"/x/b c.txt\" --n 2");
        assert_eq!(runs[0].dir, PathBuf::from("/x"));
        assert!(!runs[0].shell_syntax);
        assert_eq!(runs[0].env[1], ("OX_PATHS", "/x/a.txt\n/x/b c.txt".into()));

        c.many = Many::Each;
        c.working_dir = "{parent}/out".into();
        let runs = c.invocations(&list, Some(Path::new("/elsewhere")));
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[1].line, "tool \"/x/b c.txt\" --n 1");
        assert_eq!(runs[1].items, vec![list[1].clone()]);
        assert_eq!(runs[1].dir, PathBuf::from("/x/out"), "an absolute folder");
        assert_eq!(runs[0].env[2], ("OX_FOLDER", "/elsewhere".into()));

        // The background: the folder is the item.
        c.command = "echo {path} {name} > list.txt".into();
        let runs = c.invocations(&[], Some(Path::new("/x/My dir")));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].line, "echo \"/x/My dir\" \"My dir\" > list.txt");
        assert!(runs[0].shell_syntax);
        assert!(c.invocations(&[], None).is_empty());

        // Without a current folder (search results): the item's folder.
        c.working_dir.clear();
        let runs = c.invocations(&list[..1], None);
        assert_eq!(runs[0].dir, PathBuf::from("/x"));
    }

    #[test]
    fn menu_places_and_submenus() {
        let cmd = |place, submenu: &str| CustomCommand {
            menu_place: place,
            submenu: submenu.into(),
            ..Default::default()
        };
        let list = [
            cmd(MenuPlace::Open, ""),
            cmd(MenuPlace::Open, "Media"),
            cmd(MenuPlace::Top, ""),
            cmd(MenuPlace::Open, "media "),
            cmd(MenuPlace::Open, "Git"),
        ];
        let numbered = || list.iter().zip(0..);
        assert_eq!(
            menu_groups(numbered(), MenuPlace::Open, "Commands"),
            [
                MenuGroup::Entry(0),
                MenuGroup::Submenu("Media".into(), vec![1, 3]),
                MenuGroup::Submenu("Git".into(), vec![4]),
            ]
        );
        assert_eq!(
            menu_groups(numbered(), MenuPlace::Top, "Commands"),
            [MenuGroup::Entry(2)]
        );
        assert!(menu_groups(numbered(), MenuPlace::Bottom, "Commands").is_empty());

        // More than three with the tools: those without a submenu share one.
        let tools = [
            cmd(MenuPlace::Tools, "Own"),
            cmd(MenuPlace::Tools, ""),
            cmd(MenuPlace::Tools, ""),
            cmd(MenuPlace::Tools, ""),
            cmd(MenuPlace::Tools, ""),
        ];
        assert_eq!(
            menu_groups(tools.iter().zip(0..), MenuPlace::Tools, "Commands"),
            [
                MenuGroup::Submenu("Own".into(), vec![0]),
                MenuGroup::Submenu("Commands".into(), vec![1, 2, 3, 4]),
            ]
        );
        assert_eq!(
            menu_groups(tools[..4].iter().zip(0..), MenuPlace::Tools, "C").len(),
            4,
            "three are listed as they are"
        );
        assert_eq!(MenuPlace::Tags.on_background(), MenuPlace::Delete);
    }

    #[test]
    fn examples_are_complete_and_round_trip() {
        let list = CustomCommands(examples());
        assert!(list.0.len() >= 3);
        for c in &list.0 {
            assert!(!c.name.is_empty() && !c.command.is_empty() && !c.icon.is_empty());
            assert!(c.files || c.folders || c.background);
        }
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct File {
            commands: CustomCommands,
        }
        let file = File { commands: list };
        let text = toml::to_string_pretty(&file).unwrap();
        assert_eq!(toml::from_str::<File>(&text).unwrap(), file);
        // Missing keys get their defaults.
        let partial: File = toml::from_str("[[commands]]\nname = \"x\"\n").unwrap();
        assert_eq!(partial.commands.0[0].name, "x");
        assert!(partial.commands.0[0].files);
        assert_eq!(partial.commands.0[0].menu_place, MenuPlace::Tools);
        assert!(partial.commands.0[0].submenu.is_empty());
    }
}
