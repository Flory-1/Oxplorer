//! Preview pane: shows the selected file (image, text/code) or a summary.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crossbeam_channel::Receiver;
use egui::{ColorImage, RichText, TextureHandle, TextureOptions, Ui};
use egui_extras::syntax_highlighting::{highlight, CodeTheme};
use ox_core::dirsize::DirSize;
use ox_core::entry::{FileEntry, FileKind};
use ox_core::format;

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
    Loading(Receiver<Loaded>),
    Image(TextureHandle, [u32; 2]),
    Page(TextureHandle, u32),
    Text {
        text: String,
        numbers: String,
        truncated: bool,
    },
    Media {
        info: Vec<(&'static str, String)>,
        cover: Option<TextureHandle>,
    },
    Folder {
        folders: usize,
        files: usize,
        bytes: u64,
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
    Text,
    Page,
    Media,
    FontSample,
    Glyphs,
}

impl PreviewTab {
    fn label(self) -> &'static str {
        match self {
            PreviewTab::Info => "ℹ Info",
            PreviewTab::Image => "🖼 Image",
            PreviewTab::Text => "📝 Text",
            PreviewTab::Page => "📄 Page",
            PreviewTab::Media => "🎵 Cover",
            PreviewTab::FontSample => "Aa Preview",
            PreviewTab::Glyphs => "🔣 Glyphs",
        }
    }
}

impl Content {
    fn tabs(&self) -> Vec<PreviewTab> {
        use PreviewTab::*;
        match self {
            Content::Image(..) => vec![Info, Image],
            Content::Page(..) => vec![Info, Page],
            Content::Text { .. } => vec![Info, Text],
            Content::Media { cover: Some(_), .. } => vec![Info, Media],
            Content::Font { .. } => vec![Info, FontSample, Glyphs],
            _ => vec![Info],
        }
    }

    /// What the file type adds to the Info tab.
    fn facts(&self) -> Vec<(&'static str, String)> {
        match self {
            Content::Image(_, [w, h]) => vec![("Dimensions", format!("{w} × {h} px"))],
            Content::Page(_, pages) => vec![("Pages", pages.to_string())],
            Content::Text {
                numbers, truncated, ..
            } => {
                let lines = numbers.lines().count();
                let more = if *truncated { "+" } else { "" };
                vec![("Lines", format!("{lines}{more}"))]
            }
            Content::Media { info, .. } | Content::Font { info, .. } => info.clone(),
            Content::Folder {
                folders,
                files,
                bytes,
            } => vec![(
                "Contains",
                format!(
                    "{}, {} ({})",
                    format::count(*folders as u64, "folder"),
                    format::count(*files as u64, "file"),
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
                }
            }
            Err(e) => Loaded::Error(format!("Can't read this archive: {e}")),
        };
    }
    if is_image(&entry) {
        if entry.size > MAX_IMAGE_BYTES {
            return Loaded::Error("The image is too large to preview.".into());
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
            Err(e) => Loaded::Error(format!("Can't show this PDF: {e}")),
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
        Err(e) => Loaded::Error(format!("Can't read this image: {e}")),
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
            },
            Err(e) => Loaded::Error(e.to_string()),
        };
    }
    if is_image(entry) {
        if entry.size > MAX_IMAGE_BYTES {
            return Loaded::Error("The image is too large to preview.".into());
        }
        return match archive::read_entry(arc, inner, MAX_IMAGE_BYTES) {
            Ok(bytes) => image_result(image::load_from_memory(&bytes)),
            Err(e) => Loaded::Error(e.to_string()),
        };
    }
    match archive::read_entry(arc, inner, MAX_TEXT_BYTES as u64 + 1) {
        Ok(buf) => match text_from_bytes(buf) {
            t @ Loaded::Text { .. } => t,
            _ if is_text(entry) => Loaded::Error("This file isn't plain text.".into()),
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
    if buf.iter().take(8192).any(|&b| b == 0) {
        return Loaded::Nothing; // binary
    }
    let mut truncated = buf.len() > MAX_TEXT_BYTES;
    buf.truncate(MAX_TEXT_BYTES);
    let all = String::from_utf8_lossy(&buf);
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
            ("Title", tag.title()),
            ("Artist", tag.artist()),
            ("Album", tag.album()),
            ("Genre", tag.genre()),
        ];
        for (name, value) in fields {
            if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
                info.push((name, v.into_owned()));
            }
        }
        if let Some(date) = tag.date() {
            info.push(("Year", date.year.to_string()));
        }
        if let Some(track) = tag.track() {
            let total = tag
                .track_total()
                .map(|t| format!(" of {t}"))
                .unwrap_or_default();
            info.push(("Track", format!("{track}{total}")));
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
        info.push(("Length", length));
    }
    // For videos these describe the sound track.
    let (bitrate, rate_label) = if video {
        ("Audio bitrate", "Audio")
    } else {
        ("Bitrate", "Sample rate")
    };
    if let Some(b) = props.audio_bitrate().filter(|b| *b > 0) {
        info.push((bitrate, format!("{b} kbps")));
    }
    if let Some(rate) = props.sample_rate().filter(|r| *r > 0) {
        info.push((rate_label, format!("{:.1} kHz", rate as f32 / 1000.0)));
    }
    if let Some(ch) = props.channels() {
        let ch = match ch {
            1 => "Mono".to_owned(),
            2 => "Stereo".to_owned(),
            n => format!("{n} channels"),
        };
        info.push(("Channels", ch));
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
        ("Family", family),
        ("Style", name(name_id::SUBFAMILY)),
        ("Full name", name(name_id::FULL_NAME)),
        ("Version", name(name_id::VERSION)),
        ("Designer", name(name_id::DESIGNER)),
        ("Foundry", name(name_id::MANUFACTURER)),
    ] {
        if let Some(v) = value {
            info.push((label, v));
        }
    }
    info.push(("Weight", face.weight().to_number().to_string()));
    let mut traits = Vec::new();
    if face.is_italic() {
        traits.push("italic");
    }
    if face.is_monospaced() {
        traits.push("monospaced");
    }
    if face.is_variable() {
        traits.push("variable");
    }
    if !traits.is_empty() {
        info.push(("Traits", traits.join(", ")));
    }
    if let Some(n) = ttf_parser::fonts_in_collection(&data).filter(|n| *n > 1) {
        info.push(("Fonts in file", format!("{n} (showing the first)")));
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
    info.push(("Glyphs", face.number_of_glyphs().to_string()));
    info.push(("Characters", chars.len().to_string()));
    if let Some(c) = name(name_id::COPYRIGHT_NOTICE) {
        info.push(("Copyright", c));
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
                let _ = tx.send(load(entry, show_hidden));
                repaint.request_repaint();
            });
            self.content = Some(Content::Loading(rx));
        }
        if let Some(Content::Loading(rx)) = &self.content {
            if let Ok(loaded) = rx.try_recv() {
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
                    } => Content::Text {
                        text,
                        numbers,
                        truncated,
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
                    } => Content::Folder {
                        folders,
                        files,
                        bytes,
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
                ui.label(RichText::new("Select a file to preview it.").weak());
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
                RichText::new(format!(
                    "{} selected",
                    format::count(many.len() as u64, "item")
                ))
                .strong()
                .size(15.0),
            );
            ui.label(format!(
                "{} · {} · {}",
                format::count((many.len() - files.len()) as u64, "folder"),
                format::count(files.len() as u64, "file"),
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
    egui::ScrollArea::vertical()
        .id_salt("preview_tab")
        .auto_shrink([false, false])
        .show(ui, |ui| match tab {
            PreviewTab::Info => {
                let mut facts = content.facts();
                if let Some(t) = total.filter(|t| t.folders > 0) {
                    facts.push((
                        "Total size",
                        format!(
                            "{} ({}, {})",
                            format::size(t.bytes),
                            format::count(t.files, "file"),
                            format::count(t.folders, "folder")
                        ),
                    ));
                }
                info_grid(ui, e, facts, date_fmt);
                ui.add_space(8.0);
                let label = match content {
                    Content::Media { .. } => "▶ Play",
                    Content::Folder { .. } => "📂 Open",
                    _ => "📂 Open with default app",
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
            _ => body(ui, content, e),
        });
}

/// Name, size, place, dates and attributes, then the type's own details.
fn info_grid(ui: &mut Ui, e: &FileEntry, facts: Vec<(&'static str, String)>, date_fmt: &str) {
    let mut rows: Vec<(&str, String)> = Vec::new();
    rows.push(("Name", e.name.clone()));
    rows.push(("Type", e.type_label()));
    if !e.is_dir {
        rows.push((
            "Size",
            format!("{} ({} bytes)", format::size(e.size), group_digits(e.size)),
        ));
    }
    if let Some(parent) = e.path.parent() {
        rows.push(("Location", parent.display().to_string()));
    }
    if let Some(t) = e.modified {
        rows.push(("Modified", format::time(t, date_fmt)));
    }
    if let Some(t) = e.created {
        rows.push(("Created", format::time(t, date_fmt)));
    }
    let mut attrs = Vec::new();
    if e.hidden {
        attrs.push("hidden");
    }
    if e.readonly {
        attrs.push("read-only");
    }
    if e.is_symlink {
        attrs.push("link");
    }
    if !attrs.is_empty() {
        rows.push(("Attributes", attrs.join(", ")));
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
        *sample = SAMPLE.to_owned();
    }
    ui.add(
        egui::TextEdit::singleline(sample)
            .hint_text("Type to try the font")
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
        RichText::new(format!(
            "{} · click one to copy it",
            format::count(chars.len() as u64, "character")
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
                        actions.push(Action::Toast(format!("Copied “{c}” (U+{:04X})", c as u32)));
                    }
                }
            }
        });
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
                    RichText::new("Only the beginning of the file is shown.")
                        .weak()
                        .small(),
                );
            }
        }
        Content::Page(tex, pages) => {
            let avail = ui.available_width();
            let size = tex.size_vec2();
            ui.add(egui::Image::new((tex.id(), size * (avail / size.x))).corner_radius(4.0));
            ui.label(RichText::new(format!("Page 1 of {pages}")).weak().small());
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
