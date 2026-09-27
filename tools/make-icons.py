"""Draws the Basalt app icons from the same geometry as the in-app mark.

The mark (`HexMark.tsx` in both apps) is three flat-top hexagons on a
24-unit canvas: one above, two below, radius 4.3. This draws that shape on a
rounded graphite square at 1024 pixels, which `tauri icon` then turns into
every size Windows needs.

It also draws the phone app's icons straight into the Android project.
Android wants its icon in two layers, a picture and a background colour, and
crops them to whatever shape the phone's launcher uses — a circle on one, a
rounded square on another. So there the mark stands alone on a transparent
layer, small enough to sit inside the circle every launcher keeps, over the
same graphite.

    python tools/make-icons.py
    cd apps/client && npx tauri icon ../../tools/icon-source.png
    cd apps/host   && npx tauri icon ../../tools/icon-source.png

`tauri icon` also writes Android icons of its own, the whole rounded square
as the picture layer, which launchers then crop into. Run this script after
it, so the Android project has these instead.
"""

import math
from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 1024
SCALE = 4  # drawn large and shrunk, for smooth edges
BG = (15, 15, 17, 255)
EDGE = (255, 255, 255, 22)
TOP = (244, 244, 245)
LOWER = (140, 140, 144)  # the mark's lower columns, at 55% on this background

# The mark's own numbers, on its 24-unit canvas.
R = 4.3
STROKE = 1.4
DX = R * 1.5
DY = R * math.sqrt(3) * 0.5
MARK_OF_ICON = 0.74  # how much of the icon's width the 24-unit canvas takes
# On Android, of the 108-unit picture layer only the middle 66 are sure to
# show; the mark's corners have to fall inside that circle.
MARK_OF_ADAPTIVE = 0.46

ROOT = Path(__file__).resolve().parent.parent
ANDROID_RES = ROOT / "apps/client/src-tauri/gen/android/app/src/main/res"
# Each density, with the size of the old square icon and of the layer.
DENSITIES = {
    "mdpi": (48, 108),
    "hdpi": (72, 162),
    "xhdpi": (96, 216),
    "xxhdpi": (144, 324),
    "xxxhdpi": (192, 432),
}


def hexagon(cx, cy, r):
    return [(cx + r * math.cos(math.pi / 3 * i), cy + r * math.sin(math.pi / 3 * i)) for i in range(6)]


def draw_mark(draw, big, share, hollow):
    """The three columns, centred, their 24-unit canvas `share` of `big` wide.

    `hollow` is what fills each hexagon's middle: the background colour on
    the square icon, and nothing at all on Android's transparent layer.
    """
    unit = big * share / 24
    origin = (big - 24 * unit) / 2

    def to_px(x, y):
        return (origin + x * unit, origin + y * unit)

    # A stroke centred on the outline: an outer hexagon filled, and an inner
    # one filled with the background. For a hexagon, moving each edge out by
    # d moves the corners out by d / cos 30.
    grow = (STROKE / 2) / math.cos(math.pi / 6)
    for (cx, cy), colour in [
        ((12, 12 - DY), TOP),
        ((12 - DX, 12 + DY), LOWER),
        ((12 + DX, 12 + DY), LOWER),
    ]:
        outer = [to_px(x, y) for x, y in hexagon(cx, cy, R + grow)]
        inner = [to_px(x, y) for x, y in hexagon(cx, cy, R - grow)]
        draw.polygon(outer, fill=colour + (255,))
        draw.polygon(inner, fill=hollow)


def square_icon(size):
    """The icon as the desktop has it: the mark on a rounded graphite square."""
    big = size * SCALE
    image = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    # A rounded square, with a hairline edge, like every surface in the apps.
    radius = int(big * 0.22)
    draw.rounded_rectangle((0, 0, big - 1, big - 1), radius=radius, fill=EDGE)
    inset = int(big * 0.006)
    draw.rounded_rectangle(
        (inset, inset, big - 1 - inset, big - 1 - inset), radius=radius - inset, fill=BG
    )
    draw_mark(draw, big, MARK_OF_ICON, BG)
    return image.resize((size, size), Image.LANCZOS)


def round_icon(size):
    """For launchers that ask for a round icon and do their own masking badly."""
    big = size * SCALE
    image = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.ellipse((0, 0, big - 1, big - 1), fill=BG)
    draw_mark(draw, big, MARK_OF_ICON * 0.86, BG)
    return image.resize((size, size), Image.LANCZOS)


def adaptive_layer(size):
    """The mark alone, on nothing, for Android to put over the graphite."""
    big = size * SCALE
    image = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw_mark(ImageDraw.Draw(image), big, MARK_OF_ADAPTIVE, (0, 0, 0, 0))
    return image.resize((size, size), Image.LANCZOS)


ADAPTIVE_XML = """<?xml version="1.0" encoding="utf-8"?>
<!-- Written by tools/make-icons.py. -->
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
  <background android:drawable="@color/ic_launcher_background"/>
  <foreground android:drawable="@mipmap/ic_launcher_foreground"/>
  <monochrome android:drawable="@mipmap/ic_launcher_foreground"/>
</adaptive-icon>
"""

BACKGROUND_XML = """<?xml version="1.0" encoding="utf-8"?>
<!-- Written by tools/make-icons.py: the graphite behind the Android icon. -->
<resources>
  <color name="ic_launcher_background">#{:02X}{:02X}{:02X}</color>
</resources>
""".format(*BG[:3])


def android() -> None:
    for density, (icon, layer) in DENSITIES.items():
        folder = ANDROID_RES / f"mipmap-{density}"
        folder.mkdir(parents=True, exist_ok=True)
        square_icon(icon).save(folder / "ic_launcher.png")
        round_icon(icon).save(folder / "ic_launcher_round.png")
        adaptive_layer(layer).save(folder / "ic_launcher_foreground.png")

    anydpi = ANDROID_RES / "mipmap-anydpi-v26"
    anydpi.mkdir(parents=True, exist_ok=True)
    for name in ("ic_launcher.xml", "ic_launcher_round.xml"):
        (anydpi / name).write_text(ADAPTIVE_XML, encoding="utf-8")
    (ANDROID_RES / "values" / "ic_launcher_background.xml").write_text(
        BACKGROUND_XML, encoding="utf-8"
    )
    print(f"wrote the Android icons under {ANDROID_RES}")


def main() -> None:
    out = Path(__file__).with_name("icon-source.png")
    square_icon(SIZE).save(out)
    print(f"wrote {out}")
    if ANDROID_RES.is_dir():
        android()


if __name__ == "__main__":
    main()
