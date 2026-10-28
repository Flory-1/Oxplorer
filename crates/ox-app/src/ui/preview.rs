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
    Nothing,
    Error(String),
}

/// The preview of one item, cached until the selection (or the file) changes.
#[derive(Default)]
pub struct PreviewState {
    key: Option<(PathBuf, Option<SystemTime>)>,
    content: Option<Content>,
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
                    Loaded::Nothing => Content::Nothing,
                    Loaded::Error(e) => Content::Error(e),
                });
            }
        }
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

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
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
                    header(ui, entry, &date_fmt);
                    ui.separator();
                    let total = entry
                        .is_dir
                        .then(|| app.folder_sizes.get(&entry.path))
                        .flatten();
                    body(ui, &app.preview, entry, total, &mut actions);
                }
                [entry] => {
                    app.preview.clear();
                    header(ui, entry, &date_fmt);
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
        });
    app.actions.extend(actions);
}

fn header(ui: &mut Ui, e: &FileEntry, date_fmt: &str) {
    ui.label(RichText::new(&e.name).strong().size(15.0));
    ui.label(RichText::new(e.type_label()).weak());
    let mut info = Vec::new();
    if !e.is_dir {
        info.push(format::size(e.size));
    }
    if let Some(t) = e.modified {
        info.push(format!("modified {}", format::time(t, date_fmt)));
    }
    ui.label(RichText::new(info.join(" · ")).weak().small());
}

fn body(
    ui: &mut Ui,
    state: &PreviewState,
    e: &FileEntry,
    total: Option<DirSize>,
    actions: &mut Vec<Action>,
) {
    match &state.content {
        None | Some(Content::Loading(_)) => {
            ui.spinner();
        }
        Some(Content::Image(tex, [w, h])) => {
            let avail = ui.available_width();
            let size = tex.size_vec2();
            let scale = (avail / size.x).min(1.5);
            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
            ui.label(RichText::new(format!("{w} × {h} px")).weak().small());
        }
        Some(Content::Text {
            text,
            numbers,
            truncated,
        }) => {
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
        Some(Content::Page(tex, pages)) => {
            let avail = ui.available_width();
            let size = tex.size_vec2();
            ui.add(egui::Image::new((tex.id(), size * (avail / size.x))).corner_radius(4.0));
            ui.label(RichText::new(format!("Page 1 of {pages}")).weak().small());
        }
        Some(Content::Media { info, cover }) => {
            if let Some(tex) = cover {
                let avail = ui.available_width();
                let size = tex.size_vec2();
                // Small enough to leave room for the details below.
                let scale = (avail / size.x).min(220.0 / size.y).min(1.0);
                ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
                ui.add_space(6.0);
            }
            egui::Grid::new("media_info")
                .num_columns(2)
                .spacing(egui::vec2(12.0, 4.0))
                .show(ui, |ui| {
                    for (name, value) in info {
                        ui.label(RichText::new(*name).weak());
                        ui.label(value.as_str());
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            if ui.button("▶ Play").clicked() {
                actions.push(Action::Open(e.path.clone()));
            }
        }
        Some(Content::Folder {
            folders,
            files,
            bytes,
        }) => {
            ui.label(format!(
                "Contains {} and {} ({}).",
                format::count(*folders as u64, "folder"),
                format::count(*files as u64, "file"),
                format::size(*bytes)
            ));
            if let Some(t) = total.filter(|t| t.folders > 0) {
                ui.label(format!(
                    "Total with sub-folders: {} ({}, {})",
                    format::size(t.bytes),
                    format::count(t.files, "file"),
                    format::count(t.folders, "folder")
                ));
            }
            if ui.button("📂 Open").clicked() {
                actions.push(Action::Open(e.path.clone()));
            }
        }
        Some(Content::Nothing) => {
            ui.label(RichText::new("No preview available for this file type.").weak());
            if ui.button("📂 Open with default app").clicked() {
                actions.push(Action::Open(e.path.clone()));
            }
        }
        Some(Content::Error(err)) => {
            ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), err);
        }
    }
}
