//! SVG drawings rendered with `resvg`, as large as asked for (small icons
//! are drawn up, not blown up as pixels).

use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};

use super::{Bitmap, Source};
use crate::i18n::t as tr;

/// The system's fonts for texts in drawings, loaded once (takes a moment).
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

fn tree(src: Source, with_fonts: bool) -> Result<usvg::Tree, String> {
    let mut opt = usvg::Options::default();
    let data = match src {
        Source::File(path) => {
            // Pictures it links to are next to it.
            opt.resources_dir = path.parent().map(|p| p.to_path_buf());
            std::fs::read(path).map_err(|e| e.to_string())?
        }
        Source::Bytes(bytes) => bytes.to_vec(),
    };
    if with_fonts {
        opt.fontdb = fonts();
    }
    usvg::Tree::from_data(&data, &opt)
        .map_err(|e| crate::trf!("Can't read this drawing: {e}", e = e))
}

/// The drawing's own size in pixels.
pub(super) fn size(src: Source) -> Option<[u32; 2]> {
    let size = tree(src, false).ok()?.size();
    Some([size.width().round() as u32, size.height().round() as u32])
}

/// Draw `src` with its long side `max_side` pixels.
pub(super) fn render(src: Source, max_side: u32) -> Result<Bitmap, String> {
    let tree = tree(src, true)?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if w <= 0.0 || h <= 0.0 {
        return Err(tr("The drawing is empty.").to_owned());
    }
    let scale = max_side.max(1) as f32 / w.max(h);
    let width = ((w * scale).round() as u32).max(1);
    let height = ((h * scale).round() as u32).max(1);
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| tr("The drawing is empty.").to_owned())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Ok(Bitmap {
        width,
        height,
        rgba,
        full: [w.round() as u32, h.round() as u32],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAWING: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
        <rect x="0" y="0" width="10" height="10" fill="#ff0000"/>
    </svg>"##;

    #[test]
    fn drawings_are_rendered_at_the_size_asked_for() {
        let b = render(Source::Bytes(DRAWING.as_bytes()), 100).unwrap();
        assert_eq!((b.width, b.height), (100, 50));
        assert_eq!(b.full, [20, 10]);
        assert_eq!(b.rgba.len(), 100 * 50 * 4);
        // Left half red, right half see-through.
        assert_eq!(&b.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(b.rgba[(25 * 100 + 90) * 4 + 3], 0);
        assert_eq!(size(Source::Bytes(DRAWING.as_bytes())), Some([20, 10]));
        assert!(render(Source::Bytes(b"not a drawing"), 100).is_err());
    }
}
