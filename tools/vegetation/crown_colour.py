"""Mean colour and coverage of the plants in game_original.png and
game_lod.png (2026-11-04): for each box, the pixels that differ from the
ground. Run with a plain Python (NumPy, Pillow).

    python3 tools/vegetation/crown_colour.py
"""
from pathlib import Path

import numpy as np
from PIL import Image

SHOTS = Path(__file__).resolve().parents[2] / 'vegetation_src' / 'lod' / 'shots'
# (name, x0, y0, x1, y1) in the 1600x1000 frame of game_view.py
BOXES = [('leccio 50 m', 730, 540, 850, 640), ('cespuglio 50 m', 900, 600, 960, 645),
         ('leccio 100 m', 635, 455, 700, 505), ('leccio 200 m', 770, 415, 805, 440)]


def stats(img, box):
    a = np.asarray(img.crop(box).convert('RGB'), np.float32)
    ground = np.median(a.reshape(-1, 3), 0)
    m = np.abs(a - ground).sum(-1) > 25
    return a[m].mean(0) if m.any() else np.zeros(3), m.mean()


def main():
    o = Image.open(SHOTS / 'game_original.png')
    l = Image.open(SHOTS / 'game_lod.png')
    print(f'{"":16s} {"originale":>22s} {"livello":>22s}')
    for name, *box in BOXES:
        (co, ko), (cl, kl) = stats(o, box), stats(l, box)
        f = lambda c, k: f'{c[0]:4.0f} {c[1]:4.0f} {c[2]:4.0f} {k * 100:4.0f}%'
        print(f'{name:16s} {f(co, ko):>22s} {f(cl, kl):>22s}')


if __name__ == '__main__':
    main()
