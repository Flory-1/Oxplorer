//! The Help page (opens as a tab, like Settings): getting started, features,
//! search syntax, keyboard shortcuts and answers to common questions.

use egui::{vec2, RichText, Ui};
use ox_core::i18n::t as tr;

use crate::app::{Action, OxApp, Toast};
use crate::shortcuts::{self, COMMANDS};
use crate::tabs::Location;

const SECTIONS: [&str; 5] = [
    "🚀 Getting started",
    "✨ Features",
    "🔍 Search syntax",
    "⌨ Keyboard shortcuts",
    "❓ Questions",
];
const SHORTCUTS: usize = 3;

enum Block {
    Text(&'static str),
    List(&'static [&'static str]),
    /// Two columns: something to type or press, and what it does.
    Table(&'static [(&'static str, &'static str)]),
}

struct Topic {
    section: usize,
    title: &'static str,
    blocks: &'static [Block],
}

impl Topic {
    /// Searches the text as shown, i.e. in the current language.
    fn matches(&self, query: &str) -> bool {
        let has = |s: &'static str| tr(s).to_lowercase().contains(query);
        has(self.title)
            || self.blocks.iter().any(|b| match b {
                Block::Text(t) => has(t),
                Block::List(items) => items.iter().any(|i| has(i)),
                Block::Table(rows) => rows.iter().any(|(a, b)| has(a) || has(b)),
            })
    }
}

use Block::{List, Table, Text};

const TOPICS: &[Topic] = &[
    // ---- Getting started ----------------------------------------------------
    Topic {
        section: 0,
        title: "Welcome to Oxplorer",
        blocks: &[
            Text(
                "Oxplorer works like Windows Explorer, with tabs, a much faster search and its \
                 own copy engine. Everything you know from Explorer works here too: double-click \
                 to open, right-click for a menu, drag files to move them.",
            ),
            Text(
                "Use the filter box at the top left of this page to find a topic. Press F1 \
                 anywhere to come back here.",
            ),
        ],
    },
    Topic {
        section: 0,
        title: "Moving around",
        blocks: &[List(&[
            "The first tab is always Home: your favorites, drives and recently used folders \
             and files. What you open from it opens in a new tab, so Home stays one click away.",
            "The sidebar has Favorites, This PC, Network, Recent, Tags and Saved searches. \
             Search, Recycle Bin, Help and Settings are buttons at the right end of the \
             command bar.",
            "Click a folder name in the address bar to go there, or the › next to it to pick \
             a sub-folder. Click the empty part of the bar to type a path; %USERPROFILE% and ~ \
             work.",
            "Back, Forward and Up are next to the address bar. The side buttons of your mouse \
             work too.",
            "Type the first letters of a name to jump to it.",
        ])],
    },
    Topic {
        section: 0,
        title: "Tabs and tab groups",
        blocks: &[List(&[
            "Ctrl+T opens a new tab, Ctrl+W closes it. Middle-click a folder to open it in a \
             new tab, or a tab to close it.",
            "Drag tabs to reorder them. Drag files onto a tab to switch to it and drop them \
             inside.",
            "Right-click a tab → Add to new group. Groups have a name and colour; click the \
             group's chip to fold it away.",
            "Oxplorer can restore your tabs and groups on start (Settings → Startup).",
        ])],
    },
    Topic {
        section: 0,
        title: "Copying and moving files",
        blocks: &[
            Text(
                "Oxplorer copies with its own engine instead of the Windows copy dialog. \
                 Several files are copied at once and progress shows in the transfer panel at \
                 the bottom.",
            ),
            List(&[
                "Ctrl+C / Ctrl+X / Ctrl+V, or drag and drop. Dropping on the same drive moves, \
                 on another drive copies. Hold Ctrl to copy or Shift to move.",
                "When files already exist you are asked once for the whole job: replace, skip, \
                 keep both or replace if newer.",
                "Pause, resume or cancel a transfer in the panel. Failed files can be retried \
                 with one click.",
                "When a file fails, the job waits and asks: retry, skip, or skip all errors of \
                 the job.",
                "Waiting transfers can be held, moved up or down, or set to run after the \
                 current ones; \"Paste after current transfers\" queues a paste that way. \
                 Settings → Copy engine: what happens when all transfers are done.",
                "Settings → Copy engine: verify copies, limit the speed, and see the history.",
                "When transfers are done while Oxplorer is in the background, Windows shows a \
                 notification; click it to come back. Settings → Copy engine turns it off.",
                "The clipboard is shared with Windows Explorer in both directions.",
            ]),
        ],
    },
    Topic {
        section: 0,
        title: "Favorites, recent items and tags",
        blocks: &[List(&[
            "Right-click a folder → Pin to Favorites. Drop files on a favorite in the sidebar \
             to copy or move them there.",
            "Right-click files → Tags to mark them with colours. Tagged items show coloured \
             dots and appear under Tags in the sidebar. Right-click a tag there to rename it.",
            "Recent folders and files are on the Home page and in the sidebar.",
        ])],
    },
    // ---- Features -----------------------------------------------------------
    Topic {
        section: 1,
        title: "Preview pane",
        blocks: &[
            Text(
                "Alt+P shows the preview pane on the right. The Info tab lists the details of \
                 the selected item; the other tabs show its content.",
            ),
            List(&[
                "Images, text and code (with syntax highlighting), the first page of PDFs. \
                 Markdown files are shown formatted in the Markdown tab.",
                "Songs and videos play in the Player tab: seek, 10 seconds back and forward, \
                 repeat and volume. The Info tab shows their tags and length.",
                "Fonts: a sample at several sizes, and every character in the Glyphs tab \
                 (click one to copy it).",
                "Folders and archives: what's inside and how big it is. Folders in a Git \
                 repository have a Git tab with the history of all branches as a graph \
                 (click a commit to copy its hash).",
                "Text files: encoding, line endings and indentation in the Info tab. Compute \
                 checksums gives the SHA-256 and MD5 of any file.",
                "Pictures also in SVG, HEIC, AVIF, JPEG XL and camera RAW (with the codecs \
                 installed in Windows), and animated GIF and WebP play. For photos the Info tab \
                 shows the camera, exposure and location, with Open in map.",
                "Word, Excel, PowerPoint, Outlook mails and other documents are shown in the \
                 Document tab by the same preview handlers Explorer uses.",
                "CSV files as a table, JSON, XML, YAML and TOML as a tree you can fold, SQLite \
                 databases as tables. The Hex tab shows the bytes of any file.",
                "These views also work for items inside zip and 7z archives.",
                "Select two text files → right-click → Compare these two files shows what \
                 differs between them, in one column or side by side.",
            ]),
        ],
    },
    Topic {
        section: 1,
        title: "Player and picture tools",
        blocks: &[List(&[
            "⏮ and ⏭ (Ctrl+Left and Ctrl+Right) play the previous or next file of the folder, \
             Ctrl+Space plays and pauses. The player also has shuffle, repeat, speed and a \
             queue.",
            "Right-click a folder or several songs or videos → Play all. Playlists (.m3u) play \
             when you open them, and subtitles (.srt, .vtt) next to a video are shown over it.",
            "When you go elsewhere, playback goes on in the mini player at the bottom of the \
             window; click its title to go back to the file. Settings → Preview → Keep \
             playing while browsing turns that off.",
            "Right-click pictures → Picture tools to rotate, flip, convert or resize them. \
             Replaced originals go to the Recycle Bin, and Ctrl+Z brings them back.",
            "Rotating or flipping a JPEG only changes the orientation stored in the photo, so \
             nothing of the picture is lost.",
        ])],
    },
    Topic {
        section: 1,
        title: "Split view",
        blocks: &[Text(
            "Ctrl+\\ shows two folders side by side. Click a pane or press F6 to work in it, \
             and drag files from one pane to the other to copy or move them.",
        )],
    },
    Topic {
        section: 1,
        title: "Comparing folders",
        blocks: &[List(&[
            "Compare folders (Ctrl+Shift+D, the ⇄ button between the panes or the command \
             palette) compares the folders of the two panes. Without the split view it opens \
             it: pick the other folder in the new pane.",
            "Coloured strips mark the items: green is only on this side, blue is newer, orange \
             is older, red is different (same date but another size, or a folder with \
             differences inside). Items without a strip are the same.",
            "Files count as the same when size and date modified match. Dates up to two \
             seconds apart count as equal, since FAT and exFAT drives store them that coarsely. \
             Compare contents checks files of the same size byte by byte instead.",
            "Sub-folders also compares what's inside folders both sides have. Only differences \
             hides what's the same.",
            "Copy missing copies what only one side has to the other side. Copy newer replaces \
             older files with the newer ones, both ways. You're asked first, and the copies \
             show in the transfer panel.",
            "The comparison follows the panes: it's made again when a pane shows another \
             folder or its files change.",
            "Right-click a folder → Mirror to, or 🔁 Mirror in the compare bar, makes another \
             folder like this one (Mirror) or only copies what's new or changed (Update). A \
             preview shows first what will be copied, updated and removed; untick what you \
             want to keep. Removed items go to the Recycle Bin.",
        ])],
    },
    Topic {
        section: 1,
        title: "Searching",
        blocks: &[List(&[
            "Typing in the search box filters the current folder instantly.",
            "Enter searches all sub-folders too. On the Home page it searches your whole PC.",
            "With the search index on (Settings → Search), searching whole drives takes \
             milliseconds and results appear as you type.",
            "Filter chips above the results narrow them down by kind, date and size. Save a \
             search to find it in the sidebar.",
            "There are chips for your colour tags too, and Inside archives also finds the \
             files in zip, 7z and tar archives.",
            "The search box remembers your searches. Up and Down choose an earlier search or \
             a completion of what you type (ki → kind:); Enter runs it, Tab puts it in the \
             box. Settings → Search → Remember searches turns the history off.",
            "Settings → Search → Text index keeps the words of your documents (also PDF, \
             Word, Excel and PowerPoint), so content: searches in those folders are fast.",
            "In a library, Enter searches all of the library's folders.",
            "See Search syntax for everything you can type.",
        ])],
    },
    Topic {
        section: 1,
        title: "Archives (zip, 7z, tar)",
        blocks: &[List(&[
            "Double-click a .zip, .7z, .tar, .tar.gz or .tgz file to browse it like a folder.",
            "Drag or copy items out of an archive to extract them.",
            "Right-click an archive → Extract here, or Extract to a new folder.",
            "Right-click files → Compress to ZIP.",
        ])],
    },
    Topic {
        section: 1,
        title: "Renaming many files",
        blocks: &[
            Text(
                "Select several items and press F2. Build the new names from a pattern, find \
                 and replace text (or a regular expression), and change the case. Every new \
                 name is shown before anything changes.",
            ),
            Table(&[
                ("{name}", "the old name without its extension"),
                ("{ext}", "the extension"),
                ("{n}, {n:3}", "a counter, optionally with leading zeros"),
                ("{date}", "the date the file was modified (YYYY-MM-DD)"),
                ("{parent}", "the name of the folder it is in"),
            ]),
        ],
    },
    Topic {
        section: 1,
        title: "Command palette",
        blocks: &[Text(
            "Ctrl+Shift+P opens the command palette. Type a few letters to run any command \
             or jump to a favorite, drive, recent folder, tag or saved search.",
        )],
    },
    Topic {
        section: 1,
        title: "Folder sizes and Git status",
        blocks: &[List(&[
            "The Size column shows the size of folders too. They are calculated in the \
             background; … means a size is still being calculated.",
            "Inside a Git repository a Git column shows what changed: M modified, A added, \
             D deleted, R renamed, ? untracked, I ignored. The status bar shows the branch.",
        ])],
    },
    Topic {
        section: 1,
        title: "Disk usage, duplicates and empty folders",
        blocks: &[List(&[
            "Right-click a folder or drive → Disk usage shows what takes up the space, as a \
             map of rectangles, with the largest items next to it. Click a folder to zoom \
             in; Back or Up zooms out again.",
            "Right-click a folder → Clean up → Find duplicates lists the files with the same \
             contents and the space they waste. Tick the copies you don't need, or use Select \
             all but the newest, and Delete selected moves them to the Recycle Bin (Ctrl+Z \
             brings them back).",
            "Compare contents byte by byte reads duplicates again after their checksums \
             match, to be completely sure.",
            "Clean up → Find empty folders lists the folders with nothing in them, to review \
             and delete the same way.",
        ])],
    },
    Topic {
        section: 1,
        title: "For developers",
        blocks: &[List(&[
            "Open in editor (Ctrl+Shift+E or right-click) opens files and folders in VS Code \
             or the editor you choose in Settings → Developer.",
            "Copy path as: with forward slashes, in quotes, relative to the Git repository, \
             as a file:// URL or just the name.",
            "In Git repositories, right-click → Git opens the file or folder on GitHub or \
             GitLab, or copies the branch name. The Info tab shows the last commit that \
             changed the selected item.",
            "➕ New can create your own file types with a name and content to start with, \
             such as a Python script (Settings → New menu).",
            "The preview's Git tab stages and unstages files, commits, and switches or creates \
             branches; the Diff tab shows what changed in a file. Discarding changes asks \
             first, as it can't be undone.",
            "Settings → Custom commands adds your own command lines to the right-click menu \
             and the command palette. {path}, {paths}, {name}, {stem}, {ext}, {parent}, \
             {folder} and {n} are filled in with the selected items.",
            "Their shortcuts are on Settings → Shortcuts too, next to the built-in ones.",
            "The Git tab's Branches view lists your stashes to apply, pop or drop. Fetch, pull \
             and push show their progress and can be cancelled.",
        ])],
    },
    Topic {
        section: 1,
        title: "Remote locations and WSL",
        blocks: &[List(&[
            "Sidebar → Remote locations → Add… connects to an SFTP, FTP or WebDAV server, or \
             type an address like sftp://user@server in the address bar. Copy, move, rename \
             and delete there like in a folder; deleted files don't go to the Recycle Bin.",
            "The first time Oxplorer connects to an SFTP server it shows the server's key to \
             check. If the key changes later, it warns and doesn't connect until you confirm.",
            "Passwords are kept in the Windows Credential Manager only when you tick Remember \
             the password, never in the settings.",
            "Transfers with servers continue where they stopped after the connection dropped, \
             or from the transfer panel after a restart. Copying from one server to another \
             works too.",
            "A file you open from a server opens as a downloaded copy. When you save it, \
             Oxplorer offers to upload the changes, and warns first if the server's file \
             changed in the meantime.",
            "Your WSL Linux distributions are under This PC. Right-click a folder in one → \
             Open in WSL terminal; Copy path as → As Linux path gives its Linux path.",
        ])],
    },
    Topic {
        section: 1,
        title: "Terminal panel",
        blocks: &[Text(
            "Ctrl+` opens a terminal under the file view, in the folder you are looking at; \
             opening it again later takes it to the current folder too. Choose the shell and \
             whether it follows every folder you open in Settings → Browsing. Ctrl+Shift+V pastes.",
        )],
    },
    Topic {
        section: 1,
        title: "Recycle Bin",
        blocks: &[Text(
            "Delete moves items to the Recycle Bin; Shift+Delete deletes them for good. Open \
             the Recycle Bin with its button at the right of the command bar to restore items \
             or empty it. Drop files on that button to delete them. When something can't be \
             deleted, a dialog shows exactly why: which programs have it open (with a button \
             to end them and delete again) or that it needs administrator rights.",
        )],
    },
    Topic {
        section: 1,
        title: "Undo and redo",
        blocks: &[
            Text(
                "Ctrl+Z undoes the last file operation, Ctrl+Y (or Ctrl+Shift+Z) redoes it. \
                 The last 100 operations are remembered, also after a restart (Settings → \
                 Browsing → Keep undo history after restart).",
            ),
            List(&[
                "Rename and bulk rename: the old names come back.",
                "Move: the items are moved back.",
                "Copy, new file: the copies go to the Recycle Bin. New folder: it is removed \
                 while it's still empty.",
                "Delete to the Recycle Bin: the items are restored.",
                "Nothing is undone if the items have been changed, moved or replaced since. \
                 Permanent deletes can't be undone, and replaced files don't come back.",
            ]),
        ],
    },
    Topic {
        section: 1,
        title: "Columns, grouping and icon size",
        blocks: &[List(&[
            "Right-click the column header (or View → Columns) to show or hide columns: date \
             created, dimensions, length, date taken, authors, file version, attributes and, \
             in OneDrive or Dropbox folders, Status. Drag a header to move it, its edge to \
             resize it.",
            "Each folder remembers its view, sort order, columns and grouping. View → Reset \
             view of this folder forgets it; Use this view for all folders makes it the \
             default. Settings → Browsing turns it off.",
            "Click a column header to sort by it, also by dimensions, length, date taken, \
             authors and file version. Those are read in the background, so items move into \
             place as their values come in.",
            "Sort → Group by puts items into groups by type, date modified, month, size or tag. \
             Click a group's header to fold it away.",
            "In the grid, Ctrl+mouse wheel or the slider in the status bar makes the icons \
             smaller or larger.",
        ])],
    },
    Topic {
        section: 1,
        title: "Quick Look, selecting and the flat view",
        blocks: &[List(&[
            "Space shows the selected file large over the window. The arrow keys go to the \
             next or previous one; Space or Esc closes it.",
            "Ctrl+Shift+A selects by pattern (*.jpg; *.png), Ctrl+Shift+I inverts the \
             selection. Right-click → Select also selects all items of the same type.",
            "Ctrl+Shift+F (the flat view) lists every file of the folder and its sub-folders \
             in one list, with the folder each is in.",
        ])],
    },
    Topic {
        section: 1,
        title: "Pinned tabs, sessions and windows",
        blocks: &[List(&[
            "Right-click a tab → Pin tab: it shrinks to its icon, stays in front and isn't \
             closed by Close other tabs.",
            "Right-click the + button → Save tabs as session… keeps the open tabs and groups \
             under a name, to open again later instead of or next to the open ones.",
            "Ctrl+N opens a new window. Drag a tab out of the window, or right-click it → Move \
             to new window, to give it a window of its own. The first window keeps the tabs \
             for the next start.",
        ])],
    },
    Topic {
        section: 1,
        title: "Links, cloud files and libraries",
        blocks: &[List(&[
            "New → Symbolic link, Junction or Hard link, or drag items with Ctrl+Shift onto a \
             folder, links them there. Links show an arrow, and right-click → Open link target \
             location goes to what they point to.",
            "In OneDrive or Dropbox folders, a badge shows whether a file is online only, on \
             this device or always kept. Right-click → Always keep on this device or Free up \
             space changes it. Online-only files aren't downloaded just for a preview.",
            "The sidebar's Libraries section shows Windows' libraries (all their folders in one \
             view, refreshed when files change) and the Gallery: every picture and video of the \
             Pictures library, newest first and grouped by month.",
            "In the New link dialog, Browse… picks what the link points to, and the field \
             completes folders and files as you type.",
        ])],
    },
    Topic {
        section: 1,
        title: "Windows integration",
        blocks: &[List(&[
            "Files have the same icons as in Explorer.",
            "Show more options (Shift+F10) opens the real Explorer context menu, with \
             entries from 7-Zip, VS Code and other tools.",
            "Open with uses the Windows dialog.",
            "The tabs sit in the title bar, as in Windows 11's Explorer. Drag the free space \
             next to them to move the window, double-click it to maximize, and point at the \
             maximize button for the snap layouts. Settings → Appearance → Tabs in the title \
             bar brings Windows' title bar back.",
        ])],
    },
    // ---- Search syntax ------------------------------------------------------
    Topic {
        section: 2,
        title: "What you can type",
        blocks: &[
            Text(
                "All parts must match. Upper and lower case don't matter (unless you change \
                  that in Settings → Search).",
            ),
            Table(&[
                ("report", "names that contain \"report\""),
                ("\"annual report\"", "names that contain the exact phrase"),
                ("*.pdf, img_??", "wildcards: * is any text, ? one character"),
                ("-draft", "leave out names that contain \"draft\""),
                (
                    "/^IMG_\\d+\\.jpe?g$/",
                    "a regular expression (also regex:…)",
                ),
                ("ext:pdf,docx", "by extension"),
                ("type:folder, type:file", "only folders or only files"),
                (
                    "kind:image",
                    "by kind: image, video, audio, doc, pdf, archive, code, text, app, font",
                ),
                ("size:>10mb, size:<1kb", "by size (kb, mb, gb)"),
                ("modified:today", "changed today (also week, month, year)"),
                (
                    "modified:7d",
                    "changed in the last 7 days (m minutes, h, d, w, y)",
                ),
                (
                    "content:\"TODO\"",
                    "text inside files (fast in the folders of the text index)",
                ),
                (
                    "tag:red,blue",
                    "tagged with one of these colours or tag names",
                ),
                (
                    "archives:yes",
                    "also the files inside zip, 7z and tar archives (not with the index)",
                ),
            ]),
        ],
    },
    Topic {
        section: 2,
        title: "Examples",
        blocks: &[Table(&[
            ("kind:image size:>5mb", "big pictures"),
            (
                "ext:docx modified:week -~$",
                "Word files from the last week, without lock files",
            ),
            ("*.log content:error", "log files that mention errors"),
            ("type:folder node_modules", "every node_modules folder"),
        ])],
    },
    // ---- Questions -----------------------------------------------------------
    Topic {
        section: 4,
        title: "Why don't I see the Windows copy dialog?",
        blocks: &[Text(
            "Oxplorer copies with its own, faster engine. Progress, speed and time left are \
             in the transfer panel at the bottom of the window.",
        )],
    },
    Topic {
        section: 4,
        title: "A copy was cut off. Do I have to start again?",
        blocks: &[Text(
            "No. If Oxplorer closed, the computer went to sleep or a USB stick was pulled \
             during a copy or move, the transfer panel shows it as interrupted. Resume skips \
             the files that were copied completely and copies the rest; Discard forgets it. \
             A move only deletes an original once its copy is complete.",
        )],
    },
    Topic {
        section: 4,
        title: "Can I use Oxplorer and Windows Explorer together?",
        blocks: &[Text(
            "Yes. They share the clipboard, so you can copy in one and paste in the other. \
             Right-click → Show in Windows Explorer opens the current item there.",
        )],
    },
    Topic {
        section: 4,
        title: "Where are my settings stored?",
        blocks: &[Text(
            "In %APPDATA%\\Oxplorer: settings.toml (settings and shortcuts), \
             quick_access.toml (favorites, recent items, saved searches), tags.toml, the \
             transfer history, the search index and, in logs, the log and crash reports. \
             Settings → About shows the folder.",
        )],
    },
    Topic {
        section: 4,
        title: "Searching my whole drive is slow",
        blocks: &[
            Text(
                "Turn on the search index in Settings → Search and pick the drives to index. \
                 The first build takes a while; after that searches are instant and the index \
                 is refreshed in the background. Searches with content: always read the files.",
            ),
            Text(
                "With Live updates on, new, renamed and deleted files show up in the index \
                 within seconds. Run Oxplorer as administrator once to build the index of NTFS \
                 drives from their master file table, which is much faster.",
            ),
            Text(
                "Or install the optional Fast index service (installer, for all users) and turn \
                 it on in Settings → Search: the master file table is then read without \
                 administrator rights, and what changed while Oxplorer was closed is caught up \
                 from the change journal instead of building the index again.",
            ),
        ],
    },
    Topic {
        section: 4,
        title: "How do I change a keyboard shortcut?",
        blocks: &[Text(
            "Settings → Shortcuts: click a shortcut and press the new keys. ↺ brings the \
             default back, ✖ removes it.",
        )],
    },
    Topic {
        section: 4,
        title: "Something doesn't work",
        blocks: &[Text(
            "Please report it on GitHub (link below) with what you did and what happened. \
             Open log folder (below) has Oxplorer's log and, if it crashed, a crash report: \
             attach them to the report.",
        )],
    },
];

/// Text box at the top of the section list.
#[derive(Default)]
pub struct HelpState {
    pub section: usize,
    pub filter: String,
}

/// Shown on the first start after Oxplorer closed because of a crash.
pub fn crash_note(app: &mut OxApp) {
    app.toasts.push(Toast {
        text: tr(
            "Oxplorer closed unexpectedly last time. The crash report is in the log \
                  folder: Help → Open log folder.",
        )
        .into(),
        until: std::time::Instant::now() + std::time::Duration::from_secs(15),
    });
}

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    egui::Panel::left("help_nav")
        .resizable(false)
        .exact_size(210.0)
        .frame(egui::Frame::new().inner_margin(egui::Margin::same(14)))
        .show(ui, |ui| {
            ui.heading(RichText::new(tr("Help")).strong());
            ui.add_space(10.0);
            ui.add(
                egui::TextEdit::singleline(&mut app.ui.help.filter)
                    .hint_text(tr("🔍 Find a topic"))
                    .desired_width(180.0),
            );
            ui.add_space(8.0);
            for (i, name) in SECTIONS.iter().enumerate() {
                let selected = app.ui.help.section == i && app.ui.help.filter.is_empty();
                if super::nav_item(ui, tr(name), selected).clicked() {
                    app.ui.help.section = i;
                    app.ui.help.filter.clear();
                }
            }
        });
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(24, 16))
                .show(ui, |ui| {
                    ui.set_max_width(720.0);
                    let query = app.ui.help.filter.trim().to_lowercase();
                    if query.is_empty() {
                        section(app, ui, app.ui.help.section);
                    } else {
                        results(app, ui, &query);
                    }
                    ui.add_space(24.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tr("More help and bug reports:")).weak());
                        ui.hyperlink_to(
                            "github.com/Flory-1/Oxplorer",
                            "https://github.com/Flory-1/Oxplorer/issues",
                        );
                        ui.add_space(12.0);
                        if ui
                            .button(tr("📂 Open log folder"))
                            .on_hover_text(tr(
                                "The log and crash reports, to attach to a bug report",
                            ))
                            .clicked()
                        {
                            app.actions.push(Action::OpenInNewTab(
                                Location::Folder(ox_core::logfile::dir()),
                                true,
                            ));
                        }
                    });
                });
        });
}

fn section(app: &OxApp, ui: &mut Ui, index: usize) {
    let title = tr(SECTIONS[index]);
    let title = &title[title.find(' ').map_or(0, |i| i + 1)..];
    ui.heading(RichText::new(title).size(20.0).strong());
    ui.add_space(8.0);
    if index == SHORTCUTS {
        shortcut_table(app, ui);
        return;
    }
    for topic in TOPICS.iter().filter(|t| t.section == index) {
        topic_ui(ui, topic);
    }
}

fn results(app: &OxApp, ui: &mut Ui, query: &str) {
    let found: Vec<&Topic> = TOPICS.iter().filter(|t| t.matches(query)).collect();
    let keys = COMMANDS
        .iter()
        .any(|c| tr(c.label).to_lowercase().contains(query));
    if found.is_empty() && !keys {
        ui.label(RichText::new(tr("Nothing found. Try another word.")).weak());
        return;
    }
    for topic in found {
        let section = tr(SECTIONS[topic.section]);
        ui.label(
            RichText::new(&section[section.find(' ').map_or(0, |i| i + 1)..])
                .weak()
                .small(),
        );
        topic_ui(ui, topic);
    }
    if keys {
        ui.label(RichText::new(tr("Keyboard shortcuts")).weak().small());
        shortcut_rows(app, ui, Some(query));
    }
}

fn topic_ui(ui: &mut Ui, topic: &Topic) {
    ui.label(RichText::new(tr(topic.title)).size(16.0).strong());
    ui.add_space(2.0);
    for block in topic.blocks {
        match block {
            Block::Text(t) => {
                ui.add(egui::Label::new(tr(t)).wrap());
            }
            Block::List(items) => {
                for item in *items {
                    ui.horizontal_top(|ui| {
                        ui.label("•");
                        ui.add(egui::Label::new(tr(item)).wrap());
                    });
                }
            }
            Block::Table(rows) => {
                egui::Grid::new(topic.title)
                    .striped(true)
                    .num_columns(2)
                    .spacing(vec2(16.0, 4.0))
                    .show(ui, |ui| {
                        for (keys, what) in *rows {
                            ui.monospace(*keys);
                            ui.add(egui::Label::new(tr(what)).wrap());
                            ui.end_row();
                        }
                    });
            }
        }
        ui.add_space(4.0);
    }
    ui.add_space(12.0);
}

/// The shortcuts in effect, including the user's own.
fn shortcut_table(app: &OxApp, ui: &mut Ui) {
    ui.label(
        RichText::new(tr(
            "These are the keys in use now. Change them in Settings → Shortcuts.",
        ))
        .weak(),
    );
    ui.add_space(6.0);
    shortcut_rows(app, ui, None);
    ui.add_space(12.0);
    ui.label(RichText::new(tr("Always available")).size(16.0).strong());
    egui::Grid::new("help_fixed_keys")
        .striped(true)
        .num_columns(2)
        .spacing(vec2(16.0, 4.0))
        .show(ui, |ui| {
            for (k, d) in [
                (tr("Enter"), tr("Open the selection")),
                (
                    tr("Arrow keys, Home, End"),
                    tr("Move the selection (with Shift: extend it)"),
                ),
                (tr("Type letters"), tr("Jump to the item with that name")),
                (tr("Mouse side buttons"), tr("Back / forward")),
                (
                    tr("Middle click"),
                    tr("Open a folder in a new tab / close a tab"),
                ),
                ("Ctrl+Shift+V (terminal)", tr("Paste into the terminal")),
            ] {
                ui.monospace(k);
                ui.label(d);
                ui.end_row();
            }
        });
}

fn shortcut_rows(app: &OxApp, ui: &mut Ui, filter: Option<&str>) {
    egui::Grid::new(("help_keys", filter.is_some()))
        .striped(true)
        .num_columns(2)
        .spacing(vec2(16.0, 4.0))
        .show(ui, |ui| {
            for c in COMMANDS {
                if filter.is_some_and(|q| !tr(c.label).to_lowercase().contains(q)) {
                    continue;
                }
                let keys: Vec<String> = app
                    .keymap
                    .keys(c.cmd)
                    .iter()
                    .map(shortcuts::display)
                    .collect();
                let keys = if keys.is_empty() {
                    "—".to_owned()
                } else {
                    keys.join(" / ")
                };
                ui.monospace(keys);
                ui.label(tr(c.label));
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_has_a_valid_section_and_can_be_found() {
        for t in TOPICS {
            assert!(
                t.section < SECTIONS.len() && t.section != SHORTCUTS,
                "{}",
                t.title
            );
            assert!(t.matches(&t.title.to_lowercase()));
        }
        assert!(TOPICS.iter().any(|t| t.matches("node_modules")));
        assert!(!TOPICS.iter().any(|t| t.matches("zzzz-nothing")));
    }

    #[test]
    fn every_topic_is_translated() {
        let de = |s: &str| assert!(ox_core::i18n::german(s).is_some(), "no German for {s:?}");
        SECTIONS.iter().for_each(|s| de(s));
        for t in TOPICS {
            de(t.title);
            for block in t.blocks {
                match block {
                    Block::Text(s) => de(s),
                    Block::List(items) => items.iter().for_each(|s| de(s)),
                    // The first column is something to type and stays as is.
                    Block::Table(rows) => rows.iter().for_each(|(_, what)| de(what)),
                }
            }
        }
    }
}
