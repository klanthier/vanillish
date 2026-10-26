#!/usr/bin/env python3
"""Vanillish's Windows icon, drawn from code so it can be regenerated (needs Pillow).

    python3 windows/tools/make-icon.py   # writes windows/Vanillish.ico and windows/Vanillish-512.png

The Android launcher's mark (android/app/src/main/res/drawable/ic_launcher_foreground.xml): a
gold ring with a gem on its crown around a "V", on the radial night-blue field of
ic_launcher_background.xml, as a rounded tile. Each size is drawn on its own, supersampled, with
the strokes thickened and the mark brought closer below 48 px so they stay legible. The .ico
(Vanillish.exe's icon, embedded by windows/vanillish.rc) and the 512 px PNG (art for a game
library) are committed; CI never runs this.
"""
import io
import math
import struct
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 256]

# The vectors' 108-unit viewport.
GOLD = (0xD4, 0xAF, 0x37)
GEM = (0xFA, 0xDE, 0x82)
FIELD_IN, FIELD_OUT = (0x1C, 0x22, 0x40), (0x06, 0x07, 0x0E)
FIELD_CENTRE, FIELD_RADIUS = (54.0, 0.42 * 108), 80.0
RING_CENTRE, RING_RADIUS, RING_WIDTH = (54.0, 54.0), 27.0, 2.6
GEM_CENTRE, GEM_HALF = (54.0, 27.0), (3.4, 3.6)
V = [(40.5, 38), (46.2, 38), (54, 63.5), (61.8, 38), (67.5, 38), (57.2, 72), (50.8, 72)]
# How far each edge of V (from vertex i to i+1) moves out when the staves thicken: the slanted
# sides, not the flat top and foot.
V_GROW = [0, 1, 1, 0, 1, 0, 1]

# The smallest stroke, in pixels, that still reads.
MIN_RING_PX = 1.25
MIN_STAVE_PX = 1.75
MIN_GEM_PX = 1.1


def span(size: int) -> float:
    """Viewport units across the tile: the launcher's visible 72, nearer below 48 px."""
    return 72.0 if size >= 48 else 66.0 if size >= 32 else 62.0


def offset_polygon(points, grow):
    """`points` with edge i pushed outward by grow[i] (mitred corners), for a clockwise polygon."""
    n = len(points)
    lines = []
    for i in range(n):
        (x0, y0), (x1, y1) = points[i], points[(i + 1) % n]
        dx, dy = x1 - x0, y1 - y0
        length = math.hypot(dx, dy)
        # Outward for a clockwise polygon in y-down coordinates: the edge's left-hand normal.
        nx, ny = dy / length, -dx / length
        lines.append(((x0 + nx * grow[i], y0 + ny * grow[i]), (dx, dy)))
    out = []
    for i in range(n):
        (p, d), (q, e) = lines[i - 1], lines[i]
        det = d[0] * e[1] - d[1] * e[0]
        t = ((q[0] - p[0]) * e[1] - (q[1] - p[1]) * e[0]) / det
        out.append((p[0] + d[0] * t, p[1] + d[1] * t))
    return out


def field(size: int, units: float) -> Image.Image:
    """The radial gradient at the final size: smooth, so no supersampling."""
    img = Image.new("RGB", (size, size))
    px = img.load()
    k = size / units
    left = 54.0 - units / 2
    for y in range(size):
        for x in range(size):
            u, v = left + (x + 0.5) / k, left + (y + 0.5) / k
            t = min(1.0, math.hypot(u - FIELD_CENTRE[0], v - FIELD_CENTRE[1]) / FIELD_RADIUS)
            px[x, y] = tuple(round(a + (b - a) * t) for a, b in zip(FIELD_IN, FIELD_OUT))
    return img


def draw(size: int, tile: bool = True) -> Image.Image:
    """One size of the icon: RGBA on a rounded tile, or an RGB square with `tile=False`."""
    ss = 8 if size <= 64 else 4
    big = size * ss
    units = span(size)
    k = size / units  # pixels per viewport unit
    left = 54.0 - units / 2

    def at(u, v):
        return ((u - left) * k * ss, (v - left) * k * ss)

    # The ring: an annulus, at least MIN_RING_PX wide.
    ring = Image.new("L", (big, big), 0)
    half = max(RING_WIDTH, MIN_RING_PX / k) / 2
    rd = ImageDraw.Draw(ring)
    cx, cy = RING_CENTRE
    for r, fill in ((RING_RADIUS + half, 255), (RING_RADIUS - half, 0)):
        rd.ellipse([*at(cx - r, cy - r), *at(cx + r, cy + r)], fill=fill)

    # The V: its staves (5.45 units across) grown to MIN_STAVE_PX, half on each side.
    stave = 5.7 * 34 / math.hypot(10.3, 34)
    grow = max(0.0, MIN_STAVE_PX / k - stave) / 2
    vee = Image.new("L", (big, big), 0)
    ImageDraw.Draw(vee).polygon([at(*p) for p in offset_polygon(V, [g * grow for g in V_GROW])], fill=255)

    # The gem, over the ring's crown.
    scale = max(1.0, MIN_GEM_PX / (GEM_HALF[0] * k))
    gx, gy = GEM_HALF[0] * scale, GEM_HALF[1] * scale
    cx, cy = GEM_CENTRE
    gem = Image.new("L", (big, big), 0)
    ImageDraw.Draw(gem).polygon([at(cx, cy - gy), at(cx + gx, cy), at(cx, cy + gy), at(cx - gx, cy)], fill=255)

    img = field(size, units)
    for layer, colour in ((ring, GOLD), (vee, GOLD), (gem, GEM)):
        img = Image.composite(Image.new("RGB", (size, size), colour), img, layer.reduce(ss))
    if not tile:
        return img

    # The tile: a rounded square, inset a little where there are pixels to spare.
    inset = 0 if size < 48 else big // 32
    corner = round((big - 2 * inset) * 0.18)
    shape = Image.new("L", (big, big), 0)
    box = [inset, inset, big - inset - 1, big - inset - 1]
    ImageDraw.Draw(shape).rounded_rectangle(box, radius=corner, fill=255)
    out = img.convert("RGBA")
    out.putalpha(shape.reduce(ss))
    return out


def dib(img: Image.Image) -> bytes:
    """An icon frame as a 32-bit DIB: BGRA rows bottom-up, then the AND mask (set = transparent)."""
    w, h = img.size
    rows = img.tobytes("raw", "BGRA")
    stride = w * 4
    pixels = b"".join(rows[y * stride : (y + 1) * stride] for y in reversed(range(h)))
    alpha = img.getchannel("A").load()
    mask_stride = (w + 31) // 32 * 4
    mask = bytearray()
    for y in reversed(range(h)):
        row = bytearray(mask_stride)
        for x in range(w):
            if alpha[x, y] == 0:
                row[x // 8] |= 0x80 >> (x % 8)
        mask += row
    header = struct.pack("<IiiHHIIiiII", 40, w, 2 * h, 1, 32, 0, len(pixels) + len(mask), 0, 0, 0, 0)
    return header + pixels + bytes(mask)


def png(img: Image.Image) -> bytes:
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    return buf.getvalue()


def ico(frames) -> bytes:
    """The classic layout: DIB frames below 256 px, a PNG frame at 256."""
    data = [png(f) if f.width >= 256 else dib(f) for f in frames]
    out = struct.pack("<HHH", 0, 1, len(frames))
    offset = 6 + 16 * len(frames)
    for f, d in zip(frames, data):
        side = f.width % 256  # 0 means 256
        out += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(d), offset)
        offset += len(d)
    return out + b"".join(data)


def main() -> None:
    (OUT / "Vanillish.ico").write_bytes(ico([draw(s) for s in ICO_SIZES]))
    print("wrote", OUT / "Vanillish.ico")
    draw(512, tile=False).save(OUT / "Vanillish-512.png", optimize=True)
    print("wrote", OUT / "Vanillish-512.png")


if __name__ == "__main__":
    main()
