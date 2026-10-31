#!/usr/bin/env python3
"""Render the iris app icon and tray glyphs into PNG, ICO, and ICNS.

The mark is a six-blade aperture: six blades around a hexagonal
opening. The app icon draws it as shaded blades in a bezel on a
graphite squircle, the opening as a blue lens; the tray glyph draws
it as strokes, one color on transparency.

Every shape is drawn at up to 8x and box-filtered down, so each pixel holds
its coverage.

Outputs into packaging/icons/:
  iris-{16,24,32,48,64,128,256,512,1024}.png  Linux hicolor sizes
  iris.ico                                    Windows, 16..256
  iris.icns                                   macOS, with the macOS
                                              icon grid's margin and shadow
  tray-{16,22,24,32,48}.png                   Linux tray, white
  tray-template.png                           macOS menu bar template,
                                              36 px for 18 pt at 2x
"""
import io
import math
import os
import struct

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "icons")

SS = 8  # supersample factor, capped so a side stays within 4096 px


def ss(size):
    return max(1, min(SS, 4096 // size))

# Opening circumradius over aperture radius, and the opening's turn.
HOLE = 0.42
TURN = math.radians(-12.0)


def aperture(cx, cy, r):
    """The opening's corners and where each blade edge meets the rim.

    Corner k runs along the edge from corner k-1 past corner k to the
    rim at point k.
    """
    hole = r * HOLE
    corners = [
        (cx + hole * math.cos(TURN + k * math.tau / 6),
         cy + hole * math.sin(TURN + k * math.tau / 6))
        for k in range(6)
    ]
    rims = []
    for k in range(6):
        (ax, ay), (bx, by) = corners[k - 1], corners[k]
        dx, dy = bx - ax, by - ay
        n = math.hypot(dx, dy)
        dx, dy = dx / n, dy / n
        # Solve |b + t d - c| = r for t > 0.
        px, py = bx - cx, by - cy
        bq = px * dx + py * dy
        t = -bq + math.sqrt(bq * bq - (px * px + py * py - r * r))
        rims.append((bx + t * dx, by + t * dy))
    return corners, rims


def arc(cx, cy, r, a0, a1, steps=48):
    """Points on the circle from angle a0 to a1, the short way."""
    while a1 - a0 > math.pi:
        a1 -= math.tau
    while a0 - a1 > math.pi:
        a1 += math.tau
    return [
        (cx + r * math.cos(a0 + (a1 - a0) * i / steps),
         cy + r * math.sin(a0 + (a1 - a0) * i / steps))
        for i in range(steps + 1)
    ]


def mask(s, draw):
    """An L-mode coverage mask of side s drawn by draw(ImageDraw)."""
    m = Image.new("L", (s, s), 0)
    draw(ImageDraw.Draw(m))
    return m


def squircle(d, x0, y0, side, fill):
    """A superellipse of exponent 5 inscribed in the square at x0, y0."""
    a = side / 2
    cx, cy = x0 + a, y0 + a
    pts = []
    for i in range(720):
        t = i / 720 * math.tau
        c, s = math.cos(t), math.sin(t)
        pts.append((cx + a * math.copysign(abs(c) ** 0.4, c),
                    cy + a * math.copysign(abs(s) ** 0.4, s)))
    d.polygon(pts, fill=fill)


def vertical(s, top, bottom):
    """An RGB image of side s shading from top to bottom."""
    t = np.linspace(0.0, 1.0, s, dtype=np.float32)[:, None, None]
    rgb = np.asarray(top, np.float32) * (1 - t) + np.asarray(bottom, np.float32) * t
    return Image.fromarray(np.broadcast_to(rgb, (s, s, 3)).astype(np.uint8), "RGB")


def radial(s, cx, cy, r, inner, outer):
    """An RGB image of side s shading from inner at (cx, cy) to outer at r."""
    y, x = np.mgrid[0:s, 0:s].astype(np.float32)
    t = np.clip(np.hypot(x - cx, y - cy) / r, 0.0, 1.0)[..., None]
    rgb = np.asarray(inner, np.float32) * (1 - t) + np.asarray(outer, np.float32) * t
    return Image.fromarray(rgb.astype(np.uint8), "RGB")


def paint(canvas, rgb, coverage):
    """Composite rgb (a color or an RGB image) over canvas through coverage."""
    if isinstance(rgb, tuple):
        rgb = Image.new("RGB", canvas.size, rgb)
    layer = rgb.convert("RGBA")
    layer.putalpha(coverage)
    canvas.alpha_composite(layer)


def tile(size, body, shadow):
    """The app icon at size px: a squircle of side body*size centered,
    with a drop shadow when shadow is set."""
    s = size * ss(size)
    side = body * s
    x0 = (s - side) / 2
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    if shadow:
        sh = mask(s, lambda d: squircle(d, x0, x0 + 0.012 * s, side, 255))
        sh = sh.filter(ImageFilter.GaussianBlur(0.022 * s))
        paint(img, (0, 0, 0), sh.point(lambda v: v * 0.42))

    body_m = mask(s, lambda d: squircle(d, x0, x0, side, 255))
    paint(img, vertical(s, (58, 58, 66), (22, 22, 26)), body_m)
    # A hairline of light along the top edge of the tile.
    rim = mask(s, lambda d: squircle(d, x0, x0, side, 255))
    inner = mask(s, lambda d: squircle(d, x0 + 0.004 * s, x0 + 0.006 * s,
                                       side - 0.008 * s, 255))
    edge = Image.fromarray(np.clip(np.asarray(rim, np.int16)
                                   - np.asarray(inner, np.int16), 0, 255)
                           .astype(np.uint8), "L")
    paint(img, vertical(s, (255, 255, 255), (22, 22, 26)), edge.point(lambda v: v * 0.22))

    c = s / 2
    r = side * 0.31
    bezel = r * 1.12
    # Bezel: a machined ring lit from above.
    paint(img, vertical(s, (120, 122, 132), (34, 34, 40)),
          mask(s, lambda d: d.ellipse([c - bezel, c - bezel, c + bezel, c + bezel], fill=255)))
    paint(img, (8, 8, 10),
          mask(s, lambda d: d.ellipse([c - r * 1.02, c - r * 1.02, c + r * 1.02, c + r * 1.02],
                                      fill=255)))

    corners, rims = aperture(c, c, r)
    gap = max(1, 0.007 * s)
    light = math.radians(-125.0)
    for k in range(6):
        nk = (k + 1) % 6
        a_from = math.atan2(rims[nk][1] - c, rims[nk][0] - c)
        a_to = math.atan2(rims[k][1] - c, rims[k][0] - c)
        poly = [corners[k], rims[nk]] + arc(c, c, r, a_from, a_to)[1:]
        mx = sum(p[0] for p in poly) / len(poly)
        my = sum(p[1] for p in poly) / len(poly)
        lit = 0.5 + 0.5 * math.cos(math.atan2(my - c, mx - c) - light)
        top = tuple(int(v) for v in np.array([226, 230, 240]) * (0.72 + 0.28 * lit))
        bottom = tuple(int(v) for v in np.array([150, 156, 172]) * (0.72 + 0.28 * lit))
        blade = mask(s, lambda d, poly=poly: d.polygon(poly, fill=255))
        paint(img, vertical(s, top, bottom), blade)
    # Seams: each blade edge from its corner to the rim, in the gap color.
    seams = mask(s, lambda d: [d.line([corners[k - 1], rims[k]], fill=255, width=int(gap))
                               for k in range(6)])
    paint(img, (8, 8, 10), seams)

    # Lens: the opening, deep blue with a highlight toward the light.
    hole = mask(s, lambda d: d.polygon(corners, fill=255))
    paint(img, radial(s, c - 0.10 * r, c - 0.14 * r, r * HOLE * 1.25,
                      (64, 168, 255), (4, 44, 128)), hole)
    hl = mask(s, lambda d: d.ellipse([c - 0.30 * r, c - 0.33 * r, c - 0.02 * r, c - 0.13 * r],
                                     fill=255)).filter(ImageFilter.GaussianBlur(0.02 * r))
    paint(img, (255, 255, 255), Image.fromarray(
        (np.asarray(hl, np.float32) * np.asarray(hole, np.float32) / 255 * 0.55)
        .astype(np.uint8), "L"))

    return img.resize((size, size), Image.BOX)


def glyph(size, rgb, pad, stroke):
    """The tray glyph at size px: the aperture as strokes of width stroke
    px in rgb, inset pad px from the edge."""
    f = ss(size)
    s = size * f
    c = s / 2
    w = stroke * f
    r = c - pad * f - w / 2
    corners, rims = aperture(c, c, r)

    def draw(d):
        d.ellipse([c - r - w / 2, c - r - w / 2, c + r + w / 2, c + r + w / 2], fill=255)
        d.ellipse([c - r + w / 2, c - r + w / 2, c + r - w / 2, c + r - w / 2], fill=0)
        for k in range(6):
            (ax, ay), (bx, by) = corners[k - 1], rims[k]
            # A stroke from just inside the opening's corner to the ring.
            d.line([(ax, ay), (bx, by)], fill=255, width=int(round(w)))
            d.ellipse([ax - w / 2, ay - w / 2, ax + w / 2, ay + w / 2], fill=255)

    cov = mask(s, draw).resize((size, size), Image.BOX)
    img = Image.new("RGBA", (size, size), rgb + (0,))
    img.putalpha(cov)
    return img


def write_icns(path):
    # PNG-compressed members: icon type -> pixel size.
    entries = [
        (b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024),
        (b"ic11", 32), (b"ic12", 64), (b"ic13", 256), (b"ic14", 512),
    ]
    blocks = []
    for tag, px in entries:
        buf = io.BytesIO()
        # The macOS icon grid: an 824 px body on a 1024 px canvas.
        tile(px, 824 / 1024, True).save(buf, "PNG")
        data = buf.getvalue()
        blocks.append(tag + struct.pack(">I", len(data) + 8) + data)
    body = b"".join(blocks)
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", len(body) + 8) + body)


def main():
    os.makedirs(OUT, exist_ok=True)
    full = {n: tile(n, 1.0, False) for n in (16, 24, 32, 48, 64, 128, 256, 512, 1024)}
    for n, img in full.items():
        img.save(os.path.join(OUT, f"iris-{n}.png"))
    full[256].save(os.path.join(OUT, "iris.ico"),
                   sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)],
                   append_images=[full[n] for n in (16, 24, 32, 48, 64, 128)])
    write_icns(os.path.join(OUT, "iris.icns"))
    for n in (16, 22, 24, 32, 48):
        glyph(n, (255, 255, 255), n / 16, max(1.5, n / 12)).save(
            os.path.join(OUT, f"tray-{n}.png"))
    glyph(36, (0, 0, 0), 1.5, 3.0).save(os.path.join(OUT, "tray-template.png"))
    print("wrote icons to", OUT)


if __name__ == "__main__":
    main()
