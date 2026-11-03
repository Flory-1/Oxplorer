//! The preview inside the Properties dialog: the tabs the pane has for the
//! item, after the dialog's own General and Details tabs.

use std::sync::Arc;

use egui::{RichText, Ui};
use ox_core::details::Details;
use ox_core::entry::FileEntry;
use ox_core::git::RepoStatus;
use ox_core::i18n::t as tr;

use super::{checksums, commit_time, picture, tab_body, Content, PreviewState, PreviewTab};
use crate::app::Action;

/// A tab of the Properties dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogTab {
    /// What the item is, its size, place, dates and attributes.
    General,
    /// What its type tells, Git and checksums.
    Details,
    /// One of the preview pane's tabs.
    Preview(PreviewTab),
}

impl DialogTab {
    pub fn label(self) -> &'static str {
        match self {
            DialogTab::General => tr("ℹ General"),
            DialogTab::Details => tr("📜 Details"),
            DialogTab::Preview(t) => t.label(),
        }
    }
}

/// The preview of the one item the dialog is for (it has its own, so the
/// pane keeps its state and the two players don't meet).
#[derive(Default)]
pub struct Embedded {
    state: PreviewState,
}

impl Embedded {
    /// Load `e` (again only when it changed). `git` is the status of the
    /// repository the tab shows, for the Diff tab.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        e: &FileEntry,
        show_hidden: bool,
        git: Option<Arc<RepoStatus>>,
    ) {
        self.state.update(ctx, e, show_hidden);
        self.state.git.status = git;
    }

    /// The details read in the background, once they are there.
    pub fn details(&self) -> Option<&Details> {
        self.state.details.as_ref()
    }

    fn loaded(&self) -> Option<&Content> {
        self.state
            .content
            .as_ref()
            .filter(|c| !matches!(c, Content::Loading(_)))
    }

    /// General, Details and the tabs the item's type has. Not the Document
    /// tab: its window would cover the dialog.
    pub fn tabs(&mut self, e: &FileEntry) -> Vec<DialogTab> {
        let mut tabs = vec![DialogTab::General, DialogTab::Details];
        let Some(content) = self.loaded() else {
            return tabs;
        };
        let mut offered = content.tabs();
        self.state.hex.offer(e, &mut offered);
        self.state.git.offer(e, &mut offered);
        tabs.extend(
            offered
                .into_iter()
                .filter(|t| *t != PreviewTab::Info)
                .map(DialogTab::Preview),
        );
        tabs
    }

    /// The Details tab: the full path, what the type tells (dimensions,
    /// length, encoding, camera, …), Git, a map and checksums.
    pub fn details_tab(
        &mut self,
        ui: &mut Ui,
        e: &FileEntry,
        git_state: Option<&'static str>,
        date_fmt: &str,
        actions: &mut Vec<Action>,
    ) {
        let mut rows: Vec<(&str, String)> = vec![(tr("Path"), e.path.display().to_string())];
        if let Some(content) = self.loaded() {
            rows.extend(content.facts());
        }
        let s = &mut self.state;
        if let Some(d) = &s.details {
            rows.extend(d.facts.iter().cloned());
            let attrs: Vec<String> = super::other_attributes(d).collect();
            if !attrs.is_empty() {
                rows.push((tr("Attributes"), attrs.join(", ")));
            }
        }
        if let Some(state) = git_state {
            rows.push((tr("Git"), state.to_owned()));
        }
        if let Some(c) = &s.last_commit {
            rows.push((
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
        egui::Grid::new("props_details")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                for (name, value) in rows {
                    ui.label(RichText::new(name).weak());
                    ui.add(egui::Label::new(value).wrap());
                    ui.end_row();
                }
            });
        let loading = matches!(s.content, None | Some(Content::Loading(_))) || s.extra_rx.is_some();
        if loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr("Reading…")).weak());
            });
        }
        picture::map_button(ui, s.details.as_ref());
        if !e.is_dir && ox_core::archive::split_virtual(&e.path).is_none() {
            ui.add_space(8.0);
            checksums(ui, &mut s.checksums, e, actions);
        }
    }

    /// One of the preview's tabs.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        e: &FileEntry,
        tab: PreviewTab,
        date_fmt: &str,
        actions: &mut Vec<Action>,
    ) {
        tab_body(ui, &mut self.state, e, tab, None, None, date_fmt, actions);
    }

    /// The dialog closes: stop playing, and take a previewed font out again.
    pub fn release(&mut self, ctx: &egui::Context) {
        self.state.release(ctx);
    }
}
