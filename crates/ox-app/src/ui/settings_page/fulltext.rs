//! The text index part of the Search page: on or off, its folders, the
//! largest file it reads, and what it holds.

use std::time::{Duration, SystemTime};

use egui::{Color32, RichText, Ui};
use ox_core::fulltext::{default_folders, FullTextState};
use ox_core::i18n::{count, t as tr};
use ox_core::{format, trf};

use super::{row, toggle};
use crate::app::OxApp;

pub(super) fn fulltext_section(ui: &mut Ui, app: &mut OxApp) {
    ui.add_space(14.0);
    ui.label(RichText::new(tr("Text index")).size(16.0).strong());
    ui.label(
        RichText::new(tr(
            "Keeps the words of documents so content: searches in these folders don't have to \
             read every file. Files the index doesn't know yet are still read.",
        ))
        .weak(),
    );
    ui.add_space(8.0);
    let ft = &mut app.settings.search.fulltext;
    toggle(
        ui,
        &mut ft.enabled,
        tr("Index the text of documents"),
        tr(
            "Text files, code, PDF, Word, Excel, PowerPoint and OpenDocument files. Read in the \
            background, paused while files are copied; online-only cloud files are left out.",
        ),
    );
    if !ft.enabled {
        return;
    }
    status(ui, app);

    let ft = &mut app.settings.search.fulltext;
    let defaults = default_folders()
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    row(
        ui,
        tr("Folders"),
        tr("One per line. Empty = Documents, Desktop and your code folders."),
        |ui| {
            let mut text = ft
                .folders
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let edit = egui::TextEdit::multiline(&mut text)
                .desired_rows(3)
                .desired_width(280.0)
                .hint_text(defaults);
            if ui.add(edit).changed() {
                ft.folders = text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(Into::into)
                    .collect();
            }
        },
    );
    row(
        ui,
        tr("Largest file"),
        tr("Larger files are read by each search, as without the index."),
        |ui| {
            ui.add(egui::Slider::new(&mut ft.max_file_mb, 1..=100).suffix(" MB"));
        },
    );
}

/// What the indexer is doing and what the index holds.
fn status(ui: &mut Ui, app: &mut OxApp) {
    let service = &app.fulltext.service;
    row(ui, tr("Status"), "", |ui| {
        ui.vertical(|ui| {
            match service.state() {
                FullTextState::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("Loading the saved index…"));
                    });
                }
                FullTextState::Indexing(n) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(trf!(
                            "Reading documents… {files} looked at",
                            files = count(n, "file")
                        ));
                    });
                    ui.ctx().request_repaint_after(Duration::from_millis(500));
                }
                FullTextState::Paused => {
                    ui.label(tr("⏸ Paused while files are copied"));
                    ui.ctx().request_repaint_after(Duration::from_millis(500));
                }
                FullTextState::Ready | FullTextState::Off => {}
            }
            if let Some(e) = service.error() {
                ui.colored_label(
                    Color32::from_rgb(0xe5, 0x48, 0x4d),
                    trf!("Couldn't save the text index: {e}", e = e),
                );
            }
            let Some(index) = service.index() else { return };
            let stats = index.stats();
            if stats.files == 0 {
                ui.label(tr("Not built yet."));
            } else {
                let mut text = if stats.loaded {
                    trf!(
                        "✔ {files} with text ({size}) · {words} words · {memory} in memory",
                        files = count(stats.documents as u64, "file"),
                        size = format::size(stats.bytes),
                        words = stats.words,
                        memory = format::size(stats.memory as u64)
                    )
                } else {
                    trf!(
                        "✔ {files} with text ({size}) · {words} words · loaded when needed",
                        files = count(stats.documents as u64, "file"),
                        size = format::size(stats.bytes),
                        words = stats.words
                    )
                };
                if let Some(bytes) = service.file_size() {
                    text.push_str(&trf!(" · {size} on disk", size = format::size(bytes)));
                }
                ui.label(text);
            }
            if let Some((at, took)) = service.last_pass() {
                let age = SystemTime::now().duration_since(at).unwrap_or_default();
                ui.label(
                    RichText::new(trf!(
                        "Checked {age} ago, took {time}",
                        age = format::duration(age),
                        time = format::duration(took)
                    ))
                    .weak(),
                );
            }
            if ui.button(tr("🔄 Rebuild now")).clicked() {
                service.rebuild();
            }
        });
    });
}
