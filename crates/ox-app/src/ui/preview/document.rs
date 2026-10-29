//! The Document tab: Office documents, mails and everything else Windows has
//! a preview handler for, drawn by the handler in a native window that is
//! kept over the tab (see `ox_core::preview_handler`). The tab only says
//! where it is; [`place_documents`] moves the window there after the frame
//! is drawn, or hides it where egui draws something over it.

use super::*;
use ox_core::preview_handler::{self as handler, Colors, DocumentPreview, Handler, Status};

#[derive(Default)]
pub(super) struct DocumentState {
    /// Settings → Preview → Office documents (copied every frame).
    enabled: bool,
    /// The handler of the file last asked about.
    lookup: Option<(PathBuf, Option<Handler>)>,
    /// Where the tab was drawn this frame.
    drawn: Option<Drawn>,
    preview: Option<Result<DocumentPreview, String>>,
    /// The file (as the preview knows it) the handler was given.
    shown: Option<PreviewKey>,
}

struct Drawn {
    path: PathBuf,
    rect: egui::Rect,
    layer: egui::LayerId,
    colors: Colors,
}

impl DocumentState {
    fn handler(&mut self, path: &Path) -> Option<&Handler> {
        if self.lookup.as_ref().is_none_or(|(p, _)| p != path) {
            self.lookup = Some((path.to_owned(), handler::handler_for(path)));
        }
        self.lookup.as_ref().and_then(|(_, h)| h.as_ref())
    }
}

/// Files Oxplorer shows itself: pictures, text and code, PDF, fonts,
/// audio and video, archives (and programs, which have no preview).
fn native(e: &FileEntry) -> bool {
    use FileKind::*;
    is_image(e)
        || is_text(e)
        || is_markdown(e)
        || matches!(
            e.kind(),
            Folder | Image | Video | Audio | Pdf | Archive | Code | Text | Executable | Font
        )
        || ox_core::archive::kind(&e.path).is_some()
}

/// Add the Document tab (after Info) for files with a preview handler that
/// Oxplorer doesn't show itself: not for folders, items in archives or
/// files that are only online (the handler would download them).
pub(super) fn offer(
    tabs: &mut Vec<PreviewTab>,
    doc: &mut DocumentState,
    content: &Content,
    e: &FileEntry,
) {
    let wanted = doc.enabled
        && !e.is_dir
        && e.cloud != Some(ox_core::cloud::CloudStatus::OnlineOnly)
        && ox_core::archive::split_virtual(&e.path).is_none()
        && !native(e)
        && !matches!(
            content,
            Content::Image(..)
                | Content::Page(..)
                | Content::Media { .. }
                | Content::Font { .. }
                | Content::Folder { .. }
        );
    if wanted && doc.handler(&e.path).is_some() {
        tabs.insert(tabs.len().min(1), PreviewTab::Document);
    }
}

/// The Document tab: the space the handler draws in, and what it is doing
/// until then.
pub(super) fn show(ui: &mut Ui, doc: &mut DocumentState, e: &FileEntry, actions: &mut Vec<Action>) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    // In the pane the panel's colour; in Quick Look or the preview window
    // the window's (without the see-through backdrop's transparency).
    let v = ui.visuals();
    let back = match ui.layer_id().order {
        egui::Order::Background => v.panel_fill,
        _ => v.window_fill,
    };
    let rgb = |c: egui::Color32| {
        let [r, g, b, _] = c.to_srgba_unmultiplied();
        [r, g, b]
    };
    doc.drawn = Some(Drawn {
        path: e.path.clone(),
        rect: rect.intersect(ui.clip_rect()),
        layer: ui.layer_id(),
        colors: Colors {
            background: rgb(back),
            text: rgb(v.text_color()),
        },
    });
    let status = match &mut doc.preview {
        Some(Ok(p)) if p.path() == Some(e.path.as_path()) => p.status(),
        Some(Err(err)) => Status::Failed(err.clone()),
        _ => Status::Loading,
    };
    let message = match status {
        Status::Shown => return, // the handler draws over it
        Status::Idle | Status::Loading => {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tr("Loading…")).weak());
                });
            });
            return;
        }
        Status::TimedOut => tr("The preview is taking too long.").to_owned(),
        Status::Failed(err) => ox_core::trf!("Can't show this document: {e}", e = err),
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.add_space(8.0);
        ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), message);
        ui.add_space(6.0);
        if ui.button(tr("📂 Open with default app")).clicked() {
            actions.push(Action::Open(e.path.clone()));
        }
    });
}

/// After the frame: move the handlers' windows over their tabs, or hide them
/// (tab not shown, window minimised, something drawn over them). A file
/// that is no longer shown is unloaded.
pub fn place_documents(app: &mut OxApp, ctx: &egui::Context, frame: &eframe::Frame) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let parent = match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
        _ => None,
    };
    let enabled = app.settings.preview.documents && handler::supported();
    let layered = app.see_through;
    place(&mut app.preview, ctx, parent, layered, enabled);
    if let Some(ql) = &mut app.quick_look {
        place(&mut ql.preview, ctx, parent, layered, enabled);
    }
}

fn place(
    state: &mut PreviewState,
    ctx: &egui::Context,
    parent: Option<isize>,
    layered: bool,
    enabled: bool,
) {
    let doc = &mut state.document;
    doc.enabled = enabled;
    let drawn = doc.drawn.take();
    if !enabled {
        doc.preview = None; // its thread unloads the handler
        doc.shown = None;
        return;
    }
    // Unloaded when the tab isn't shown (the handler may keep the file open)
    // or another file is selected.
    let drawn = drawn.filter(|d| state.key.as_ref().is_some_and(|k| k.0 == d.path));
    let Some(drawn) = drawn else {
        if let Some(Ok(p)) = &mut doc.preview {
            p.unload();
            p.place(None);
        }
        doc.shown = None;
        return;
    };
    if doc.shown.is_some() && doc.shown != state.key {
        if let Some(Ok(p)) = &mut doc.preview {
            p.unload();
        }
        doc.shown = None;
    }
    if doc.preview.is_none() {
        let Some(parent) = parent else {
            return;
        };
        let repaint = ctx.clone();
        doc.preview = Some(
            DocumentPreview::new(parent, layered, move || repaint.request_repaint())
                .map_err(|e| e.to_string()),
        );
    }
    let handler = doc.handler(&drawn.path).cloned();
    let Some(Ok(preview)) = &mut doc.preview else {
        return;
    };
    let hidden = ctx.input(|i| i.viewport().minimized.unwrap_or(false))
        || egui::DragAndDrop::has_any_payload(ctx)
        || covered(ctx, drawn.layer, drawn.rect);
    let ppp = ctx.pixels_per_point();
    let rect = (!hidden && drawn.rect.is_positive())
        .then(|| handler::to_pixels(drawn.rect.min.into(), drawn.rect.max.into(), ppp));
    // Placed first, so the handler starts out at the right size.
    preview.place(rect);
    if doc.shown.is_none() {
        if let Some(h) = handler {
            preview.open(&drawn.path, &h, drawn.colors);
            doc.shown = state.key.clone();
        }
    }
    if preview.status() == Status::Loading {
        // Tell when it takes too long, even if nothing else happens.
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

/// Does a menu, popup, tooltip or window above `layer` overlap `rect`? The
/// native window would hide it.
fn covered(ctx: &egui::Context, layer: egui::LayerId, rect: egui::Rect) -> bool {
    use egui::Order;
    ctx.memory(|m| {
        let order: Vec<egui::LayerId> = m.layer_ids().collect();
        let ours = order.iter().position(|l| *l == layer);
        let visible = m.areas().visible_layer_ids();
        order
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                *l != &layer
                    && !matches!(l.order, Order::Background | Order::Debug)
                    && visible.contains(*l)
                    && (l.order > layer.order || ours.is_none_or(|o| *i > o))
            })
            .any(|(_, l)| m.area_rect(l.id).is_some_and(|r| r.intersects(rect)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(format!(r"C:\docs\{name}")),
            name: name.into(),
            is_dir: false,
            is_symlink: false,
            size: 1,
            modified: None,
            created: None,
            hidden: false,
            readonly: false,
            cloud: None,
        }
    }

    #[test]
    fn oxplorer_keeps_its_own_previews() {
        for name in [
            "a.png", "a.txt", "a.md", "a.csv", "a.rs", "a.pdf", "a.mp4", "a.zip", "a.ttf",
        ] {
            assert!(native(&entry(name)), "{name}");
        }
        for name in [
            "a.docx", "a.xlsx", "a.pptx", "a.msg", "a.eml", "a.vsdx", "a.rtf",
        ] {
            assert!(!native(&entry(name)), "{name}");
        }
    }
}
