//! Picture tools: the "Picture tools" submenu, the Convert/Resize dialog and
//! the progress window of a running job.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Align2, Context, Id, RichText, Ui};
use ox_core::entry::display_name;
use ox_core::i18n::{count, t as tr};
use ox_core::picture_tools::{
    self as pt, Convert, Destination, OutputFormat, Preview, Resize, Transform,
};

use crate::app::{doing_pictures, Action, OxApp, PictureAction};

/// The icon of a rotation or flip.
pub fn icon(tf: Transform) -> &'static str {
    match tf {
        Transform::RotateRight => "↻",
        Transform::RotateLeft => "↺",
        Transform::Rotate180 => "🔄",
        Transform::FlipHorizontal => "⇄",
        Transform::FlipVertical => "⇅",
    }
}

/// Whether the picture tools work on all of `paths`.
pub fn all_pictures(paths: &[PathBuf]) -> bool {
    !paths.is_empty() && paths.iter().all(|p| pt::is_supported(p))
}

/// The "Picture tools" submenu, when every selected item is a picture.
pub fn menu(ui: &mut Ui, sel: &[PathBuf], actions: &mut Vec<Action>) {
    if !all_pictures(sel) {
        return;
    }
    ui.menu_button(tr("🖼 Picture tools"), |ui| {
        for tf in Transform::ALL {
            if tf == Transform::FlipHorizontal {
                ui.separator();
            }
            if ui.button(format!("{} {}", icon(tf), tf.label())).clicked() {
                actions.push(Action::Pictures(PictureAction::Transform(sel.to_vec(), tf)));
            }
        }
        ui.separator();
        if ui.button(tr("🔁 Convert to…")).clicked() {
            actions.push(Action::Pictures(PictureAction::Dialog(sel.to_vec(), false)));
        }
        if ui.button(tr("🖼 Resize…")).clicked() {
            actions.push(Action::Pictures(PictureAction::Dialog(sel.to_vec(), true)));
        }
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Percent,
    Fit,
    Longest,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Put {
    Beside,
    Folder,
    Replace,
}

/// The Convert/Resize dialog.
pub struct PictureDialog {
    paths: Vec<PathBuf>,
    format: OutputFormat,
    quality: u8,
    resize: bool,
    mode: Mode,
    percent: u32,
    width: u32,
    height: u32,
    keep_aspect: bool,
    longest: u32,
    enlarge: bool,
    put: Put,
    folder: String,
    /// Favorites and open folders, offered for "Into a folder".
    destinations: Vec<(String, PathBuf)>,
    /// What the first picture turns into, for the settings it was worked out for.
    preview: Option<(Convert, Option<Preview>)>,
}

impl PictureDialog {
    /// `last`: the settings used the last time; `resize`: opened as "Resize…".
    pub fn new(
        paths: Vec<PathBuf>,
        last: Convert,
        resize: bool,
        destinations: Vec<(String, PathBuf)>,
    ) -> Self {
        let first_dir = paths
            .first()
            .and_then(|p| p.parent())
            .unwrap_or(Path::new(""));
        let mut d = Self {
            format: last.format,
            quality: last.quality,
            resize,
            mode: Mode::Longest,
            percent: 50,
            width: 1920,
            height: 1080,
            keep_aspect: true,
            longest: 1600,
            enlarge: last.enlarge,
            put: Put::Beside,
            folder: first_dir.join(tr("Converted")).display().to_string(),
            destinations,
            preview: None,
            paths,
        };
        match last.resize {
            Some(Resize::Percent(p)) => (d.mode, d.percent) = (Mode::Percent, p),
            Some(Resize::Fit {
                width,
                height,
                keep_aspect,
            }) => {
                (d.mode, d.width, d.height) = (Mode::Fit, width, height);
                d.keep_aspect = keep_aspect;
            }
            Some(Resize::LongestSide(n)) => (d.mode, d.longest) = (Mode::Longest, n),
            None => {}
        }
        // Replacing the originals is never offered unasked.
        if let Destination::Folder(f) = last.destination {
            d.put = Put::Folder;
            d.folder = f.display().to_string();
        }
        if resize {
            // "Resize…" keeps the format unless asked otherwise.
            d.format = OutputFormat::Original;
        } else if d.format == OutputFormat::Original {
            d.format = OutputFormat::Jpeg;
        }
        d
    }

    /// The folder for "Into a folder" (relative to the first picture's).
    fn folder_path(&self) -> Option<PathBuf> {
        let f = self.folder.trim().trim_matches('"').trim();
        if f.is_empty() {
            return None;
        }
        let p = PathBuf::from(f);
        if p.is_absolute() {
            return Some(p);
        }
        Some(self.paths.first()?.parent()?.join(p))
    }

    fn settings(&self) -> Option<Convert> {
        let resize = self.resize.then_some(match self.mode {
            Mode::Percent => Resize::Percent(self.percent),
            Mode::Fit => Resize::Fit {
                width: self.width,
                height: self.height,
                keep_aspect: self.keep_aspect,
            },
            Mode::Longest => Resize::LongestSide(self.longest),
        });
        let destination = match self.put {
            Put::Beside => Destination::Beside,
            Put::Folder => Destination::Folder(self.folder_path()?),
            Put::Replace => Destination::Replace,
        };
        Some(Convert {
            format: self.format,
            quality: self.quality,
            resize,
            enlarge: self.enlarge,
            destination,
        })
    }

    /// What the first picture turns into (worked out again when the settings change).
    fn preview(&mut self, c: &Convert) -> Option<&Preview> {
        if self.preview.as_ref().is_none_or(|(p, _)| p != c) {
            let p = self.paths.first().and_then(|f| pt::preview(f, c));
            self.preview = Some((c.clone(), p));
        }
        self.preview.as_ref().and_then(|(_, p)| p.as_ref())
    }
}

/// Draw the dialog; returns whether it stays open.
pub fn dialog(ui: &mut Ui, d: &mut PictureDialog, actions: &mut Vec<Action>) -> bool {
    ui.set_width(480.0);
    ui.heading(if d.resize {
        tr("Resize pictures")
    } else {
        tr("Convert pictures")
    });
    ui.label(
        RichText::new(match d.paths.as_slice() {
            [one] => display_name(one),
            many => count(many.len() as u64, "picture"),
        })
        .weak(),
    );
    ui.add_space(6.0);
    egui::Grid::new("picture_tools")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr("Format"));
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    for f in OutputFormat::ALL {
                        ui.selectable_value(&mut d.format, f, f.label());
                    }
                });
                if matches!(d.format, OutputFormat::Jpeg | OutputFormat::Original) {
                    ui.horizontal(|ui| {
                        ui.label(tr("JPG quality"));
                        ui.add(egui::Slider::new(&mut d.quality, 1..=100));
                    });
                }
                if d.format == OutputFormat::WebP {
                    ui.label(
                        RichText::new(tr(
                            "WebP pictures are saved lossless: nothing is lost, but the files are larger than lossy WebP or JPG.",
                        ))
                        .weak(),
                    );
                }
            });
            ui.end_row();

            ui.label(tr("Size"));
            ui.vertical(|ui| size_settings(ui, d));
            ui.end_row();

            ui.label(tr("Save"));
            ui.vertical(|ui| destination_settings(ui, d));
            ui.end_row();
        });

    let settings = d.settings();
    ui.add_space(6.0);
    match &settings {
        Some(c) => {
            if let Some(p) = d.preview(c) {
                let (w, h) = p.size;
                let (nw, nh) = p.new_size;
                ui.label(RichText::new(format!("{w} × {h} → {nw} × {nh} · {}", p.new_name)).weak());
            }
        }
        None => {
            ui.label(RichText::new(tr("Choose a folder for the pictures.")).weak());
        }
    }
    ui.add_space(8.0);
    let mut keep = true;
    ui.horizontal(|ui| {
        let start = if d.resize {
            tr("Resize")
        } else {
            tr("Convert")
        };
        if ui
            .add_enabled(settings.is_some(), egui::Button::new(start))
            .clicked()
        {
            if let Some(c) = settings {
                actions.push(Action::Pictures(PictureAction::Convert(d.paths.clone(), c)));
            }
            keep = false;
        }
        if ui.button(tr("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}

fn size_settings(ui: &mut Ui, d: &mut PictureDialog) {
    ui.checkbox(&mut d.resize, tr("Resize"));
    if !d.resize {
        return;
    }
    let px = tr("px");
    ui.horizontal(|ui| {
        ui.radio_value(&mut d.mode, Mode::Longest, tr("Longest side"));
        if d.mode == Mode::Longest {
            ui.add(egui::DragValue::new(&mut d.longest).range(1..=65_535));
            ui.label(px);
        }
    });
    ui.horizontal(|ui| {
        ui.radio_value(&mut d.mode, Mode::Fit, tr("Fit within"));
        if d.mode == Mode::Fit {
            ui.add(egui::DragValue::new(&mut d.width).range(1..=65_535));
            ui.label("×");
            ui.add(egui::DragValue::new(&mut d.height).range(1..=65_535));
            ui.label(px);
        }
    });
    if d.mode == Mode::Fit {
        ui.checkbox(&mut d.keep_aspect, tr("Keep the aspect ratio"));
    }
    ui.horizontal(|ui| {
        ui.radio_value(&mut d.mode, Mode::Percent, tr("Percent"));
        if d.mode == Mode::Percent {
            ui.add(egui::DragValue::new(&mut d.percent).range(1..=1000));
            ui.label("%");
        }
    });
    let mut small_stay = !d.enlarge;
    ui.checkbox(&mut small_stay, tr("Don't enlarge smaller pictures"));
    d.enlarge = !small_stay;
}

fn destination_settings(ui: &mut Ui, d: &mut PictureDialog) {
    ui.radio_value(&mut d.put, Put::Beside, tr("Next to the originals"));
    ui.radio_value(&mut d.put, Put::Folder, tr("Into a folder"));
    if d.put == Put::Folder {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut d.folder).desired_width(320.0));
            if !d.destinations.is_empty() {
                ui.menu_button("⏷", |ui| {
                    for (label, path) in &d.destinations {
                        if ui.button(label).clicked() {
                            d.folder = path.display().to_string();
                        }
                    }
                })
                .response
                .on_hover_text(tr("Favorites and open folders"));
            }
        });
        ui.label(RichText::new(tr("It is made if it doesn't exist.")).weak());
    }
    ui.radio_value(&mut d.put, Put::Replace, tr("Replace the originals"));
    if d.put == Put::Replace {
        ui.label(
            RichText::new(tr(
                "The originals go to the Recycle Bin; Undo (Ctrl+Z) brings them back.",
            ))
            .weak(),
        );
    }
}

/// The progress of a running job (with Cancel), or what the last one
/// couldn't do.
pub fn progress(app: &mut OxApp, ctx: &Context) {
    let mut actions = Vec::new();
    if let Some(r) = &app.pictures.running {
        let (done, total) = (r.job.done(), r.job.total);
        egui::Window::new(tr("🖼 Picture tools"))
            .id(Id::unique("picture-tools-progress"))
            .anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(doing_pictures(r.kind)).strong());
                ui.add(
                    egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                        .desired_width(260.0)
                        .text(ox_core::trf!(
                            "{v1} of {v2}",
                            v1 = done,
                            v2 = count(total as u64, "picture")
                        )),
                );
                let cancelled = r.job.is_cancelled();
                let label = if cancelled {
                    tr("Stopping…")
                } else {
                    tr("Cancel")
                };
                if ui
                    .add_enabled(!cancelled, egui::Button::new(label))
                    .clicked()
                {
                    actions.push(Action::Pictures(PictureAction::Cancel));
                }
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    } else if let Some((title, lines)) = &app.pictures.failures {
        egui::Window::new(tr("🖼 Picture tools"))
            .id(Id::unique("picture-tools-failures"))
            .anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(title).strong());
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for line in lines.iter().take(500) {
                            ui.label(line);
                        }
                    });
                ui.add_space(4.0);
                if ui.button(tr("OK")).clicked() {
                    actions.push(Action::Pictures(PictureAction::DismissFailures));
                }
            });
    }
    app.actions.extend(actions);
}
