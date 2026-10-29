"""Build crates/ox-app/assets/icons.ttf: Oxplorer's icons, drawn by Lucide.

The interface writes its icons as symbols in the text ("📂 Open", "✂ Cut").
This font gives those symbols Lucide's line icons (https://lucide.dev, ISC
licence). The app puts it first in egui's font list, so every label, menu and
translation shows Lucide icons without changing a single string. Symbols that
aren't listed here (emoji in file names, say) still come from the other fonts.

The icons are scaled to the size of capital letters of the interface font and
centred on them, so they line up with the text next to them.

Usage (needs `pip install fonttools`):

    npm pack lucide-static        # or download lucide-static-<version>.tgz
    tar xzf lucide-static-*.tgz
    python3 tools/icon-font/build_icon_font.py package/font

Then copy package/LICENSE to crates/ox-app/assets/LICENSE-lucide.txt if the
version changed.
"""

import json
import sys
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.recordingPen import RecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

# Symbol in the code → Lucide icon. A name ending in "+fill" keeps only the
# icon's outer outline, which makes a filled shape (★ for "is a favorite").
ICONS = {
    # Files and folders
    "📁": "folder",
    "📂": "folder-open",
    "📄": "file",
    "📝": "file-pen-line",
    "📖": "book-open",
    "🖼": "image",
    "🎵": "music",
    "🎬": "clapperboard",
    "📦": "package",
    "🗑": "trash-2",
    "♻": "undo-2",
    "⛔": "ban",
    # Editing and clipboard
    "✂": "scissors",
    "📋": "copy",
    "📥": "clipboard-paste",
    "📤": "upload",
    "✏": "pencil",
    "➕": "plus",
    "✖": "x",
    "✔": "check",
    "☑": "square-check",
    "🔗": "link",
    "🔐": "hash",
    # Navigation
    "⬅": "arrow-left",
    "➡": "arrow-right",
    "⬆": "arrow-up",
    "⬇": "arrow-down",
    "🔄": "refresh-cw",
    "↻": "rotate-cw",
    "↺": "rotate-ccw",
    "⟲": "rotate-ccw",
    "🏠": "house",
    "🕘": "history",
    "🔍": "search",
    "⭐": "star",
    "☆": "star",
    "★": "star+fill",
    "🏷": "tag",
    "📑": "copy-plus",
    "🔖": "group",
    "⛓": "unlink",
    "⏵": "chevron-right",
    "⏶": "chevron-up",
    "⏷": "chevron-down",
    # Places
    "🖴": "hard-drive",
    "🖧": "network",
    "🌐": "globe",
    "🖥": "monitor",
    "🖳": "square-terminal",
    "🗗": "external-link",
    "☁": "cloud",
    "📌": "pin",
    # Views and panels
    "☰": "list",
    "▦": "layout-grid",
    "◧": "panel-left",
    "◨": "panel-right",
    "◫": "columns-2",
    "▭": "square-terminal",
    "▬": "panel-bottom",
    "👁": "eye",
    "👻": "eye-off",
    "🔤": "file-type",
    "🔣": "type",
    "⇅": "arrow-up-down",
    "⇄": "arrow-left-right",
    "⇆": "arrow-right-left",
    "⋯": "ellipsis",
    "⌨": "keyboard",
    # Player
    "▶": "play",
    "⏸": "pause",
    "⏹": "square",
    "⏩": "fast-forward",
    "⏪": "rewind",
    "🔁": "repeat",
    "🔊": "volume-2",
    "🔇": "volume-x",
    # Settings, help and the rest
    "⚙": "settings",
    "🎨": "palette",
    "☀": "sun",
    "🌙": "moon",
    "🚀": "rocket",
    "🛠": "wrench",
    "ℹ": "info",
    "❓": "circle-help",
    "⚠": "triangle-alert",
    "⚡": "zap",
    "✨": "sparkles",
    "📜": "scroll-text",
    "🌿": "git-branch",
}

UNITS = 1000
# Icon size in font units (the interface font's capitals are 693 high) …
SIZE = 820
# … centred on half the capital height …
CENTER_Y = 346
# … with this much room on each side.
SIDE = 40
# The interface font's (Ubuntu Light) line metrics, so lines keep their height.
ASCENT, DESCENT = 932, -189


def contours(recording):
    """Split a recorded outline into its contours."""
    out, current = [], []
    for op, args in recording.value:
        current.append((op, args))
        if op in ("closePath", "endPath"):
            out.append(current)
            current = []
    if current:
        out.append(current)
    return out


def bounds_area(contour):
    pts = [p for _, args in contour for p in args]
    if not pts:
        return 0
    xs, ys = [p[0] for p in pts], [p[1] for p in pts]
    return (max(xs) - min(xs)) * (max(ys) - min(ys))


def main(font_dir):
    font_dir = Path(font_dir)
    info = json.loads((font_dir / "info.json").read_text())
    lucide = TTFont(font_dir / "lucide.ttf")
    cmap = lucide.getBestCmap()
    glyphs = lucide.getGlyphSet()
    src_units = lucide["head"].unitsPerEm

    scale = SIZE / src_units
    # Lucide's icons fill its em square from the baseline up.
    dy = CENTER_Y - SIZE / 2
    order, outlines, char_map = [".notdef"], {}, {}
    empty = TTGlyphPen(None)
    outlines[".notdef"] = empty.glyph()
    for symbol, spec in ICONS.items():
        name, _, variant = spec.partition("+")
        if name not in info:
            sys.exit(f"Lucide has no icon {name!r} (for {symbol})")
        source = cmap[int(info[name]["encodedCode"].lstrip("\\"), 16)]
        glyph_name = f"{name}.{variant}" if variant else name
        char_map[ord(symbol)] = glyph_name
        if glyph_name in outlines:
            continue
        recording = RecordingPen()
        glyphs[source].draw(recording)
        parts = contours(recording)
        if variant == "fill":
            parts = [max(parts, key=bounds_area)]
        pen = TTGlyphPen(None)
        moved = TransformPen(pen, (scale, 0, 0, scale, SIDE, dy))
        for part in parts:
            for op, args in part:
                getattr(moved, op)(*args)
        outlines[glyph_name] = pen.glyph()
        order.append(glyph_name)

    fb = FontBuilder(UNITS, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(char_map)
    fb.setupGlyf(outlines)
    advance = SIZE + 2 * SIDE
    fb.setupHorizontalMetrics(
        {g: (advance if g != ".notdef" else 0, 0) for g in order}
    )
    fb.setupHorizontalHeader(ascent=ASCENT, descent=DESCENT)
    fb.setupNameTable(
        {
            "familyName": "Oxplorer Icons",
            "styleName": "Regular",
            "copyright": "Icons from Lucide (https://lucide.dev), ISC licence",
        }
    )
    fb.setupOS2(
        sTypoAscender=ASCENT,
        sTypoDescender=DESCENT,
        usWinAscent=ASCENT,
        usWinDescent=-DESCENT,
    )
    fb.setupPost()
    out = Path(__file__).resolve().parents[2] / "crates/ox-app/assets/icons.ttf"
    fb.save(out)
    print(f"{out}: {len(order) - 1} icons for {len(char_map)} symbols")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "package/font")
