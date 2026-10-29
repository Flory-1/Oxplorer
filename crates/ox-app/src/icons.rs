//! File icons. On Windows the real Shell icons are used (loaded in the
//! background and cached per file type); everywhere else, and until a Shell
//! icon has loaded, vector icons painted with egui shapes are shown.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use egui::{
    pos2, vec2, Align2, Color32, ColorImage, Context, CornerRadius, FontId, Painter, Pos2, Rect,
    Shape, Stroke, TextureHandle, TextureId, TextureOptions,
};
use ox_core::entry::{FileEntry, FileKind};
use ox_core::shell::{IconKey, Shell};

/// Shell icons uploaded as textures, keyed like the Shell caches them.
pub struct IconCache {
    pub shell: Shell,
    textures: RefCell<HashMap<IconKey, Option<TextureHandle>>>,
}

impl IconCache {
    pub fn new(ctx: &Context) -> Self {
        let repaint = ctx.clone();
        Self {
            shell: Shell::start(move || repaint.request_repaint()),
            textures: RefCell::default(),
        }
    }

    /// Upload icons that finished loading. Call once per frame.
    pub fn poll(&self, ctx: &Context) {
        for (key, image) in self.shell.poll_icons() {
            let texture = image.map(|img| {
                let size = img.size as usize;
                let color = ColorImage::from_rgba_unmultiplied([size, size], &img.rgba);
                let options = TextureOptions {
                    mipmap_mode: Some(egui::TextureFilter::Linear),
                    ..TextureOptions::LINEAR
                };
                ctx.load_texture(format!("icon:{key:?}"), color, options)
            });
            self.textures.borrow_mut().insert(key, texture);
        }
    }

    /// Texture for an entry, if its Shell icon is loaded. Requests it otherwise.
    /// `by_name` looks the icon up by file name only (Recycle Bin entries).
    pub fn texture(&self, e: &FileEntry, by_name: bool) -> Option<TextureId> {
        let key = if by_name {
            IconKey::for_path(Path::new(&e.name), e.is_dir)
        } else {
            IconKey::for_path(&e.path, e.is_dir)
        };
        // Online-only cloud files: reading their own icon would download them.
        let online = e.cloud == Some(ox_core::cloud::CloudStatus::OnlineOnly);
        let key = match key {
            IconKey::Path(_) if by_name || (online && !e.is_dir) => {
                IconKey::Extension(e.extension())
            }
            k => k,
        };
        if let Some(t) = self.textures.borrow().get(&key) {
            return t.as_ref().map(TextureHandle::id);
        }
        self.shell.request_icon(&key);
        None
    }
}

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
        FileKind::Font => Color32::from_rgb(0xd9, 0x6c, 0x3b),
        FileKind::Other => Color32::from_rgb(0x9a, 0xa0, 0xaa),
    }
}

/// Paint the icon for an entry into a square `rect`.
/// Paint an entry's icon: the Shell icon `texture` if given, else a vector icon.
pub fn paint_entry(
    p: &Painter,
    rect: Rect,
    e: &FileEntry,
    faded: bool,
    texture: Option<TextureId>,
) {
    if let Some(tex) = texture {
        let tint = if faded {
            Color32::from_white_alpha(110)
        } else {
            Color32::WHITE
        };
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        p.image(tex, rect, uv, tint);
    } else if e.is_dir {
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

/// A network server: two stacked units with a status light each. Drawn
/// rather than taken from a font: Windows' emoji font has no 🖧.
pub fn paint_server(p: &Painter, rect: Rect, accent: Color32) {
    let s = rect.width();
    let unit = s * 0.36;
    let gap = s * 0.08;
    let top = rect.center().y - unit - gap / 2.0;
    for i in 0..2 {
        let y = top + i as f32 * (unit + gap);
        let body = Rect::from_min_size(pos2(rect.left(), y), vec2(s, unit));
        p.rect_filled(
            body,
            CornerRadius::same((s * 0.1) as u8),
            Color32::from_rgb(0x6c, 0x75, 0x83),
        );
        // Two vent lines and the light.
        for k in 0..2 {
            let x = body.left() + s * (0.16 + 0.13 * k as f32);
            p.line_segment(
                [
                    pos2(x, body.top() + unit * 0.3),
                    pos2(x, body.bottom() - unit * 0.3),
                ],
                Stroke::new((s / 14.0).max(1.0), Color32::from_rgb(0x4a, 0x52, 0x5e)),
            );
        }
        p.circle_filled(
            pos2(body.right() - s * 0.17, body.center().y),
            s * 0.07,
            accent,
        );
    }
}

/// Paint `text` (usually one emoji or symbol) with its visible pixels
/// centred on `center`. Centring the text line instead puts icons too high or
/// too low: on Windows most of them come from the emoji fonts, whose line
/// metrics differ from the UI font's.
pub fn paint_glyph_centered(p: &Painter, center: Pos2, text: &str, font: FontId, color: Color32) {
    let galley = p.layout_no_wrap(text.to_owned(), font, color);
    let ink = galley
        .rows
        .iter()
        .flat_map(|row| {
            row.row
                .glyphs
                .iter()
                .filter(|g| !g.uv_rect.is_nothing())
                .map(move |g| {
                    Rect::from_min_size(
                        row.pos + g.pos.to_vec2() + g.uv_rect.offset,
                        g.uv_rect.size,
                    )
                })
        })
        .reduce(|a, b| a.union(b))
        .unwrap_or(galley.rect);
    p.galley(center - ink.center().to_vec2(), galley, color);
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
    /// A network server (or mapped network drive).
    Server,
    File(FileKind),
    /// A filled circle, e.g. a colour tag.
    Dot(Color32),
}

pub fn paint_icon(p: &Painter, rect: Rect, icon: Icon<'_>, text_color: Color32, accent: Color32) {
    match icon {
        Icon::Glyph(g) => {
            paint_glyph_centered(
                p,
                rect.center(),
                g,
                FontId::proportional(rect.height() * 0.95),
                text_color,
            );
        }
        Icon::Folder => paint_folder(p, rect, false),
        Icon::Drive => paint_drive(p, rect, accent),
        Icon::Server => paint_server(p, rect, accent),
        Icon::File(kind) => paint_file(p, rect, kind_color(kind), None, false),
        Icon::Dot(color) => {
            p.circle_filled(rect.center(), rect.height() * 0.3, color);
        }
    }
}
