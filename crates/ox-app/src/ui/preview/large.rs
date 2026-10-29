//! Quick Look's own views: pictures and pages fitted into the space, and
//! the items of folders and archives.

use super::*;

/// Draws `tab` if it is one of these. Returns whether it did.
pub(super) fn show(ui: &mut Ui, quick_look: bool, tab: PreviewTab, content: &Content) -> bool {
    match (tab, content) {
        (PreviewTab::Contents, Content::Folder { items, .. }) => contents(ui, items),
        _ if !quick_look => return false,
        (PreviewTab::Image, Content::Image(tex, _)) => fitted(ui, tex, None),
        (PreviewTab::Page, Content::Page(tex, pages)) => fitted(
            ui,
            tex,
            Some(ox_core::trf!("Page 1 of {pages}", pages = pages)),
        ),
        (
            PreviewTab::Media,
            Content::Media {
                cover: Some(tex), ..
            },
        ) => fitted(ui, tex, None),
        _ => return false,
    }
    true
}

/// The picture as large as fits (small ones at most doubled), centred,
/// with an optional line below it.
fn fitted(ui: &mut Ui, tex: &TextureHandle, note: Option<String>) {
    let note_h = if note.is_some() { 22.0 } else { 0.0 };
    let avail = ui.available_size() - egui::vec2(0.0, note_h);
    let avail = avail.max(egui::vec2(1.0, 1.0));
    let size = tex.size_vec2().max(egui::vec2(1.0, 1.0));
    let scale = (avail.x / size.x).min(avail.y / size.y).min(2.0);
    let (rect, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
    let r = egui::Rect::from_center_size(rect.center(), size * scale);
    ui.put(r, egui::Image::new((tex.id(), r.size())).corner_radius(4.0));
    if let Some(note) = note {
        ui.vertical_centered(|ui| ui.label(RichText::new(note).weak().small()));
    }
}

/// What's in a folder or archive: one row per item, folders first.
fn contents(ui: &mut Ui, items: &[FileEntry]) {
    let row_h = 22.0;
    let full = items.len() >= MAX_ITEMS;
    egui::ScrollArea::vertical()
        .id_salt("quick_look_items")
        .auto_shrink([false, false])
        .show_rows(ui, row_h, items.len() + usize::from(full), |ui, range| {
            for i in range {
                let Some(e) = items.get(i) else {
                    ui.label(
                        RichText::new(ox_core::trf!(
                            "Only the first {v1} items are shown.",
                            v1 = MAX_ITEMS
                        ))
                        .weak()
                        .small(),
                    );
                    continue;
                };
                let (rect, resp) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), row_h),
                    egui::Sense::hover(),
                );
                let v = ui.visuals();
                if resp.hovered() {
                    ui.painter()
                        .rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
                }
                let icon = crate::icons::icon_rect(rect.left() + 4.0, rect.center().y, 16.0);
                crate::icons::paint_entry(ui.painter(), icon, e, e.hidden, None);
                let color = if e.hidden {
                    v.weak_text_color()
                } else {
                    v.text_color()
                };
                let font = egui::FontId::proportional(13.0);
                let size_w = if e.is_dir { 0.0 } else { 90.0 };
                let mut job = egui::text::LayoutJob::single_section(
                    e.name.clone(),
                    egui::TextFormat::simple(font.clone(), color),
                );
                job.wrap.max_width = (rect.width() - 34.0 - size_w).max(20.0);
                job.wrap.max_rows = 1;
                job.wrap.overflow_character = Some('…');
                let name = ui.painter().layout_job(job);
                let name_pos =
                    egui::pos2(icon.right() + 8.0, rect.center().y - name.size().y / 2.0);
                ui.painter().galley(name_pos, name, color);
                if !e.is_dir {
                    ui.painter().text(
                        egui::pos2(rect.right() - 6.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        format::size(e.size),
                        font,
                        v.weak_text_color(),
                    );
                }
            }
        });
}
