#!/usr/bin/env python3
"""Draws texcopy's icon: a sigma (the math) inside capture-corner marks (the selection).

    python3 packaging/icon/make_icon.py

Writes icon-1024.png (the master) and icon-256.png (embedded in the program for the window/taskbar).
Needs Pillow and a serif font with the Greek sigma; edit FONT_CANDIDATES for your machine.
"""
import os
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
FONT_CANDIDATES = [
    "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
    "/System/Library/Fonts/Supplemental/Georgia.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
    "C:/Windows/Fonts/times.ttf",
]
S = 1024
BG_TOP, BG_BOTTOM, INK, ACCENT = (36, 62, 105), (22, 38, 70), (255, 255, 255), (96, 176, 255)


def main():
    font_path = next((p for p in FONT_CANDIDATES if os.path.exists(p)), None)
    if not font_path:
        raise SystemExit("no serif font found; add one to FONT_CANDIDATES")
    # vertical gradient, then a rounded-square mask (the macOS icon grid leaves a margin of ~10%)
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    grad = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        t = y / (S - 1)
        gd.line([(0, y), (S, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOTTOM)) + (255,))
    margin, radius = 100, 190
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([margin, margin, S - margin, S - margin], radius=radius, fill=255)
    img.paste(grad, (0, 0), mask)

    d = ImageDraw.Draw(img)
    # capture corners
    a, b, arm, w = margin + 150, S - margin - 150, 120, 30
    for (x, y, dx, dy) in [(a, a, 1, 1), (b, a, -1, 1), (a, b, 1, -1), (b, b, -1, -1)]:
        d.line([(x, y), (x + dx * arm, y)], fill=ACCENT, width=w)
        d.line([(x, y), (x, y + dy * arm)], fill=ACCENT, width=w)
        d.ellipse([x - w // 2, y - w // 2, x + w // 2, y + w // 2], fill=ACCENT)
        d.ellipse([x + dx * arm - w // 2, y - w // 2, x + dx * arm + w // 2, y + w // 2], fill=ACCENT)
        d.ellipse([x - w // 2, y + dy * arm - w // 2, x + w // 2, y + dy * arm + w // 2], fill=ACCENT)
    # the sigma, centred
    font = ImageFont.truetype(font_path, 520)
    box = d.textbbox((0, 0), "\u03a3", font=font)
    d.text(((S - (box[2] - box[0])) / 2 - box[0], (S - (box[3] - box[1])) / 2 - box[1]), "\u03a3", font=font, fill=INK)
    img.save(os.path.join(HERE, "icon-1024.png"))
    img.resize((256, 256), Image.LANCZOS).save(os.path.join(HERE, "icon-256.png"))
    print("wrote icon-1024.png and icon-256.png from", font_path)


if __name__ == "__main__":
    main()
