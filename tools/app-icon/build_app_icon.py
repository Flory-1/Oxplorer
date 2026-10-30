"""Build Oxplorer's app icon: a white folder with two chevrons cut out of
it, on a transparent background (like the monochrome taskbar icons of other
apps).

Writes crates/ox-app/assets/oxplorer.png (256 px, the window icon) and
crates/ox-app/assets/oxplorer.ico (16-256 px, the exe, shortcuts and the
installer). Needs Pillow:

    python tools/app-icon/build_app_icon.py
"""

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

S = 1024  # drawn this large, then scaled down (smooth edges)
LEFT, RIGHT = 96, 928
TAB_TOP, TAB_RIGHT = 196, 430
BACK_TOP, FRONT_TOP, BOTTOM = 270, 372, 836
RADIUS = 60
GAP = 26  # between the back and the front of the folder
CHEVRON_W, CHEVRON_H, STROKE = 118, 132, 66

ASSETS = Path(__file__).resolve().parents[2] / "crates" / "ox-app" / "assets"


def mask() -> Image.Image:
    def blank() -> Image.Image:
        return Image.new("L", (S, S), 0)

    back = blank()
    d = ImageDraw.Draw(back)
    d.rounded_rectangle((LEFT, TAB_TOP, TAB_RIGHT, BACK_TOP + RADIUS), RADIUS * 0.8, fill=255)
    d.rounded_rectangle((LEFT, BACK_TOP, RIGHT, BOTTOM), RADIUS, fill=255)

    front = blank()
    ImageDraw.Draw(front).rounded_rectangle((LEFT, FRONT_TOP, RIGHT, BOTTOM), RADIUS, fill=255)

    # The back shows only above the front, with a gap between them.
    around = front.filter(ImageFilter.MaxFilter(GAP * 2 + 1))
    shape = ImageChops.lighter(ImageChops.subtract(back, around), front)

    # Two chevrons ">>" cut out of the front.
    cut = blank()
    d = ImageDraw.Draw(cut)
    cx, cy = S // 2, (FRONT_TOP + BOTTOM) // 2
    for x in (cx - CHEVRON_W - 10, cx + 30):
        points = [(x, cy - CHEVRON_H), (x + CHEVRON_W, cy), (x, cy + CHEVRON_H)]
        d.line(points, fill=255, width=STROKE, joint="curve")
        for px, py in (points[0], points[2]):
            r = STROKE / 2
            d.ellipse((px - r, py - r, px + r, py + r), fill=255)
    return ImageChops.subtract(shape, cut)


def main() -> None:
    alpha = mask()
    white = Image.new("RGBA", (S, S), (255, 255, 255, 0))
    white.putalpha(alpha)
    white.resize((256, 256), Image.LANCZOS).save(ASSETS / "oxplorer.png")
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    white.save(ASSETS / "oxplorer.ico", sizes=[(n, n) for n in sizes])
    print("wrote", ASSETS / "oxplorer.png", "and", ASSETS / "oxplorer.ico")


if __name__ == "__main__":
    main()
