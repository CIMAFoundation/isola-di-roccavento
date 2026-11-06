#!/usr/bin/env python3
"""Ground detail layers for the realistic terrain (2026-11-06, `KIOSK_SCANS=1`).

Reads Poly Haven diffuse maps (CC0, 1K, downloaded beforehand: one folder per
asset with `diff_1k.jpg`, the folder named as the asset) and writes, for each
layer of `LAYERS`, `assets/ground/<layer>.png`: the texture *high-passed* into
a ratio around its local mean, in linear light, stored as ratio * ENCODE
(8 bit, not sRGB). The terrain shader multiplies its own macro colour by that
ratio (`terrain.wesl`, `real_ground`): far away the mips average the ratio to
one and the ground is the macro colour, near it the photo's grain; the blotches
larger than a quarter of a tile are gone, so the repeat does not show.
`assets/ground/ground.json` keeps each layer's mean colour (linear, which the
macro colour is built from: `terrain_mesh::ground_color`) and its size in metres.

    .venv/bin/python -I tools/terrain/ground_detail.py <downloads> assets/ground
"""

import json
import pathlib
import sys

import numpy as np
from PIL import Image

# layer name, Poly Haven asset, tile size (m), how much of the tile the
# high-pass keeps (the blur's sigma as a share of the side)
LAYERS = [
    ("grass", "withered_grass", 2.0, 0.10),
    ("soil", "dry_ground_rocks", 4.0, 0.10),
    ("needles", "forest_leaves_04", 1.5, 0.10),
    ("leaves", "dry_decay_leaves", 2.0, 0.10),
    ("rock", "marble_cliff_05", 20.0, 0.08),
    ("sand", "coast_sand_01", 15.0, 0.08),
    ("track", "rocky_trail_02", 2.0, 0.10),
    ("burnt", "burned_ground_01", 1.0, 0.12),
    ("asphalt", "aerial_asphalt_01", 30.0, 0.06),
]
ENCODE = 0.33
SIDE = 1024


def to_linear(s):
    return np.where(s <= 0.04045, s / 12.92, ((s + 0.055) / 1.055) ** 2.4)


def blur_wrapped(a, sigma):
    """Gaussian blur of a tileable image, wrapping (FFT)."""
    n = a.shape[0]
    f = np.fft.fftfreq(n)
    g = np.exp(-2.0 * (np.pi * sigma) ** 2 * (f[:, None] ** 2 + f[None, :] ** 2))
    return np.real(np.fft.ifft2(np.fft.fft2(a, axes=(0, 1)) * g[:, :, None], axes=(0, 1)))


def main():
    src, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    index = {"encode": ENCODE, "side": SIDE, "layers": []}
    for name, asset, size_m, keep in LAYERS:
        img = Image.open(src / asset / "diff_1k.jpg").convert("RGB")
        if img.size != (SIDE, SIDE):
            img = img.resize((SIDE, SIDE), Image.LANCZOS)
        lin = to_linear(np.asarray(img, dtype=np.float64) / 255.0)
        low = np.maximum(blur_wrapped(lin, keep * SIDE), 1e-4)
        ratio = lin / low
        mean = lin.reshape(-1, 3).mean(axis=0)
        q = np.clip(np.round(ratio * ENCODE * 255.0), 0, 255).astype(np.uint8)
        Image.fromarray(q, "RGB").save(out / f"{name}.png", optimize=True)
        lum = ratio @ np.array([0.2126, 0.7152, 0.0722])
        print(f"{name:8s} {asset:20s} mean {mean.round(3)} ratio sd {lum.std():.3f} clipped {(ratio * ENCODE > 1).mean() * 100:.2f} %")
        index["layers"].append({"name": name, "asset": asset, "size_m": size_m, "mean": [round(float(v), 4) for v in mean]})
    (out / "ground.json").write_text(json.dumps(index, indent=1) + "\n")


if __name__ == "__main__":
    main()
