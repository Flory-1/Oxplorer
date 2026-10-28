//! Vector icons for folders, files and drives, painted with egui shapes so
//! they look crisp at every size. Real shell icons and thumbnails are planned
//! (see ROADMAP Phase 2).

use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke};
use ox_core::entry::{FileEntry, FileKind};

pub const HOME: &str = "🏠";

const FOLDER_BACK: Color32 = Color32::from_rgb(0xe0, 0xa1, 0x1b);
const FOLDER_FRONT: Color32 = Color32::from_rgb(0xf7, 0xc5, 0x4a);

/// Accent colour for a file kind (used for the strip on file icons).
pub fn kind_color(kind: FileKind) -> Color32 {
    match kind {
        FileKind::Folder => FOLDER_FRONT,
        FileKind::Image => Color32::from_rgb(0x9b, 0x6b, 0xf2),
        FileKind::Video => Color32::from_rgb(0xe0, 0x5c, 0xb0),
        FileKind::Audio => Color32::from_rgb(0x2c, 0xb5, 0xc8),
        FileKind::Document => Color32::from_rgb(0x2f, 0x6f, 0xe0),
        FileKind::Spreadsheet => Color32::from_rgb(0x21, 0x9a, 0x5b),
        FileKind::Presentation => Color32::from_rgb(0xe8, 0x6a, 0x2c),
        FileKind::Pdf => Color32::from_rgb(0xd9, 0x36, 0x3e),
        FileKind::Archive => Color32::from_rgb(0xa8, 0x7b, 0x4c),
        FileKind::Code => Color32::from_rgb(0x3d, 0x9b, 0xe9),
        FileKind::Text => Color32::from_rgb(0x8a, 0x93, 0xa3),
        FileKind::Executable => Color32::from_rgb(0x5b, 0x67, 0x7a),
        FileKind::Other => Color32::from_rgb(0x9a, 0xa0, 0xaa),
    }
}

/// Paint the icon for an entry into a square `rect`.
pub fn paint_entry(p: &Painter, rect: Rect, e: &FileEntry, faded: bool) {
    if e.is_dir {
        paint_folder(p, rect, faded);
    } else {
        let ext = e.extension();
        let label =
            (rect.width() >= 30.0 && !ext.is_empty() && ext.len() <= 4).then_some(ext.as_str());
        paint_file(p, rect, kind_color(e.kind()), label, faded);
    }
    if e.is_symlink {
        let r = rect.width() * 0.18;
        let c = rect.left_bottom() + vec2(r, -r);
        p.circle_filled(c, r, Color32::WHITE);
        p.text(
            c,
            Align2::CENTER_CENTER,
            "↗",
            FontId::proportional(r * 1.6),
            Color32::from_rgb(0x2f, 0x6f, 0xe0),
        );
    }
}

fn fade(c: Color32, faded: bool) -> Color32 {
    if faded {
        c.gamma_multiply(0.45)
    } else {
        c
    }
}

pub fn paint_folder(p: &Painter, rect: Rect, faded: bool) {
    let s = rect.width();
    let r = (s * 0.08).round().clamp(1.0, 6.0) as u8;
    let top = rect.top() + s * 0.12;
    let bottom = rect.bottom() - s * 0.1;
    // Back with tab
    let tab = Rect::from_min_size(pos2(rect.left(), top), vec2(s * 0.42, s * 0.2));
    p.rect_filled(tab, CornerRadius::same(r), fade(FOLDER_BACK, faded));
    let back = Rect::from_min_max(pos2(rect.left(), top + s * 0.1), pos2(rect.right(), bottom));
    p.rect_filled(back, CornerRadius::same(r), fade(FOLDER_BACK, faded));
    // Front flap
    let front = Rect::from_min_max(
        pos2(rect.left(), top + s * 0.24),
        pos2(rect.right(), bottom),
    );
    p.rect_filled(front, CornerRadius::same(r), fade(FOLDER_FRONT, faded));
}

pub fn paint_file(p: &Painter, rect: Rect, color: Color32, label: Option<&str>, faded: bool) {
    let s = rect.height();
    let w = s * 0.76;
    let page = Rect::from_center_size(rect.center(), vec2(w, s * 0.96));
    let fold = w * 0.3;
    let paper = fade(Color32::from_rgb(0xf3, 0xf4, 0xf6), faded);
    let edge = fade(Color32::from_rgb(0xb8, 0xbd, 0xc6), faded);
    let pts: Vec<Pos2> = vec![
        page.left_top(),
        pos2(page.right() - fold, page.top()),
        pos2(page.right(), page.top() + fold),
        page.right_bottom(),
        page.left_bottom(),
    ];
    p.add(Shape::convex_polygon(
        pts,
        paper,
        Stroke::new((s / 40.0).max(0.8), edge),
    ));
    // Folded corner
    p.add(Shape::convex_polygon(
        vec![
            pos2(page.right() - fold, page.top()),
            pos2(page.right(), page.top() + fold),
            pos2(page.right() - fold, page.top() + fold),
        ],
        edge,
        Stroke::NONE,
    ));
    // Coloured type strip
    let strip_h = if label.is_some() { s * 0.3 } else { s * 0.2 };
    let strip = Rect::from_min_size(
        pos2(page.left(), page.bottom() - strip_h - s * 0.12),
        vec2(w, strip_h),
    );
    p.rect_filled(strip, CornerRadius::ZERO, fade(color, faded));
    if let Some(label) = label {
        p.text(
            strip.center(),
            Align2::CENTER_CENTER,
            label.to_uppercase(),
            FontId::proportional(strip_h * 0.68),
            Color32::WHITE,
        );
    }
}

pub fn paint_drive(p: &Painter, rect: Rect, accent: Color32) {
    let s = rect.width();
    let body = Rect::from_center_size(rect.center(), vec2(s, s * 0.56));
    p.rect_filled(
        body,
        CornerRadius::same((s * 0.12) as u8),
        Color32::from_rgb(0x7d, 0x86, 0x94),
    );
    let face = Rect::from_min_max(pos2(body.left(), body.center().y), body.right_bottom());
    p.rect_filled(
        face,
        CornerRadius::same((s * 0.12) as u8),
        Color32::from_rgb(0x5f, 0x68, 0x76),
    );
    p.circle_filled(
        pos2(body.right() - s * 0.16, face.center().y),
        s * 0.06,
        accent,
    );
}

/// Square rect of `size` centred vertically at the left of `x`.
pub fn icon_rect(x: f32, center_y: f32, size: f32) -> Rect {
    Rect::from_min_size(pos2(x, center_y - size / 2.0), vec2(size, size))
}

/// Any icon used in lists: a glyph from the font or one of the painted icons.
#[derive(Clone, Copy)]
pub enum Icon<'a> {
    Glyph(&'a str),
    Folder,
    Drive,
    File(FileKind),
}

pub fn paint_icon(p: &Painter, rect: Rect, icon: Icon<'_>, text_color: Color32, accent: Color32) {
    match icon {
        Icon::Glyph(g) => {
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                g,
                FontId::proportional(rect.height() * 0.95),
                text_color,
            );
        }
        Icon::Folder => paint_folder(p, rect, false),
        Icon::Drive => paint_drive(p, rect, accent),
        Icon::File(kind) => paint_file(p, rect, kind_color(kind), None, false),
    }
}
