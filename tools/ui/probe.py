#!/usr/bin/env python3
"""Measure a screenshot instead of squinting at it.

Every check here answers a question a screenshot alone cannot: *is the
left inset the same as the right one*, *do these three runs of text sit
on one baseline*, *is this corner actually round*. Eyeballing got four
of those wrong in a row, which is why this file exists.

Coordinates are in **points**, not pixels: captures are Retina, so the
image is 2x the window's own geometry. `--scale` (default 2) converts.

    uv run --with pillow probe.py edges shot.png --y0 40 --y1 400
    uv run --with pillow probe.py align shot.png --y0 12 --y1 30
    uv run --with pillow probe.py rules shot.png out.png --h 40,80 --v 10,382
"""
import argparse
import json
import os
import sys
from collections import Counter

from PIL import Image, ImageDraw


def load(path, scale):
    """Open the image, and work out its backing scale.

    The preview binary captures at the display's own scale and writes a
    layout sidecar (`name.json`, a list of probes with the window's width
    in points among them); when that is present the scale is measured
    from it rather than assumed. `--scale` applies otherwise. Captures on
    a 1x display are 1x -- pass `--scale 1`.
    """
    im = Image.open(path).convert("RGB")
    side = os.path.splitext(path)[0] + ".json"
    if os.path.exists(side):
        with open(side) as f:
            data = json.load(f)
        if isinstance(data, dict) and data.get("points", [None])[0]:
            return im, im.width / data["points"][0]
        if isinstance(data, list):
            rows = [p for p in data if p.get("name") == "inbox.row"]
            if rows and rows[0]["w"]:
                return im, im.width / rows[0]["w"]
    return im, scale


def _bg_of_row(px, w, y):
    """The modal colour of a scanline — its background."""
    c = Counter(px[x, y] for x in range(w))
    return c.most_common(1)[0][0]


def _differs(a, b, tol):
    return abs(a[0] - b[0]) + abs(a[1] - b[1]) + abs(a[2] - b[2]) > tol


def edges(im, scale, y0, y1, tol=24, margin=4.0):
    """Leading and trailing inset of drawn content, per scanline.

    Answers "does the list have padding on the left but not the right".
    Rows that are entirely background are skipped rather than reported
    as zero-width, which would drag the summary toward the middle.

    `margin` (points) is the frame to ignore: the panel's own 1pt border
    and rounded corners reach the very edge of the capture on nearly
    every row, and counting them as content reports every screen as
    perfectly flush on both sides. Insets are still reported from the
    true image edge, so a row padded 12pt reads as 12.
    """
    w, h = im.size
    px = im.load()
    m = int(margin * scale)
    lo, hi = m, w - 1 - m
    out = []
    for y in range(int(y0 * scale), min(int(y1 * scale), h)):
        bg = _bg_of_row(px, w, y)
        left = right = None
        for x in range(lo, hi + 1):
            if _differs(px[x, y], bg, tol):
                left = x
                break
        for x in range(hi, lo - 1, -1):
            if _differs(px[x, y], bg, tol):
                right = x
                break
        if left is None:
            continue
        out.append((y / scale, left / scale, (w - 1 - right) / scale))
    return out


def summarize_edges(rows):
    """Summarise per-scanline insets.

    The mode alone lies on a screen with mixed content — a scrollbar
    hugging the right edge and section headers reaching further left
    than rows do will each own a mode of their own. The histograms are
    reported so the structure is visible rather than averaged away.
    """
    if not rows:
        return {"rows": 0}
    lefts = [r[1] for r in rows]
    rights = [r[2] for r in rows]

    def hist(values):
        c = Counter(round(v) for v in values)
        return {str(k): n for k, n in sorted(c.items(), key=lambda kv: -kv[1])[:6]}

    return {
        "rows": len(rows),
        "left_min": round(min(lefts), 2),
        "left_mode": round(Counter(round(v, 1) for v in lefts).most_common(1)[0][0], 2),
        "right_min": round(min(rights), 2),
        "right_mode": round(Counter(round(v, 1) for v in rights).most_common(1)[0][0], 2),
        "left_hist": hist(lefts),
        "right_hist": hist(rights),
    }


def ink_bands(im, scale, y0, y1, x0=0, x1=None, tol=24):
    """Contiguous runs of scanlines that contain drawn content.

    A text run shows up as one band; the band's centre is the thing to
    compare when checking whether two labels sit on the same line.
    """
    w, h = im.size
    px = im.load()
    x1 = w if x1 is None else int(x1 * scale)
    x0 = int(x0 * scale)
    counts = Counter(
        px[x, y]
        for y in range(int(y0 * scale), min(int(y1 * scale), h))
        for x in range(x0, min(x1, w))
    )
    if not counts:
        return []
    bg = counts.most_common(1)[0][0]
    rows = []
    for y in range(int(y0 * scale), min(int(y1 * scale), h)):
        has = any(_differs(px[x, y], bg, tol) for x in range(x0, min(x1, w)))
        rows.append(has)
    bands, start = [], None
    for i, has in enumerate(rows):
        if has and start is None:
            start = i
        elif not has and start is not None:
            bands.append((start, i - 1))
            start = None
    if start is not None:
        bands.append((start, len(rows) - 1))
    base = int(y0 * scale)
    return [
        {
            "top": round((base + a) / scale, 2),
            "bottom": round((base + b) / scale, 2),
            "center": round((base + (a + b) / 2) / scale, 2),
            "height": round((b - a + 1) / scale, 2),
        }
        for a, b in bands
    ]


def align(im, scale, y0, y1, columns, tol=24):
    """Where each named column's ink actually sits, vertically.

    This is the check for "the text doesn't sit on the same line". The
    centre reported is the ink-weighted centroid rather than the midpoint
    of the first and last inked scanline: a single stray antialiased
    pixel at the top of a descender would move a min/max midpoint by
    several points and hide a real misalignment behind noise.

    `_spread` is the gap between the highest and lowest centroid. Over
    about a point is visible to the eye.
    """
    w, h = im.size
    px = im.load()
    out = {}
    for name, cx0, cx1 in columns:
        a, b = int(cx0 * scale), min(int(cx1 * scale), w)
        rows = range(int(y0 * scale), min(int(y1 * scale), h))
        counts = Counter(px[x, y] for y in rows for x in range(a, b))
        if not counts:
            out[name] = None
            continue
        bg = counts.most_common(1)[0][0]
        total = 0.0
        weighted = 0.0
        first = last = None
        for y in rows:
            ink = sum(1 for x in range(a, b) if _differs(px[x, y], bg, tol))
            if ink:
                if first is None:
                    first = y
                last = y
                total += ink
                weighted += ink * y
        if not total:
            out[name] = None
            continue
        out[name] = {
            "top": round(first / scale, 2),
            "bottom": round(last / scale, 2),
            "center": round(weighted / total / scale, 2),
        }
    centers = [v["center"] for v in out.values() if v]
    out["_spread"] = round(max(centers) - min(centers), 2) if centers else None
    return out


def rules(im, scale, out_path, hs, vs, zoom=1):
    """Overlay horizontal and vertical rules at given point coordinates."""
    im = im.copy()
    if zoom != 1:
        im = im.resize((im.width * zoom, im.height * zoom), Image.NEAREST)
    d = ImageDraw.Draw(im)
    for y in hs:
        yy = y * scale * zoom
        d.line([(0, yy), (im.width, yy)], fill=(255, 0, 128), width=1)
        d.text((3, yy + 2), f"y={y}", fill=(255, 0, 128))
    for x in vs:
        xx = x * scale * zoom
        d.line([(xx, 0), (xx, im.height)], fill=(0, 160, 255), width=1)
        d.text((xx + 2, 3), f"x={x}", fill=(0, 160, 255))
    im.save(out_path)
    return out_path


def zoom_crop(im, scale, out_path, x, y, w, h, factor=4):
    box = (int(x * scale), int(y * scale), int((x + w) * scale), int((y + h) * scale))
    crop = im.crop(box)
    crop = crop.resize((crop.width * factor, crop.height * factor), Image.NEAREST)
    crop.save(out_path)
    return out_path


def corner_radius(im, scale, corner="tl", probe=24, tol=24):
    """Fit the drawn corner profile, to check a rounded rect is round.

    Returns the inset of the first drawn pixel on each of the first
    `probe` scanlines. A square corner gives a constant; a rounded one
    gives a decreasing curve.
    """
    w, h = im.size
    px = im.load()
    bg = px[0, 0] if corner in ("tl", "tr") else px[0, h - 1]
    prof = []
    for i in range(int(probe * scale)):
        y = i if corner in ("tl", "tr") else h - 1 - i
        rng = range(w) if corner in ("tl", "bl") else range(w - 1, -1, -1)
        for j, x in enumerate(rng):
            if _differs(px[x, y], bg, tol):
                prof.append(round(j / scale, 2))
                break
        else:
            prof.append(None)
    return prof


def main():
    p = argparse.ArgumentParser()
    p.add_argument("cmd")
    p.add_argument("image")
    p.add_argument("out", nargs="?")
    p.add_argument("--scale", type=float, default=2.0)
    p.add_argument("--y0", type=float, default=0)
    p.add_argument("--y1", type=float, default=10_000)
    p.add_argument("--x0", type=float, default=0)
    p.add_argument("--x1", type=float, default=None)
    p.add_argument("--w", type=float, default=100)
    p.add_argument("--h", default="")
    p.add_argument("--v", default="")
    p.add_argument("--factor", type=int, default=4)
    p.add_argument("--zoom", type=int, default=1)
    p.add_argument("--columns", default="", help="name:x0:x1,name:x0:x1")
    p.add_argument("--corner", default="tl")
    p.add_argument("--full", action="store_true")
    p.add_argument("--margin", type=float, default=4.0)
    a = p.parse_args()

    im, scale = load(a.image, a.scale)

    if a.cmd == "edges":
        rows = edges(im, scale, a.y0, a.y1, margin=a.margin)
        print(json.dumps(summarize_edges(rows), indent=2))
        if a.full:
            for y, l, r in rows:
                print(f"  y={y:7.2f}  left={l:6.2f}  right={r:6.2f}")
    elif a.cmd == "bands":
        print(json.dumps(ink_bands(im, scale, a.y0, a.y1, a.x0, a.x1), indent=2))
    elif a.cmd == "align":
        cols = []
        for spec in a.columns.split(","):
            if not spec.strip():
                continue
            name, x0, x1 = spec.split(":")
            cols.append((name, float(x0), float(x1)))
        print(json.dumps(align(im, scale, a.y0, a.y1, cols), indent=2))
    elif a.cmd == "rules":
        hs = [float(v) for v in a.h.split(",") if v.strip()]
        vs = [float(v) for v in a.v.split(",") if v.strip()]
        print(rules(im, scale, a.out, hs, vs, a.zoom))
    elif a.cmd == "zoom":
        print(zoom_crop(im, scale, a.out, a.x0, a.y0, a.w, a.y1 - a.y0, a.factor))
    elif a.cmd == "corner":
        print(json.dumps(corner_radius(im, scale, a.corner), indent=2))
    else:
        print(f"unknown command {a.cmd}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
