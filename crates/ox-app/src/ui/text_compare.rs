//! Compare two text files: a window with their diff, drawn like Git's
//! diffs (`preview::git_diff`). The diff is computed in the background by
//! `ox_core::textdiff`; ignoring whitespace computes it again.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::Receiver;
use egui::{vec2, Context, Id, RichText, Ui};
use ox_core::i18n::t as tr;
use ox_core::textdiff::{self, Comparison, Options};
use ox_core::textinfo::{Encoding, LineEndings};
use ox_core::{format, trf};

use super::preview::git_diff;
use crate::app::{Action, OxApp};

pub struct TextCompare {
    old: PathBuf,
    new: PathBuf,
    opts: Options,
    side_by_side: bool,
    /// What is shown, and the options it was computed with.
    shown: Option<(Options, Result<Arc<Comparison>, String>)>,
    computing: Option<(Options, Receiver<Result<Comparison, String>>)>,
}

impl TextCompare {
    fn new(old: PathBuf, new: PathBuf) -> Self {
        Self {
            old,
            new,
            opts: Options::default(),
            side_by_side: false,
            shown: None,
            computing: None,
        }
    }

    /// Take in a finished comparison; start one if the options changed.
    fn update(&mut self, ctx: &Context) {
        if let Some((opts, rx)) = &self.computing {
            if let Ok(result) = rx.try_recv() {
                self.shown = Some((*opts, result.map(Arc::new)));
                self.computing = None;
            }
        }
        let wanted = self.opts;
        let current = self
            .computing
            .as_ref()
            .map(|(o, _)| *o)
            .or(self.shown.as_ref().map(|(o, _)| *o));
        if current == Some(wanted) {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (old, new, repaint) = (self.old.clone(), self.new.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = textdiff::compare_files(&old, &new, wanted).map_err(|e| e.to_string());
            let _ = tx.send(result);
            repaint.request_repaint();
        });
        self.computing = Some((wanted, rx));
    }

    fn swap(&mut self) {
        std::mem::swap(&mut self.old, &mut self.new);
        self.shown = None;
        self.computing = None;
    }
}

/// Open the window comparing `old` with `new`.
pub fn open(app: &mut OxApp, old: PathBuf, new: PathBuf) {
    app.text_compare = Some(Box::new(TextCompare::new(old, new)));
}

fn is_file(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file())
}

/// The two files of `sel`, if it is exactly two files.
pub fn two_files(sel: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    match sel {
        [a, b] if is_file(a) && is_file(b) => Some((a.clone(), b.clone())),
        _ => None,
    }
}

/// What "Compare these two files" compares: two files selected in this
/// tab, or in the split view the file selected in each pane (left as the
/// old one).
pub fn pair(app: &OxApp) -> Option<(PathBuf, PathBuf)> {
    let tab = app.tabs.active();
    let sel = tab.view.selected_paths();
    if let Some(two) = two_files(&sel) {
        return Some(two);
    }
    let panes = app.tabs.panes?;
    let one = |id| {
        let paths = app.tabs.get(id)?.view.selected_paths();
        match paths.as_slice() {
            [p] if is_file(p) => Some(p.clone()),
            _ => None,
        }
    };
    Some((one(panes[0])?, one(panes[1])?))
}

/// The item in the context menu, when exactly two files are selected.
pub fn menu(ui: &mut Ui, sel: &[PathBuf], actions: &mut Vec<Action>) {
    if let Some((a, b)) = two_files(sel) {
        if ui.button(tr("⇄ Compare these two files")).clicked() {
            actions.push(Action::CompareTextFiles(a, b));
        }
    }
}

pub fn show(app: &mut OxApp, ctx: &Context) {
    let Some(tc) = app.text_compare.as_mut() else {
        return;
    };
    tc.update(ctx);
    let screen = ctx.content_rect();
    let mut open = true;
    egui::Window::new(tr("Compare files"))
        .id(Id::unique("text_compare"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(screen.width() * 0.8, screen.height() * 0.75))
        .default_pos(screen.center() - vec2(screen.width() * 0.4, screen.height() * 0.375))
        .show(ctx, |ui| body(ui, tc));
    if !open {
        app.text_compare = None;
    }
}

/// Encodings that read the same for comparing (ASCII is UTF-8).
fn same_encoding(a: Encoding, b: Encoding) -> bool {
    let plain = |e| {
        if e == Encoding::Ascii {
            Encoding::Utf8
        } else {
            e
        }
    };
    plain(a) == plain(b)
}

fn body(ui: &mut Ui, tc: &mut TextCompare) {
    let colors = crate::theme::diff_colors(ui.visuals().dark_mode);
    let mut swap = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            for (sign, path, color) in [
                ("-", &tc.old, colors.removed_text),
                ("+", &tc.new, colors.added_text),
            ] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(sign).monospace().strong().color(color));
                    ui.label(RichText::new(ox_core::entry::display_name(path)).strong());
                    let folder = path.parent().map(|p| p.display().to_string());
                    ui.label(RichText::new(folder.unwrap_or_default()).weak().small());
                });
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            swap = ui
                .button("⇄")
                .on_hover_text(tr("Swap the two files"))
                .clicked();
        });
    });
    if swap {
        tc.swap();
        return;
    }
    ui.separator();
    let loading = tc.computing.is_some();
    let c = match &tc.shown {
        None => {
            ui.spinner();
            return;
        }
        Some((_, Err(e))) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
            return;
        }
        Some((_, Ok(c))) => c.clone(),
    };
    notes(ui, &c);
    if c.identical {
        ui.label(RichText::new(tr("The files are identical.")).strong());
        return;
    }
    if c.binary() || c.diff.too_large {
        let text = if c.binary() {
            tr("A binary file: there are no lines to compare.").to_owned()
        } else {
            trf!(
                "Too large to compare line by line (more than {size} or {lines} lines).",
                size = format::size(textdiff::MAX_BYTES),
                lines = textdiff::MAX_LINES
            )
        };
        ui.label(RichText::new(text).weak());
        if let Some(at) = c.first_difference {
            ui.label(trf!(
                "The bytes differ from offset {offset} on.",
                offset = format!("0x{at:X}")
            ));
        }
        return;
    }
    let TextCompare {
        opts, side_by_side, ..
    } = tc;
    git_diff::draw(
        ui,
        side_by_side,
        &c.diff,
        "text_compare_diff",
        f32::INFINITY,
        |ui| {
            ui.checkbox(&mut opts.ignore_whitespace, tr("Ignore whitespace"));
            if loading {
                ui.spinner();
            }
        },
    );
}

/// Differences the line diff doesn't show: encodings and line endings.
fn notes(ui: &mut Ui, c: &Comparison) {
    let note = |ui: &mut Ui, text: String| {
        ui.label(RichText::new(format!("ℹ {text}")).weak());
    };
    if c.binary() {
        return;
    }
    if !same_encoding(c.old.encoding, c.new.encoding) {
        note(
            ui,
            trf!(
                "Encodings differ: {old} → {new}",
                old = c.old.encoding.label(),
                new = c.new.encoding.label()
            ),
        );
    }
    let real = |e: LineEndings| e != LineEndings::None;
    if c.old.endings != c.new.endings && real(c.old.endings) && real(c.new.endings) {
        note(
            ui,
            trf!(
                "Line endings differ: {old} → {new} (not shown as changes)",
                old = c.old.endings.label(),
                new = c.new.endings.label()
            ),
        );
    }
}
