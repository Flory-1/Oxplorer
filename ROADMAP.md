# Oxplorer Roadmap

What's still to do and what would be nice to have. Everything that is already in
Oxplorer is listed in the [README](README.md) and, release by release, in the
[CHANGELOG](CHANGELOG.md).

Each phase should end with a usable release. The order within a phase is a
rough priority, not a promise.

Legend: ⏳ planned · 💡 idea, needs a closer look first

---

## Phase 7 — Tried and tested on Windows

Oxplorer is developed on Linux and its automated tests run on Windows in CI. These
Windows-only parts build and pass their tests but haven't been tried by hand on a real
Windows machine yet.

- ⏳ **Manual test on Windows 10 and 11** and fix what comes up:
  - Shell icons, thumbnails, the native context menu, Properties and *Open with*
  - drag & drop out of the app
  - network shares and mapped drives
  - Mica/Acrylic, *Open folders in Oxplorer* (incl. Win+E), HDD/SSD detection
  - the built-in player and the tabs in the title bar (snap layouts, resizing at the top
    edge, Mica without a second set of caption buttons)
  - Explorer clipboard interop, hidden/system attributes, junctions, long paths, high DPI and
    moving the window between monitors with different scaling
- ⏳ **Copy engine benchmarks** against Explorer, robocopy and TeraCopy (many small files, one
  huge file, SSD → USB stick, network share), with the numbers in the README
- ⏳ **Update check**: a quiet note on the Home page when a newer release is on GitHub (can be
  turned off), and a link to the release notes
- ⏳ Code-signed releases, so SmartScreen doesn't warn on the first start

## Phase 8 — Safer file operations

- ⏳ **Undo** (Ctrl+Z) for the last operations: rename, bulk rename, move, copy (deletes the
  copies), new folder/file and delete to the Recycle Bin, with Redo (Ctrl+Y)
- ⏳ **Resume interrupted transfers**: a journal per job, so a copy cut off by a crash, sleep or a
  pulled USB stick continues where it stopped
- ⏳ **Ask about errors while the job runs** (retry / skip / skip all) instead of only
  collecting them for the end
- ⏳ **Transfer queue**: reorder or pause single jobs, *run after the current one*, and a
  setting for what happens when a job finishes (nothing, notification, sleep)
- ⏳ Keep permissions (ACLs), alternate data streams and timestamps of folders, optional
- ⏳ **Compare folders**: two folders side by side with new, changed and missing files marked,
  and copy over what's missing (uses the split view)
- 💡 **Mirror / sync** a folder to another (one way, with a preview of what will change)
- ⏳ Benchmarks in CI, so a slower copy engine fails the build

## Phase 9 — Views and browsing

- ⏳ **Remember the view per folder**: sort order, view mode and columns (e.g. *Downloads* by
  date, *Pictures* as a grid)
- ⏳ **Column chooser**: show, hide, reorder and resize columns, with extra ones such as
  dimensions, length, author, version, attributes and *date taken*
- ⏳ **Grid zoom** with Ctrl+mouse wheel and a size slider, from small icons to large thumbnails
- ⏳ **Group by** type, date, size or tag, with collapsible groups
- ⏳ **Select by pattern** (`*.jpg`), invert selection, select all of the same type
- ⏳ **Quick Look**: Space opens a large preview of the selected file over the window; arrow
  keys go to the next file
- ⏳ **Flat view**: every file in a folder and its sub-folders in one list
- ⏳ **Pinned tabs** (small, always first, not closed by *Close other tabs*) and **named
  sessions/workspaces** (save a set of tabs and groups, open it again later)
- ⏳ **More windows**: open a new window (Ctrl+N), drag a tab out of the window into its own
- ⏳ **Cloud files**: OneDrive / Dropbox status (online only, on this device, syncing) as an
  icon overlay and a column, plus *Always keep on this device* / *Free up space*
- ⏳ **Links**: create symbolic links, junctions and hard links from the New menu or by
  dragging with Ctrl+Shift, and show where a link points
- 💡 Libraries and the Windows Gallery as places in the sidebar

## Phase 10 — Search and disk usage

- ⏳ **Live index updates** from file system events (today the index is refreshed on a schedule)
- ⏳ **NTFS MFT reader** (`FSCTL_ENUM_USN_DATA`) as a much faster index builder on Windows, and
  the USN journal for live updates (needs admin rights → optional helper service)
- ⏳ `tag:` in the query syntax and tags as a filter chip
- ⏳ **Search inside archives** (names of the files in zip/7z/tar)
- ⏳ Search history and suggestions in the search box
- ⏳ **Disk usage view**: a treemap or sunburst of a drive or folder, built from the folder
  size cache, to find what fills the disk
- ⏳ **Find duplicates** (same size, then same hash) and **empty folders**, with a list to
  review before deleting
- 💡 Optional full-text index of documents (txt, md, code, PDF and Office text) for fast
  `content:` searches

## Phase 11 — Previews and media

- ⏳ **Office and other documents** in the preview through Windows' preview handlers
  (`IPreviewHandler`): Word, Excel, PowerPoint, Outlook mails and everything else Explorer
  can preview
- ⏳ **More picture formats**: SVG, HEIC/HEIF, AVIF, RAW (through Windows' codecs), animated
  GIF/WebP, and the photo's **EXIF details** (camera, exposure, date taken, location)
- ⏳ **Data files**: CSV/TSV as a table, JSON/XML/YAML as a collapsible tree, SQLite tables
- ⏳ **Hex view** for binary files
- ⏳ **Player**: next/previous file of the folder, playlists, subtitles (`.srt`), playback speed,
  and *keep playing* while browsing other folders
- ⏳ **Picture tools**: rotate, convert (PNG/JPG/WebP) and resize one or many pictures
- 💡 Diff of two selected text files, and the changes of a file in a Git repository

## Phase 12 — Your own tools

- ⏳ **Custom commands** in the right-click menu and the command palette: a command line with
  placeholders (`{path}`, `{folder}`, `{name}`, `{selection}`), for files of certain types,
  optionally with a shortcut
- ⏳ **Remote locations**: SFTP, FTP and WebDAV as places in the sidebar, browsed and copied
  like local folders through the copy engine
- ⏳ **WSL**: Linux distributions under *This PC* (`\\wsl$`)
- ⏳ **More Git**: stage, commit and switch branches from the Git tab, and the diff of a changed file
- ⏳ **Portable mode**: settings next to `oxplorer.exe` when a `portable` file is there
- ⏳ **Export and import all settings** (incl. favorites, tags, themes and shortcuts) as one file
- ⏳ **Tray icon and global hotkey** to open a window quickly (optional)
- 💡 Plugins or scripts (e.g. Rhai or Lua) for previews, columns and commands
