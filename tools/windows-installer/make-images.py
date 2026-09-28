"""The pictures in Basalt's Windows installers.

    python tools/windows-installer/make-images.py

Writes, for each app, into `apps/<app>/src-tauri/installer/`:

* `sidebar.bmp`, 164 x 314: down the left of the Welcome and Finish pages.
* `header.bmp`, 150 x 57: in the corner of the pages between.

Drawn four times larger and scaled down, so the edges are smooth; saved as
24-bit bitmaps, which is what the installer takes. The mark is the same three
hexagons as the apps' HexMark, with the same numbers.
"""

import math
import os

from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, '..', '..'))
FONTS = os.path.join(HERE, 'fonts')
S = 4  # drawn at four times the size

INK = (11, 11, 12)
INK_2 = (22, 22, 25)
TEXT = (245, 245, 244)
DIM = (169, 169, 174)
FAINT = (111, 111, 118)
EMBER = (255, 122, 69)

APPS = {
    'client': ('Basalt', ['Your home drive,', 'on every screen', 'in the house.']),
    'host': ('Basalt Host', ['Share one drive', 'with every screen', 'in the house.']),
}


def font(weight, size):
    name = 'PlusJakartaSans-Bold.ttf' if weight == 'bold' else 'PlusJakartaSans-Regular.ttf'
    return ImageFont.truetype(os.path.join(FONTS, name), size * S)


def hexagon(cx, cy, r):
    return [(cx + r * math.cos(math.pi / 3 * i), cy + r * math.sin(math.pi / 3 * i)) for i in range(6)]


def mark(draw, x, y, size, colour, faint):
    """The mark in a `size` square at (x, y): one column above, two below."""
    unit = size / 24
    r = 4.3 * unit
    dx = r * 1.5
    dy = r * math.sqrt(3) * 0.5
    width = max(1, round(1.4 * unit))
    for (cx, cy), c in (((12, 12 - dy / unit), colour), ((12 - dx / unit, 12 + dy / unit), faint), ((12 + dx / unit, 12 + dy / unit), faint)):
        points = hexagon(x + cx * unit, y + cy * unit, r)
        draw.line(points + [points[0]], fill=c, width=width, joint='curve')


def sidebar(title, tagline):
    w, h = 164 * S, 314 * S
    image = Image.new('RGB', (w, h), INK)
    # A graphite fall from top to bottom.
    shade = Image.linear_gradient('L').resize((w, h))
    image = Image.composite(Image.new('RGB', (w, h), INK), Image.new('RGB', (w, h), INK_2), shade)
    # An ember glow at the top left, as on the website.
    glow = Image.new('RGB', (w, h), EMBER)
    mask = Image.new('L', (w, h), 0)
    ImageDraw.Draw(mask).ellipse((-90 * S, -120 * S, 150 * S, 110 * S), fill=60)
    mask = mask.filter(ImageFilter.GaussianBlur(40 * S))
    image = Image.composite(glow, image, mask)
    # The basalt columns as a quiet field behind.
    field = Image.new('L', (w, h), 0)
    fd = ImageDraw.Draw(field)
    r = 21 * S
    for row in range(-1, 12):
        for col in range(-1, 6):
            cx = col * r * 1.5 * 2 + (r * 1.5 if row % 2 else 0)
            cy = row * r * math.sqrt(3) * 0.5 * 2 * 0.5 + 200 * S
            points = hexagon(cx, cy, r)
            fd.line(points + [points[0]], fill=20, width=S)
    fade = Image.linear_gradient('L').resize((w, h))
    field = Image.composite(field, Image.new('L', (w, h), 0), fade)
    image = Image.composite(Image.new('RGB', (w, h), (255, 255, 255)), image, field)

    draw = ImageDraw.Draw(image)
    mark(draw, 22 * S, 30 * S, 40 * S, TEXT, (150, 150, 156))
    draw.text((22 * S, 86 * S), title, font=font('bold', 22 if len(title) < 8 else 19), fill=TEXT)
    y = 122 * S
    for line in tagline:
        draw.text((22 * S, y), line, font=font('regular', 10), fill=DIM)
        y += 15 * S
    draw.rectangle((22 * S, 280 * S, 34 * S, 281 * S), fill=EMBER)
    draw.text((22 * S, 288 * S), 'Refora Technologies', font=font('regular', 8), fill=FAINT)
    return image.resize((164, 314), Image.LANCZOS)


def header(title):
    """White, as the installer's header strip is, with the mark and name in ink."""
    w, h = 150 * S, 57 * S
    image = Image.new('RGB', (w, h), (255, 255, 255))
    draw = ImageDraw.Draw(image)
    name = font('bold', 13)
    width = draw.textlength(title, font=name)
    x = w - width - 12 * S
    draw.text((x, 20 * S), title, font=name, fill=(17, 17, 19))
    mark(draw, x - 30 * S, 16 * S, 26 * S, (17, 17, 19), (120, 120, 126))
    return image.resize((150, 57), Image.LANCZOS)


for app, (title, tagline) in APPS.items():
    out = os.path.join(ROOT, 'apps', app, 'src-tauri', 'installer')
    os.makedirs(out, exist_ok=True)
    sidebar(title, tagline).save(os.path.join(out, 'sidebar.bmp'))
    header(title).save(os.path.join(out, 'header.bmp'))
    print('wrote', os.path.relpath(out, ROOT))
