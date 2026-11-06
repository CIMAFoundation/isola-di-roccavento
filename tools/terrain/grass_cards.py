#!/usr/bin/env python3
"""Dry grass tuft cards for the near grass of the realistic ground (2026-11-06).

Poly Haven's «Grass Medium 01» (CC0) ships, at the foot of its texture atlas,
five photographic tufts rendered as impostors, in a dry variant
(`dry_diff`) with their alpha. Four of them are cut out here, each fitted to a
512 x 256 cell with its base on the cell's bottom edge, the colour bled into
the transparent texels (so the mips do not darken the edges), and written as
`assets/ground/grass_cards.png` (2 x 2 cells, RGBA, sRGB colour). The near
grass shader (`near_cover.wesl`, `REAL_GROUND`) stands one card per tuft.

    .venv/bin/python -I tools/terrain/grass_cards.py <grass_medium_01 folder> assets/ground

The folder holds `dry_diff.png` and `alpha.png`, the 1K PNG maps
(https://api.polyhaven.com/files/grass_medium_01: dry_diff, Alpha).
"""

import pathlib
import sys

import numpy as np
from PIL import Image

# tufts in the 1024 atlas: left, top, right, base (the row the blades start
# from; below it the atlas has padding streaks)
TUFTS = [(200, 760, 510, 897), (25, 890, 210, 999), (245, 905, 512, 1005), (590, 880, 805, 1020)]
CELL = (512, 256)


def bleed(rgb, alpha, steps=24):
    """Spread the colour of opaque texels into the transparent ones."""
    rgb = rgb.astype(np.float64)
    known = alpha > 0.5
    for _ in range(steps):
        acc = np.zeros_like(rgb)
        cnt = np.zeros(alpha.shape)
        for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            k = np.roll(known, (dy, dx), (0, 1))
            acc += np.roll(rgb, (dy, dx), (0, 1)) * k[..., None]
            cnt += k
        grow = (~known) & (cnt > 0)
        rgb[grow] = acc[grow] / cnt[grow][:, None]
        known = known | grow
    mean = rgb[alpha > 0.5].mean(axis=0)
    rgb[~known] = mean
    return rgb


def rooted(alpha):
    """Only what is connected to the tuft's base (the bottom rows): the
    atlas's neighbouring pieces that a crop catches are dropped."""
    solid = alpha > 0.08
    keep = np.zeros_like(solid)
    keep[-6:] = solid[-6:]
    while True:
        grown = keep.copy()
        for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)):
            grown |= np.roll(keep, (dy, dx), (0, 1))
        grown &= solid
        if (grown == keep).all():
            return keep
        keep = grown


def main():
    src, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    colour = Image.open(src / "dry_diff.png").convert("RGB")
    alpha = Image.open(src / "alpha.png").convert("L")
    sheet = Image.new("RGBA", (CELL[0] * 2, CELL[1] * 2))
    for i, (x0, y0, x1, y1) in enumerate(TUFTS):
        c = colour.crop((x0, y0, x1, y1))
        a = alpha.crop((x0, y0, x1, y1))
        w, h = c.size
        k = min(CELL[0] / w, CELL[1] / h) * 0.96
        size = (round(w * k), round(h * k))
        c, a = c.resize(size, Image.LANCZOS), a.resize(size, Image.LANCZOS)
        cell_rgb = np.zeros((CELL[1], CELL[0], 3))
        cell_a = np.zeros((CELL[1], CELL[0]))
        ox, oy = (CELL[0] - size[0]) // 2, CELL[1] - size[1]
        cell_rgb[oy:, ox:ox + size[0]] = np.asarray(c, dtype=np.float64)
        cell_a[oy:, ox:ox + size[0]] = np.asarray(a, dtype=np.float64) / 255.0
        cell_a = cell_a * rooted(cell_a)
        rgb = bleed(cell_rgb, cell_a)
        px = np.dstack([np.clip(rgb, 0, 255), cell_a * 255.0]).round().astype(np.uint8)
        sheet.paste(Image.fromarray(px, "RGBA"), ((i % 2) * CELL[0], (i // 2) * CELL[1]))
    sheet.save(out / "grass_cards.png", optimize=True)
    print("grass_cards.png", sheet.size)


if __name__ == "__main__":
    main()
