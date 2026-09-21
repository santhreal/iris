#!/usr/bin/env python3
"""Render the iris app icon into PNG, ICO, and ICNS.

The glyph is the iris mark: a 6-blade aperture pinwheel on a dark
rounded tile, matching `home.rs::iris_mark`. Rendered at 1024 then
downsampled so every size is anti-aliased.

Outputs into packaging/icons/:
  iris-256.png, iris-512.png, iris-1024.png  (Linux + general)
  iris.ico                                    (Windows)
  iris.icns                                   (macOS)
"""
import math
import os
import struct
import zlib
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "icons")
os.makedirs(OUT, exist_ok=True)

# Palette from theme.rs.
BG_ELEV = (38, 38, 43, 255)      # rgb(0.149,0.149,0.169)
FG = (242, 242, 244, 255)        # rgb(0.949,0.949,0.957)
FG_DIM = (242, 242, 244, 199)    # alpha 0.78

SS = 4  # supersample factor


def render(size):
    s = size * SS
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # Rounded tile.
    radius = s * 0.24
    d.rounded_rectangle([0, 0, s - 1, s - 1], radius=radius, fill=BG_ELEV)

    # Aperture pinwheel, matching iris_mark at full open (t=1).
    u = s / 64.0
    cx = cy = 32.0 * u
    rot = 0.0
    hole = 16.0 * u
    r_out = 24.0 * u
    for pass_ in range(2):
        shade = FG if pass_ == 0 else FG_DIM
        for i in range(pass_, 6, 2):
            a0 = i / 6.0 * math.tau + rot
            a1 = (i + 1.0) / 6.0 * math.tau + rot
            o0 = (cx + r_out * math.cos(a0), cy + r_out * math.sin(a0))
            o1 = (cx + r_out * math.cos(a1), cy + r_out * math.sin(a1))
            i0 = (cx + hole * math.cos(a0), cy + hole * math.sin(a0))
            i1 = (cx + hole * math.cos(a1), cy + hole * math.sin(a1))
            d.polygon([o0, o1, i1, i0], fill=shade)

    return img.resize((size, size), Image.LANCZOS)


def write_png(img, path):
    img.save(path, "PNG")


def write_ico(base, path):
    # Multi-size ICO: 16,24,32,48,64,128,256.
    sizes = [16, 24, 32, 48, 64, 128, 256]
    base.save(path, "ICO", sizes=[(n, n) for n in sizes])


def write_icns(base, path):
    # Minimal ICNS: embed PNG data for each size. Apple accepts
    # PNG-compressed icon data for icns (ic10..ic14, ic07/ic08).
    # Map: icon type -> pixel size.
    entries = [
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
        (b"ic11", 32),   # 16x16@2x
        (b"ic12", 64),   # 32x32@2x
        (b"ic13", 256),  # 128x128@2x
        (b"ic14", 512),  # 256x256@2x
    ]
    blocks = []
    for tag, px in entries:
        import io
        buf = io.BytesIO()
        base.resize((px, px), Image.LANCZOS).save(buf, "PNG")
        data = buf.getvalue()
        blocks.append(tag + struct.pack(">I", len(data) + 8) + data)
    body = b"".join(blocks)
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", len(body) + 8) + body)


def main():
    base = render(1024)
    for n in (256, 512, 1024):
        write_png(base.resize((n, n), Image.LANCZOS),
                  os.path.join(OUT, f"iris-{n}.png"))
    write_ico(base, os.path.join(OUT, "iris.ico"))
    write_icns(base, os.path.join(OUT, "iris.icns"))
    print("wrote icons to", OUT)


if __name__ == "__main__":
    main()
