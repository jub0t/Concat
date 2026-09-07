#!/usr/bin/env python3
"""Build the README hero image from a window screenshot.

Usage:
    scripts/make-preview.py [screenshot.png] [out.png]

Defaults to assets/screenshot-dark.png -> assets/preview.png.

Takes a macOS window capture (Cmd-Shift-4, Space, click the window: the PNG
has a transparent margin with the window's drop shadow in it), trims it to
the solid window, rounds the corners, and floats it with a soft shadow on a
muted gradient backdrop. The bottom of the window runs off the frame so the
timeline reads as continuing below the fold. Needs Pillow.
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SRC = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "assets" / "screenshot-dark.png"
OUT = Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "assets" / "preview.png"

CANVAS_W = 3200
CANVAS_H = 1780
WINDOW_FRACTION = 0.92  # window width as a share of the canvas width
WINDOW_TOP = 136
WINDOW_RADIUS = 26
CARD_RADIUS = 56
SHADOW_BLUR = 70
SHADOW_OFFSET = 44
SHADOW_ALPHA = 150


def solid_bbox(im: Image.Image):
    """Bounds of the fully opaque window, ignoring the capture's soft shadow."""
    alpha = im.getchannel("A")
    return alpha.point(lambda v: 255 if v > 250 else 0).getbbox()


def rounded_mask(size, radius):
    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size[0] - 1, size[1] - 1), radius, fill=255)
    return mask


def backdrop(size):
    """Soft, slightly moody gradient: painted at low resolution, then blown up."""
    w, h = size
    small = (200, round(200 * h / w))
    bg = Image.new("RGB", small, (158, 160, 180))
    d = ImageDraw.Draw(bg)
    sw, sh = small
    blobs = [
        # (cx, cy, rx, ry, colour) as fractions of the small canvas
        (0.12, 0.10, 0.45, 0.40, (210, 209, 224)),
        (0.55, 0.05, 0.42, 0.34, (78, 80, 96)),
        (0.95, 0.20, 0.30, 0.45, (200, 200, 216)),
        (0.30, 0.95, 0.55, 0.32, (218, 217, 228)),
        (0.85, 0.95, 0.40, 0.32, (226, 225, 234)),
        (0.02, 0.60, 0.22, 0.35, (126, 128, 150)),
    ]
    for cx, cy, rx, ry, colour in blobs:
        d.ellipse(
            (sw * (cx - rx), sh * (cy - ry), sw * (cx + rx), sh * (cy + ry)),
            fill=colour,
        )
    bg = bg.filter(ImageFilter.GaussianBlur(28))
    return bg.resize(size, Image.BICUBIC).convert("RGBA")


def main():
    shot = Image.open(SRC).convert("RGBA")
    box = solid_bbox(shot)
    if box is None:
        sys.exit(f"{SRC}: no opaque region found")
    window = shot.crop(box)

    target_w = round(CANVAS_W * WINDOW_FRACTION)
    if window.width != target_w:
        window = window.resize(
            (target_w, round(window.height * target_w / window.width)), Image.LANCZOS
        )
    window.putalpha(rounded_mask(window.size, WINDOW_RADIUS))

    canvas = backdrop((CANVAS_W, CANVAS_H))
    x = (CANVAS_W - window.width) // 2
    y = WINDOW_TOP

    pad = SHADOW_BLUR * 3
    shadow = Image.new("RGBA", (window.width + 2 * pad, window.height + 2 * pad), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (pad, pad, pad + window.width - 1, pad + window.height - 1),
        WINDOW_RADIUS,
        fill=(0, 0, 0, SHADOW_ALPHA),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(SHADOW_BLUR))
    canvas.alpha_composite(shadow, (x - pad, y - pad + SHADOW_OFFSET))
    canvas.alpha_composite(window, (x, y))

    canvas.putalpha(rounded_mask(canvas.size, CARD_RADIUS))
    OUT.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(OUT, optimize=True)
    print(f"wrote {OUT} ({canvas.width}x{canvas.height})")


if __name__ == "__main__":
    main()
