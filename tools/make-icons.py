"""Draws the Basalt app icon from the same geometry as the in-app mark.

The mark (`HexMark.tsx` in both apps) is three flat-top hexagons on a
24-unit canvas: one above, two below, radius 4.3. This draws that shape on a
rounded graphite square at 1024 pixels, which `tauri icon` then turns into
every size Windows needs.

    python tools/make-icons.py
    cd apps/client && npx tauri icon ../../tools/icon-source.png
    cd apps/host   && npx tauri icon ../../tools/icon-source.png
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


def hexagon(cx, cy, r):
    return [(cx + r * math.cos(math.pi / 3 * i), cy + r * math.sin(math.pi / 3 * i)) for i in range(6)]


def main() -> None:
    big = SIZE * SCALE
    image = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    # A rounded square, with a hairline edge, like every surface in the apps.
    radius = int(big * 0.22)
    draw.rounded_rectangle((0, 0, big - 1, big - 1), radius=radius, fill=EDGE)
    inset = int(big * 0.006)
    draw.rounded_rectangle(
        (inset, inset, big - 1 - inset, big - 1 - inset), radius=radius - inset, fill=BG
    )

    unit = big * MARK_OF_ICON / 24
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
        draw.polygon(inner, fill=BG)

    out = Path(__file__).with_name("icon-source.png")
    image.resize((SIZE, SIZE), Image.LANCZOS).save(out)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
