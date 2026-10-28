# Oxplorer

A fast file explorer for Windows, written in Rust.

Oxplorer looks and works like Windows Explorer, but adds tab groups, a
quick-access start page, much faster search and its own copy engine. When you
copy or move files you don't get the slow Windows copy dialog. A multi-threaded
Rust engine does the work and shows progress in a panel inside the window.

![Details view with tab groups and a paused transfer](docs/screenshots/details-dark.png)

> **Status: early development (Phases 1–4 done, Phase 5 in progress, see [ROADMAP.md](ROADMAP.md)).**
> The app is developed and tested by hand on Linux. The automated tests,
> including the `CopyFileExW` copy path, pass on Windows in CI. The
> Windows-only UI features (Shell icons, context menu, Properties, Explorer
> clipboard sharing) build for Windows but have not been tried by hand yet.

## Features

**Explorer basics**
- Details and grid views, sortable columns, natural sort order (`file2` before `file10`)
- Multi-select with Ctrl/Shift, Ctrl+A, arrow keys, Home/End, and type-to-jump
- Back/forward/up (mouse side buttons work too) and a breadcrumb address bar.
  Click a `›` to jump into a sub-folder. Click the empty part of the bar to type
  a path; `%USERPROFILE%` and `~` are expanded.
- New folder or file, rename (F2), delete to the Recycle Bin, permanent delete (Shift+Del), properties with folder size
- The view refreshes by itself when files change on disk
- Open in Terminal, Show in Windows Explorer, Copy path
- **Recycle Bin view**: browse deleted items with their original location, restore them,
  delete them permanently or empty the bin. Drop files onto it in the sidebar to delete them.

**Windows integration**
- The same file icons as Explorer (loaded in the background, cached per file type)
- **Show more options** (or Shift+F10) opens the real Explorer context menu, so
  entries from 7-Zip, VS Code, Git and other tools are there
- Native Properties dialog and "Open with…"
- App icon, per-monitor DPI awareness and long-path support through the app manifest

**Tabs and tab groups**
- Tabs in the header bar. Drag them to reorder, middle-click to close.
- Chrome-style **tab groups** with names and colours. Click a group chip to collapse or expand it.
- Drag files onto a tab to switch to it, then drop them inside
- Optionally restores your tabs and groups on startup

**Quick access**
- **Home start page** with pinned favorites, drives with usage bars, recent folders and recent files
- **Sidebar** with Home, Favorites, This PC (drives), Recent, Tags and Saved searches.
  Recycle Bin and Settings are pinned at the bottom.
- Pin or unpin any folder from its context menu; drop files onto a favorite to copy or move them there

**Custom copy engine** (`ox-core::transfer`)
- Never opens the Windows shell copy dialog
- Several files are copied in parallel. On Windows each file goes through
  `CopyFileExW` (the kernel fast path), and very large files use unbuffered I/O.
- Moves on the same drive are a single rename and finish instantly. Across drives it copies, then deletes.
- Asks **once per job** when files already exist (Replace / Skip / Keep both / Replace if newer), not once per file
- Pause, resume, cancel, live speed and time left, and a per-file error list
- **Per-disk queue**: copies on the same drive wait for each other, copies between different drives run in parallel
- **Auto worker count**: one file at a time on HDDs (no seek storms), 8 in parallel on SSDs
- Optional **verification** (xxh3 hash of original and copy). Moves only delete originals that were copied correctly.
- **Retry failed files** with one click, an optional **speed limit**, and a **transfer history** log
- Shares the clipboard with Windows Explorer in both directions (CF_HDROP + Preferred DropEffect).
  Copy in Explorer and paste in Oxplorer, and the paste still runs through the fast engine.

**Search** (`ox-core::search`, `ox-core::index`)
- Typing in the search box filters the current folder instantly
- A **file-name index** of your drives makes searching everything instant: start
  typing on the Home page and results appear as you type (280k files are
  searched in milliseconds). The index is saved between runs and refreshed in the
  background.
- **Enter** searches all sub-folders: from the index when it covers the folder,
  otherwise with a multi-threaded walker (the same parallel walker as ripgrep)
- **Filter chips** for kind, date and size, and **saved searches** in the sidebar
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

**Power tools**
- **Preview pane** (Alt+P): images, code with syntax highlighting and line numbers, the first
  page of PDFs (on Windows), song and video details with cover art, folder contents and
  multi-selection totals. Loads in the background, so large files never block the window.
- **Bulk rename** (F2 with several items selected): build names from a pattern
  (`{name}`, `{ext}`, `{n:3}`, `{date}`, `{parent}`), find & replace with text or regex,
  change case, and see every new name before anything changes. Name clashes are
  caught in the preview, and if one rename fails the others are rolled back.
- **Command palette** (Ctrl+Shift+P or F1): type a few letters to run any command or
  jump to a favorite, drive, recent folder or saved search
- **Split view** (Ctrl+\\ or the ◫ button): two folders side by side. Click a pane or press
  F6 to work in it, and drag files from one pane to the other.
- **Archives as folders**: double-click a `.zip`, `.tar`, `.tar.gz` or `.tgz` file to browse it,
  preview files inside it, and drag or copy items out to extract them. Right-click an archive for
  *Extract here* / *Extract to "name"*, or any selection for *Compress to ZIP*.
- **Git status**: inside a Git repository a *Git* column shows what changed (M, A, D, R, ?, I).
  Folders show the most important change inside them, and the status bar shows the branch
  (`Git: main ↑2 · 3 changes`). Needs Git installed.
- **Folder sizes** in the Size column, calculated in the background and cached. Sort by
  size to find what fills your disk.
- **Colour tags**: right-click → Tags to mark files and folders. Tagged items get coloured
  dots and show up under *Tags* in the sidebar. Rename a tag (e.g. Red → Urgent) from its
  right-click menu. Tags follow items you rename or move in Oxplorer.

**Settings page**
- Appearance (system/light/dark theme, accent colour, UI scale, compact rows)
- Browsing, startup, sidebar, copy engine and search options, plus a list of keyboard shortcuts
- Saved as TOML in `%APPDATA%\Oxplorer\`

| Home / Quick access | Grid view (light) |
|---|---|
| ![Home](docs/screenshots/home-dark.png) | ![Grid](docs/screenshots/grid-light.png) |
| **Recycle Bin** | **Settings** |
| ![Recycle Bin](docs/screenshots/recycle-bin-dark.png) | ![Settings](docs/screenshots/settings-light.png) |
| **Search as you type, filter chips, saved searches** | **Preview pane** |
| ![Search](docs/screenshots/search-dark.png) | ![Preview](docs/screenshots/preview-dark.png) |
| **Colour tags and folder sizes** | **Split view** |
| ![Tags and folder sizes](docs/screenshots/tags-sizes-dark.png) | ![Split view](docs/screenshots/split-dark.png) |

*The screenshots were taken on Linux during development, which is why the drives
show as mount points and the built-in icons are used. On Windows the drives show
as `Local Disk (C:)` and so on, with the real Windows file icons.*

## Building

You need the latest stable [Rust](https://rustup.rs/) toolchain.

```bash
cargo run --release
```

The binary is `target/release/oxplorer.exe`. To open a folder directly, pass its path:

```bash
oxplorer.exe "C:\Users\me\Documents"
```

CI builds a Windows binary on every push to `main`. Download it from the
**Actions** tab, under the artifact `oxplorer-windows-x64`. Pushing a version
tag (e.g. `v0.2.0`) publishes a portable zip on the **Releases** page.

Run the tests:

```bash
cargo test --workspace
```

## Project layout

```
crates/
  ox-core/            UI-independent logic (unit tested)
    transfer/         copy/move engine: scan → conflict decision → parallel copy
    search.rs         query language, parallel walker, indexed search
    index.rs          persistent file-name index (build, query, cache)
    rename.rs         bulk rename: patterns, preview, safe apply with rollback
    fuzzy.rs          fuzzy matching for the command palette
    dirsize.rs        background folder sizes with a shared cache
    archive.rs        zip/tar browsing, extraction and zip compression
    git.rs            Git status via the git command line
    pdf.rs            PDF first-page previews (Windows.Data.Pdf)
    tags.rs           colour tags (stored in tags.toml)
    disk.rs           volume ids, HDD/SSD detection
    entry.rs          directory listing, file kinds, sorting
    clipboard.rs      file clipboard, shared with Windows Explorer
    shell.rs          Windows Shell: icons, native context menu, Properties, Open with
    recycle.rs        Recycle Bin: list, restore, purge
    places.rs         favorites, recent items, known folders, drives
    settings.rs       settings (TOML)
    fsops.rs          create / rename / delete / shell helpers
    watcher.rs        auto-refresh when a folder changes
  ox-app/             the egui/eframe desktop app ("oxplorer" binary)
    app.rs            state + action dispatcher + keyboard shortcuts
    tabs.rs           tabs, tab groups, history, session restore
    shortcuts.rs      keyboard shortcuts: commands, defaults, the user's keys
    ui/               header (tabs, address, search, commands), sidebar,
                      home page, file view, preview pane, command palette,
                      settings page, transfers, dialogs
```

The UI is [egui](https://github.com/emilk/egui) (immediate mode, GPU
rendered). That keeps the app a single small native `.exe` with no web runtime.

## Keyboard shortcuts

All of these can be changed in **Settings → Shortcuts**: click a shortcut and press the new keys.
Menus, tooltips and the command palette always show the keys in use.

| Keys | Action |
|---|---|
| Ctrl+T / Ctrl+W | New tab / close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Alt+← / Alt+→ / Alt+↑ / Backspace | Back / forward / up |
| Ctrl+L | Edit address |
| Ctrl+F | Search |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste (works with Explorer too) |
| Ctrl+Shift+C | Copy path |
| F2 / Del / Shift+Del | Rename (bulk rename for several items) / delete / delete permanently |
| Ctrl+Shift+N | New folder |
| Alt+Enter | Properties |
| Shift+F10 | Windows context menu ("Show more options") |
| F5 | Refresh |
| Ctrl+, | Settings |
| Ctrl+Shift+P / F1 | Command palette |
| Alt+P | Preview pane |
| Ctrl+\\ / F6 | Split view / switch pane |

## Roadmap

See [ROADMAP.md](ROADMAP.md). Next up: the rest of Phase 5 (terminal
panel, 7z/rar), then Phase 6 polish with an
in-app Help page.

## License

MIT
