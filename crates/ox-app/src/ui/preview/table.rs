//! The Table tab: rows of a CSV file or a database table, drawn only where
//! visible, with a fixed header and resizable columns.

use super::*;
use egui_extras::{Column, TableBuilder};
use ox_core::datatable::Table;

const ROW_H: f32 = 20.0;
/// Longest text drawn in a cell (the rest is cut).
const MAX_CELL_CHARS: usize = 200;

/// Draws `t`. `id` tells tables apart (column widths are kept per id).
pub(super) fn show(ui: &mut Ui, t: &Table, id: &str, actions: &mut Vec<Action>) {
    if t.columns.is_empty() || t.rows.is_empty() {
        ui.label(RichText::new(tr("There are no rows.")).weak());
        return;
    }
    let note_h = if t.truncated { 20.0 } else { 0.0 };
    let height = (ui.available_height() - note_h).max(60.0);
    let widths = widths(t);
    let null = RichText::new("NULL").weak().italics();
    egui::ScrollArea::horizontal()
        .id_salt(("preview_table_h", id))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let mut builder = TableBuilder::new(ui)
                .id_salt(("preview_table", id))
                .striped(true)
                .resizable(true)
                .auto_shrink([false, false])
                .min_scrolled_height(0.0)
                .max_scroll_height(height - ROW_H)
                .column(Column::auto().at_least(28.0));
            for w in &widths {
                builder = builder.column(Column::initial(*w).at_least(36.0).clip(true));
            }
            builder
                .header(ROW_H + 2.0, |mut header| {
                    header.col(|ui| {
                        ui.label(RichText::new("#").weak());
                    });
                    for c in &t.columns {
                        header.col(|ui| {
                            let r = ui
                                .add(egui::Label::new(RichText::new(&c.name).strong()).truncate());
                            if !c.decl.is_empty() {
                                r.on_hover_text(&c.decl);
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW_H, t.rows.len(), |mut row| {
                        let i = row.index();
                        let cells = &t.rows[i];
                        row.col(|ui| {
                            ui.label(RichText::new((i + 1).to_string()).weak().small());
                        });
                        for (c, col) in t.columns.iter().enumerate() {
                            row.col(|ui| {
                                let text = match cells.get(c) {
                                    Some(Some(s)) => RichText::new(one_line(s)),
                                    Some(None) => null.clone(),
                                    None => return,
                                };
                                let label = egui::Label::new(text)
                                    .truncate()
                                    .sense(egui::Sense::click());
                                let r = if col.numeric {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| ui.add(label),
                                    )
                                    .inner
                                } else {
                                    ui.add(label)
                                };
                                r.context_menu(|ui| cell_menu(ui, t, i, c, actions));
                            });
                        }
                    });
                });
        });
    if t.truncated {
        ui.label(
            RichText::new(ox_core::trf!(
                "Only the first {v1} rows are shown.",
                v1 = t.rows.len()
            ))
            .weak()
            .small(),
        );
    }
}

/// Starting widths from the header and the first rows.
fn widths(t: &Table) -> Vec<f32> {
    (0..t.columns.len())
        .map(|c| {
            let longest = t
                .rows
                .iter()
                .take(100)
                .filter_map(|r| r.get(c).cloned().flatten())
                .map(|s| s.chars().count())
                .chain([t.columns[c].name.chars().count() + 2])
                .max()
                .unwrap_or(4);
            (longest as f32 * 7.5 + 12.0).clamp(40.0, 320.0)
        })
        .collect()
}

/// A cell's text on one line, cut when very long.
fn one_line(s: &str) -> String {
    let mut out: String = s
        .chars()
        .take(MAX_CELL_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if s.chars().nth(MAX_CELL_CHARS).is_some() {
        out.push('…');
    }
    out
}

fn cell_menu(ui: &mut Ui, t: &Table, row: usize, col: usize, actions: &mut Vec<Action>) {
    let cells = &t.rows[row];
    if ui.button(tr("📋 Copy value")).clicked() {
        let value = cells.get(col).cloned().flatten().unwrap_or_default();
        copy(ui, value, actions);
        ui.close();
    }
    if ui.button(tr("📋 Copy row")).clicked() {
        let line = cells
            .iter()
            .map(|c| c.as_deref().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\t");
        copy(ui, line, actions);
        ui.close();
    }
}

/// Put `text` on the clipboard and say so.
pub(super) fn copy(ui: &Ui, text: String, actions: &mut Vec<Action>) {
    ui.ctx().copy_text(text);
    actions.push(Action::Toast(tr("Copied to the clipboard").to_owned()));
}
