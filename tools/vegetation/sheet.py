"""The contact sheet of the normalised plants (2026-11-04): every shot of
vegetation_src/normalized/shots/ with its name, height and triangles, from
summary.json. Writes vegetation_src/normalized/sheet.png.

    .venv/bin/python tools/vegetation/sheet.py
"""
import json
from pathlib import Path

from PIL import Image, ImageDraw

NORM = Path(__file__).resolve().parents[2] / 'vegetation_src' / 'normalized'
summary = json.loads((NORM / 'summary.json').read_text())
shots = sorted(p for p in (NORM / 'shots').glob('*.png'))
W, COLS = 256, 8
rows = (len(shots) + COLS - 1) // COLS
sheet = Image.new('RGB', (W * COLS, (W + 34) * rows), 'white')
d = ImageDraw.Draw(sheet)
for i, p in enumerate(shots):
    x, y = (i % COLS) * W, (i // COLS) * (W + 34)
    sheet.paste(Image.open(p).convert('RGB').resize((W, W)), (x, y))
    s = summary.get(p.stem, {})
    d.text((x + 4, y + W + 2), p.stem, fill='black')
    d.text((x + 4, y + W + 17), f"{s.get('height_m', '?')} m  foglie {s.get('leaf_tris', 0) / 1e3:.0f}k  legno {s.get('wood_tris', 0) / 1e3:.0f}k", fill='black')
sheet.save(NORM / 'sheet.png')
print(NORM / 'sheet.png', len(shots))
