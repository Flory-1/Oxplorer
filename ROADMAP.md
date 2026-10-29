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
  - OneDrive / Dropbox status, *Always keep on this device* and *Free up space* with a real
    sync root, and libraries with folders on other drives
  - several windows: Ctrl+N, dragging a tab out (also onto another monitor), and which window
    keeps the session
  - the search index run as administrator: built from the NTFS master file table and kept up to
    date by the change journal (`cargo test -p ox-core -- --ignored mft` checks it against C:\)
  - the disk usage treemap, the duplicates and empty folders pages, and the search suggestions
- ⏳ **Copy engine benchmarks** against Explorer, robocopy and TeraCopy (many small files, one
  huge file, SSD → USB stick, network share), with the numbers in the README. The
  `copy_bench` example measures the engine, robocopy and xcopy; Explorer and TeraCopy by hand
- ⏳ Code-signed releases, so SmartScreen doesn't warn on the first start

## Phase 8 — Safer file operations

- 💡 **Mirror / sync** a folder to another (one way, with a preview of what will change)
- 💡 Real Windows notifications when transfers are done (needs an AppUserModelID from the
  installer's Start menu shortcut; today it's a message in the app and a flashing taskbar button)
- 💡 Keep the undo history across restarts

## Phase 9 — Views and browsing

- 💡 Sort by the columns read from the files (dimensions, length, date taken, authors, version)
- 💡 Libraries: refresh by themselves when files change outside Oxplorer, and search inside a
  library from the search box
- 💡 The Gallery grouped by month
- 💡 A *Browse* button and path completion in the *New link* dialog

## Phase 10 — Search and disk usage

- 💡 A small **helper service**, so the index is read from the NTFS master file table and follows
  the change journal without running Oxplorer as administrator
- 💡 Catch up on what changed while Oxplorer was closed (from the change journal) instead of
  rebuilding the index
- 💡 Duplicates: hash on several threads, and an optional byte-by-byte compare
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
