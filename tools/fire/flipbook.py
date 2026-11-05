"""The flame flipbook the game's tongues are drawn with (2026-11-05,
`shaders/flame.wesl`).

    python3 -I tools/fire/flipbook.py <Flame02_16x4.tga> <Flame03_16x4.tga> assets/models/flame_flipbook.png

Sources: Unity Labs «Free VFX image sequences and flipbooks», CC0
(https://unity.com/blog/engine-platform/free-vfx-image-sequences-flipbooks),
`Flame02-flipbooks.zip` and `Flame03-flipbooks.zip`, the TGA 16x4 sheets.

Writes one 2048x2048 sheet, 16 x 8 frames of 128 x 256: rows 0-3 Flame02,
rows 4-7 Flame03 (enlarged from 64 x 128). Grey + alpha: grey is how hot
(the luminance of the sheet's light), alpha how much flame; the game colours
it with its own ramp. The last LOOP frames of each sequence fade into its
first, so a sequence loops without a jump.
"""
import sys

import numpy as np
from PIL import Image

COLS, ROWS, FW, FH = 16, 4, 128, 256
LOOP = 10


def frames(path):
    im = Image.open(path).convert('RGBA')
    w, h = im.size
    cw, ch = w // COLS, h // ROWS
    out = []
    for r in range(ROWS):
        for c in range(COLS):
            f = im.crop((c * cw, r * ch, (c + 1) * cw, (r + 1) * ch))
            if (cw, ch) != (FW, FH):
                f = f.resize((FW, FH), Image.LANCZOS)
            a = np.asarray(f, np.float32) / 255.0
            alpha = a[..., 3]
            # the sheets' colour is the light the flame gives (bright at
            # the core, dim in the thin tops): its luminance is the heat
            heat = np.clip(0.3 * a[..., 0] + 0.55 * a[..., 1] + 0.15 * a[..., 2], 0.0, 1.0)
            out.append(np.stack([heat, alpha], -1))
    n = len(out)
    looped = list(out)
    for i in range(LOOP):
        w = (i + 1) / (LOOP + 1)
        k = n - LOOP + i
        looped[k] = (1 - w) * out[k] + w * out[i]
    return looped[:n]


def main():
    a, b, dst = sys.argv[1:4]
    sheet = np.zeros((FH * ROWS * 2, FW * COLS, 2), np.float32)
    for block, path in enumerate((a, b)):
        for i, f in enumerate(frames(path)):
            r, c = divmod(i, COLS)
            y = (block * ROWS + r) * FH
            sheet[y:y + FH, c * FW:(c + 1) * FW] = f
    px = (sheet * 255 + 0.5).astype(np.uint8)
    Image.fromarray(px, 'LA').save(dst, optimize=True)
    print('wrote', dst, px.shape)


if __name__ == '__main__':
    main()
