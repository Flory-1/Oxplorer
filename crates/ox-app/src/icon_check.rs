//! Every symbol used as an icon must be in the icon font, or it would be
//! drawn by an emoji font instead (Windows' colour emoji, in the worst case).
//! Add new ones to `tools/icon-font/build_icon_font.py` and rebuild the font.

/// Symbols that are text, not icons, and stay in the text font.
const TEXT_SYMBOLS: &str = "↑→↓↔↗≈≤●";

#[test]
fn every_icon_symbol_is_in_the_icon_font() {
    let font = ttf_parser::Face::parse(include_bytes!("../assets/icons.ttf"), 0)
        .expect("icons.ttf parses");
    let mut missing = Vec::new();
    for (file, src) in crate::i18n_check::SOURCES {
        // Comments may use any character.
        let code = src
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        for c in code.chars() {
            let symbol = (c as u32) > 0x2100 && !"–—’“”„…€".contains(c);
            if symbol
                && !TEXT_SYMBOLS.contains(c)
                && font.glyph_index(c).is_none()
                && !missing.iter().any(|(m, _)| *m == c)
            {
                missing.push((c, *file));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "not in the icon font (add them to tools/icon-font/build_icon_font.py): {missing:?}"
    );
}
