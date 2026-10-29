# Oxplorer

A fast file explorer for Windows, written in Rust.

Oxplorer looks and works like Windows Explorer and adds what Explorer is missing:
- tab groups and a split view
- instant search over all your drives
- its own copy engine instead of the slow Windows copy dialog
- a preview pane that plays media and shows Markdown and Git history
- a built-in terminal
- tools for developers

It is a single native `.exe` with no web runtime.

![Details view with tab groups, a selection and a running transfer](docs/screenshots/details-dark.png)

## Download

**[Latest release](https://github.com/Flory-1/oxplorer/releases/latest)** for Windows 10 (1809)
and Windows 11, 64-bit:

- **Installer** (`oxplorer-<version>-setup-x64.exe`): installs for your user, no admin rights
  needed. It adds a Start menu entry and an optional desktop icon, and can make folders open
  in Oxplorer.
- **Portable** (`oxplorer-<version>-windows-x64.zip`): unzip and run `oxplorer.exe`
- `.sha256` files to check the downloads

What changed in each version: [CHANGELOG.md](CHANGELOG.md).

> Oxplorer is developed on Linux; its automated tests, including the Windows copy path, run
> on Windows in CI. Some Windows-only parts haven't been tried by hand on a real Windows machine
> yet: Shell icons, thumbnails and context menu, drag out, network locations, the window
> backdrop, opening folders by default, the media player and the tabs in the title bar.
> Please [report anything that doesn't work](https://github.com/Flory-1/oxplorer/issues).
> The installer isn't code-signed yet, so SmartScreen may warn on the first start
> (*More info → Run anyway*).

## Features

### Browsing

- **Details and grid views** with sortable columns and natural sort order (`file2` before `file10`)
- **Choose your columns**: show, hide, move and resize them, with extra ones such as dimensions,
  length, date taken, authors, file version and attributes
- **Each folder remembers its view**: sort order, view mode, columns and grouping (e.g.
  *Downloads* by date, *Pictures* as a grid)
- **Group by** type, date, size or tag, with groups you can fold away
- **Grid zoom** with Ctrl+mouse wheel or a slider, from small icons to large thumbnails
- **Quick Look**: Space shows the selected file large over the window; the arrow keys go
  through the folder
- **Flat view** (Ctrl+Shift+F): every file of a folder and its sub-folders in one list
- Multi-select with Ctrl/Shift, Ctrl+A, arrow keys, Home/End, and **type-to-jump**;
  **select by pattern** (`*.jpg; *.png`), invert the selection or select all of the same type
- Back, forward and up (the mouse side buttons work too) and a **breadcrumb address bar**:
  click a `›` to jump into a sub-folder, or click the empty part to type a path
  (`%USERPROFILE%` and `~` are expanded)
- New folder or file, rename (F2), delete to the Recycle Bin, delete permanently (Shift+Del).
  Add **your own file types to ➕ New**, such as *Python script* or *Markdown document*, each
  with a file name and content to start with (`{name}` and `{date}` are filled in).
- The view refreshes by itself when files change on disk
- **Folder sizes** in the Size column, calculated in the background and cached. Sort by size
  to find what fills your disk.
- **Disk usage view**: a treemap of a folder or drive, with the largest items next to it
- **Find duplicates and empty folders** in a folder, with a list to review before they go to
  the Recycle Bin
- **Colour tags** with your own names (e.g. Red → *Urgent*): coloured dots in the file view, a
  *Tags* section in the sidebar, and tags follow the items you rename or move
- **Archives as folders**: open `.zip`, `.7z`, `.tar`, `.tar.gz` and `.tgz` files to browse
  them, preview what's inside and drag or copy items out. *Extract here*, *Extract to "name"*
  and *Compress to ZIP* are in the right-click menu.
- **Picture tools**: rotate, flip, convert (PNG, JPG, WebP) and resize one or many pictures,
  with undo
- **Recycle Bin view**: deleted items with their original location; restore, delete
  permanently or empty the bin, and drop files onto it to delete them
- **Undo and redo** (Ctrl+Z / Ctrl+Y) for renames, moves, copies, new items and deletes to the
  Recycle Bin; Oxplorer checks first that nothing changed since
- **Links**: create symbolic links, junctions and hard links from ➕ New or by dragging with
  Ctrl+Shift; links show where they point

### Tabs, split view and quick access

- **Tabs** in the title bar: drag to reorder, middle-click to close, drop files onto a tab
  to open it
- Chrome-style **tab groups** with names and colours; click a group's chip to collapse it
- **Pinned tabs** stay small and in front, and **named sessions** save a set of tabs and
  groups to open again later
- **More windows**: Ctrl+N, or drag a tab out of the window into one of its own
- **Split view** (Ctrl+\\): two folders side by side. F6 or a click switches the pane, and
  you can drag files between the panes.
- **Compare folders** (Ctrl+Shift+D) in the split view: what only one side has, what's newer
  and what differs is marked, and missing or newer files are copied over in one go
- **Home tab**, always the first tab: favorites, drives with usage bars, and recent folders and
  files. What you open from it opens in a new tab.
- **Sidebar** with Favorites, This PC, Network, Recent, Tags, Saved searches and **Libraries**:
  Windows' libraries with all their folders in one view, and a **Gallery** of every picture and
  video, newest first
- **Command palette** (Ctrl+Shift+P): type a few letters to run any command or jump to a
  favorite, drive, recent folder, tag or saved search
- Your tabs and groups can be restored at startup, and the window opens where you left it

### Preview pane

Alt+P shows the preview on the right, or in its own window that you can move and resize.
The **Info** tab has the item's details; the other tabs show its content. Everything loads in
the background, so large files never block the window.

- **Pictures**, **text and code** with syntax highlighting, the **first page of PDFs**
- **More picture formats**: SVG, HEIC/HEIF, AVIF, JPEG XL and camera RAW (through Windows'
  codecs), animated GIF and WebP, and the photo's details (camera, exposure, location)
- **Office documents and mails** through the same preview handlers Explorer uses
- **Data files**: CSV/TSV as a table, JSON/XML/YAML/TOML as a tree, SQLite tables, and a **hex
  view** for every file
- **Player**: audio and video play right in the preview with Windows' own media engine, so
  every format Windows can play works. It has a seek bar, 10 s back/forward, repeat, shuffle,
  speed, a queue, playlists and subtitles, and keeps playing in a mini player while you browse.
- **Markdown** files formatted, with tables, task lists, highlighted code and the pictures
  next to them
- **Git**: for folders in a repository, the history of all branches as a commit graph with
  branches, tags, authors and dates
- **Fonts**: a sample at several sizes and a grid of every character (click one to copy it)
- Folders and archives: what's inside and how big it is

### Search

- Typing in the search box **filters the current folder** instantly
- A **file-name index** of your drives finds files anywhere as you type: 280,000 files are
  searched in milliseconds. The index is kept between runs and follows file changes as they
  happen. Run as administrator, NTFS drives are read straight from the master file table and
  followed through the change journal.
- **Enter** searches all sub-folders: from the index when it covers the folder, otherwise with
  a multi-threaded walker
- **Filter chips** for kind, date, size and tags, and **saved searches** in the sidebar
- **Search history and suggestions**: earlier searches and completions of the query syntax
  under the search box
- **Search inside archives**: also find the files in zip, 7z and tar archives by name
- Query syntax:

  | Query | Meaning |
  |---|---|
  | `report` / `"annual report"` | name contains the word / phrase |
  | `*.rs`, `img_??` | wildcards |
  | `/^IMG_\d+\.jpe?g$/` | regular expression (also `regex:…`) |
  | `-tmp` | exclude |
  | `ext:pdf,docx` | by extension |
  | `type:folder`, `type:file` | only folders / files |
  | `kind:image,video` | by kind (image, video, audio, document, archive, code, app) |
  | `size:>10mb`, `size:<1kb` | by size |
  | `modified:7d`, `modified:today` | recently changed |
  | `content:"TODO"` | text inside files |
  | `tag:red,blue` | by colour tag (also your tag names) |
  | `archives:yes` | also inside zip, 7z and tar archives |

### Copy engine

Copying and moving never opens the Windows copy dialog. Oxplorer's own engine
(`ox-core::transfer`) does the work and shows it in a panel inside the window.

- **Several files at once**; on Windows each file goes through `CopyFileExW` (the kernel's
  fast path), and very large files use unbuffered I/O
- Moves on the same drive are a rename and finish instantly
- Asks **once per job** when files already exist (replace, skip, keep both, replace if newer),
  not once per file
- Pause, resume and cancel, with live speed, time left and a list of failed files
- **Per-disk queue** (jobs on the same drive wait for each other) and an **automatic worker
  count**: one file at a time on HDDs, 8 in parallel on SSDs
- Optional **verification** (xxh3 hash of original and copy); moves only delete originals that
  were copied correctly
- **Retry failed files** with one click, an optional **speed limit** and a **transfer history**
- **Asks about errors** while it runs (retry / skip / skip all) and **resumes interrupted
  transfers** after a crash, sleep or a pulled USB stick
- A **queue**: hold, reorder or run jobs after the current ones, and when all are done, show a
  notification or put the computer to sleep
- Optionally keeps **permissions**, **alternate data streams** and **folder dates**
- The **clipboard is shared with Explorer** both ways: copy in Explorer, paste in Oxplorer,
  and the paste still runs through the fast engine

### Windows integration

- **Tabs in the title bar** as in Windows 11's Explorer, with Windows 11 style minimize,
  maximize and close buttons. The window keeps its frame, shadow, snapping and the snap
  layouts on the maximize button.
- The same **file icons and thumbnails** as Explorer
- **Show more options** (Shift+F10) opens the real Explorer context menu, so entries from
  7-Zip, VS Code and other tools are there. Properties and *Open with* use the Windows dialogs.
- **Drag files out** of Oxplorer into other programs
- **OneDrive and Dropbox**: online only, on this device or always kept, as a badge and a Status
  column, with *Always keep on this device* and *Free up space*. Online-only files aren't
  downloaded just to show a thumbnail or preview.
- **Network locations**: type `\\server` to see its shares, browse them like any folder, mapped
  drives under *This PC*, and *Map network drive…*
- **Mica or Acrylic** window backdrop on Windows 11, rounded corners, dark or light title bar
- **Open folders in Oxplorer** instead of Explorer: folders and drives opened from the desktop
  or other programs, and Win+E. Turning it off gives them back to Explorer.
- Per-monitor DPI awareness and long paths

### Terminal

- **Terminal panel** (Ctrl+`): a real terminal under the file view, running PowerShell or any
  shell you set, with colours, scrollback and full-screen programs
- It opens in the current folder, or follows every folder you open (Settings → Browsing)

### For developers

- **Open in editor** (Ctrl+Shift+E, right-click, command palette) for files and folders.
  It uses VS Code by default, or any editor command, with presets for Cursor, Zed,
  Sublime Text and Notepad++.
- **Copy path as**: with forward slashes, in quotes, relative to the Git repository, as a
  `file://` URL, or just the name
- **Git status column** (M, A, D, R, ?, I; folders show the most important change inside
  them) and the branch with ahead/behind in the status bar (needs Git installed)
- Right-click → **Git**: open a file or folder on GitHub, GitLab or Bitbucket, copy the branch
  name. The Info tab shows the last commit that changed the item.
- Text files show their **encoding**, **line endings** and **indentation**; **SHA-256 and MD5
  checksums** of any file on request
- **Bulk rename** (F2 on several items): names from a pattern (`{name}`, `{ext}`, `{n:3}`,
  `{date}`, `{parent}`), find & replace with text or regex, case changes, and a preview of
  every new name. If one rename fails, the others are rolled back.

### Make it yours

- **Colour themes**, one for light and one for dark mode: Dark, Light, Nord, Dracula, Solarized
  Light, High contrast, or your own. Change any colour, **export** a theme as a small `.toml`
  file, and right-click a theme file → *Use as theme* to install one.
- Accent colour, compact rows, thumbnails, single-click to open, date format, and more
- Clean [Lucide](https://lucide.dev) line icons throughout, the same on every computer
- **Keyboard shortcuts** you can change: click one in Settings → Shortcuts and press the new keys
- **English and German**. Oxplorer follows the Windows display language or the one you
  choose. Everything is translated, including Help, messages and key names (*Strg+C*).
- **Help page** (F1): getting started, every feature, the search syntax, your current
  shortcuts and common questions, with a filter
- Settings are saved as TOML in `%APPDATA%\Oxplorer\`; problems and crashes are written to a
  log there (Help → *Open log folder*), to attach to a bug report
- A quiet **update check**: a note on the Home page when a newer release is on GitHub (can be
  turned off in Settings → Startup)

## Screenshots

| Home / Quick access | Grid view with thumbnails (light) |
|---|---|
| ![Home](docs/screenshots/home-dark.png) | ![Grid with thumbnails](docs/screenshots/grid-light.png) |
| **Search: deep search from the index, filter chips** | **Preview pane** |
| ![Search](docs/screenshots/search-dark.png) | ![Preview](docs/screenshots/preview-dark.png) |
| **Git status, colour tags and folder sizes** | **Split view** |
| ![Git, tags and folder sizes](docs/screenshots/tags-sizes-dark.png) | ![Split view](docs/screenshots/split-dark.png) |
| **Git graph in the preview** | **Markdown preview (light)** |
| ![Git graph](docs/screenshots/git-graph-dark.png) | ![Markdown preview](docs/screenshots/markdown-light.png) |
| **Font preview with the Glyphs grid** | **Nord theme and code preview** |
| ![Font glyphs](docs/screenshots/fonts-dark.png) | ![Nord theme](docs/screenshots/theme-nord.png) |
| **Recycle Bin** | **Settings (light)** |
| ![Recycle Bin](docs/screenshots/recycle-bin-dark.png) | ![Settings](docs/screenshots/settings-light.png) |
| **Help page (F1)** | **German interface** |
| ![Help](docs/screenshots/help-dark.png) | ![German](docs/screenshots/german-dark.png) |

*The screenshots were taken on Linux, so the drives show as mount points and the built-in
icons are used. On Windows the drives show as `Local Disk (C:)` and so on, with the real
Windows file icons and thumbnails.*

## Keyboard shortcuts

All of these can be changed in **Settings → Shortcuts**. Menus, tooltips and the command
palette always show the keys in use.

| Keys | Action |
|---|---|
| Ctrl+T / Ctrl+W | New tab / close tab |
| Ctrl+N | New window |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Alt+← / Alt+→ / Alt+↑ or Backspace | Back / forward / up |
| Ctrl+L or Alt+D | Edit address |
| Ctrl+F or Ctrl+E | Search |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste (works with Explorer too) |
| Ctrl+Z / Ctrl+Y or Ctrl+Shift+Z | Undo / redo |
| Ctrl+Shift+C | Copy path |
| F2 | Rename (bulk rename for several items) |
| Del / Shift+Del | Delete / delete permanently |
| Ctrl+Shift+N | New folder |
| Ctrl+A | Select all |
| Ctrl+Shift+A / Ctrl+Shift+I | Select by pattern / invert selection |
| Space | Quick Look |
| Ctrl+Shift+F | Flat view |
| Alt+Enter | Properties |
| Shift+F10 | Windows context menu ("Show more options") |
| F5 or Ctrl+R | Refresh |
| Ctrl+, | Settings |
| Ctrl+Shift+P | Command palette |
| F1 | Help |
| Alt+P | Preview pane |
| Ctrl+\\ / F6 | Split view / switch pane |
| Ctrl+Shift+D | Compare the folders of the split view |
| Ctrl+` | Terminal panel |
| Ctrl+Shift+E | Open in editor |

## Building

You need the latest stable [Rust](https://rustup.rs/) toolchain.

```bash
cargo run --release
```

The binary is `target/release/oxplorer.exe`. To open a folder directly, pass its path:

```bash
oxplorer.exe "C:\Users\me\Documents"
```

Run the tests (CI runs them on Windows and Linux, together with `cargo fmt --check` and
`cargo clippy -D warnings`):

```bash
cargo test --workspace
```

CI builds a Windows binary and installer on every push to `main`; download them from the
**Actions** tab (artifact `oxplorer-windows-x64`). The installer is built with
[Inno Setup](https://jrsoftware.org/isinfo.php) from `installer/oxplorer.iss`, and
`installer/build.ps1` builds it together with the winget manifests.

### Copy benchmark

A local tool that measures the copy engine (CI and `cargo test` only run its unit tests):

```bash
cargo run --release -p ox-core --example copy_bench -- --dest D:\bench --compare robocopy,std
```

It generates its test data in a temp folder (or under `--src`), copies it to `--dest` (another
drive, a USB stick or a network share; default: the temp folder) and deletes everything
afterwards. Scenarios (`--scenarios small,large,mixed`):

- **small**: 10 000 files of 4 KB in nested folders (`--small-count`, `--small-size`)
- **large**: one 1 GB file of random data (`--large-size`)
- **mixed**: a fifth of the small files with 1–256 KB each, 16 medium files and one file a
  quarter of the large size

Each tool runs `--runs 3` times; the table shows the median, MB/s and files/s. `--compare`
adds robocopy (`/E /MT:8`), xcopy and a sequential `std::fs::copy` loop, `--move` adds a move
with the engine. The OS file cache makes repeated runs faster than a cold copy; a `--large-size`
larger than the RAM avoids that.

Before a release, check that the engine didn't get slower: save a baseline once with
`--save-baseline bench.toml`, later run the same command with `--baseline bench.toml`. The
exit code is 1 if an Oxplorer result is more than 15 % slower (`--max-slowdown 1.15`). Use the
same machine, destination and sizes; results under 50 ms aren't checked. `--help` lists all
options.

### Releases

1. Set the new version in `Cargo.toml`, `crates/ox-app/assets/oxplorer.rc` and
   `crates/ox-app/assets/oxplorer.manifest` (a test fails if they differ).
2. Add a `## [x.y.z] - date` section to `CHANGELOG.md`; it becomes the release notes.
3. Merge to `main`, then tag that commit `vx.y.z` and push the tag. The release workflow
   refuses a tag that doesn't match the version.

The release workflow publishes:
- the installer
- the portable zip
- their `.sha256` files
- `oxplorer-<version>-winget.zip`: winget manifests, ready to submit to
  [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) (then
  `winget install Flory-1.Oxplorer`)

### Translations

The English text is the key: wrap texts people see in `tr("…")` (`ox_core::i18n::t`), or
`ox_core::trf!("{n} selected", n = count)` for texts with values, and add the German version to
`crates/ox-core/src/i18n_de.rs`. A test lists every wrapped text without a German version, so
a missing translation fails CI.

## Project layout

```
crates/
  ox-core/            UI-independent logic (unit tested)
    transfer/         copy/move engine: scan → conflict decision → parallel copy,
                      verification, speed limit, history, queue, error questions,
                      resume journal, permissions/streams/folder dates
    search.rs         query language, parallel walker, indexed search
    index.rs          persistent file-name index (build, query, cache)
    rename.rs         bulk rename: patterns, preview, safe apply with rollback
    archive.rs        zip/7z/tar browsing, extraction and zip compression
    git.rs            Git status, log and commit graph layout, remotes
    player.rs         audio/video playback (Windows.Media.Playback)
    pdf.rs            PDF first-page previews (Windows.Data.Pdf)
    textinfo.rs       encoding, line endings, indentation, checksums
    pathfmt.rs        "Copy path as" formats
    dirsize.rs        background folder sizes with a shared cache
    fuzzy.rs          fuzzy matching for the command palette
    tags.rs           colour tags (stored in tags.toml)
    themes.rs         colour themes (built-in + .toml files)
    titlebar.rs       tabs in the title bar: window subclass and hit testing
    shell.rs          Windows Shell: icons, thumbnails, native context menu,
                      Properties, Open with, drag out, window backdrop
    network.rs        \\server shares, mapped network drives
    default_app.rs    "open folders in Oxplorer" (registry, HKCU)
    recycle.rs        Recycle Bin: list, restore, purge
    undo.rs           undo/redo history of file operations, with safety checks
    compare.rs        compare two folders (size and date, or contents)
    clipboard.rs      file clipboard, shared with Windows Explorer
    disk.rs           volume ids, HDD/SSD detection
    entry.rs          directory listing, file kinds, sorting
    places.rs         favorites, recent items, known folders, drives
    settings.rs       settings (TOML)
    logfile.rs        log file and crash reports
    update.rs         update check (newest release on GitHub, via WinHTTP)
    i18n.rs           translations (English text as key); i18n_de.rs: German
    fsops.rs          create / rename / delete, terminal and editor helpers
    watcher.rs        auto-refresh when a folder changes
  ox-app/             the egui/eframe desktop app ("oxplorer" binary)
    app.rs            state, action dispatcher, keyboard handling, window style
    tabs.rs           tabs, tab groups, history, session restore
    shortcuts.rs      keyboard shortcuts: commands, defaults, the user's keys
    terminal.rs       terminal panel (alacritty_terminal + egui)
    thumbs.rs         thumbnail cache for the grid view
    icons.rs          file icons (Windows Shell or built-in)
    ui/               header (tabs, caption buttons, address, search, commands),
                      sidebar, home page, file view, preview pane, command palette,
                      settings and help pages, transfers, dialogs
installer/            Inno Setup script, winget manifest templates, build script
tools/icon-font/      builds the icon font (crates/ox-app/assets/icons.ttf) from Lucide
```

The UI is [egui](https://github.com/emilk/egui) (immediate mode, GPU rendered), which keeps
Oxplorer a single small native `.exe`.

## License

MIT. The icons are from [Lucide](https://lucide.dev) (ISC licence, see
`crates/ox-app/assets/LICENSE-lucide.txt`); `tools/icon-font` builds the icon font from them.
