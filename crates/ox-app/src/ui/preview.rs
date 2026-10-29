//! Preview pane: shows the selected file (image, text/code) or a summary.

use ox_core::i18n::t as tr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::Receiver;
use egui::{ColorImage, RichText, TextureHandle, TextureOptions, Ui};
use egui_extras::syntax_highlighting::{highlight, CodeTheme};
use ox_core::dirsize::DirSize;
use ox_core::entry::{FileEntry, FileKind};
use ox_core::format;
use ox_core::git::{Commit, GitRef, GraphRow};

use crate::app::{Action, OxApp};

/// Largest file we try to decode as an image.
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// Longest image side after downscaling.
const MAX_IMAGE_SIDE: u32 = 1024;
/// How much of a text file is shown.
const MAX_TEXT_BYTES: usize = 128 * 1024;
/// How many lines of it.
const MAX_TEXT_LINES: usize = 3000;
/// Larger texts are shown without syntax highlighting.
const MAX_HIGHLIGHT_BYTES: usize = 64 * 1024;

enum Loaded {
    Image(ColorImage, [u32; 2]),
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
    },
    /// A font file: its bytes (installed into egui to draw it), details
    /// and every character it has a glyph for.
    Font {
        data: Vec<u8>,
        info: Vec<(&'static str, String)>,
        chars: Vec<char>,
    },
    Nothing,
    Error(String),
}

enum Content {
    Loading(Receiver<(Loaded, Option<Commit>)>),
    Image(TextureHandle, [u32; 2]),
    Page(TextureHandle, u32),
    Text {
        text: String,
        numbers: String,
        truncated: bool,
        /// Encoding, line endings, indentation.
        details: Vec<(&'static str, String)>,
        /// A Markdown file: also shown rendered.
        markdown: bool,
    },
    Media {
        info: Vec<(&'static str, String)>,
        cover: Option<TextureHandle>,
    },
    Folder {
        folders: usize,
        files: usize,
        bytes: u64,
        /// The history of the Git repository the folder is in.
        git: Option<Arc<GitInfo>>,
    },
    Font {
        /// egui font family the file was installed as.
        family: String,
        info: Vec<(&'static str, String)>,
        chars: Vec<char>,
    },
    Nothing,
    Error(String),
}

/// Tabs of the preview pane: Info first, then what the file type offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreviewTab {
    Info,
    Image,
    Markdown,
    Text,
    Page,
    Media,
    FontSample,
    Glyphs,
    Git,
}

impl PreviewTab {
    fn label(self) -> &'static str {
        match self {
            PreviewTab::Info => tr("ℹ Info"),
            PreviewTab::Image => tr("🖼 Image"),
            PreviewTab::Markdown => tr("📖 Markdown"),
            PreviewTab::Text => tr("📝 Text"),
            PreviewTab::Page => tr("📄 Page"),
            PreviewTab::Media => tr("🎵 Cover"),
            PreviewTab::FontSample => tr("Aa Preview"),
            PreviewTab::Glyphs => tr("🔣 Glyphs"),
            PreviewTab::Git => tr("🌿 Git"),
        }
    }
}

impl Content {
    fn tabs(&self) -> Vec<PreviewTab> {
        use PreviewTab::*;
        match self {
            Content::Image(..) => vec![Info, Image],
            Content::Page(..) => vec![Info, Page],
            Content::Text { markdown: true, .. } => vec![Info, Markdown, Text],
            Content::Text { .. } => vec![Info, Text],
            Content::Media { cover: Some(_), .. } => vec![Info, Media],
            Content::Font { .. } => vec![Info, FontSample, Glyphs],
            Content::Folder { git: Some(_), .. } => vec![Info, Git],
            _ => vec![Info],
        }
    }

    /// What the file type adds to the Info tab.
    fn facts(&self) -> Vec<(&'static str, String)> {
        match self {
            Content::Image(_, [w, h]) => vec![(tr("Dimensions"), format!("{w} × {h} px"))],
            Content::Page(_, pages) => vec![(tr("Pages"), pages.to_string())],
            Content::Text {
                numbers,
                truncated,
                details,
                ..
            } => {
                let lines = numbers.lines().count();
                let more = if *truncated { "+" } else { "" };
                let mut facts = vec![(tr("Lines"), format!("{lines}{more}"))];
                facts.extend(details.iter().cloned());
                facts
            }
            Content::Media { info, .. } | Content::Font { info, .. } => info.clone(),
            Content::Folder {
                folders,
                files,
                bytes,
                ..
            } => vec![(
                tr("Contains"),
                format!(
                    "{}, {} ({})",
                    ox_core::i18n::count(*folders as u64, "folder"),
                    ox_core::i18n::count(*files as u64, "file"),
                    format::size(*bytes)
                ),
            )],
            _ => Vec::new(),
        }
    }
}

/// The preview of one item, cached until the selection (or the file) changes.
#[derive(Default)]
pub struct PreviewState {
    key: Option<(PathBuf, Option<SystemTime>)>,
    content: Option<Content>,
    /// The tab last picked; used whenever the file offers it.
    wanted: Option<PreviewTab>,
    /// Text for the font preview.
    sample: String,
    /// The previewed font installed into egui (removed when the next loads).
    font_family: Option<String>,
    font_seq: u64,
    /// Layout cache of the Markdown view.
    markdown: egui_commonmark::CommonMarkCache,
    /// The last commit that changed the item, in Git repositories.
    last_commit: Option<Commit>,
    /// SHA-256 and MD5, once asked for.
    checksums: Option<Checksums>,
}

enum Checksums {
    Running(Receiver<Result<(String, String), String>>),
    Done(Result<(String, String), String>),
}

/// The last commit that changed `e` (not for items inside archives).
fn last_commit(e: &FileEntry) -> Option<Commit> {
    if ox_core::archive::split_virtual(&e.path).is_some() {
        return None;
    }
    let root = ox_core::git::repo_root(&e.path)?;
    ox_core::git::last_commit(&root, &e.path).ok().flatten()
}

fn is_markdown(e: &FileEntry) -> bool {
    matches!(e.extension().as_str(), "md" | "markdown" | "mdown" | "mkd")
}

fn is_image(e: &FileEntry) -> bool {
    matches!(
        e.extension().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tif" | "tiff"
    )
}

/// Code and config files get syntax highlighting (by extension).
fn highlightable(e: &FileEntry) -> bool {
    e.kind() == FileKind::Code
        || matches!(
            e.extension().as_str(),
            "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "cfg" | "md" | "csv" | "sql"
        )
}

fn is_text(e: &FileEntry) -> bool {
    matches!(e.kind(), FileKind::Text | FileKind::Code)
        || matches!(
            e.extension().as_str(),
            "csv" | "tsv" | "srt" | "gitignore" | "env" | "lock"
        )
}

fn load(entry: FileEntry, show_hidden: bool) -> Loaded {
    if let Some((arc, inner)) = ox_core::archive::split_virtual(&entry.path)
        .filter(|(_, inner)| !inner.as_os_str().is_empty())
    {
        return load_from_archive(&entry, &arc, &inner);
    }
    if entry.is_dir {
        return match ox_core::entry::list_dir(&entry.path, show_hidden) {
            Ok(items) => Loaded::Folder {
                folders: items.iter().filter(|i| i.is_dir).count(),
                files: items.iter().filter(|i| !i.is_dir).count(),
                bytes: items.iter().filter(|i| !i.is_dir).map(|i| i.size).sum(),
                git: git_info(&entry.path),
            },
            Err(e) => Loaded::Error(e.to_string()),
        };
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
                Loaded::Folder {
                    folders: folders.len(),
                    files: files.len(),
                    bytes: files.iter().map(|e| e.size).sum(),
                    git: None,
                }
            }
            Err(e) => Loaded::Error(ox_core::trf!("Can't read this archive: {e}", e = e)),
        };
    }
    if is_image(&entry) {
        if entry.size > MAX_IMAGE_BYTES {
            return Loaded::Error(tr("The image is too large to preview.").into());
        }
        return image_result(image::open(&entry.path));
    }
    if matches!(entry.kind(), FileKind::Audio | FileKind::Video) {
        if let Some(media) = media_info(&entry.path, entry.kind() == FileKind::Video) {
            return media;
        }
    }
    if entry.kind() == FileKind::Font {
        return font_info(&entry.path).unwrap_or(Loaded::Nothing);
    }
    if entry.kind() == FileKind::Pdf {
        return match ox_core::pdf::render_first_page(&entry.path, 800) {
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
    if is_text(&entry) || entry.size == 0 {
        return read_text(&entry.path);
    }
    // Unknown type: show it as text if it looks like text.
    match read_text(&entry.path) {
        t @ Loaded::Text { .. } => t,
        _ => Loaded::Nothing,
    }
}

fn image_result(img: image::ImageResult<image::DynamicImage>) -> Loaded {
    match img {
        Ok(img) => {
            let (w, h) = (img.width(), img.height());
            let img = if w.max(h) > MAX_IMAGE_SIDE {
                img.thumbnail(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE)
            } else {
                img
            };
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            Loaded::Image(
                ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                [w, h],
            )
        }
        Err(e) => Loaded::Error(ox_core::trf!("Can't read this image: {e}", e = e)),
    }
}

/// Preview of an item inside an archive, read without extracting it.
fn load_from_archive(entry: &FileEntry, arc: &Path, inner: &Path) -> Loaded {
    use ox_core::archive;
    if entry.is_dir {
        return match archive::list(arc, inner) {
            Ok(items) => Loaded::Folder {
                folders: items.iter().filter(|i| i.is_dir).count(),
                files: items.iter().filter(|i| !i.is_dir).count(),
                bytes: items.iter().filter(|i| !i.is_dir).map(|i| i.size).sum(),
                git: None,
            },
            Err(e) => Loaded::Error(e.to_string()),
        };
    }
    if is_image(entry) {
        if entry.size > MAX_IMAGE_BYTES {
            return Loaded::Error(tr("The image is too large to preview.").into());
        }
        return match archive::read_entry(arc, inner, MAX_IMAGE_BYTES) {
            Ok(bytes) => image_result(image::load_from_memory(&bytes)),
            Err(e) => Loaded::Error(e.to_string()),
        };
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

fn read_text(path: &Path) -> Loaded {
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

fn text_from_bytes(mut buf: Vec<u8>) -> Loaded {
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
fn media_info(path: &Path, video: bool) -> Option<Loaded> {
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
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;

/// Names, details and the character set of a TrueType/OpenType font.
fn font_info(path: &Path) -> Option<Loaded> {
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

impl PreviewState {
    /// Make sure the preview shows `entry`, starting a background load if needed.
    fn update(&mut self, ctx: &egui::Context, entry: &FileEntry, show_hidden: bool) {
        let key = (entry.path.clone(), entry.modified);
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            let (tx, rx) = crossbeam_channel::bounded(1);
            let entry = entry.clone();
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                let loaded = load(entry.clone(), show_hidden);
                let _ = tx.send((loaded, last_commit(&entry)));
                repaint.request_repaint();
            });
            self.content = Some(Content::Loading(rx));
            self.last_commit = None;
            self.checksums = None;
        }
        if let Some(Content::Loading(rx)) = &self.content {
            if let Ok((loaded, commit)) = rx.try_recv() {
                self.last_commit = commit;
                self.content = Some(match loaded {
                    Loaded::Image(img, size) => {
                        let tex = ctx.load_texture("preview", img, TextureOptions::LINEAR);
                        Content::Image(tex, size)
                    }
                    Loaded::Page(img, pages) => {
                        let tex = ctx.load_texture("preview", img, TextureOptions::LINEAR);
                        Content::Page(tex, pages)
                    }
                    Loaded::Text {
                        text,
                        numbers,
                        truncated,
                        details,
                    } => Content::Text {
                        text,
                        numbers,
                        truncated,
                        details,
                        markdown: is_markdown(entry),
                    },
                    Loaded::Media { info, cover } => Content::Media {
                        info,
                        cover: cover
                            .map(|c| ctx.load_texture("preview_cover", c, TextureOptions::LINEAR)),
                    },
                    Loaded::Folder {
                        folders,
                        files,
                        bytes,
                        git,
                    } => Content::Folder {
                        folders,
                        files,
                        bytes,
                        git,
                    },
                    Loaded::Font { data, info, chars } => Content::Font {
                        family: self.install_font(ctx, data),
                        info,
                        chars,
                    },
                    Loaded::Nothing => Content::Nothing,
                    Loaded::Error(e) => Content::Error(e),
                });
            }
        }
    }

    /// Make the font drawable under its own family name, replacing the
    /// previously previewed font so they don't pile up in memory.
    fn install_font(&mut self, ctx: &egui::Context, data: Vec<u8>) -> String {
        use egui::{FontData, FontFamily};
        self.font_seq += 1;
        let name = format!("oxplorer-preview-{}", self.font_seq);
        let mut defs = ctx.fonts_mut(|f| f.definitions().clone());
        if let Some(old) = self.font_family.take() {
            defs.font_data.remove(&old);
            defs.families.remove(&FontFamily::Name(old.into()));
        }
        defs.font_data.insert(
            name.clone(),
            std::sync::Arc::new(FontData::from_owned(data)),
        );
        defs.families
            .insert(FontFamily::Name(name.clone().into()), vec![name.clone()]);
        ctx.set_fonts(defs);
        self.font_family = Some(name.clone());
        name
    }

    fn clear(&mut self) {
        self.key = None;
        self.content = None;
    }
}

pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let selected: Vec<FileEntry> = app
        .tabs
        .active()
        .view
        .selected_entries()
        .into_iter()
        .cloned()
        .collect();
    let in_bin = app.tabs.active().location().is_bin();
    let date_fmt = app.settings.browsing.date_format.clone();
    let mut actions = Vec::new();

    ui.add_space(4.0);
    match selected.as_slice() {
        [] => {
            app.preview.clear();
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("👁").size(32.0).weak());
                ui.label(RichText::new(tr("Select a file to preview it.")).weak());
            });
        }
        [entry] if !in_bin => {
            app.preview
                .update(ui.ctx(), entry, app.settings.browsing.show_hidden);
            header(ui, entry);
            let total = entry
                .is_dir
                .then(|| app.folder_sizes.get(&entry.path))
                .flatten();
            tabbed(ui, &mut app.preview, entry, total, &date_fmt, &mut actions);
        }
        [entry] => {
            app.preview.clear();
            header(ui, entry);
            ui.separator();
            info_grid(ui, entry, Vec::new(), &date_fmt);
        }
        many => {
            app.preview.clear();
            let files: Vec<&FileEntry> = many.iter().filter(|e| !e.is_dir).collect();
            let bytes: u64 = files.iter().map(|e| e.size).sum();
            ui.label(
                RichText::new(ox_core::trf!(
                    "{v1} selected",
                    v1 = ox_core::i18n::count(many.len() as u64, "item")
                ))
                .strong()
                .size(15.0),
            );
            ui.label(format!(
                "{} · {} · {}",
                ox_core::i18n::count((many.len() - files.len()) as u64, "folder"),
                ox_core::i18n::count(files.len() as u64, "file"),
                format::size(bytes)
            ));
        }
    }
    app.actions.extend(actions);
}

fn header(ui: &mut Ui, e: &FileEntry) {
    ui.label(RichText::new(&e.name).strong().size(15.0));
    ui.label(RichText::new(e.type_label()).weak());
}

/// The tab row and the chosen tab's content.
fn tabbed(
    ui: &mut Ui,
    state: &mut PreviewState,
    e: &FileEntry,
    total: Option<DirSize>,
    date_fmt: &str,
    actions: &mut Vec<Action>,
) {
    let Some(content) = state
        .content
        .as_ref()
        .filter(|c| !matches!(c, Content::Loading(_)))
    else {
        ui.separator();
        ui.spinner();
        return;
    };
    let tabs = content.tabs();
    // The tab picked last, if this file has it; otherwise its main view.
    let tab = state
        .wanted
        .filter(|t| tabs.contains(t))
        .unwrap_or_else(|| tabs.get(1).copied().unwrap_or(PreviewTab::Info));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for t in &tabs {
            if ui.selectable_label(*t == tab, t.label()).clicked() {
                state.wanted = Some(*t);
            }
        }
    });
    ui.separator();

    let content = state.content.as_ref().expect("checked above");
    if let (PreviewTab::Glyphs, Content::Font { family, chars, .. }) = (tab, content) {
        glyph_grid(ui, family, chars, actions);
        return;
    }
    if let (PreviewTab::Git, Content::Folder { git: Some(git), .. }) = (tab, content) {
        git_graph(ui, git, date_fmt, actions);
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("preview_tab")
        .auto_shrink([false, false])
        .show(ui, |ui| match tab {
            PreviewTab::Info => {
                let mut facts = content.facts();
                if let Some(t) = total.filter(|t| t.folders > 0) {
                    facts.push((
                        tr("Total size"),
                        format!(
                            "{} ({}, {})",
                            format::size(t.bytes),
                            ox_core::i18n::count(t.files, "file"),
                            ox_core::i18n::count(t.folders, "folder")
                        ),
                    ));
                }
                if let Some(c) = &state.last_commit {
                    facts.push((
                        tr("Last commit"),
                        format!(
                            "{} {}\n{}, {}",
                            c.short(),
                            c.subject,
                            c.author,
                            commit_time(c, date_fmt)
                        ),
                    ));
                }
                info_grid(ui, e, facts, date_fmt);
                if !e.is_dir && ox_core::archive::split_virtual(&e.path).is_none() {
                    ui.add_space(6.0);
                    checksums(ui, &mut state.checksums, e, actions);
                }
                ui.add_space(8.0);
                let label = match content {
                    Content::Media { .. } => tr("▶ Play"),
                    Content::Folder { .. } => tr("📂 Open"),
                    _ => tr("📂 Open with default app"),
                };
                if ui.button(label).clicked() {
                    actions.push(Action::Open(e.path.clone()));
                }
                if let Content::Error(err) = content {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), err);
                }
            }
            PreviewTab::FontSample => {
                if let Content::Font { family, .. } = content {
                    font_sample(ui, family, &mut state.sample);
                }
            }
            PreviewTab::Markdown => {
                if let Content::Text { text, .. } = content {
                    markdown(ui, &mut state.markdown, text, e);
                }
            }
            _ => body(ui, content, e),
        });
}

/// Name, size, place, dates and attributes, then the type's own details.
fn info_grid(ui: &mut Ui, e: &FileEntry, facts: Vec<(&'static str, String)>, date_fmt: &str) {
    let mut rows: Vec<(&str, String)> = Vec::new();
    rows.push((tr("Name"), e.name.clone()));
    rows.push((tr("Type"), e.type_label()));
    if !e.is_dir {
        rows.push((
            tr("Size"),
            ox_core::trf!(
                "{v1} ({v2} bytes)",
                v1 = format::size(e.size),
                v2 = group_digits(e.size)
            ),
        ));
    }
    if let Some(parent) = e.path.parent() {
        rows.push((tr("Location"), parent.display().to_string()));
    }
    if let Some(t) = e.modified {
        rows.push((tr("Modified"), format::time(t, date_fmt)));
    }
    if let Some(t) = e.created {
        rows.push((tr("Created"), format::time(t, date_fmt)));
    }
    let mut attrs = Vec::new();
    if e.hidden {
        attrs.push(tr("hidden"));
    }
    if e.readonly {
        attrs.push(tr("read-only"));
    }
    if e.is_symlink {
        attrs.push(tr("link"));
    }
    if !attrs.is_empty() {
        rows.push((tr("Attributes"), attrs.join(", ")));
    }
    rows.extend(facts);
    egui::Grid::new("preview_info")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 4.0))
        .show(ui, |ui| {
            for (name, value) in rows {
                ui.label(RichText::new(name).weak());
                ui.add(egui::Label::new(value).wrap());
                ui.end_row();
            }
        });
}

/// 1234567 → "1,234,567".
fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

const SAMPLE: &str = "The quick brown fox jumps over the lazy dog";

/// Editable sample text at several sizes, plus the alphabet.
fn font_sample(ui: &mut Ui, family: &str, sample: &mut String) {
    use egui::{FontFamily, FontId};
    if sample.is_empty() {
        *sample = tr(SAMPLE).to_owned();
    }
    ui.add(
        egui::TextEdit::singleline(sample)
            .hint_text(tr("Type to try the font"))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(6.0);
    let font = |size: f32| FontId::new(size, FontFamily::Name(family.into()));
    ui.label(
        RichText::new(
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ\nabcdefghijklmnopqrstuvwxyz\n0123456789 .,;:!?&@#%*()[]{}",
        )
        .font(font(18.0)),
    );
    for size in [12.0, 16.0, 20.0, 28.0, 36.0, 48.0, 64.0] {
        ui.add_space(6.0);
        ui.label(RichText::new(format!("{size:.0} px")).weak().small());
        ui.add(egui::Label::new(RichText::new(sample.as_str()).font(font(size))).wrap());
    }
}

/// Every character of the font in a grid; only visible rows are drawn.
/// Clicking a glyph copies it.
fn glyph_grid(ui: &mut Ui, family: &str, chars: &[char], actions: &mut Vec<Action>) {
    use egui::{Align2, FontFamily, FontId, Sense};
    const CELL: egui::Vec2 = egui::vec2(44.0, 48.0);
    ui.label(
        RichText::new(ox_core::trf!(
            "{v1} · click one to copy it",
            v1 = ox_core::i18n::count(chars.len() as u64, "character")
        ))
        .weak()
        .small(),
    );
    let cols = ((ui.available_width() / CELL.x).floor() as usize).max(1);
    let rows = chars.len().div_ceil(cols);
    let font = FontId::new(24.0, FontFamily::Name(family.into()));
    let v = ui.visuals().clone();
    egui::ScrollArea::vertical()
        .id_salt("preview_glyphs")
        .auto_shrink([false, false])
        .show_rows(ui, CELL.y, rows, |ui, range| {
            for row in range {
                let (line, _) = ui
                    .allocate_exact_size(egui::vec2(cols as f32 * CELL.x, CELL.y), Sense::hover());
                for col in 0..cols {
                    let Some(&c) = chars.get(row * cols + col) else {
                        break;
                    };
                    let rect = egui::Rect::from_min_size(
                        line.min + egui::vec2(col as f32 * CELL.x, 0.0),
                        CELL,
                    )
                    .shrink(2.0);
                    let id = egui::Id::unique(("glyph", row, col));
                    let resp = ui.interact(rect, id, Sense::click());
                    let fill = if resp.hovered() {
                        v.widgets.hovered.weak_bg_fill
                    } else {
                        v.faint_bg_color
                    };
                    ui.painter().rect_filled(rect, 4.0, fill);
                    ui.painter().text(
                        rect.center() - egui::vec2(0.0, 5.0),
                        Align2::CENTER_CENTER,
                        c,
                        font.clone(),
                        v.text_color(),
                    );
                    ui.painter().text(
                        rect.center_bottom() - egui::vec2(0.0, 6.0),
                        Align2::CENTER_CENTER,
                        format!("{:04X}", c as u32),
                        FontId::monospace(8.0),
                        v.weak_text_color(),
                    );
                    let resp = resp.on_hover_text(format!("U+{:04X}", c as u32));
                    if resp.clicked() {
                        ui.ctx().copy_text(c.to_string());
                        actions.push(Action::Toast(ox_core::trf!(
                            "Copied “{c}” ({code})",
                            c = c,
                            code = format!("U+{:04X}", c as u32)
                        )));
                    }
                }
            }
        });
}

/// A Markdown file rendered. Relative image paths are taken from its folder.
fn markdown(ui: &mut Ui, cache: &mut egui_commonmark::CommonMarkCache, text: &str, e: &FileEntry) {
    let folder = e
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    egui_commonmark::CommonMarkViewer::new()
        .default_implicit_uri_scheme(format!("file://{folder}/"))
        .max_image_width(Some(ui.available_width().max(50.0) as usize))
        .show(ui, cache, text);
}

/// The type-specific tabs (Image, Text, Page, Cover).
fn body(ui: &mut Ui, content: &Content, e: &FileEntry) {
    match content {
        Content::Image(tex, _) => {
            let avail = ui.available_width();
            let size = tex.size_vec2();
            let scale = (avail / size.x).min(1.5);
            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
        }
        Content::Text {
            text,
            numbers,
            truncated,
            ..
        } => {
            const SIZE: f32 = 11.5;
            // Long lines scroll sideways instead of widening the pane.
            egui::ScrollArea::horizontal()
                .id_salt("preview_text")
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        ui.label(RichText::new(numbers).monospace().size(SIZE).weak());
                        let code = if highlightable(e) && text.len() <= MAX_HIGHLIGHT_BYTES {
                            let theme = if ui.visuals().dark_mode {
                                CodeTheme::dark(SIZE)
                            } else {
                                CodeTheme::light(SIZE)
                            };
                            let job = highlight(ui.ctx(), ui.style(), &theme, text, &e.extension());
                            egui::Label::new(job)
                        } else {
                            egui::Label::new(RichText::new(text.as_str()).monospace().size(SIZE))
                        };
                        ui.add(code.wrap_mode(egui::TextWrapMode::Extend));
                    });
                });
            if *truncated {
                ui.label(
                    RichText::new(tr("Only the beginning of the file is shown."))
                        .weak()
                        .small(),
                );
            }
        }
        Content::Page(tex, pages) => {
            let avail = ui.available_width();
            let size = tex.size_vec2();
            ui.add(egui::Image::new((tex.id(), size * (avail / size.x))).corner_radius(4.0));
            ui.label(
                RichText::new(ox_core::trf!("Page 1 of {pages}", pages = pages))
                    .weak()
                    .small(),
            );
        }
        Content::Media {
            cover: Some(tex), ..
        } => {
            let avail = ui.available_width();
            let size = tex.size_vec2();
            let scale = (avail / size.x).min(1.0);
            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
        }
        _ => {}
    }
}

/// How many commits the Git tab shows.
const MAX_COMMITS: usize = 500;

/// A repository's recent history, laid out as a graph.
pub struct GitInfo {
    root: PathBuf,
    /// e.g. "main ↑2 ↓1".
    branch: Option<String>,
    changed: usize,
    commits: Vec<Commit>,
    rows: Vec<GraphRow>,
    lanes: usize,
    /// The repository's page on GitHub, GitLab, … (from its remote).
    web: Option<String>,
}

/// The history of the repository `dir` is in. Kept for a few seconds, so
/// going through the folders of one repository doesn't run `git log` for
/// each of them.
fn git_info(dir: &Path) -> Option<Arc<GitInfo>> {
    static CACHE: Mutex<Option<(Instant, Arc<GitInfo>)>> = Mutex::new(None);
    const KEEP: Duration = Duration::from_secs(10);
    let root = ox_core::git::repo_root(dir)?;
    if let Some((at, info)) = CACHE.lock().ok()?.as_ref() {
        if info.root == root && at.elapsed() < KEEP {
            return Some(info.clone());
        }
    }
    let commits = ox_core::git::log(&root, MAX_COMMITS).ok()?;
    let status = ox_core::git::status(&root).ok();
    let (rows, lanes) = ox_core::git::graph(&commits);
    let web = ox_core::git::remote_url(&root)
        .ok()
        .flatten()
        .and_then(|r| ox_core::git::web_url(&r));
    let info = Arc::new(GitInfo {
        web,
        branch: status.as_ref().and_then(|s| s.branch_label()),
        changed: status.as_ref().map_or(0, |s| s.changed_files()),
        root,
        commits,
        rows,
        lanes,
    });
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), info.clone()));
    }
    Some(info)
}

/// Colours of the graph's lanes, repeating.
const LANE_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0x4c, 0x9a, 0xff),
    egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
    egui::Color32::from_rgb(0x22, 0xc5, 0x5e),
    egui::Color32::from_rgb(0xe1, 0x4d, 0x8a),
    egui::Color32::from_rgb(0x8b, 0x5c, 0xf6),
    egui::Color32::from_rgb(0x14, 0xb8, 0xa6),
    egui::Color32::from_rgb(0xef, 0x44, 0x44),
    egui::Color32::from_rgb(0xa3, 0xa3, 0x3a),
];

fn lane_color(lane: usize) -> egui::Color32 {
    LANE_COLORS[lane % LANE_COLORS.len()]
}

/// The repository's branch and history as a commit graph: one row per
/// commit with its branches and tags, message, author and date. Clicking a
/// commit copies its hash.
fn git_graph(ui: &mut Ui, git: &GitInfo, date_fmt: &str, actions: &mut Vec<Action>) {
    use egui::{epaint::CubicBezierShape, pos2, vec2, Sense, Stroke};
    const LANE_W: f32 = 14.0;
    const ROW_H: f32 = 38.0;
    /// More lanes than this are cut off at the right.
    const MAX_LANES: usize = 8;

    let name = git
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| git.root.display().to_string());
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("📦 {name}")).strong());
        if let Some(branch) = &git.branch {
            let chip =
                egui::Button::new(RichText::new(format!("🌿 {branch}")).color(lane_color(0)))
                    .frame(false);
            if ui.add(chip).on_hover_text(tr("Copy branch name")).clicked() {
                actions.push(Action::CopyGitBranch(git.root.clone()));
            }
        }
        if let Some(web) = &git.web {
            let host = web
                .trim_start_matches("https://")
                .split('/')
                .next()
                .unwrap_or_default();
            if ui
                .small_button(format!("🌐 {host}"))
                .on_hover_text(web.as_str())
                .clicked()
            {
                actions.push(Action::OpenGitRemote(git.root.clone()));
            }
        }
    });
    let changes = if git.changed == 0 {
        tr("No changes").to_owned()
    } else {
        ox_core::trf!(
            "{v1} changed",
            v1 = ox_core::i18n::count(git.changed as u64, "file")
        )
    };
    ui.label(RichText::new(changes).weak().small());
    if git.commits.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new(tr("No commits yet.")).weak());
        return;
    }
    ui.add_space(4.0);

    let graph_w = git.lanes.min(MAX_LANES) as f32 * LANE_W + 6.0;
    let v = ui.visuals().clone();
    let bg = v.panel_fill;
    egui::ScrollArea::vertical()
        .id_salt("preview_git")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, git.commits.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let (c, row) = (&git.commits[i], &git.rows[i]);
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let painter = ui.painter_at(rect);
                if resp.hovered() {
                    painter.rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
                }
                // The graph.
                let x = |lane: usize| rect.left() + 3.0 + LANE_W * (lane as f32 + 0.5);
                let (top, mid, bottom) = (rect.top(), rect.center().y, rect.bottom());
                let graph = painter.with_clip_rect(rect.with_max_x(rect.left() + graph_w));
                let stroke = |lane: usize| Stroke::new(2.0, lane_color(lane));
                let curve = |from: egui::Pos2, to: egui::Pos2, lane: usize| {
                    if (from.x - to.x).abs() < 0.5 {
                        graph.line_segment([from, to], stroke(lane));
                    } else {
                        let h = (to.y - from.y) / 2.0;
                        graph.add(CubicBezierShape::from_points_stroke(
                            [from, pos2(from.x, from.y + h), pos2(to.x, to.y - h), to],
                            false,
                            egui::Color32::TRANSPARENT,
                            stroke(lane),
                        ));
                    }
                };
                for &l in &row.through {
                    graph.line_segment([pos2(x(l), top), pos2(x(l), bottom)], stroke(l));
                }
                for &l in &row.from_above {
                    curve(pos2(x(l), top), pos2(x(row.lane), mid), l);
                }
                for &l in &row.to_below {
                    curve(pos2(x(row.lane), mid), pos2(x(l), bottom), l);
                }
                let dot = pos2(x(row.lane), mid);
                let color = lane_color(row.lane);
                if c.parents.len() > 1 {
                    // Merges: a ring.
                    graph.circle(dot, 4.0, bg, Stroke::new(2.0, color));
                } else {
                    graph.circle(dot, 4.5, color, Stroke::new(1.5, bg));
                }

                // Branches, tags and the message, then hash, author and date.
                let text_x = rect.left() + graph_w + 4.0;
                let text_w = (rect.right() - text_x - 4.0).max(10.0);
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = text_w;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                let font = egui::FontId::proportional(12.5);
                for r in &c.refs {
                    // A remote branch next to its local one says nothing new.
                    if let GitRef::Remote(name) = r {
                        let local = name.split_once('/').map_or("", |(_, b)| b);
                        if c.refs
                            .iter()
                            .any(|o| matches!(o, GitRef::Head(n) | GitRef::Branch(n) if n == local))
                        {
                            continue;
                        }
                    }
                    let (label, color) = ref_style(r);
                    job.append(
                        &format!(" {label} "),
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(11.0),
                            color: egui::Color32::WHITE,
                            background: color.gamma_multiply(0.85),
                            ..Default::default()
                        },
                    );
                    job.append(
                        " ",
                        0.0,
                        egui::TextFormat::simple(font.clone(), v.text_color()),
                    );
                }
                job.append(
                    &c.subject,
                    0.0,
                    egui::TextFormat::simple(font, v.text_color()),
                );
                let galley = painter.layout_job(job);
                painter.galley(pos2(text_x, top + 4.0), galley, v.text_color());
                let when = commit_time(c, date_fmt);
                painter.text(
                    pos2(text_x, top + 21.0),
                    egui::Align2::LEFT_TOP,
                    format!("{} · {} · {when}", c.short(), c.author),
                    egui::FontId::proportional(11.0),
                    v.weak_text_color(),
                );

                let resp = resp.on_hover_ui(|ui| {
                    ui.set_max_width(360.0);
                    ui.label(RichText::new(&c.subject).strong());
                    if !c.refs.is_empty() {
                        let names: Vec<&str> = c.refs.iter().map(GitRef::name).collect();
                        ui.label(names.join(", "));
                    }
                    ui.label(RichText::new(&c.hash).monospace().small());
                    ui.label(format!("{} · {when}", c.author));
                    ui.label(RichText::new(tr("Click to copy the hash.")).weak().small());
                });
                if resp.clicked() {
                    ui.ctx().copy_text(c.hash.clone());
                    actions.push(Action::Toast(ox_core::trf!(
                        "Copied {what}",
                        what = c.short()
                    )));
                }
                resp.context_menu(|ui| {
                    if ui.button(tr("Copy commit hash")).clicked() {
                        ui.ctx().copy_text(c.hash.clone());
                        ui.close();
                    }
                    if ui.button(tr("Copy message")).clicked() {
                        ui.ctx().copy_text(c.subject.clone());
                        ui.close();
                    }
                });
            }
        });
    if git.commits.len() >= MAX_COMMITS {
        ui.label(
            RichText::new(ox_core::trf!(
                "The newest {n} commits are shown.",
                n = MAX_COMMITS
            ))
            .weak()
            .small(),
        );
    }
}

/// A ref's chip text and colour: the current branch, tags, remote branches
/// and local branches each look different.
fn ref_style(r: &GitRef) -> (String, egui::Color32) {
    use egui::Color32;
    match r {
        GitRef::Head(b) => (format!("● {b}"), Color32::from_rgb(0x16, 0xa3, 0x4a)),
        GitRef::Detached => ("HEAD".into(), Color32::from_rgb(0x16, 0xa3, 0x4a)),
        GitRef::Branch(b) => (b.clone(), Color32::from_rgb(0x25, 0x63, 0xeb)),
        GitRef::Remote(b) => (b.clone(), Color32::from_rgb(0x6b, 0x72, 0x80)),
        GitRef::Tag(t) => (format!("🏷 {t}"), Color32::from_rgb(0xb4, 0x53, 0x09)),
    }
}

fn commit_time(c: &Commit, date_fmt: &str) -> String {
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(c.time.max(0) as u64);
    format::time(t, date_fmt)
}

/// SHA-256 and MD5 of the file, computed when asked for (big files take a
/// while), with buttons to copy them.
fn checksums(ui: &mut Ui, state: &mut Option<Checksums>, e: &FileEntry, actions: &mut Vec<Action>) {
    if let Some(Checksums::Running(rx)) = state {
        if let Ok(result) = rx.try_recv() {
            *state = Some(Checksums::Done(result));
        }
    }
    match state {
        None => {
            if ui
                .button(tr("🔐 Compute checksums"))
                .on_hover_text(tr("SHA-256 and MD5 of the file"))
                .clicked()
            {
                let (tx, rx) = crossbeam_channel::bounded(1);
                let path = e.path.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let result = ox_core::textinfo::checksums(&path).map_err(|e| e.to_string());
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
                *state = Some(Checksums::Running(rx));
            }
        }
        Some(Checksums::Running(_)) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr("Computing checksums…")).weak());
            });
        }
        Some(Checksums::Done(Err(err))) => {
            ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), err.as_str());
        }
        Some(Checksums::Done(Ok((sha, md5)))) => {
            for (name, value) in [("SHA-256", sha), ("MD5", md5)] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).weak());
                    if ui.small_button("📋").on_hover_text(tr("Copy")).clicked() {
                        ui.ctx().copy_text(value.clone());
                        actions.push(Action::Toast(ox_core::trf!("Copied {what}", what = name)));
                    }
                });
                ui.add(
                    egui::Label::new(RichText::new(value.as_str()).monospace().size(10.5))
                        .wrap_mode(egui::TextWrapMode::Wrap),
                );
            }
        }
    }
}
