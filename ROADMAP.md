# Oxplorer Roadmap

Oxplorer is a fast Windows file explorer written in Rust. It has tabs and tab
groups, a quick-access start page, a custom copy engine (no slow native copy
dialog) and fast parallel search.

The roadmap is split into phases. Each phase should end with a usable build.

Status legend: ✅ done · 🚧 in progress · ⏳ planned

---

## Phase 0 — Foundation ✅

- ✅ Cargo workspace: `ox-core` (logic, no GUI) + `ox-app` (egui/eframe UI)
- ✅ Project docs: README, ROADMAP, license
- ✅ CI (GitHub Actions): `cargo fmt`, `clippy`, tests on Windows and Linux, release build artifact for Windows

## Phase 1 — MVP Explorer 🚧

Goal: a daily-driver replacement for the basic Explorer tasks.

Everything marked ✅ below is implemented and was tested by hand on Linux. The
automated tests (copy engine, conflicts, search, settings, tabs) also pass on
Windows in CI. The phase is finished once the app has been tried by hand on Windows:

- ⏳ **Manual test on Windows 10/11** and fix what comes up: Explorer clipboard
  interop, hidden/system attributes, drive labels, junctions, long paths, high-DPI scaling
- ⏳ Benchmark the copy engine against Explorer on Windows (many small files, one huge file, SSD → USB)

**Browsing**
- ✅ Directory listing on a background thread (big folders don't freeze the UI)
- ✅ Details view (sortable columns) and grid view
- ✅ Natural sort order (`file2` before `file10`), folders first
- ✅ Multi-select (Ctrl/Shift click, Ctrl+A, keyboard navigation)
- ✅ Back / forward / up, breadcrumb address bar with edit mode
- ✅ Auto refresh when the folder changes on disk (file watcher)
- ✅ Hidden files / extensions toggles

**Layout**
- ✅ Header bar with tabs and **tab groups** (named and coloured, collapsible, Chrome-style)
- ✅ Sidebar: Home, Favorites, Drives, Recent folders
- ✅ Quick Access / Favorites **start page** (pinned folders, drives with usage bars, recent folders and files)
- ✅ Status bar (item count, selection size)
- ✅ Restore the last session (tabs and groups) on startup

**File operations**
- ✅ **Custom Rust copy engine**: parallel workers, `CopyFileExW` fast path on Windows, pause/resume/cancel, live speed and ETA
- ✅ Instant moves on the same volume (rename), copy + delete across volumes
- ✅ One conflict prompt per job (replace all / skip / keep both / replace if newer) instead of one per file
- ✅ Transfer panel instead of the native Windows copy dialog
- ✅ Clipboard works both ways with Windows Explorer (CF_HDROP + Preferred DropEffect)
- ✅ Drag & drop inside the app (onto folders, favorites, tabs); dropping files from other apps
- ✅ New folder / new file, rename, delete to Recycle Bin, permanent delete (Shift+Del)
- ✅ Copy path, open in terminal, show in Windows Explorer, "Copy to / Move to" favorites and open tabs

**Search**
- ✅ Instant filter of the current folder while typing
- ✅ Press Enter for a deep, multi-threaded recursive search (streams results)
- ✅ Query syntax: wildcards, `ext:`, `type:`, `size:`, `modified:`, `content:`, `-exclude`

**Settings page**
- ✅ Appearance (theme, accent colour, UI scale, compact rows)
- ✅ Browsing, startup, sidebar, copy engine and search options
- ✅ Saved as TOML in `%APPDATA%\Oxplorer`

## Phase 2 — Deep Windows integration 🚧

Items marked ✅ build and pass CI on Windows. The ones marked *(untested)* use
Windows-only APIs and still need a manual check on a real Windows machine.

- ✅ Real Windows file icons (`SHGetFileInfo` + system image list, loaded in the background and cached per file type) *(untested)*
- ✅ Native shell context menu under "Show more options" / Shift+F10 (`IContextMenu`, incl. owner-drawn submenus like "Send to"), so 7-Zip, VS Code and other entries work *(untested)*
- ✅ Native Properties dialog (`ShellExecuteEx` "properties") and "Open with…" (`SHOpenWithDialog`) *(untested)*
- ✅ Recycle Bin view: browse, restore, delete permanently, empty, drop files onto it
- ✅ App icon, version info and manifest (per-monitor DPI v2, long paths, modern common controls)
- ✅ Portable zip on GitHub Releases (push a `v*` tag)
- ⏳ Thumbnails for images and videos (`IShellItemImageFactory`)
- ⏳ OLE drag & drop **out** of the app into other programs (`IDropSource`)
- ⏳ Network locations: browse `\\server` shares, show mapped drives in the sidebar
- ⏳ Mica/Acrylic window backdrop, Windows 11 rounded corners
- ⏳ Option to "Open folders in Oxplorer" by default (Win+E / folder association)
- ⏳ Installer: MSIX / winget

## Phase 3 — Copy Engine 2.0 🚧

- ✅ Detect the drive type (HDD vs SSD) and pick the worker count: one file at a time on HDDs, 8 in parallel on SSDs ("Auto" setting). Linux via sysfs; Windows via the seek-penalty IOCTL *(untested)*
- ✅ Per-disk queue: jobs on the same drive run one after another, jobs on different drives run in parallel
- ✅ Optional verification after copying (xxh3-128). Moves only delete originals that were copied correctly
- ✅ Retry failed files with one click (each file goes back to its exact target path)
- ✅ Speed limit per transfer
- ✅ Transfer history log (`transfers.log`, with errors), opened from the transfer panel or Settings
- ⏳ Resume interrupted transfers (journal file)
- ⏳ Per-file error dialog while the job runs (retry / skip / skip all) instead of collecting errors
- ⏳ Keep ACLs (security descriptors), optional
- ⏳ Benchmarks against Explorer, robocopy and TeraCopy in CI

## Phase 4 — Search 2.0 🚧

- ✅ Persistent file-name index (`ox-core::index`) for all drives: ~24 bytes per item plus names, saved to the cache folder and loaded in milliseconds at startup, rebuilt in the background on start and every N minutes. Benchmark (Linux, warm disk cache): 280k items indexed in 0.24 s, queries in 2–12 ms
- ✅ Search-as-you-type across all indexed drives (Home page and search tabs)
- ✅ Searches inside a folder use the index too when it covers the folder; otherwise the parallel walker is used
- ✅ Faster content search: one compiled pattern per query, reused buffers, binary files skipped (memory-mapping was rejected: a file truncated during the search would crash the app)
- ✅ Filter chips (kind, date, size) and saved searches in the sidebar
- ✅ Regex mode (`/pattern/` or `regex:`), `kind:` filter (image, video, audio, document, archive, code, app)
- ✅ Index settings: folders, excluded names, refresh interval, rebuild now, live status
- ⏳ Live index updates from file system events (today the index is refreshed on a schedule)
- ⏳ NTFS MFT reader (`FSCTL_ENUM_USN_DATA`) as a much faster index builder on Windows, plus USN journal for live updates (needs admin rights → optional helper service)

## Phase 5 — Power-user features ✅

- ✅ Dual-pane / split view (Ctrl+\\): two panes side by side, each with its own tab; F6 or a click switches the focused pane, drag & drop between panes, narrow panes hide less important columns
- ✅ Preview pane (Alt+P): images, text and code with line numbers, folder and multi-selection summaries
- ✅ Preview pane extras: syntax highlighting for code and config files, PDF first page (Windows' built-in PDF renderer), audio/video details with cover art
- ✅ Tabbed preview pane: Info first, then the file type's tabs; fonts get a sample text tab and a Glyphs grid
- ✅ Bulk rename (F2 on several items): name patterns with `{name}`/`{n}`/`{date}` tokens, find & replace (text or regex), counters, case changes, live preview with conflict checks, all-or-nothing apply
- ✅ Browse zip and tar (.tar, .tar.gz, .tgz) archives like folders, with previews; extract by drag & drop, copy/paste or *Extract here / Extract to*; *Compress to ZIP*
- ✅ 7z archives (browse, preview, extract)
- ✅ Folder size column: computed in the background (one walker, easy on HDDs), sub-folders cached on the way, refreshed when files change; sorting by size includes folders
- ✅ Colour tags with custom names: tag from the context menu or palette, dots in the file view, a Tags section in the sidebar, tags follow renames and moves
- ✅ Command palette (Ctrl+Shift+P / F1): fuzzy search over commands, favorites, drives, recent folders and saved searches
- ✅ Custom keyboard shortcuts: Settings → Shortcuts, click and press the new keys; conflicts are flagged and menus, tooltips and the palette show your keys
- ✅ Git status column for repositories (M/A/D/R/?/I, folders summarise their contents) and the branch with ahead/behind in the status bar
- ✅ Built-in terminal panel (Ctrl+`): your shell in a real terminal (ConPTY on Windows) under the file view, with *cd here* and optional folder following

## Phase 6 — Polish & 1.0 ⏳

- ⏳ Themes and theme import/export
- ⏳ Localisation (English, German, …)
- ⏳ Accessibility pass (keyboard-only use)
- ⏳ In-app **Help** page (a tab like Settings) opened from the bottom of the sidebar: getting started, features, search syntax, keyboard shortcuts, FAQ (no separate documentation website)
- ⏳ 1.0 release
