//! Preview pane: shows the selected file (image, text/code) or a summary.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crossbeam_channel::Receiver;
use egui::{ColorImage, RichText, TextureHandle, TextureOptions, Ui};
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

enum Loaded {
    Image(ColorImage, [u32; 2]),
    Text {
        text: String,
        truncated: bool,
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
    Text {
        text: String,
        truncated: bool,
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

fn is_text(e: &FileEntry) -> bool {
    matches!(e.kind(), FileKind::Text | FileKind::Code)
        || matches!(
            e.extension().as_str(),
            "csv" | "tsv" | "srt" | "gitignore" | "env" | "lock"
        )
}

fn load(entry: FileEntry, show_hidden: bool) -> Loaded {
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
    if is_image(&entry) {
        if entry.size > MAX_IMAGE_BYTES {
            return Loaded::Error("The image is too large to preview.".into());
        }
        return match image::open(&entry.path) {
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
    if buf.iter().take(8192).any(|&b| b == 0) {
        return Loaded::Nothing; // binary
    }
    let truncated = buf.len() > MAX_TEXT_BYTES;
    buf.truncate(MAX_TEXT_BYTES);
    Loaded::Text {
        text: String::from_utf8_lossy(&buf).into_owned(),
        truncated,
    }
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
                    Loaded::Text { text, truncated } => Content::Text { text, truncated },
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
        Some(Content::Text { text, truncated }) => {
            let mut numbered = String::new();
            for (i, line) in text.lines().take(3000).enumerate() {
                numbered.push_str(&format!("{:>4}  {line}\n", i + 1));
            }
            // Long lines scroll sideways instead of widening the pane.
            egui::ScrollArea::horizontal()
                .id_salt("preview_text")
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(numbered).monospace().size(11.5))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
            if *truncated {
                ui.label(
                    RichText::new("Only the beginning of the file is shown.")
                        .weak()
                        .small(),
                );
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
