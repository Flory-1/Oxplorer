//! Preview pane: shows the selected file (image, text/code) or a summary.

mod animation;
mod data;
mod document;
mod font;
mod git_branches;
mod git_changes;
mod git_diff;
mod git_graph;
mod git_panel;
mod hex;
mod info;
mod large;
mod load;
mod picture;
mod player;
mod table;
mod tree;
use font::*;
use git_graph::*;
use info::{checksums, info_grid};
pub use info::{info_rows, other_attributes, type_facts};
use load::*;
use ox_core::i18n::t as tr;
pub(crate) use picture::{is_picture, still};
use player::*;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub use document::place_documents;

use crossbeam_channel::Receiver;
use egui::{ColorImage, RichText, TextureHandle, TextureOptions, Ui};
use egui_extras::syntax_highlighting::{highlight, CodeTheme};
use ox_core::details::Details;
use ox_core::dirsize::DirSize;
use ox_core::entry::{FileEntry, FileKind};
use ox_core::format;
use ox_core::git::{Commit, GitRef, GraphRow};

use crate::app::{Action, OxApp};

/// Larger texts are shown without syntax highlighting.
const MAX_HIGHLIGHT_BYTES: usize = 64 * 1024;

enum Content {
    Loading(Receiver<Loaded>),
    Image(TextureHandle, [u32; 2]),
    /// An animated GIF/WebP.
    Animation(animation::Animation),
    Page(TextureHandle, u32),
    Text {
        text: String,
        numbers: String,
        truncated: bool,
        /// Encoding, line endings, indentation.
        details: Vec<(&'static str, String)>,
        /// A Markdown file: also shown rendered.
        markdown: bool,
    },
    Media {
        info: Vec<(&'static str, String)>,
        cover: Option<TextureHandle>,
    },
    Folder {
        folders: usize,
        files: usize,
        bytes: u64,
        /// The history of the Git repository the folder is in.
        git: Option<Arc<GitInfo>>,
        /// What's in it (only for Quick Look).
        items: Vec<FileEntry>,
    },
    Font {
        /// egui font family the file was installed as.
        family: String,
        info: Vec<(&'static str, String)>,
        chars: Vec<char>,
    },
    /// CSV, JSON/XML/YAML/TOML or SQLite: a table or tree (and the text).
    Data(Box<data::Data>),
    Nothing,
    Error(String),
}

/// Tabs of the preview pane: Info first, then what the file type offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreviewTab {
    Info,
    Image,
    Markdown,
    Text,
    Page,
    Media,
    FontSample,
    Glyphs,
    Git,
    /// The changes of a file in a Git repository.
    Diff,
    Player,
    /// The items of a folder or archive (Quick Look).
    Contents,
    /// Shown by the file type's preview handler (Office documents, mails…).
    Document,
    Table,
    Tree,
    Hex,
}

impl PreviewTab {
    fn label(self) -> &'static str {
        match self {
            PreviewTab::Info => tr("ℹ Info"),
            PreviewTab::Image => tr("🖼 Image"),
            PreviewTab::Markdown => tr("📖 Markdown"),
            PreviewTab::Text => tr("📝 Text"),
            PreviewTab::Page => tr("📄 Page"),
            PreviewTab::Media => tr("🎵 Cover"),
            PreviewTab::FontSample => tr("Aa Preview"),
            PreviewTab::Glyphs => tr("🔣 Glyphs"),
            PreviewTab::Git => tr("🌿 Git"),
            PreviewTab::Diff => tr("± Diff"),
            PreviewTab::Player => tr("▶ Player"),
            PreviewTab::Contents => tr("📂 Contents"),
            PreviewTab::Document => tr("📄 Document"),
            PreviewTab::Table => tr("▦ Table"),
            PreviewTab::Tree => tr("🗃 Tree"),
            PreviewTab::Hex => tr("🔐 Hex"),
        }
    }
}

impl Content {
    fn tabs(&self) -> Vec<PreviewTab> {
        use PreviewTab::*;
        match self {
            Content::Image(..) => vec![Info, Image],
            Content::Animation(..) => vec![Info, Image],
            Content::Page(..) => vec![Info, Page],
            Content::Text { markdown: true, .. } => vec![Info, Markdown, Text],
            Content::Text { .. } => vec![Info, Text],
            Content::Media { cover, .. } if ox_core::player::supported() => {
                let mut tabs = vec![Info, Player];
                tabs.extend(cover.as_ref().map(|_| Media));
                tabs
            }
            Content::Media { cover: Some(_), .. } => vec![Info, Media],
            Content::Font { .. } => vec![Info, FontSample, Glyphs],
            Content::Data(d) => d.tabs(),
            Content::Folder { git, items, .. } => {
                let mut tabs = vec![Info];
                tabs.extend((!items.is_empty()).then_some(Contents));
                tabs.extend(git.as_ref().map(|_| Git));
                tabs
            }
            _ => vec![Info],
        }
    }

    /// What the file type adds to the Info tab.
    fn facts(&self) -> Vec<(&'static str, String)> {
        match self {
            Content::Image(_, [w, h]) => vec![(tr("Dimensions"), format!("{w} × {h} px"))],
            Content::Animation(a) => a.facts(),
            Content::Page(_, pages) => vec![(tr("Pages"), pages.to_string())],
            Content::Text {
                numbers,
                truncated,
                details,
                ..
            } => {
                let lines = numbers.lines().count();
                let more = if *truncated { "+" } else { "" };
                let mut facts = vec![(tr("Lines"), format!("{lines}{more}"))];
                facts.extend(details.iter().cloned());
                facts
            }
            Content::Media { info, .. } | Content::Font { info, .. } => info.clone(),
            Content::Data(d) => d.facts(),
            Content::Folder {
                folders,
                files,
                bytes,
                ..
            } => vec![(
                tr("Contains"),
                format!(
                    "{}, {} ({})",
                    ox_core::i18n::count(*folders as u64, "folder"),
                    ox_core::i18n::count(*files as u64, "file"),
                    format::size(*bytes)
                ),
            )],
            _ => Vec::new(),
        }
    }
}

/// What the preview shows: path, modified time and cloud status.
type PreviewKey = (
    PathBuf,
    Option<SystemTime>,
    Option<ox_core::cloud::CloudStatus>,
);

/// The preview of one item, cached until the selection (or the file) changes.
#[derive(Default)]
pub struct PreviewState {
    key: Option<PreviewKey>,
    content: Option<Content>,
    /// The tab last picked; used whenever the file offers it.
    wanted: Option<PreviewTab>,
    /// Text for the font preview.
    sample: String,
    /// The previewed font installed into egui (removed when the next loads).
    font_family: Option<String>,
    /// Layout cache of the Markdown view.
    markdown: egui_commonmark::CommonMarkCache,
    /// The last commit that changed the item (in Git repositories) and the
    /// item's details. Looked up on their own, so a slow preview doesn't
    /// hold them back.
    last_commit: Option<Commit>,
    details: Option<Details>,
    extra_rx: Option<Receiver<(Option<Commit>, Details)>>,
    /// SHA-256 and MD5, once asked for.
    checksums: Option<Checksums>,
    play: PlayerState,
    /// The Document tab's preview handler.
    document: document::DocumentState,
    hex: hex::HexView,
    /// The Git tab's changes, branches and diffs.
    git: git_panel::GitPanel,
    /// Quick Look: opens on what the file shows rather than Info, with
    /// sharper pictures, pictures fitted, folder contents and autoplay.
    quick_look: bool,
}

enum Checksums {
    Running(Receiver<Result<(String, String), String>>),
    Done(Result<(String, String), String>),
}

impl PreviewState {
    pub fn for_quick_look() -> Self {
        Self {
            quick_look: true,
            ..Default::default()
        }
    }

    /// Make sure the preview shows `entry`, starting a background load if needed.
    fn update(&mut self, ctx: &egui::Context, entry: &FileEntry, show_hidden: bool) {
        let key = (entry.path.clone(), entry.modified, entry.cloud);
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            let (tx, rx) = crossbeam_channel::bounded(1);
            let (extra_tx, extra_rx) = crossbeam_channel::bounded(1);
            let (e, repaint) = (entry.clone(), ctx.clone());
            std::thread::spawn(move || {
                let details = if ox_core::archive::split_virtual(&e.path).is_some() {
                    Details::default()
                } else {
                    ox_core::details::read(&e.path, e.is_dir)
                };
                let _ = extra_tx.send((last_commit(&e), details));
                repaint.request_repaint();
            });
            let (entry, repaint, large) = (entry.clone(), ctx.clone(), self.quick_look);
            std::thread::spawn(move || {
                let _ = tx.send(load(entry, show_hidden, large));
                repaint.request_repaint();
            });
            self.content = Some(Content::Loading(rx));
            self.last_commit = None;
            self.details = None;
            self.extra_rx = Some(extra_rx);
            self.checksums = None;
            self.play.playback = None;
            if self.quick_look {
                self.play.autoplay = true;
            }
        }
        if let Some((commit, details)) = self.extra_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.last_commit = commit;
            self.details = Some(details);
            self.extra_rx = None;
        }
        if let Some(Content::Loading(rx)) = &self.content {
            if let Ok(loaded) = rx.try_recv() {
                self.content = Some(match loaded {
                    Loaded::Image(img, size) => {
                        let tex = ctx.load_texture("preview", img, TextureOptions::LINEAR);
                        Content::Image(tex, size)
                    }
                    Loaded::Animation(frames, size) => {
                        Content::Animation(animation::Animation::new(ctx, frames, size))
                    }
                    Loaded::Page(img, pages) => {
                        let tex = ctx.load_texture("preview", img, TextureOptions::LINEAR);
                        Content::Page(tex, pages)
                    }
                    Loaded::Text {
                        text,
                        numbers,
                        truncated,
                        details,
                    } => Content::Text {
                        text,
                        numbers,
                        truncated,
                        details,
                        markdown: is_markdown(entry),
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
                        git,
                        items,
                    } => Content::Folder {
                        folders,
                        files,
                        bytes,
                        git,
                        items,
                    },
                    Loaded::Font { data, info, chars } => Content::Font {
                        family: self.install_font(ctx, data),
                        info,
                        chars,
                    },
                    Loaded::Data(d) => data::content(*d),
                    Loaded::Nothing => Content::Nothing,
                    Loaded::Error(e) => Content::Error(e),
                });
            }
        }
    }

    /// Make the font drawable under its own family name, replacing the
    /// previously previewed font so they don't pile up in memory.
    fn install_font(&mut self, ctx: &egui::Context, data: Vec<u8>) -> String {
        use egui::{FontData, FontFamily};
        // Shared by the pane and Quick Look, so their names differ.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let name = format!("oxplorer-preview-{seq}");
        let mut defs = ctx.fonts_mut(|f| f.definitions().clone());
        if let Some(old) = self.font_family.take() {
            defs.font_data.remove(&old);
            defs.families.remove(&FontFamily::Name(old.into()));
        }
        defs.font_data.insert(
            name.clone(),
            std::sync::Arc::new(FontData::from_owned(data)),
        );
        defs.families
            .insert(FontFamily::Name(name.clone().into()), vec![name.clone()]);
        ctx.set_fonts(defs);
        self.font_family = Some(name.clone());
        name
    }

    fn clear(&mut self) {
        self.key = None;
        self.content = None;
        self.play.playback = None;
        self.hex = Default::default();
    }

    /// Stop and close the player.
    pub fn stop_playback(&mut self) {
        self.play.playback = None;
    }

    /// Start on the Info tab next time.
    pub fn reset_tab(&mut self) {
        self.wanted = None;
    }

    /// Let go of everything (Quick Look closing): the player stops and a
    /// previewed font is taken out of egui again.
    pub fn release(&mut self, ctx: &egui::Context) {
        self.clear();
        if let Some(old) = self.font_family.take() {
            let mut defs = ctx.fonts_mut(|f| f.definitions().clone());
            defs.font_data.remove(&old);
            defs.families.remove(&egui::FontFamily::Name(old.into()));
            ctx.set_fonts(defs);
        }
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
    // In a Git repository: the item's state there.
    let git_state = app.tabs.active().view.git.as_ref().and_then(|git| {
        let [e] = selected.as_slice() else {
            return None;
        };
        e.path.starts_with(&git.root).then(|| {
            git.state(&e.path)
                .map_or(tr("Unchanged"), |s| tr(s.label()))
        })
    });
    let date_fmt = app.settings.browsing.date_format.clone();
    let mut actions = Vec::new();

    ui.add_space(4.0);
    match selected.as_slice() {
        [] => {
            app.preview.clear();
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("👁").size(32.0).weak());
                ui.label(RichText::new(tr("Select a file to preview it.")).weak());
            });
        }
        [entry] if !in_bin => {
            app.preview
                .update(ui.ctx(), entry, app.settings.browsing.show_hidden);
            header(ui, entry);
            let total = entry
                .is_dir
                .then(|| app.folder_sizes.get(&entry.path))
                .flatten();
            // The Player tab plays with the app's player (see `preview/player.rs`).
            std::mem::swap(&mut app.preview.play, &mut app.player);
            app.preview.git.status = app.tabs.active().view.git.clone();
            tabbed(
                ui,
                &mut app.preview,
                entry,
                total,
                git_state,
                &date_fmt,
                &mut actions,
            );
            std::mem::swap(&mut app.preview.play, &mut app.player);
        }
        [entry] => {
            app.preview.clear();
            header(ui, entry);
            ui.separator();
            info_grid(ui, entry, None, Vec::new(), &date_fmt);
        }
        many => {
            app.preview.clear();
            let files: Vec<&FileEntry> = many.iter().filter(|e| !e.is_dir).collect();
            let bytes: u64 = files.iter().map(|e| e.size).sum();
            ui.label(
                RichText::new(ox_core::trf!(
                    "{v1} selected",
                    v1 = ox_core::i18n::count(many.len() as u64, "item")
                ))
                .strong()
                .size(15.0),
            );
            ui.label(format!(
                "{} · {} · {}",
                ox_core::i18n::count((many.len() - files.len()) as u64, "folder"),
                ox_core::i18n::count(files.len() as u64, "file"),
                format::size(bytes)
            ));
        }
    }
    app.actions.extend(actions);
}

/// Quick Look's preview of `entry` in the space left (see `quick_look`).
pub fn show_large(
    ui: &mut Ui,
    state: &mut PreviewState,
    entry: &FileEntry,
    show_hidden: bool,
    total: Option<DirSize>,
    date_fmt: &str,
    actions: &mut Vec<Action>,
) {
    state.update(ui.ctx(), entry, show_hidden);
    tabbed(ui, state, entry, total, None, date_fmt, actions);
}

fn header(ui: &mut Ui, e: &FileEntry) {
    ui.label(RichText::new(&e.name).strong().size(15.0));
    ui.label(RichText::new(e.type_label()).weak());
}

/// The tab row and the chosen tab's content.
fn tabbed(
    ui: &mut Ui,
    state: &mut PreviewState,
    e: &FileEntry,
    total: Option<DirSize>,
    git_state: Option<&'static str>,
    date_fmt: &str,
    actions: &mut Vec<Action>,
) {
    let Some(content) = state
        .content
        .as_ref()
        .filter(|c| !matches!(c, Content::Loading(_)))
    else {
        ui.separator();
        ui.spinner();
        return;
    };
    let mut tabs = content.tabs();
    document::offer(&mut tabs, &mut state.document, content, e);
    state.hex.offer(e, &mut tabs);
    state.git.offer(e, &mut tabs);
    // The tab picked last, if this file has it; otherwise Info (Quick
    // Look: what the file shows).
    let first = match state.quick_look {
        true => tabs.get(1).copied().unwrap_or(PreviewTab::Info),
        false => PreviewTab::Info,
    };
    let tab = state.wanted.filter(|t| tabs.contains(t)).unwrap_or(first);
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for t in &tabs {
            if ui.selectable_label(*t == tab, t.label()).clicked() {
                state.wanted = Some(*t);
            }
        }
    });
    ui.separator();

    let content = state.content.as_ref().expect("checked above");
    if large::show(ui, state.quick_look, tab, content) {
        return;
    }
    if data::show(ui, &mut state.hex, tab, content, e, actions) {
        return;
    }
    // Quick Look: the video and its buttons fit without scrolling.
    let max_h = match state.quick_look {
        true => (ui.available_height() - 90.0).max(120.0),
        false => f32::INFINITY,
    };
    if let (PreviewTab::Glyphs, Content::Font { family, chars, .. }) = (tab, content) {
        glyph_grid(ui, family, chars, actions);
        return;
    }
    if let (PreviewTab::Git, Content::Folder { git: Some(git), .. }) = (tab, content) {
        git_changes::git_tab(ui, &mut state.git, git, date_fmt, actions);
        return;
    }
    if tab == PreviewTab::Diff {
        return git_diff::file_tab(ui, &mut state.git, e);
    }
    if tab == PreviewTab::Document {
        return document::show(ui, &mut state.document, e, actions);
    }
    egui::ScrollArea::vertical()
        .id_salt("preview_tab")
        .auto_shrink([false, false])
        .show(ui, |ui| match tab {
            PreviewTab::Info => {
                let mut facts = content.facts();
                if let Some(t) = total.filter(|t| t.folders > 0) {
                    facts.push((
                        tr("Total size"),
                        format!(
                            "{} ({}, {})",
                            format::size(t.bytes),
                            ox_core::i18n::count(t.files, "file"),
                            ox_core::i18n::count(t.folders, "folder")
                        ),
                    ));
                }
                if let Some(s) = git_state {
                    facts.push((tr("Git"), s.to_owned()));
                }
                if let Some(c) = &state.last_commit {
                    facts.push((
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
                info_grid(ui, e, state.details.as_ref(), facts, date_fmt);
                picture::map_button(ui, state.details.as_ref());
                if !e.is_dir && ox_core::archive::split_virtual(&e.path).is_none() {
                    ui.add_space(6.0);
                    checksums(ui, &mut state.checksums, e, actions);
                }
                ui.add_space(8.0);
                let playable =
                    matches!(content, Content::Media { .. }) && ox_core::player::supported();
                let label = match content {
                    Content::Media { .. } => tr("▶ Play"),
                    Content::Folder { .. } => tr("📂 Open"),
                    _ => tr("📂 Open with default app"),
                };
                ui.horizontal(|ui| {
                    if ui.button(label).clicked() {
                        if playable {
                            state.wanted = Some(PreviewTab::Player);
                            state.play.autoplay = true;
                        } else {
                            actions.push(Action::Open(e.path.clone()));
                        }
                    }
                    if playable && ui.button(tr("📂 Open with default app")).clicked() {
                        actions.push(Action::Open(e.path.clone()));
                    }
                });
                if let Content::Error(err) = content {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), err);
                }
            }
            PreviewTab::FontSample => {
                if let Content::Font { family, .. } = content {
                    font_sample(ui, family, &mut state.sample);
                }
            }
            PreviewTab::Player => {
                let cover = match content {
                    Content::Media { cover, .. } => cover.as_ref(),
                    _ => None,
                };
                player_tab(ui, &mut state.play, e, cover, max_h);
            }
            PreviewTab::Markdown => {
                if let Content::Text { text, .. } = content {
                    markdown(ui, &mut state.markdown, text, e);
                }
            }
            _ => body(ui, content, e),
        });
}

/// A Markdown file rendered. Relative image paths are taken from its folder.
fn markdown(ui: &mut Ui, cache: &mut egui_commonmark::CommonMarkCache, text: &str, e: &FileEntry) {
    let folder = e
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    egui_commonmark::CommonMarkViewer::new()
        .default_implicit_uri_scheme(format!("file://{folder}/"))
        .max_image_width(Some(ui.available_width().max(50.0) as usize))
        .show(ui, cache, text);
}

/// The type-specific tabs (Image, Text, Page, Cover).
fn body(ui: &mut Ui, content: &Content, e: &FileEntry) {
    match content {
        Content::Image(tex, _) => {
            let avail = ui.available_width();
            let size = tex.size_vec2();
            let scale = (avail / size.x).min(1.5);
            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
        }
        Content::Animation(a) => a.show(ui),
        Content::Text {
            text,
            numbers,
            truncated,
            ..
        } => {
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
                            // The highlighter takes the font from the style, not the
                            // theme: give it the line numbers' size so rows line up.
                            let mut style = (**ui.style()).clone();
                            style.override_font_id = Some(egui::FontId::monospace(SIZE));
                            let job = highlight(ui.ctx(), &style, &theme, text, &e.extension());
                            egui::Label::new(job)
                        } else {
                            egui::Label::new(RichText::new(text.as_str()).monospace().size(SIZE))
                        };
                        ui.add(code.wrap_mode(egui::TextWrapMode::Extend));
                    });
                });
            if *truncated {
                ui.label(
                    RichText::new(tr("Only the beginning of the file is shown."))
                        .weak()
                        .small(),
                );
            }
        }
        Content::Page(tex, pages) => {
            let avail = ui.available_width();
            let size = tex.size_vec2();
            ui.add(egui::Image::new((tex.id(), size * (avail / size.x))).corner_radius(4.0));
            ui.label(
                RichText::new(ox_core::trf!("Page 1 of {pages}", pages = pages))
                    .weak()
                    .small(),
            );
        }
        Content::Media {
            cover: Some(tex), ..
        } => {
            let avail = ui.available_width();
            let size = tex.size_vec2();
            let scale = (avail / size.x).min(1.0);
            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(6.0));
        }
        Content::Data(d) => d.text.iter().for_each(|t| body(ui, t, e)),
        _ => {}
    }
}
