# Changelog

All notable changes to Oxplorer. Versions follow [Semantic Versioning](https://semver.org/).
The release workflow publishes the section of the tagged version as the release notes.

## [Unreleased]

## [1.0.1] - 2026-10-29

### Added
- **Tabs in the title bar** (Windows), as in Windows 11's Explorer: the separate title bar
  above the tabs is gone, and minimize, maximize and close sit next to the tabs, drawn like
  Windows 11's. Drag the free space next to the tabs to move the window, double-click it to
  maximize or right-click it for the window menu; the window keeps its borders, shadow, rounded
  corners and snapping, and pointing at maximize shows the snap layouts. Settings → Appearance →
  *Tabs in the title bar* brings Windows' title bar back.
- **Built-in player** (Windows): audio and video files play in the preview pane's new
  *Player* tab, with a seek bar, 10 seconds back and forward, repeat and volume. Windows' own
  media engine plays them, so every format Windows can play works. *▶ Play* in the Info tab
  plays the file there.
- **Markdown preview**: Markdown files get a *Markdown* tab that shows them formatted:
  headings, lists, task lists, quotes, tables, highlighted code blocks and pictures next to
  the file.
- **Git graph**: folders in a Git repository get a *Git* tab with the branch, the number of
  changed files and the newest 500 commits of all branches as a graph, with branches, tags,
  message, author and date. Click a commit to copy its hash; the branch name and a button for
  the repository's GitHub/GitLab page are at the top.
- **Your own entries in ➕ New** (toolbar, right-click menu and command palette): a file type
  with a label, a file name and content to start with, where `{name}` and `{date}` are filled
  in (Settings → New menu). Markdown document, Python script, PowerShell script and JSON file
  are there to start with.
- **Settings → Developer** and developer features:
  - *Open in editor* (Ctrl+Shift+E, right-click menus, command palette) for files and
    folders, with VS Code or any editor command (presets for Cursor, Zed, Sublime Text,
    Notepad++ and others).
  - *Copy path as*: with forward slashes, in quotes, relative to the Git repository, as a
    `file://` URL, or just the name.
  - *Git* in the right-click menu: open the file or folder on GitHub, GitLab or Bitbucket, or
    copy the branch name. The Info tab shows the last commit that changed the item.
  - The Info tab shows the encoding, line endings and indentation of text files, and
    *Compute checksums* gives the SHA-256 and MD5 of any file.
  - The developer entries in the menus can be turned off.
- Settings → **Preview pane**: open the preview as its own window (move and resize it freely)
  instead of a pane on the right, and set the pane's width.
- Settings → Sidebar: **sidebar width**. Dragging the edge of the sidebar or the preview pane
  updates the setting, and the width is kept between runs.

### Fixed
- Text files with a UTF-16 byte order mark (e.g. some files Windows tools write) are shown as
  text in the preview instead of nothing; a UTF-8 byte order mark is no longer shown as a
  stray character.
- The line numbers in the text preview line up with the code again (highlighted code was
  drawn bigger than the numbers).
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
- The command palette button in the toolbar is a small drawn keyboard, the same size as the
  icons next to it and with the same spacing (Windows' ⌨ emoji was much wider).

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
