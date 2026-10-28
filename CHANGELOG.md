# Changelog

All notable changes to Oxplorer. Versions follow [Semantic Versioning](https://semver.org/).
The release workflow publishes the section of the tagged version as the release notes.

## [1.0.0] - 2026-10-28

The first release: a fast file explorer for Windows, written in Rust.

### Downloads

- `oxplorer-v1.0.0-setup-x64.exe`: installer for the current user (no admin rights). Start menu
  entry, optional desktop icon and an option to open folders in Oxplorer instead of Explorer.
- `oxplorer-v1.0.0-windows-x64.zip`: portable version, unzip and run `oxplorer.exe`.
- `.sha256` files to check the downloads.

Windows 10 (1809) or newer, 64-bit.

### Highlights

**Browsing**
- Details and grid views, natural sort order, multi-select, type-to-jump, folders first
- Tabs and Chrome-style tab groups (named, coloured, collapsible); the last session can be restored
- Home page with favorites, drives and recent folders and files; sidebar with Favorites,
  This PC, Network, Recent, Tags and Saved searches
- Split view with two folders side by side (Ctrl+\\)
- Preview pane (Alt+P): images, text and code with syntax highlighting, PDFs, audio and video
  details with cover art, fonts with a glyph grid
- Thumbnails of pictures, videos and PDFs in the grid view
- Archives (zip, 7z, tar, tar.gz) open like folders: browse, preview, extract; compress to ZIP
- Folder sizes in the Size column, Git status column, colour tags with your own names

**Copying and moving**
- Own copy engine instead of the Windows copy dialog: parallel copies, `CopyFileExW` fast path,
  pause, resume and cancel, speed and time left
- One question per job when files already exist (replace, skip, keep both, replace if newer)
- Per-disk queue, automatic worker count for HDDs and SSDs, optional verification (xxh3),
  speed limit, retry of failed files, transfer history
- Clipboard shared with Windows Explorer in both directions; drag & drop inside the app, from
  other apps and out into other apps

**Search**
- Instant filter of the current folder; Enter searches all sub-folders
- File-name index of your drives: search as you type, results in milliseconds
- Query syntax: wildcards, regex, `ext:`, `kind:`, `type:`, `size:`, `modified:`, `content:`,
  exclusions; filter chips and saved searches

**Windows integration**
- Explorer's file icons, the real Explorer context menu (*Show more options*), Properties and
  *Open with*
- Recycle Bin view: restore, delete permanently, empty
- Network locations: `\\server` shares, mapped drives, Map network drive
- Mica or Acrylic window backdrop, rounded corners, title bar following dark/light mode
- Optional: open folders, drives and Win+E in Oxplorer

**Everything else**
- Command palette (Ctrl+Shift+P), custom keyboard shortcuts, built-in terminal panel (Ctrl+`)
- Colour themes (6 built in, editor, export and import as `.toml` files)
- English and German, following the Windows language
- Help page (F1) with every feature, the search syntax and your keyboard shortcuts

### Known limitations

- Several Windows features have so far been checked by the automated Windows build and tests,
  but not yet by hand on a real Windows machine: Shell icons, thumbnails and context menu,
  drag out, network locations, the window backdrop and opening folders by default. Please
  [report problems](https://github.com/Flory-1/oxplorer/issues).
- The installer is not code-signed, so Windows SmartScreen may warn before the first start
  (*More info → Run anyway*).
- Still planned (see the [roadmap](ROADMAP.md)): a keyboard-only accessibility pass, resuming
  interrupted transfers, live index updates and an NTFS MFT index builder.
