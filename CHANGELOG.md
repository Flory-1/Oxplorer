# Changelog

All notable changes to Oxplorer. Versions follow [Semantic Versioning](https://semver.org/).
The release workflow publishes the section of the tagged version as the release notes.

## [Unreleased]

### Added
- **Phones and cameras on a USB cable**: Android phones (and other MTP devices) connected by
  cable show up under *This PC* in the sidebar and on the Home page. Open one to browse its
  storage, open and preview its files, and copy or drag them to your PC. Phones are read-only
  for now: copying to a phone, renaming and deleting on it come later. Unlock the phone and
  allow file transfer on it for it to appear.
- **List of folders with a view of their own**: Settings → Browsing lists every folder (and
  library) whose view is remembered, with its view mode, sort order, grouping and whether it
  has its own columns. Click a folder to open it, or ✖ to forget its view.

### Changed
- The transfers panel sits under the files like the terminal, so it no longer covers the
  sidebar. Each transfer shows where it copies or moves from and to (full paths), the names of
  its items, the files done, how many are copied at a time and for how long it has run. Its
  buttons have their icons centred.
- The search button in the command bar always opens a search tab: with what is typed in the
  search box (as Enter does), or an empty one. Enter in an empty search box opens an empty
  search tab too. An empty search tab searches nothing until you type or pick a filter.

### Fixed
- With *Open folders in Oxplorer* on, the Recycle Bin on the desktop (and anywhere else it is
  opened from) opened in Explorer. It now opens in Oxplorer. Turned on by an older version, the
  setting picks this up at the next start.
- *Show transfers* in the command palette did nothing when no transfers were running. It now
  opens the transfers panel even then (with the history of finished transfers), until it is
  hidden again.
- *Remember the view of each folder* now works in Windows libraries too: each library keeps
  its own view mode, sort order, columns and grouping.
- Double-clicking a picture while the preview pane was open sometimes opened another picture:
  the first click opened or closed the pane, the items moved, and the second click landed on
  another one. The pane now waits until a double-click can no longer follow.
- Going back with the mouse's Back button sometimes copied or moved the file under the
  pointer into the folder that was under it after going back. Files and tabs are now only
  dragged with the left button, and only releasing the left button drops them.

## [1.1.4] - 2026-11-03

### Added
- **Custom commands where you want them**: each command can be placed under an entry of the
  right-click menu (at the top, under Open, Copy, Delete, Tags, Compress to ZIP, Copy path or
  Properties) and, optionally, in a submenu with a name of your choice. Commands with the same
  submenu name share it.
- **Disk usage in a window or the preview panel**: Settings → Browsing → *Disk usage opens*
  chooses between the tab (as before), a window over the app, or a panel at the right where
  the preview is. The window and the panel zoom into folders in place, and can move their
  view to a tab.
- **Properties in tabs**: the Properties dialog of a single item has *General*, *Details*
  (full path, dimensions, length, encoding, camera details, Git, a map for photos with a
  location, checksums) and the tabs of the preview: Image, Text, Markdown, Table, Tree, Hex,
  Player, Git, Diff and so on.

### Changed
- The custom command form is laid out in sections with the labels at the top of each field.
  The icon picker shows the icons as buttons of the same size (they were cut off at the
  bottom), and there are twice as many icons to choose from. The command line says what goes
  into it and shows an example.

## [1.1.3] - 2026-11-01

### Changed
- **Less memory again**: the full-text index is only kept in memory while it is used. It is
  put away two minutes after the last `content:` search or indexing pass and read again in a
  fraction of a second when needed (Settings → Search index then says *loaded when needed*).
- The drive index takes about 30% less memory (116 MB → 81 MB for all of C:\) and its file
  on disk gets smaller too: equal file names are stored once, and each entry takes 20 bytes
  instead of 24.
- Both indexes are read from disk in pieces, so starting Oxplorer needs less memory at its
  peak.

## [1.1.2] - 2026-11-01

### Added
- **Delete engine**: deleting runs as a job like copying does. Deletes that take more than a
  moment show in the transfer panel with progress, items found, space freed and the current
  file, and can be paused or cancelled. Permanent deletes remove files in parallel on SSDs
  (one at a time on HDDs), clear read-only flags, and remove links and junctions without
  touching what they point to. Large selections go to the Recycle Bin in batches, so they
  show progress too.

### Changed
- When a file inside a folder can't be deleted, the "Couldn't delete" dialog names that file
  (and the program holding it) instead of only the folder. Everything else in the folder is
  still deleted.
- **About 40% less memory** with a drive index and full-text search turned on (for example
  440 MB → 255 MB with all of C:\ indexed): the full-text index keeps its words in one block
  instead of one allocation per word (half the memory), and the file index no longer doubles
  its reserved room when files are added while it runs.

### Fixed
- The file index added a whole folder a second time (with duplicate search results) when a
  change inside it was reported with a short name, like `C:\Users\ABCDEF~1\…`. This happened
  all the time for the user folder, because `%TEMP%` is often written that way.

## [1.1.1] - 2026-10-30

### Added
- **Why something can't be deleted**: instead of a short message, a dialog shows the exact
  error for each item, which programs have it (or files inside it) open with their process
  ID, a button to end such a program and delete again, *Delete as administrator* for
  protected folders, and *Copy details*.
- **Search, Recycle Bin, Help and Settings buttons** at the right of the command bar. The
  Recycle Bin button takes dropped files like the sidebar entry did.
- Settings → Sidebar: show or hide *This PC*, *Network* and *Remote locations* separately
  (instead of one *Drives* switch).
- Settings → Terminal panel: *Open in Terminal uses a separate window*, for Windows Terminal.

### Changed
- *Open in Terminal* opens the built-in terminal panel in that folder (turn on the new
  setting for Windows Terminal).
- Recycle Bin, Settings and Help are no longer pinned to the bottom of the sidebar (they are
  in the command bar).
- Clicking the only selected item again deselects it.
- A long path scrolls inside the address bar (with the mouse wheel) instead of pushing the
  search box and the preview pane out of the window.
- Less memory: the file index no longer keeps unused room after loading (about a third less
  for a whole drive), folder sizes only remember the folders near the ones you look at, and
  the text index gives back unused room after indexing.
- New app icon: a white folder without a background, like the taskbar icons of other apps.
- Less drawing while nothing happens: the drive index and the text index no longer wake the
  window for every change they pick up.

### Fixed
- A search in a folder answered from the index said it went through all items of the index
  (e.g. "2,300,202 items"); it now counts the items in that folder.
- Mapped network drives that Windows hadn't connected again yet (after signing in) were
  missing from This PC; they are listed and connected when opened, like in Explorer.
- The window flickered (a blank frame now and then, most visible while scrolling): on Windows
  Oxplorer now draws with DirectX 12 instead of Vulkan, which showed empty frames when drawing
  started again after a pause. `WGPU_BACKEND` still overrides it.
- Typing just a drive (`C:`) in the address bar opened the folder Oxplorer was started in on
  that drive instead of the drive itself.
- The seek bar of the media player (preview and mini player) jumped forward in steps while
  playing; it now moves smoothly.

## [1.1.0] - 2026-10-29

### Added
- **Remote locations**: SFTP, FTP/FTPS and WebDAV servers as places in the sidebar (*Remote
  locations → Add…*), or type `sftp://`, `ftp://`, `ftps://` or `webdavs://` in the address bar.
  Browse them like folders, copy and move files to and from them with the copy engine (progress,
  speed limit, retry and skip), rename, make folders and delete (after asking, as nothing goes to
  the Recycle Bin). SFTP checks the server's key and warns clearly when it changed; passwords are
  only kept in the Windows Credential Manager when you ask for it.
- **Custom commands** (Settings → Custom commands): your own command lines with placeholders
  (`{path}`, `{paths}`, `{name}`, `{stem}`, `{ext}`, `{parent}`, `{folder}`, `{n}`) in the
  right-click menu and the command palette, for certain file types, run hidden, in a console or
  with their output in a window, optionally with a shortcut. A few examples are one click away.
- **WSL**: your Linux distributions under *This PC* and on Home, with *Open in WSL terminal* and
  *Copy path as → As Linux path*. Dotfiles count as hidden there. Listing them never starts WSL.
- **More Git** in the preview's Git tab: stage and unstage files, discard changes (after asking),
  commit (also amend), switch and create branches (*Stash and switch* when changes are in the
  way), fetch, pull and push. A **Diff** tab shows what changed in a file, also side by side.
- **Mirror / sync** a folder to another, one way (right-click a folder → *Mirror to*, the split
  view's compare bar, or the command palette). *Mirror* makes the target exactly like the source,
  *Update* only copies what's new or changed; compare by size and date or by contents, and leave
  things out (`*.tmp; node_modules; .git`). A preview lists what will be copied, updated and
  removed, and every row can be unticked. Removed items go to the Recycle Bin, copies run as
  normal transfers, and Ctrl+Z undoes it. A target inside the source is refused, and removing
  more than half of the target needs an extra tick. Recent pairs are remembered.
- **Undo history kept after a restart** (Settings → Browsing → *Keep undo history after
  restart*): the last 100 operations of the last 30 days, still checked before anything is
  undone. Several windows don't overwrite each other's history.
- **Windows notifications** when transfers are done while Oxplorer is in the background
  (Settings → Transfers → *Notifications when transfers are done*). Clicking one brings the
  window to the front with the transfers panel open.
- **Custom commands in Settings → Shortcuts**, next to the built-in ones, with the same warning
  when two commands share keys.
- **Git stashes**: the Branches view lists the stashes with *Apply*, *Pop* and *Drop* (asks
  first), and *Stash changes* with an optional message. Fetch, pull and push show git's progress
  live and can be cancelled.
- **Sort by the columns read from the files**: Dimensions, Length, Date taken, Authors and File
  version. Items not read yet come last and move into place as their values arrive.
- **Libraries** refresh by themselves when files change in their folders, and the search box
  searches all of a library's folders.
- **Group by month** ("October 2026", by date taken where known), on by default in the Gallery
  and available in every folder.
- **New link dialog**: *Browse…* picks the target, and the target field completes folders and
  files as you type (also after `%USERPROFILE%` and `~`). The address bar completes the same way.
- **Compare two text files** (select two files → right-click → *Compare these two files*, or one
  file in each pane of the split view from the command palette): a diff window like the Git tab's,
  one column or side by side, with *Ignore whitespace*. Binary and very large files show where
  they first differ.
- **Data views and the hex view for items inside archives**: tables, trees and SQLite databases
  inside a zip or 7z show like files on disk.
- **Text index** (Settings → Search → *Index the text of documents*, off by default): keeps the
  words of text files, code, PDF, Word, Excel, PowerPoint and OpenDocument files in chosen
  folders, so `content:` searches there are fast and also find text inside those documents.
  Built in the background and paused while files are copied; files it doesn't know yet are still
  read.
- **Index service** (optional installer task *Fast index service*, for all users, off by default;
  then Settings → Search → *Use the Oxplorer index service*): reads NTFS drives from the master
  file table and follows the change journal **without running Oxplorer as administrator**. It
  answers only the user who installed it (and administrators), and only with what the master
  file table holds: names, sizes and dates of every file on the fixed NTFS drives, never their
  contents.
- **Catch-up after a restart**: with NTFS drives read through the service or as administrator,
  the index applies what changed while Oxplorer was closed from the change journal instead of
  building again.
- **Duplicates**: *Compare contents byte by byte* after the hashes match, and hashing on several
  threads (one on a hard disk), so scans on SSDs are faster.
- **Server transfers resume and verify**: downloads, uploads and server copies continue where
  they stopped after the connection dropped, and interrupted ones can be resumed from the
  transfer panel after a restart. With *Verify* on, they're checked with the server's checksum
  where it offers one (FTP `HASH`, Nextcloud/ownCloud), otherwise by size ("size checked only").
- **Copying between two servers** (or within one), streamed through Oxplorer without a temporary
  file.
- **Upload after editing**: a file opened from a server can be uploaded again after you saved
  it, with a warning when the server's file changed in the meantime.
- **Thumbnails in remote folders** for pictures up to 8 MB (Settings → Appearance).

### Changed
- **Rotating and flipping JPEGs is lossless**: only the EXIF orientation is changed, every other
  byte stays as it was. Settings → Preview → *Re-encode JPEGs instead* for programs that ignore
  the orientation.
- *Stash and switch* now stashes untracked files too (they were left behind before).

## [1.0.4] - 2026-10-29

### Added
- **Office documents and mails in the preview**: a *Document* tab shows Word, Excel, PowerPoint,
  Outlook (.msg), Visio, RTF and everything else Windows can preview, with the same preview
  handlers Explorer uses. They run in Windows' own preview process, so a handler that crashes
  can't take Oxplorer with it. Settings → Preview → *Office documents and mails*.
- **More picture formats**: SVG drawings, and HEIC/HEIF, AVIF, JPEG XL, JPEG XR and camera RAW
  files through the codecs installed in Windows (when one is missing, the preview says which
  extension to get from the Microsoft Store). Animated GIF and WebP play in the preview and in
  Quick Look, and photos are turned the way the camera says.
- **Photo details**: camera, lens, exposure time, aperture, ISO, focal length, flash and the
  location in the Info tab, with *Open in map* (OpenStreetMap, only when you click it).
- **Data files**: CSV and TSV as a table; JSON, XML, YAML and TOML as a tree you can fold, with
  *Copy value* and *Copy path*; SQLite databases as tables (opened read-only, never locked).
- **Hex view** for every file: offset, bytes and text, reading only what's on screen, so huge
  files open instantly. Go to an offset, select and copy as hex or text.
- **Player**: previous / next file of the folder (Ctrl+Left / Ctrl+Right, Ctrl+Space plays
  and pauses), repeat all or one, shuffle, playback speed from 0.5× to 2×, a queue, *Play all*
  for a folder or the selection, `.m3u` playlists and `.srt` / `.vtt` subtitles next to the
  video. Playback goes on in a **mini player** at the bottom of the window while you browse
  (Settings → Preview → *Keep playing while browsing*).
- **Picture tools** (right-click pictures → *Picture tools*): rotate, flip, convert to PNG, JPG
  or WebP and resize one or many pictures, next to the originals, into a folder or in place.
  Replaced originals go to the Recycle Bin, so Ctrl+Z brings them back; photo details are kept.
- **Live index updates**: the search index follows new, deleted, renamed and changed files as
  they happen (within about 2 seconds) instead of waiting for the next refresh. Settings →
  Search → *Live updates*; removable and network drives aren't watched, so USB sticks can still
  be ejected.
- **Faster index on NTFS drives**: run as administrator, Oxplorer reads whole NTFS drives
  straight from the master file table, much faster than walking every folder, and follows
  changes through the NTFS change journal. Without admin rights it walks the folders and watches
  them as before.
- **Disk usage**: right-click a folder or drive (also the drive cards on Home, or the command
  palette) → *Disk usage* shows what takes up the space as a treemap, with the largest items
  next to it. Click a folder to zoom in, Back or Up to zoom out; sizes fill in while it scans,
  and the scan also fills the Size column for every sub-folder.
- **Find duplicates and empty folders**: right-click a folder → *Clean up*. Duplicates are found
  by size, then by their first and last bytes, then by a hash of the whole file, so big files
  are only read when they have to be; hard links and links aren't counted. Tick what to remove
  (or *Select all but the newest / oldest*) and *Delete selected* moves it to the Recycle Bin,
  with an extra warning when every copy of a file is ticked.
- **Search history and suggestions**: the search box remembers the searches you ran and, while
  you type, suggests earlier ones and completes the query syntax (`ki` → `kind:` →
  `kind:image`). Up/Down choose, Enter runs, Tab puts it in the box. Settings → Search →
  *Remember searches* turns it off.
- **`tag:` in searches** (`tag:red,blue`, also your tag names) and a chip for each tag colour
  you use. It works across whole drives instantly.
- **Search inside archives**: `archives:yes` or the *Inside archives* chip also finds the files
  in zip, 7z and tar archives by name; results open inside the archive.
- **Column chooser**: right-click the details header (or View → Columns) to show or hide
  columns, drag a header to move it and an edge to resize it. New columns: date created,
  dimensions, length, date taken, authors, file version, attributes and, in OneDrive or Dropbox
  folders, Status. They're read in the background, and files that are only in the cloud aren't
  downloaded for them.
- **Remember the view per folder**: each folder keeps its view, sort order, columns and grouping
  (e.g. *Downloads* by date, *Pictures* as a grid). *Reset view of this folder* and *Use this
  view for all folders* are in the View menu; Settings → Browsing → *Remember the view of each
  folder* turns it off.
- **Group by** type, date modified, size or tag (Sort → Group by), with collapsible groups in the
  details and grid views; keyboard navigation and Select all go over the open groups.
- **Grid zoom**: Ctrl+mouse wheel over the grid makes the icons smaller or larger, from small
  icons to large thumbnails (zoom out past the smallest to get the details view), or use the size
  slider in the status bar. Thumbnails and large Windows icons load in a matching size.
- **Select by pattern** (`*.jpg; *.png`) and deselect by pattern (Ctrl+Shift+A), **Invert
  selection** (Ctrl+Shift+I) and **Select all of the same type**, in the right-click menu's
  Select submenu and the command palette.
- **Quick Look**: Space shows a large preview of the selected file over the window (pictures,
  text and code, Markdown, PDF, audio and video, and the contents of folders and archives); the
  arrow keys go to the previous or next item, Space or Esc closes it.
- **Flat view** (View menu, toolbar or Ctrl+Shift+F): every file in a folder and all its
  sub-folders in one list, with a Folder column. It lists in the background up to 200 000 files,
  skips hidden and unreadable folders and doesn't follow junctions or links to folders.
- **Pinned tabs**: pin a tab from its menu or the command palette. It shrinks to its icon, stays
  at the front, and survives *Close other tabs* and restarts.
- **Named sessions**: save the open tabs and groups under a name (right-click the + button or use
  the command palette) and open them again later, instead of the open tabs or next to them.
  Sessions can be renamed and deleted.
- **More windows**: Ctrl+N opens a new window on the current folder, and *Move to new window* or
  dragging a tab out of the window gives it a window of its own. Only the first window restores
  and saves the tabs, and windows don't overwrite each other's settings, favorites and tags.
- **Links**: create symbolic links, junctions and hard links from the New menu, or drag with
  Ctrl+Shift to link items into a folder (folders get a junction, files a symbolic link or,
  without Developer Mode, a hard link). Links show an arrow, their target in the tooltip and
  status bar, and *Open link target location*; Ctrl+Z removes a new link (never its target).
- **Cloud files**: the OneDrive / Dropbox status of files and folders (online only, on this
  device, always kept, sync pending, sync problem) as a badge on the icon, a Status column and in
  the Info tab, plus *Always keep on this device* and *Free up space* in the right-click menu.
- **Libraries and Gallery** in the sidebar: a Windows library (Documents, Music, Pictures, Videos
  or your own) shows the items of all its folders together, and files dropped or pasted into it
  go to its save location. The Gallery shows every picture and video of the Pictures library,
  newest first, as a grid. Settings → Sidebar turns the section off.
- **Undo and redo**: Ctrl+Z undoes the last file operation and Ctrl+Y (or Ctrl+Shift+Z) redoes
  it: renames and bulk renames, moves, copies, new folders and files, and deletes to the
  Recycle Bin. The last 50 are remembered. Undone copies and new files go to the Recycle Bin,
  and deleted items come back from it. If an item was changed, moved or replaced since, Oxplorer
  says so instead of guessing; permanent deletes can't be undone. The command palette shows what
  Undo and Redo would do.
- **Errors are asked about while copying**: when a file can't be copied or moved, the transfer
  waits and asks: retry, skip, or skip all errors of that job. Skipped files are still listed at
  the end and can be retried. Settings → Copy engine → *Ask when a file fails* turns it off.
- **Transfer queue**: waiting transfers start in order and can be put on hold, moved earlier or
  later, or set to run after the current ones. *Paste after current transfers* (folder menu and
  command palette) queues a paste that way.
- **When all transfers are done**: a setting can show a notification (a message in the app and a
  flashing taskbar button) or put the computer to sleep. Sleep only follows transfers that
  weren't cancelled, after a 30-second countdown you can cancel.
- **Interrupted transfers can be resumed**: if Oxplorer closes, the computer sleeps or a USB
  stick is pulled mid-copy, the transfer panel offers to resume or discard the transfer. Files
  that were copied completely are skipped, a file that was cut off is copied again, and a move
  only deletes an original once its copy is complete.
- **Keep folder dates** (on by default): copied folders keep the modified date of the originals.
- **Keep permissions** and **keep alternate data streams** (both optional, Settings → Copy
  engine): copies take over the access rules (ACLs) of the originals, and the owner where
  Windows allows it, or the extra data programs attach to files and folders. With streams off,
  copies get only the contents, but the "downloaded from the internet" mark is always kept.
- **Compare folders**: Ctrl+Shift+D (or the ⇄ button between the panes, or the command
  palette) compares the two folders of the split view. Colour strips mark what only one side
  has, what's newer or older and what differs; a bar above the panes sums it up, with *Only
  differences* to hide the rest. Copy what's missing to the left or right, or replace older
  files with the newer ones, after a confirmation and through the transfer panel. Sub-folders
  and a byte-by-byte check of the contents can be turned on.
- **Copy benchmark** for developers: `cargo run --release -p ox-core --example copy_bench` times
  the copy engine on many small files, one large file and a mix, optionally next to robocopy,
  xcopy and a plain copy loop, on any destination. Saved as a baseline, a later run fails when
  the engine got more than 15 % slower. It's run locally before a release, not in CI.
- **Update check**: once a day Oxplorer asks GitHub for the newest release and, when there's a
  newer one, says so on the Home page with a link to what's new. ✖ hides the note for that
  version; Settings → Startup → *Check for updates* turns it off.
- **Properties dialog of its own** instead of Windows': type, the program that opens it, size
  and size on disk, place, dates (also last opened and, for photos, taken), Read-only and Hidden
  to change, a program's version, a photo's camera details, and the Git state and last commit.
- **Log file and crash reports**: problems and crashes are written to the `logs` folder in the
  config folder, with the version, Windows version and where it happened. *Open log folder* on
  the Help page opens it, to attach to a bug report; after a crash, the next start says so.
- **More on the Info tab**: the same details as in Properties, and the item's Git state, so an
  item that was never committed says *Untracked* instead of showing no commit.

### Changed
- With *Remember the view of each folder* on, a folder without a view of its own opens with the
  default view instead of keeping the sort order of the folder before.
- *Open file location* (search results, flat view, libraries) goes to the folder and selects the
  item there.
- Browsing no longer downloads online-only cloud files: thumbnails come from the Windows cache
  only, the preview and content searches skip them, and their icons are the generic ones.
- The sidebar and the preview pane have **fixed widths** that line up with the toolbar: the
  sidebar ends under the separator after the edit buttons, the preview starts between the
  address bar and the search box. The width settings are gone.
- The preview pane **only opens when something is selected**, and always opens on Info.
- Settings, Help and the Recycle Bin **open only once**: opening one again switches to its tab.

### Fixed
- Copying a OneDrive folder skipped its sub-folders, because cloud folders were taken for links.
- The window could flash when switching back to it from another window.
- The status bar jumped in height between Settings and folders.
- The close button on tabs wasn't centred.
- The last commit was sometimes missing on the Info tab (after slow previews, and for file names
  like `[id].tsx`).

## [1.0.2] - 2026-10-29

### Changed
- **The first tab is always Home**: a small tab with the house icon that can't be closed or
  moved. Folders, drives and searches opened from it open in a new tab, so Home stays one click
  away; *Go to Home* in the command palette switches to it. The *Home* entry at the top
  of the sidebar is gone.
- **New icons**: the icons in menus, buttons, tabs, the sidebar and Settings are now
  [Lucide](https://lucide.dev)'s line icons, the same on every computer, instead of Windows'
  colourful emoji. Other emoji (e.g. in file names) are shown in black and white.

### Fixed
- Icons Windows' emoji font doesn't have, like the sort arrow next to a column name and the
  favourite star in the toolbar, showed an empty box.

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
- Still planned at the time: a keyboard-only accessibility pass, resuming
  interrupted transfers, live index updates and an NTFS MFT index builder.
