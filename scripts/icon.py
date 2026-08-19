#!/usr/bin/env -S uv run --with pillow python3
"""Draw hi5's app icon and write assets/hi5.icns.

The icon is drawn here rather than kept as a source file: a raised hand
on the system's accent blue, in the proportions macOS gives every app
icon (an 824-point rounded square on a 1024-point canvas, corner radius
185). SF Symbols and Apple's emoji are not licensed for app icons, so
the hand is a few capsules and an ellipse of our own.

    ./scripts/icon.py            # writes assets/hi5.icns (+ a 1024 png)
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFilter

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
S = 4  # supersampling
N = 1024

BLUE_TOP = (30, 144, 255)
BLUE_BOTTOM = (0, 92, 224)
WHITE = (255, 255, 255)


def canvas() -> Image.Image:
    im = Image.new("RGBA", (N * S, N * S), (0, 0, 0, 0))
    # macOS icon plate: 824/1024 of the canvas, radius 185/1024.
    inset = 100 * S
    radius = 185 * S
    plate = Image.new("RGBA", im.size, (0, 0, 0, 0))
    grad = Image.new("RGBA", im.size)
    gd = ImageDraw.Draw(grad)
    for y in range(im.size[1]):
        t = y / (im.size[1] - 1)
        c = tuple(int(BLUE_TOP[i] + (BLUE_BOTTOM[i] - BLUE_TOP[i]) * t) for i in range(3))
        gd.line([(0, y), (im.size[0], y)], fill=c + (255,))
    mask = Image.new("L", im.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [inset, inset, N * S - inset, N * S - inset], radius=radius, fill=255
    )
    plate.paste(grad, (0, 0), mask)
    # A soft shadow under the plate, as macOS icons carry.
    shadow = Image.new("RGBA", im.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        [inset, inset + 10 * S, N * S - inset, N * S - inset + 10 * S],
        radius=radius,
        fill=(0, 0, 0, 90),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(14 * S))
    im.alpha_composite(shadow)
    im.alpha_composite(plate)
    return im


def hand(im: Image.Image) -> None:
    """A raised, open hand — palm, five fingers, wrist — centred on the plate."""
    d = ImageDraw.Draw(im)
    s = S

    def capsule(x0, y0, x1, y1, w, fill=WHITE):
        d.line([(x0 * s, y0 * s), (x1 * s, y1 * s)], fill=fill, width=int(w * s))
        r = w * s / 2
        for x, y in ((x0 * s, y0 * s), (x1 * s, y1 * s)):
            d.ellipse([x - r, y - r, x + r, y + r], fill=fill)

    # Palm.
    d.rounded_rectangle([340 * s, 470 * s, 690 * s, 760 * s], radius=130 * s, fill=WHITE)
    # Fingers: index, middle, ring, little. Bases sit inside the palm.
    fw = 92
    capsule(410, 560, 400, 300, fw)
    capsule(500, 545, 500, 250, fw)
    capsule(590, 555, 598, 290, fw)
    capsule(672, 585, 690, 380, fw - 6)
    # Thumb, angled out.
    capsule(360, 640, 250, 500, fw + 2)
    # Wrist: narrower than the palm, squared off, so it reads as a wrist
    # and not a second palm.
    d.rounded_rectangle([420 * s, 690 * s, 610 * s, 815 * s], radius=40 * s, fill=WHITE)


def main() -> None:
    im = canvas()
    hand(im)
    full = im.resize((N, N), Image.LANCZOS)
    os.makedirs(os.path.join(ROOT, "assets"), exist_ok=True)
    full.save(os.path.join(ROOT, "assets", "hi5-1024.png"))

    with tempfile.TemporaryDirectory() as tmp:
        iconset = os.path.join(tmp, "hi5.iconset")
        os.makedirs(iconset)
        for size in (16, 32, 128, 256, 512):
            full.resize((size, size), Image.LANCZOS).save(
                os.path.join(iconset, f"icon_{size}x{size}.png")
            )
            full.resize((size * 2, size * 2), Image.LANCZOS).save(
                os.path.join(iconset, f"icon_{size}x{size}@2x.png")
            )
        out = os.path.join(ROOT, "assets", "hi5.icns")
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", out], check=True)
        print(out)


if __name__ == "__main__":
    sys.exit(main())
