# Oxplorer

A fast file explorer for Windows, written in Rust.

Oxplorer looks and works like Windows Explorer, but adds tab groups, a
quick-access start page, much faster search and its own copy engine. When you
copy or move files you don't get the slow Windows copy dialog. A multi-threaded
Rust engine does the work and shows progress in a panel inside the window.

![Details view with tab groups and a paused transfer](docs/screenshots/details-dark.png)

> **Status: early development (Phase 1, see [ROADMAP.md](ROADMAP.md)).**
> So far it has been developed and tested on Linux. The Windows-specific
> code (`CopyFileExW`, Explorer clipboard sharing, hidden attributes) compiles
> for Windows but has not yet been tested on a real Windows machine.

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

**Tabs and tab groups**
- Tabs in the header bar. Drag them to reorder, middle-click to close.
- Chrome-style **tab groups** with names and colours. Click a group chip to collapse or expand it.
- Drag files onto a tab to switch to it, then drop them inside
- Optionally restores your tabs and groups on startup

**Quick access**
- **Home start page** with pinned favorites, drives with usage bars, recent folders and recent files
- **Sidebar** with Home, Favorites, This PC (drives) and Recent sections
- Pin or unpin any folder from its context menu; drop files onto a favorite to copy or move them there

**Custom copy engine** (`ox-core::transfer`)
- Never opens the Windows shell copy dialog
- Several files are copied in parallel. On Windows each file goes through
  `CopyFileExW` (the kernel fast path), and very large files use unbuffered I/O.
- Moves on the same drive are a single rename and finish instantly. Across drives it copies, then deletes.
- Asks **once per job** when files already exist (Replace / Skip / Keep both / Replace if newer), not once per file
- Pause, resume, cancel, live speed and time left, a job queue, and a per-file error list
- Shares the clipboard with Windows Explorer in both directions (CF_HDROP + Preferred DropEffect).
  Copy in Explorer and paste in Oxplorer, and the paste still runs through the fast engine.

**Search** (`ox-core::search`)
- Typing in the search box filters the current folder instantly
- **Enter** runs a deep, multi-threaded search through all sub-folders. It uses
  the same parallel directory walker as ripgrep, and results appear while it runs.
- Query syntax:

  | Query | Meaning |
  |---|---|
  | `report` / `"annual report"` | name contains the word / phrase |
  | `*.rs`, `img_??` | wildcards |
  | `-tmp` | exclude |
  | `ext:pdf,docx` | by extension |
  | `type:folder`, `type:file` | only folders / files |
  | `size:>10mb`, `size:<1kb` | by size |
  | `modified:7d`, `modified:today` | recently changed |
  | `content:"TODO"` | text inside files |

**Settings page**
- Appearance (system/light/dark theme, accent colour, UI scale, compact rows)
- Browsing, startup, sidebar, copy engine and search options, plus a list of keyboard shortcuts
- Saved as TOML in `%APPDATA%\Oxplorer\`

| Home / Quick access | Grid view (light) | Settings |
|---|---|---|
| ![Home](docs/screenshots/home-dark.png) | ![Grid](docs/screenshots/grid-light.png) | ![Settings](docs/screenshots/settings-light.png) |

*The screenshots were taken on Linux during development, which is why the drives
show as mount points. On Windows they show as `Local Disk (C:)` and so on.*

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
**Actions** tab, under the artifact `oxplorer-windows-x64`.

Run the tests:

```bash
cargo test --workspace
```

## Project layout

```
crates/
  ox-core/            UI-independent logic (unit tested)
    transfer/         copy/move engine: scan → conflict decision → parallel copy
    search.rs         parallel recursive search + query language
    entry.rs          directory listing, file kinds, sorting
    clipboard.rs      file clipboard, shared with Windows Explorer
    places.rs         favorites, recent items, known folders, drives
    settings.rs       settings (TOML)
    fsops.rs          create / rename / delete / shell helpers
    watcher.rs        auto-refresh when a folder changes
  ox-app/             the egui/eframe desktop app ("oxplorer" binary)
    app.rs            state + action dispatcher + keyboard shortcuts
    tabs.rs           tabs, tab groups, history, session restore
    ui/               header (tabs, address, search, commands), sidebar,
                      home page, file view, settings page, transfers, dialogs
```

The UI is [egui](https://github.com/emilk/egui) (immediate mode, GPU
rendered). That keeps the app a single small native `.exe` with no web runtime.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| Ctrl+T / Ctrl+W | New tab / close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Alt+← / Alt+→ / Alt+↑ / Backspace | Back / forward / up |
| Ctrl+L | Edit address |
| Ctrl+F | Search |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste (works with Explorer too) |
| Ctrl+Shift+C | Copy path |
| F2 / Del / Shift+Del | Rename / delete / delete permanently |
| Ctrl+Shift+N | New folder |
| Alt+Enter | Properties |
| F5 | Refresh |
| Ctrl+, | Settings |

## Roadmap

See [ROADMAP.md](ROADMAP.md). Next up: real Windows shell icons and
thumbnails, the native context menu, drag & drop *out* to other apps, and an
"Everything"-style NTFS index for instant search across the whole PC.

## License

MIT
