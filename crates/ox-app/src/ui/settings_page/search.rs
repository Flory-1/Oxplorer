//! The Search and Index pages of the settings, with the query syntax help.

use egui::{Color32, RichText, Ui};
use ox_core::i18n::t as tr;
use ox_core::index::IndexState;
use ox_core::settings::Settings;

use super::{row, toggle};
use crate::app::{Action, OxApp};

/// Query examples on the Search page: what to type, and what it does.
const QUERY_SYNTAX: [(&str, &str); 13] = [
    ("report", "name contains \"report\""),
    ("\"annual report\"", "exact phrase"),
    ("*.rs   img_??", "wildcards"),
    ("/^IMG_\\d+\\.jpe?g$/", "regular expression (also regex:…)"),
    ("-tmp", "exclude names containing \"tmp\""),
    ("ext:pdf,docx", "by extension"),
    ("type:folder / type:file", "only folders / files"),
    (
        "kind:image,video",
        "by kind: image, video, audio, document, archive, code, app",
    ),
    ("size:>10mb   size:<1kb", "by size"),
    ("modified:7d   modified:today", "recently modified"),
    ("content:\"TODO\"", "text inside files (≤ 32 MB)"),
    (
        "tag:red,blue",
        "tagged with one of these colours or tag names",
    ),
    (
        "archives:yes",
        "also the files inside zip, 7z and tar archives (not with the index)",
    ),
];

pub(super) fn search(ui: &mut Ui, s: &mut Settings) {
    let se = &mut s.search;
    toggle(
        ui,
        &mut se.instant_filter,
        tr("Filter while typing"),
        tr("The search box filters the current folder instantly; Enter searches sub-folders."),
    );
    toggle(ui, &mut se.include_hidden, tr("Include hidden files"), "");
    toggle(ui, &mut se.case_sensitive, tr("Case sensitive"), "");
    row(ui, tr("Maximum results"), "", |ui| {
        ui.add(
            egui::DragValue::new(&mut se.max_results)
                .range(100..=1_000_000)
                .speed(100),
        );
    });
    row(
        ui,
        tr("Search threads"),
        tr("0 = one per CPU core."),
        |ui| {
            ui.add(egui::Slider::new(&mut se.threads, 0..=64));
        },
    );
    ui.add_space(8.0);
    ui.label(RichText::new(tr("Query syntax")).strong());
    egui::Grid::new("syntax")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            for (q, d) in QUERY_SYNTAX {
                ui.monospace(q);
                ui.label(tr(d));
                ui.end_row();
            }
        });
}

pub(super) fn index_section(ui: &mut Ui, app: &mut OxApp) {
    ui.add_space(14.0);
    ui.label(RichText::new(tr("Search index")).size(16.0).strong());
    ui.label(
        RichText::new(tr(
            "Keeps the names of all files in memory (about 50 MB per million files) so searching \
             whole drives takes milliseconds. Built in the background and saved between runs.",
        ))
        .weak(),
    );
    ui.add_space(8.0);
    let se = &mut app.settings.search;
    toggle(ui, &mut se.index_enabled, tr("Index file names"), "");
    if !se.index_enabled {
        return;
    }
    toggle(
        ui,
        &mut se.search_as_you_type,
        tr("Results while typing"),
        tr("On the Home page and in search tabs, results update with every key."),
    );

    let index = app.index.clone();
    row(ui, tr("Status"), "", |ui| {
        ui.vertical(|ui| {
            match index.state() {
                IndexState::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("Loading the saved index…"));
                    });
                }
                IndexState::Building => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(ox_core::trf!(
                            "Indexing… {v1} items so far",
                            v1 = index.progress()
                        ));
                    });
                }
                IndexState::Failed(e) => {
                    ui.colored_label(Color32::from_rgb(0xe5, 0x48, 0x4d), e);
                }
                IndexState::Empty | IndexState::Ready => {}
            }
            if let Some(idx) = index.index() {
                let age = std::time::SystemTime::now()
                    .duration_since(idx.built_at())
                    .unwrap_or_default();
                let mut text = ox_core::trf!(
                    "✔ {v1} items · {v2} in memory · updated {v3} ago",
                    v1 = idx.len(),
                    v2 = ox_core::format::size(idx.memory_bytes() as u64),
                    v3 = ox_core::format::duration(age)
                );
                if let Some((_, took)) = index.last_build() {
                    text.push_str(&ox_core::trf!(
                        " · built in {secs} s",
                        secs = ox_core::format::decimal(took.as_secs_f64(), 1)
                    ));
                }
                if idx.mft_roots() > 0 {
                    text.push_str(tr(" · read from the NTFS master file table"));
                }
                ui.label(text);
                live_status(ui, &index, &idx);
            } else if index.state() == IndexState::Empty {
                ui.label(tr("Not built yet."));
            }
            ui.horizontal(|ui| {
                if index.is_building() {
                    if ui.button(tr("⏹ Stop")).clicked() {
                        index.cancel();
                    }
                } else if ui.button(tr("🔄 Rebuild now")).clicked() {
                    app.actions.push(Action::RebuildIndex);
                }
            });
        });
    });

    let se = &mut app.settings.search;
    row(
        ui,
        tr("Indexed folders"),
        if cfg!(windows) {
            tr("One per line. Empty = all drives.")
        } else {
            tr("One per line. Empty = your home folder.")
        },
        |ui| {
            let mut text = se
                .index_roots
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let edit = egui::TextEdit::multiline(&mut text)
                .desired_rows(3)
                .desired_width(280.0)
                .hint_text(if cfg!(windows) {
                    tr("C:\\\nD:\\Projects")
                } else {
                    "/home/me"
                });
            if ui.add(edit).changed() {
                se.index_roots = text
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
        tr("Skip folders named"),
        tr("Comma-separated, e.g. node_modules, .git"),
        |ui| {
            let mut text = se.index_exclude.join(", ");
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(280.0))
                .changed()
            {
                se.index_exclude = text
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
            }
        },
    );
    toggle(
        ui,
        &mut se.index_live,
        tr("Live updates"),
        tr("Apply file changes to the index as they happen, between refreshes."),
    );
    row(ui, tr("Refresh every"), "", |ui| {
        ui.add(
            egui::Slider::new(&mut se.index_refresh_minutes, 0..=240).custom_formatter(|v, _| {
                if v == 0.0 {
                    tr("Only at start").into()
                } else {
                    format!("{v:.0} min")
                }
            }),
        );
    });
    super::index_service::section(ui, app);
}

/// Remembering searches for the suggestions under the search box.
pub(super) fn history_section(ui: &mut Ui, app: &mut OxApp) {
    ui.add_space(14.0);
    ui.label(RichText::new(tr("Search history")).size(16.0).strong());
    ui.add_space(8.0);
    toggle(
        ui,
        &mut app.settings.search.remember_searches,
        tr("Remember searches"),
        tr("Suggest earlier searches under the search box. Kept only on this computer."),
    );
    app.search_history.refresh();
    let empty = app.search_history.entries().is_empty();
    row(
        ui,
        tr("Clear search history"),
        if empty {
            tr("Nothing remembered yet")
        } else {
            ""
        },
        |ui| {
            if ui
                .add_enabled(!empty, egui::Button::new(tr("🗑 Clear")))
                .clicked()
            {
                app.actions
                    .push(Action::SearchHistory(crate::app::HistoryAction::Clear));
            }
        },
    );
}

/// Whether live updates run and how changes are followed.
fn live_status(ui: &mut Ui, index: &ox_core::index::IndexService, idx: &ox_core::index::FileIndex) {
    if let Some(live) = index.live() {
        let mut text = if live.journal_drives > 0 {
            tr("Live updates on (NTFS change journal)").to_owned()
        } else {
            tr("Live updates on").to_owned()
        };
        if live.changes > 0 {
            text.push_str(&ox_core::trf!(
                " · {v1} applied",
                v1 = ox_core::i18n::count(live.changes, "change")
            ));
        }
        ui.label(RichText::new(text).weak());
    }
    // Whole drives were walked although reading the MFT would be much faster.
    let drives = idx.roots().iter().any(|r| r.parent().is_none());
    if cfg!(windows) && drives && idx.mft_roots() == 0 && index.last_build().is_some() {
        ui.label(
            RichText::new(tr(
                "Run Oxplorer as administrator to read NTFS drives directly from the master \
                 file table (much faster) and follow changes through the change journal.",
            ))
            .weak()
            .small(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_syntax_is_translated() {
        for s in QUERY_SYNTAX.map(|(_, what)| what) {
            assert!(ox_core::i18n::german(s).is_some(), "no German for {s:?}");
        }
    }
}
