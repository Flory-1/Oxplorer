# Changelog

All notable changes to Oxplorer. Versions follow [Semantic Versioning](https://semver.org/).
The release workflow publishes the section of the tagged version as the release notes.

## [Unreleased]

### Added
- Settings → **Preview pane**: open the preview as its own window (move and resize it freely)
  instead of a pane on the right, and set the pane's width.
- Settings → Sidebar: **sidebar width**. Dragging the edge of the sidebar or the preview pane
  updates the setting, and the width is kept between runs.

### Fixed
- The window opens where it was closed, with the same size and maximized state, instead of in
  the top left corner. If its monitor is gone, it is moved onto a connected one.
- The search index is no longer rebuilt at every start: a saved index that is younger than
  *Refresh every* is used as is, and the refresh waits until it is due.
  (*Only at start* still rebuilds at every start.)
- Network servers and mapped network drives showed an empty box instead of an icon on Windows
  (its emoji font has no 🖧); they now have their own drawn server icon. Network drives on the
  Home page show their share instead of "0 B free of 0 B".
- The terminal opens in the current folder also when it is shown again later, not only when
  it starts.
- Tabs that aren't active can be closed with their ✖ (it disappeared under the pointer before,
  so the click only activated the tab).
- Icons on tabs, in the sidebar, the command palette and the section lists of Settings and
  Help are centred on their visible shape, so emoji icons on Windows (from its emoji fonts,
  which sit higher or lower than the text font) line up.

### Changed
- The terminal panel's close button has no background any more.
- The command palette button in the toolbar is a small drawn keyboard, as wide as the icons
  next to it (Windows' ⌨ emoji was much wider).

### Removed
- The *UI scale* setting.
- The terminal's *cd here* button (the terminal goes to the current folder when it opens).

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
