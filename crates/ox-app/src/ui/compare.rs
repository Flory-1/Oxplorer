//! Compare folders: the bar above the split view (summary, legend, options
//! and copy buttons), the marks in the panes' file lists and the question
//! before copying.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{vec2, Color32, CornerRadius, Id, Key, Rect, RichText, Ui};
use ox_core::compare::{Counts, Mark};
use ox_core::i18n::{count, t as tr};
use ox_core::{format, trf};

use crate::app::{Action, CompareAction, OxApp};
use crate::shortcuts::{self, Command};

fn mark_color(mark: Mark) -> Option<Color32> {
    match mark {
        Mark::OnlyHere => Some(Color32::from_rgb(0x3b, 0xb2, 0x73)),
        Mark::Newer => Some(Color32::from_rgb(0x4c, 0x8d, 0xf6)),
        Mark::Older => Some(Color32::from_rgb(0xf0, 0x8a, 0x24)),
        Mark::Different => Some(Color32::from_rgb(0xe5, 0x48, 0x4d)),
        Mark::Same => None,
    }
}

/// A colour strip at the left edge of a row or tile whose item differs.
pub fn paint_mark(ui: &Ui, rect: Rect, marks: Option<&HashMap<PathBuf, Mark>>, path: &Path) {
    let Some(color) = marks.and_then(|m| m.get(path)).and_then(|&m| mark_color(m)) else {
        return;
    };
    let strip = Rect::from_min_size(
        rect.left_top() + vec2(1.0, 3.0),
        vec2(3.0, rect.height() - 6.0),
    );
    ui.painter()
        .rect_filled(strip, CornerRadius::same(2), color);
}

/// Small button between the panes' title bars that compares the two folders.
pub fn split_button(app: &mut OxApp, ui: &mut Ui, at: egui::Pos2) {
    let rect = Rect::from_min_size(at + vec2(-26.0, 2.0), vec2(22.0, 22.0));
    let active = app.compare.is_some();
    let resp = ui
        .put(
            rect,
            egui::Button::new(RichText::new("⇄").size(14.0))
                .frame(active)
                .selected(active),
        )
        .on_hover_text(shortcuts::hint(
            tr("Compare folders"),
            Command::CompareFolders,
        ));
    if resp.clicked() {
        app.push(Action::Compare(if active {
            CompareAction::Close
        } else {
            CompareAction::Start
        }));
    }
}

/// The bar above the panes while folders are compared.
pub fn bar(app: &mut OxApp, ui: &mut Ui) {
    let Some(state) = &mut app.compare else {
        return;
    };
    let mut actions = Vec::new();
    let counts = state.result.as_ref().map(|c| c.counts());
    let frame = egui::Frame::new()
        .fill(ui.visuals().panel_fill)
        .inner_margin(egui::Margin::symmetric(10, 5));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("⇄ Compare folders")).strong());
            ui.separator();
            if let Some(hint) = state.hint {
                ui.label(RichText::new(hint).weak());
            } else if let Some(e) = &state.error {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    trf!("Can't compare the folders: {e}", e = e),
                );
            } else if let (Some(c), Some(result)) = (&counts, &state.result) {
                summary(ui, c);
                if result.errors > 0 {
                    ui.label(
                        RichText::new(trf!(
                            "⚠ {v1} couldn't be read",
                            v1 = count(result.errors as u64, "item")
                        ))
                        .color(ui.visuals().warn_fg_color),
                    );
                }
            }
            if let Some(job) = &state.job {
                ui.spinner();
                ui.label(
                    RichText::new(trf!(
                        "Comparing… {v1} scanned",
                        v1 = count(job.scanned(), "item")
                    ))
                    .weak(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("✖")
                    .on_hover_text(tr("Stop comparing"))
                    .clicked()
                {
                    actions.push(Action::Compare(CompareAction::Close));
                }
                if ui
                    .small_button("↻")
                    .on_hover_text(tr("Compare again"))
                    .clicked()
                {
                    actions.push(Action::Compare(CompareAction::Start));
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut state.only_differences, tr("Only differences"));
            ui.checkbox(&mut state.options.subfolders, tr("Sub-folders"))
                .on_hover_text(tr("Also compare what's inside folders both sides have."));
            ui.checkbox(&mut state.options.content, tr("Compare contents"))
                .on_hover_text(tr(
                    "Compare files of the same size byte by byte instead of by date. Slower.",
                ));
            ui.separator();
            let c = counts.unwrap_or_default();
            let busy = state.job.is_some();
            let mut button = |ui: &mut Ui, label: &str, enabled: bool, action: CompareAction| {
                if ui
                    .add_enabled(enabled && !busy, egui::Button::new(label))
                    .clicked()
                {
                    actions.push(Action::Compare(action));
                }
            };
            button(
                ui,
                tr("➡ Copy missing to the right"),
                c.only_left > 0,
                CompareAction::CopyMissing(true),
            );
            button(
                ui,
                tr("⬅ Copy missing to the left"),
                c.only_right > 0,
                CompareAction::CopyMissing(false),
            );
            button(
                ui,
                tr("🔄 Copy newer"),
                c.newer_left + c.newer_right > 0,
                CompareAction::CopyNewer,
            );
            ui.separator();
            legend(ui);
        });
    });
    confirm(app, ui.ctx(), &mut actions);
    app.actions.extend(actions);
}

/// "12 only left · 3 newer right · 40 same".
fn summary(ui: &mut Ui, c: &Counts) {
    if c.differences() == 0 {
        ui.label(tr("✔ No differences"));
        if c.same > 0 {
            ui.label(RichText::new(trf!("{v1} same", v1 = c.same)).weak());
        }
        return;
    }
    let parts = [
        (c.only_left, tr("{v1} only left")),
        (c.only_right, tr("{v1} only right")),
        (c.newer_left, tr("{v1} newer left")),
        (c.newer_right, tr("{v1} newer right")),
        (c.different, tr("{v1} different")),
        (c.same, tr("{v1} same")),
    ];
    let text: Vec<String> = parts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, template)| ox_core::i18n::fill(template, &[("v1", n as &dyn std::fmt::Display)]))
        .collect();
    ui.label(text.join(" · "));
}

/// What the colours in the panes mean.
fn legend(ui: &mut Ui) {
    for (mark, label) in [
        (Mark::OnlyHere, tr("only here")),
        (Mark::Newer, tr("newer")),
        (Mark::Older, tr("older")),
        (Mark::Different, tr("different")),
    ] {
        let (rect, _) = ui.allocate_exact_size(vec2(4.0, 14.0), egui::Sense::hover());
        if let Some(color) = mark_color(mark) {
            ui.painter().rect_filled(rect, CornerRadius::same(2), color);
        }
        ui.label(RichText::new(label).weak().small());
    }
}

/// Ask before copying, with what is copied where.
fn confirm(app: &mut OxApp, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let Some(state) = &mut app.compare else {
        return;
    };
    let (Some(pending), Some(result)) = (&state.confirm, &state.result) else {
        return;
    };
    let mut close = false;
    let modal = egui::Modal::new(Id::unique("compare_confirm")).show(ctx, |ui| {
        ui.set_width(440.0);
        let plan = &pending.plan;
        if pending.newer {
            ui.heading(tr("Replace older files?"));
            ui.add_space(6.0);
            ui.label(trf!(
                "Copy the newer version of {v1} to the right and {v2} to the left. The older \
                 files are replaced.",
                v1 = count(plan.to_right as u64, "file"),
                v2 = count(plan.to_left as u64, "file")
            ));
        } else {
            let (n, dest) = if plan.to_right > 0 {
                (plan.to_right, &result.right)
            } else {
                (plan.to_left, &result.left)
            };
            ui.heading(tr("Copy missing items?"));
            ui.add_space(6.0);
            ui.label(trf!(
                "Copy {v1} that only the other side has to {dest}?",
                v1 = count(n as u64, "item"),
                dest = dest.display()
            ));
        }
        if plan.bytes > 0 {
            ui.label(RichText::new(trf!("Files: {v1}", v1 = format::size(plan.bytes))).weak());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            if ui.button(RichText::new(tr("Copy")).strong()).clicked() || enter {
                actions.push(Action::Compare(CompareAction::ConfirmCopy));
            }
            if ui.button(tr("Cancel")).clicked() {
                close = true;
            }
        });
    });
    if close || modal.should_close() {
        state.confirm = None;
    }
}
