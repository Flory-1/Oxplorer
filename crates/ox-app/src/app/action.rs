//! The [`Action`] and [`TabAction`] commands that UI code pushes and `process` handles.

use std::path::PathBuf;

use ox_core::recycle::TrashItem;
use ox_core::tags::TagColor;
use ox_core::transfer::TransferKind;

use crate::tabs::{GroupId, Location, TabId};

pub enum Action {
    Navigate(Location),
    OpenInNewTab(Location, bool),
    Back,
    Forward,
    Up,
    Refresh,
    /// Windows' "Map network drive" (`true`) or "Disconnect" (`false`) dialog.
    NetworkDriveDialog(bool),
    /// Open folders in Oxplorer instead of Explorer (or stop doing so).
    SetDefaultFolderHandler(bool),
    /// Open a folder (navigate) or file (default app).
    Open(PathBuf),
    CopyItems(Vec<PathBuf>),
    CutItems(Vec<PathBuf>),
    Paste(PathBuf),
    /// Paste as a transfer that waits until the current ones are done.
    PasteAfterCurrent(PathBuf),
    Transfer(TransferKind, Vec<PathBuf>, PathBuf),
    /// Items dropped onto a folder; the kind is decided from modifiers and drives.
    Drop(Vec<PathBuf>, PathBuf),
    Delete(Vec<PathBuf>, bool),
    DeleteConfirmed(Vec<PathBuf>, bool),
    /// Delete again (after the "Couldn't delete" dialog), ending the
    /// programs with these process ids first.
    RetryDelete {
        paths: Vec<PathBuf>,
        to_trash: bool,
        end: Vec<u32>,
    },
    /// Delete permanently with administrator rights.
    DeleteAsAdmin(Vec<PathBuf>),
    Rename(PathBuf),
    /// Rename several items at once with a pattern.
    BulkRename(Vec<PathBuf>),
    /// Add the tag to all items, or remove it if all have it.
    ToggleTag(Vec<PathBuf>, TagColor),
    ClearTags(Vec<PathBuf>),
    /// Extract a whole archive next to it: into a new folder named after it,
    /// or straight into the archive's folder.
    ExtractArchive {
        archive: PathBuf,
        own_folder: bool,
    },
    /// Put these items into a new zip file next to them.
    Compress(Vec<PathBuf>),
    /// Copy a theme file into the themes folder and use it.
    InstallTheme(PathBuf),
    /// Show or hide the terminal panel (starting the shell the first time).
    ToggleTerminal,
    RestartTerminal,
    /// Items were renamed or moved (old path, new path): tags go along.
    PathsMoved(Vec<(PathBuf, PathBuf)>),
    /// Give a tag colour a custom name (empty = default).
    RenameTag(TagColor, String),
    NewFolder(PathBuf),
    NewFile(PathBuf),
    /// Create a file from the user's "New" entry with this index.
    NewFromItem(PathBuf, usize),
    Pin(PathBuf),
    Unpin(PathBuf),
    MoveFavorite(PathBuf, isize),
    CopyPathText(Vec<PathBuf>),
    /// "Copy path as": the paths in another form.
    CopyPathAs(Vec<PathBuf>, ox_core::pathfmt::PathFormat),
    /// Open files or folders with the editor from Settings → Developer.
    OpenInEditor(Vec<PathBuf>),
    /// Open the page of a file or folder on GitHub, GitLab, … (its repository's remote).
    OpenGitRemote(PathBuf),
    /// Copy the checked out branch of the repository a path is in.
    CopyGitBranch(PathBuf),
    /// What the Git tab asks of the app.
    Git(super::GitAction),
    OpenTerminal(PathBuf),
    /// A shell of the WSL distribution the folder is in, in that folder.
    OpenWslTerminal(PathBuf),
    Reveal(PathBuf),
    Properties(Vec<PathBuf>),
    /// Cloud files: *Always keep on this device*, *Free up space*, or neither.
    KeepOnDevice(Vec<PathBuf>, ox_core::cloud::Pin),
    /// Make items read-only or not, hidden or not (`None` leaves it).
    SetAttributes(Vec<PathBuf>, Option<bool>, Option<bool>),
    Search(Option<PathBuf>, String),
    /// Search while typing: updates the current search tab instead of adding history.
    SearchLive(Option<PathBuf>, String),
    SaveSearch(Option<PathBuf>, String),
    /// The searches that were run (suggested under the search box).
    SearchHistory(super::HistoryAction),
    RemoveSavedSearch(usize),
    RebuildIndex,
    SelectAll,
    /// Select by pattern, invert, same type.
    Select(super::SelectAction),
    /// Open Quick Look on the selected item, or close it.
    QuickLook,
    ForgetRecent(PathBuf),
    ClearRecent,
    SettingsChanged,
    Tab(TabAction),
    Toast(String),
    /// Windows: show the Explorer context menu ("Show more options").
    ShellMenu(Vec<PathBuf>),
    /// Windows: "Open with…" dialog.
    OpenWith(PathBuf),
    /// Recycle Bin: put entries (by their bin path) back where they came from.
    RestoreFromBin(Vec<PathBuf>),
    /// Recycle Bin: delete entries for good (asks first).
    PurgeFromBin(Vec<PathBuf>),
    EmptyBin,
    PurgeConfirmed(Vec<TrashItem>, bool),
    /// Move items to the Recycle Bin without asking (e.g. dropped onto it).
    MoveToBin(Vec<PathBuf>),
    /// Compare folders (the split view's panes).
    Compare(super::CompareAction),
    /// Mirror / sync a folder to another.
    Mirror(super::mirror::MirrorAction),
    /// Find duplicates / empty folders: delete the ticked items, show one.
    Cleanup(super::CleanupAction),
    /// Picture tools: rotate, flip, convert and resize pictures.
    Pictures(super::PictureAction),
    /// Custom commands (Settings → Custom commands).
    Command(super::CommandAction),
    /// Turn the flat view (all files of the sub-folders in one list) on or off.
    ToggleFlatView,
    /// Go to the folder an item is in and select it there.
    ShowInFolder(PathBuf),
    /// Open the dialog for a new symbolic link, junction or hard link in this folder.
    NewLink(PathBuf, ox_core::links::LinkKind),
    /// Go to where a link points (its target's folder, with the target selected).
    ShowLinkTarget(PathBuf),
    /// Named sessions: save, open, rename and delete sets of tabs.
    Session(super::SessionAction),
    /// How the file view looks (columns, …).
    View(super::ViewAction),
    /// The disk usage view: scan again, stop.
    DiskUsage(super::DiskUsageAction),
    /// The built-in player: play, next, play all, playlists, ...
    Player(super::PlayerAction),
    /// Remote locations: connect, open, add/edit/remove places.
    Remote(super::RemoteAction),
    /// Compare two text files (old, new) in a window.
    CompareTextFiles(PathBuf, PathBuf),
}

pub enum TabAction {
    New,
    /// Open a new window showing what the active tab shows (Ctrl+N).
    NewWindow,
    /// Open a tab in a new window (at this screen position) and close it here.
    ToNewWindow(TabId, Option<[f32; 2]>),
    Close(TabId),
    Activate(TabId),
    Duplicate(TabId),
    /// Pin (`true`) or unpin a tab.
    Pin(TabId, bool),
    CloseOthers(TabId),
    NewGroup(TabId),
    AddToGroup(TabId, GroupId),
    RemoveFromGroup(TabId),
    ToggleGroup(GroupId),
    CloseGroup(GroupId),
    Ungroup(GroupId),
    RenameGroup(GroupId),
    SetGroupColor(GroupId, usize),
    NewTabInGroup(GroupId),
    Move(TabId, usize),
    Cycle(isize),
    /// Turn the split view (two panes) on or off.
    ToggleSplit,
    /// Focus the other pane.
    SwitchPane,
}
