//! The "New symbolic link / junction / hard link" dialog: what the link
//! points to and what it's called. The target can be chosen in Windows'
//! *Open* dialog (*Browse…*), and folders and files complete while it's
//! typed (the search box's list, `header/nav/suggest.rs`).

use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, TryRecvError};
use egui::{Color32, Key, RichText, Ui};
use ox_core::i18n::t as tr;
use ox_core::links::LinkKind;
use ox_core::picker::{self, PickKind};

use super::header::nav::suggest::{self, Dropdown, Picked};
use crate::app::OxApp;

pub struct LinkDialog {
    /// The folder the link goes into.
    pub dir: PathBuf,
    pub kind: LinkKind,
    pub target: String,
    pub name: String,
    /// The name was typed, so it no longer follows the target.
    pub name_typed: bool,
    pub error: Option<String>,
    pub focus: bool,
    /// What was chosen with *Browse…* (`None`: cancelled), while it's open.
    browsing: Option<Receiver<Option<PathBuf>>>,
}

impl LinkDialog {
    /// `target`: what to link to at first (e.g. the selected item).
    pub fn new(dir: PathBuf, kind: LinkKind, target: Option<&Path>) -> Self {
        let mut d = Self {
            dir,
            kind,
            target: target.map(|t| t.display().to_string()).unwrap_or_default(),
            name: String::new(),
            name_typed: false,
            error: None,
            focus: true,
            browsing: None,
        };
        d.name = d.default_name();
        d
    }

    /// The target as a full path (without quotes, relative to the folder).
    fn target_path(&self) -> Option<PathBuf> {
        let t = self.target.trim().trim_matches('"').trim();
        if t.is_empty() {
            return None;
        }
        let p = PathBuf::from(t);
        Some(if p.is_absolute() { p } else { self.dir.join(p) })
    }

    /// The target changed: the name follows it unless it was typed.
    fn target_changed(&mut self) {
        self.error = None;
        if !self.name_typed {
            self.name = self.default_name();
        }
    }

    /// Open Windows' dialog to choose the target, on its own thread.
    fn browse(&mut self, ui: &Ui, kind: PickKind) {
        let owner = picker::owner_window();
        let start = self
            .target_path()
            .and_then(|p| picker::start_folder(&p))
            .unwrap_or_else(|| self.dir.clone());
        let title = tr("Choose what the link points to");
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ui.ctx().clone();
        std::thread::spawn(move || {
            let _ = tx.send(picker::pick(kind, Some(&start), title, owner));
            ctx.request_repaint();
        });
        self.browsing = Some(rx);
    }

    /// Take what was chosen with *Browse…*.
    fn poll_browse(&mut self) {
        let Some(rx) = &self.browsing else { return };
        match rx.try_recv() {
            Err(TryRecvError::Empty) => {}
            Ok(picked) => {
                self.browsing = None;
                if let Some(p) = picked {
                    self.target = p.display().to_string();
                    self.target_changed();
                }
            }
            Err(TryRecvError::Disconnected) => self.browsing = None,
        }
    }

    fn default_name(&self) -> String {
        self.target_path()
            .map(|p| ox_core::entry::display_name(&p))
            .filter(|n| !n.contains(['\\', '/', ':']))
            .unwrap_or_default()
    }

    /// What the target is, or why it doesn't fit this kind of link.
    fn target_note(&self) -> Option<(bool, String)> {
        let p = self.target_path()?;
        let Ok(meta) = std::fs::metadata(&p) else {
            return Some((false, tr("⚠ Not found").into()));
        };
        let note = match (self.kind, meta.is_dir()) {
            (LinkKind::Junction, false) => (false, tr("⚠ A junction can only point to a folder.")),
            (LinkKind::Hard, true) => (false, tr("⚠ A hard link can only point to a file.")),
            (LinkKind::Hard, false) if !ox_core::fsops::same_volume(&p, &self.dir) => (
                false,
                tr("⚠ A hard link can only point to a file on the same drive."),
            ),
            (_, true) => (true, tr("✔ Folder")),
            (_, false) => (true, tr("✔ File")),
        };
        Some((note.0, note.1.into()))
    }
}

/// The target's text field with its completions, and *Browse…*.
fn target_field(ui: &mut Ui, d: &mut LinkDialog) -> egui::Response {
    let id = ui.make_persistent_id("link_target");
    let suggestions = suggest::path_suggestions(ui, id, &d.target, Some(&d.dir));
    let dropdown = Dropdown::before(ui, id);
    let row = ui.horizontal(|ui| {
        let browse_w = if cfg!(windows) { 90.0 } else { 0.0 };
        let edit = egui::TextEdit::singleline(&mut d.target)
            .id(id)
            .hint_text(r"C:\Users\…")
            .desired_width(ui.available_width() - browse_w)
            .event_filter(dropdown.event_filter());
        let target = dropdown.add_field(ui, |ui| ui.add(edit));
        if cfg!(windows) {
            ui.add_enabled_ui(d.browsing.is_none(), |ui| browse_button(ui, d));
        }
        target
    });
    let target = row.inner;
    if d.focus {
        d.focus = false;
        target.request_focus();
    }
    if target.changed() {
        d.target_changed();
    }
    if let Some(Picked::Take(s, _)) = dropdown.show(ui, &target, &target, &d.target, suggestions) {
        d.target = s.text;
        suggest::cursor_to_end(ui, id, &d.target);
        d.target_changed();
    }
    target
}

/// A junction points to a folder, a hard link to a file, a symbolic link
/// to either (a menu then).
fn browse_button(ui: &mut Ui, d: &mut LinkDialog) {
    let label = tr("Browse…");
    match d.kind {
        LinkKind::Junction => {
            if ui.button(label).clicked() {
                d.browse(ui, PickKind::Folder);
            }
        }
        LinkKind::Hard => {
            if ui.button(label).clicked() {
                d.browse(ui, PickKind::File);
            }
        }
        LinkKind::Symbolic => {
            ui.menu_button(label, |ui| {
                if ui.button(tr("Folder…")).clicked() {
                    d.browse(ui, PickKind::Folder);
                }
                if ui.button(tr("File…")).clicked() {
                    d.browse(ui, PickKind::File);
                }
            });
        }
    }
}

fn heading(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Symbolic => tr("New symbolic link"),
        LinkKind::Junction => tr("New junction"),
        LinkKind::Hard => tr("New hard link"),
    }
}

fn explanation(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Symbolic => tr(
            "Points to a file or folder anywhere, also on the network. Needs Developer Mode or administrator rights.",
        ),
        LinkKind::Junction => {
            tr("Points to a folder on this computer's drives. Works without special rights.")
        }
        LinkKind::Hard => tr(
            "A second name for a file on the same drive. The file stays until all its names are deleted.",
        ),
    }
}

/// Draw the dialog. Returns whether it stays open.
pub fn show(ui: &mut Ui, d: &mut LinkDialog, app: &mut OxApp) -> bool {
    let mut keep = true;
    d.poll_browse();
    ui.heading(heading(d.kind));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for kind in [LinkKind::Symbolic, LinkKind::Junction, LinkKind::Hard] {
            if ui.radio_value(&mut d.kind, kind, kind.label()).changed() {
                d.error = None;
            }
        }
    });
    ui.label(RichText::new(explanation(d.kind)).weak());
    ui.add_space(8.0);

    ui.label(tr("Link to (full path):"));
    let target = target_field(ui, d);
    if let Some((ok, note)) = d.target_note() {
        let color = if ok {
            ui.visuals().weak_text_color()
        } else {
            ui.visuals().warn_fg_color
        };
        ui.colored_label(color, note);
    }
    ui.add_space(6.0);
    ui.label(tr("Name of the link:"));
    let name = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY));
    if name.changed() {
        d.name_typed = true;
        d.error = None;
    }
    if let Some(e) = &d.error {
        ui.colored_label(Color32::from_rgb(0xe5, 0x48, 0x4d), e.as_str());
    }
    let enter =
        (target.lost_focus() || name.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let ready = d.target_note().is_some_and(|(ok, _)| ok) && !d.name.trim().is_empty();
        let create = ui
            .add_enabled(ready, egui::Button::new(tr("Create")))
            .clicked();
        if ui.button(tr("Cancel")).clicked() {
            keep = false;
        }
        if (create || (enter && ready)) && keep {
            let name = d.name.trim();
            match ox_core::fsops::validate_name(name) {
                Err(e) => d.error = Some(e),
                Ok(()) => {
                    let target = d.target_path().unwrap_or_default();
                    match app.create_link(d.kind, &d.dir.join(name), &target) {
                        Ok(()) => keep = false,
                        Err(e) => d.error = Some(e),
                    }
                }
            }
        }
    });
    keep
}
