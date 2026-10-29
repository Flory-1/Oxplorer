//! Loading previews in the background: pictures, PDF pages, text,
//! audio/video details, fonts, folders and archives.

use super::*;

/// Largest file we try to decode as an image.
pub(super) const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// Longest image side after downscaling (in the pane, in Quick Look).
pub(super) const MAX_IMAGE_SIDE: u32 = 1024;
pub(super) const MAX_IMAGE_SIDE_LARGE: u32 = 2560;
/// Width PDF pages are rendered at (in the pane, in Quick Look).
const PDF_WIDTH: u32 = 800;
const PDF_WIDTH_LARGE: u32 = 1600;
/// Items of a folder or archive listed in Quick Look.
pub(super) const MAX_ITEMS: usize = 2000;
/// How much of a text file is shown.
pub(super) const MAX_TEXT_BYTES: usize = 128 * 1024;
/// How many lines of it.
pub(super) const MAX_TEXT_LINES: usize = 3000;

pub(super) enum Loaded {
    Image(ColorImage, [u32; 2]),
    /// An animated GIF/WebP: its frames, how long each stays, and its size.
    Animation(Vec<(ColorImage, Duration)>, [u32; 2]),
    /// First page of a PDF and the number of pages.
    Page(ColorImage, u32),
    Text {
        text: String,
        numbers: String,
        truncated: bool,
        /// Encoding, line endings, indentation.
        details: Vec<(&'static str, String)>,
    },
    /// Audio/video details and the embedded cover picture.
    Media {
        info: Vec<(&'static str, String)>,
        cover: Option<ColorImage>,
    },
    Folder {
        folders: usize,
        files: usize,
        bytes: u64,
        /// The history of the Git repository the folder is in.
        git: Option<Arc<GitInfo>>,
        /// What's in it, folders first (only for Quick Look).
        items: Vec<FileEntry>,
    },
    /// A font file: its bytes (installed into egui to draw it), details
    /// and every character it has a glyph for.
    Font {
        data: Vec<u8>,
        info: Vec<(&'static str, String)>,
        chars: Vec<char>,
    },
    /// A data file: its table or tree (see `data`).
    Data(Box<data::LoadedData>),
    Nothing,
    Error(String),
}

/// The last commit that changed `e` (not for items inside archives).
pub(super) fn last_commit(e: &FileEntry) -> Option<Commit> {
    if ox_core::archive::split_virtual(&e.path).is_some() {
        return None;
    }
    let root = ox_core::git::repo_root(&e.path)?;
    ox_core::git::last_commit(&root, &e.path).ok().flatten()
}

pub(super) fn is_markdown(e: &FileEntry) -> bool {
    matches!(e.extension().as_str(), "md" | "markdown" | "mdown" | "mkd")
}

pub(super) fn is_image(e: &FileEntry) -> bool {
    is_picture(&e.extension())
}

/// Code and config files get syntax highlighting (by extension).
pub(super) fn highlightable(e: &FileEntry) -> bool {
    e.kind() == FileKind::Code
        || matches!(
            e.extension().as_str(),
            "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "cfg" | "md" | "csv" | "sql"
        )
}

pub(super) fn is_text(e: &FileEntry) -> bool {
    matches!(e.kind(), FileKind::Text | FileKind::Code)
        || matches!(
            e.extension().as_str(),
            "csv" | "tsv" | "srt" | "gitignore" | "env" | "lock"
        )
}

/// The items of a folder for Quick Look (`large`), sorted and capped.
fn listed(mut items: Vec<FileEntry>, large: bool) -> Vec<FileEntry> {
    if !large {
        return Vec::new();
    }
    use ox_core::entry::{SortKey, SortOrder};
    ox_core::entry::sort_entries(&mut items, SortKey::Name, SortOrder::Ascending, true);
    items.truncate(MAX_ITEMS);
    items
}

/// Load the preview of `entry`; `large` for Quick Look (sharper pictures
/// and pages, and the items of folders).
pub(super) fn load(entry: FileEntry, show_hidden: bool, large: bool) -> Loaded {
    if let Some((arc, inner)) = ox_core::archive::split_virtual(&entry.path)
        .filter(|(_, inner)| !inner.as_os_str().is_empty())
    {
        return load_from_archive(&entry, &arc, &inner, large);
    }
    let side = if large {
        MAX_IMAGE_SIDE_LARGE
    } else {
        MAX_IMAGE_SIDE
    };
    if entry.is_dir {
        return match ox_core::entry::list_dir(&entry.path, show_hidden) {
            Ok(items) => Loaded::Folder {
                folders: items.iter().filter(|i| i.is_dir).count(),
                files: items.iter().filter(|i| !i.is_dir).count(),
                bytes: items.iter().filter(|i| !i.is_dir).map(|i| i.size).sum(),
                git: git_info(&entry.path),
                items: listed(items, large),
            },
            Err(e) => Loaded::Error(e.to_string()),
        };
    }
    if std::fs::metadata(&entry.path).is_ok_and(|m| ox_core::cloud::online(&m)) {
        return Loaded::Nothing; // Online only: reading it would download it.
    }
    if ox_core::archive::kind(&entry.path).is_some() {
        // Archives: what's inside (opening them browses them).
        return match ox_core::archive::index(&entry.path) {
            Ok(idx) => {
                let files: Vec<_> = idx.entries.iter().filter(|e| !e.is_dir).collect();
                let folders: std::collections::HashSet<&Path> = idx
                    .entries
                    .iter()
                    .flat_map(|e| e.path.ancestors().skip(usize::from(!e.is_dir)))
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect();
                let items = if large {
                    ox_core::archive::list(&entry.path, Path::new("")).unwrap_or_default()
                } else {
                    Vec::new()
                };
                Loaded::Folder {
                    folders: folders.len(),
                    files: files.len(),
                    bytes: files.iter().map(|e| e.size).sum(),
                    git: None,
                    items: listed(items, large),
                }
            }
            Err(e) => Loaded::Error(ox_core::trf!("Can't read this archive: {e}", e = e)),
        };
    }
    if is_image(&entry) {
        if entry.size > MAX_IMAGE_BYTES && !ox_core::picture::is_wic(&entry.extension()) {
            return Loaded::Error(tr("The image is too large to preview.").into());
        }
        return picture::load(
            ox_core::picture::Source::File(&entry.path),
            &entry.extension(),
            side,
        );
    }
    if matches!(entry.kind(), FileKind::Audio | FileKind::Video) {
        if let Some(media) = media_info(&entry.path, entry.kind() == FileKind::Video) {
            return media;
        }
        if ox_core::player::supported() {
            // No details, but it can still be played.
            return Loaded::Media {
                info: Vec::new(),
                cover: None,
            };
        }
    }
    if entry.kind() == FileKind::Font {
        return font_info(&entry.path).unwrap_or(Loaded::Nothing);
    }
    if entry.kind() == FileKind::Pdf {
        return match ox_core::pdf::render_first_page(
            &entry.path,
            if large { PDF_WIDTH_LARGE } else { PDF_WIDTH },
        ) {
            Ok(pdf) => match image::load_from_memory(&pdf.png) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    Loaded::Page(
                        ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                        pdf.pages,
                    )
                }
                Err(e) => Loaded::Error(e.to_string()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::Unsupported => Loaded::Nothing,
            Err(e) => Loaded::Error(ox_core::trf!("Can't show this PDF: {e}", e = e)),
        };
    }
    if let Some(data) = data::load(&entry, data::Place::File) {
        return data;
    }
    if is_text(&entry) || entry.size == 0 {
        return read_text(&entry.path);
    }
    // Unknown type: show it as text if it looks like text.
    match read_text(&entry.path) {
        t @ Loaded::Text { .. } => t,
        _ => Loaded::Nothing,
    }
}

/// Preview of an item inside an archive, read without extracting it.
pub(super) fn load_from_archive(
    entry: &FileEntry,
    arc: &Path,
    inner: &Path,
    large: bool,
) -> Loaded {
    use ox_core::archive;
    if entry.is_dir {
        return match archive::list(arc, inner) {
            Ok(items) => Loaded::Folder {
                folders: items.iter().filter(|i| i.is_dir).count(),
                files: items.iter().filter(|i| !i.is_dir).count(),
                bytes: items.iter().filter(|i| !i.is_dir).map(|i| i.size).sum(),
                git: None,
                items: listed(items, large),
            },
            Err(e) => Loaded::Error(e.to_string()),
        };
    }
    if is_image(entry) {
        if entry.size > MAX_IMAGE_BYTES {
            return Loaded::Error(tr("The image is too large to preview.").into());
        }
        return match archive::read_entry(arc, inner, MAX_IMAGE_BYTES) {
            Ok(bytes) => {
                let side = if large {
                    MAX_IMAGE_SIDE_LARGE
                } else {
                    MAX_IMAGE_SIDE
                };
                picture::load(
                    ox_core::picture::Source::Bytes(&bytes),
                    &entry.extension(),
                    side,
                )
            }
            Err(e) => Loaded::Error(e.to_string()),
        };
    }
    if let Some(data) = data::load(entry, data::Place::Archive(arc, inner)) {
        return data;
    }
    match archive::read_entry(arc, inner, MAX_TEXT_BYTES as u64 + 1) {
        Ok(buf) => match text_from_bytes(buf) {
            t @ Loaded::Text { .. } => t,
            _ if is_text(entry) => Loaded::Error(tr("This file isn't plain text.").into()),
            other => other,
        },
        Err(e) => Loaded::Error(e.to_string()),
    }
}

pub(super) fn read_text(path: &Path) -> Loaded {
    use std::io::Read;
    let mut buf = Vec::new();
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return Loaded::Error(e.to_string()),
    };
    if let Err(e) = file.take(MAX_TEXT_BYTES as u64 + 1).read_to_end(&mut buf) {
        return Loaded::Error(e.to_string());
    }
    text_from_bytes(buf)
}

pub(super) fn text_from_bytes(mut buf: Vec<u8>) -> Loaded {
    use ox_core::textinfo::{self, Encoding};
    let enc = textinfo::encoding(&buf);
    let utf16 = matches!(enc, Encoding::Utf16Le | Encoding::Utf16Be);
    if !utf16 && buf.iter().take(8192).any(|&b| b == 0) {
        return Loaded::Nothing; // binary
    }
    let mut truncated = buf.len() > MAX_TEXT_BYTES;
    buf.truncate(MAX_TEXT_BYTES);
    let all = textinfo::decode(&buf, enc);
    let details = vec![
        (tr("Encoding"), enc.label().to_owned()),
        (
            tr("Line endings"),
            textinfo::line_endings(&all).label().to_owned(),
        ),
        (tr("Indentation"), textinfo::indent(&all).label()),
    ];
    let lines: Vec<&str> = all.lines().take(MAX_TEXT_LINES + 1).collect();
    truncated |= lines.len() > MAX_TEXT_LINES;
    let lines = &lines[..lines.len().min(MAX_TEXT_LINES)];
    let numbers = (1..=lines.len())
        .map(|n| format!("{n:>4}"))
        .collect::<Vec<_>>()
        .join("\n");
    Loaded::Text {
        text: lines.join("\n"),
        numbers,
        truncated,
        details,
    }
}

/// Tags and stream details of audio (and some video) files, with cover art.
pub(super) fn media_info(path: &Path, video: bool) -> Option<Loaded> {
    use lofty::prelude::*;
    let file = lofty::read_from_path(path).ok()?;
    let mut info: Vec<(&'static str, String)> = Vec::new();
    let tag = file.primary_tag().or_else(|| file.first_tag());
    if let Some(tag) = tag {
        let fields = [
            (tr("Title"), tag.title()),
            (tr("Artist"), tag.artist()),
            (tr("Album"), tag.album()),
            (tr("Genre"), tag.genre()),
        ];
        for (name, value) in fields {
            if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
                info.push((name, v.into_owned()));
            }
        }
        if let Some(date) = tag.date() {
            info.push((tr("Year"), date.year.to_string()));
        }
        if let Some(track) = tag.track() {
            let total = tag
                .track_total()
                .map(|t| format!(" of {t}"))
                .unwrap_or_default();
            info.push((tr("Track"), format!("{track}{total}")));
        }
    }
    let props = file.properties();
    let secs = props.duration().as_secs();
    if secs > 0 {
        let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
        let length = if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m}:{s:02}")
        };
        info.push((tr("Length"), length));
    }
    // For videos these describe the sound track.
    let (bitrate, rate_label) = if video {
        (tr("Audio bitrate"), tr("Audio"))
    } else {
        (tr("Bitrate"), tr("Sample rate"))
    };
    if let Some(b) = props.audio_bitrate().filter(|b| *b > 0) {
        info.push((bitrate, format!("{b} kbps")));
    }
    if let Some(rate) = props.sample_rate().filter(|r| *r > 0) {
        info.push((rate_label, format!("{:.1} kHz", rate as f32 / 1000.0)));
    }
    if let Some(ch) = props.channels() {
        let ch = match ch {
            1 => tr("Mono").to_owned(),
            2 => tr("Stereo").to_owned(),
            n => ox_core::trf!("{n} channels", n = n),
        };
        info.push((tr("Channels"), ch));
    }
    let cover = tag
        .and_then(|t| t.pictures().first())
        .and_then(|pic| image::load_from_memory(pic.data()).ok())
        .map(|img| {
            let img = img.thumbnail(512, 512).to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            ColorImage::from_rgba_unmultiplied(size, img.as_raw())
        });
    (!info.is_empty() || cover.is_some()).then_some(Loaded::Media { info, cover })
}

/// Largest font file that is loaded for the preview.
pub(super) const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;

/// Names, details and the character set of a TrueType/OpenType font.
pub(super) fn font_info(path: &Path) -> Option<Loaded> {
    use ttf_parser::name_id;
    if std::fs::metadata(path).ok()?.len() > MAX_FONT_BYTES {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    let face = ttf_parser::Face::parse(&data, 0).ok()?;
    let name = |id: u16| {
        face.names()
            .into_iter()
            .filter(|n| n.name_id == id)
            .find_map(|n| n.to_string())
            .filter(|s| !s.trim().is_empty())
    };
    let mut info: Vec<(&'static str, String)> = Vec::new();
    let family = name(name_id::TYPOGRAPHIC_FAMILY).or_else(|| name(name_id::FAMILY));
    for (label, value) in [
        (tr("Family"), family),
        (tr("Style"), name(name_id::SUBFAMILY)),
        (tr("Full name"), name(name_id::FULL_NAME)),
        (tr("Version"), name(name_id::VERSION)),
        (tr("Designer"), name(name_id::DESIGNER)),
        (tr("Foundry"), name(name_id::MANUFACTURER)),
    ] {
        if let Some(v) = value {
            info.push((label, v));
        }
    }
    info.push((tr("Weight"), face.weight().to_number().to_string()));
    let mut traits = Vec::new();
    if face.is_italic() {
        traits.push(tr("italic"));
    }
    if face.is_monospaced() {
        traits.push(tr("monospaced"));
    }
    if face.is_variable() {
        traits.push(tr("variable"));
    }
    if !traits.is_empty() {
        info.push((tr("Traits"), traits.join(", ")));
    }
    if let Some(n) = ttf_parser::fonts_in_collection(&data).filter(|n| *n > 1) {
        info.push((
            tr("Fonts in file"),
            ox_core::trf!("{n} (showing the first)", n = n),
        ));
    }

    let mut chars = std::collections::BTreeSet::new();
    if let Some(cmap) = face.tables().cmap {
        for sub in cmap.subtables {
            if !sub.is_unicode() {
                continue;
            }
            sub.codepoints(|cp| {
                let has_glyph = sub.glyph_index(cp).is_some_and(|g| g.0 != 0);
                if let Some(c) = char::from_u32(cp).filter(|c| has_glyph && !c.is_control()) {
                    chars.insert(c);
                }
            });
        }
    }
    info.push((tr("Glyphs"), face.number_of_glyphs().to_string()));
    info.push((tr("Characters"), chars.len().to_string()));
    if let Some(c) = name(name_id::COPYRIGHT_NOTICE) {
        info.push((tr("Copyright"), c));
    }
    Some(Loaded::Font {
        data,
        info,
        chars: chars.into_iter().collect(),
    })
}
