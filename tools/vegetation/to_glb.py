"""A downloaded plant as one GLB the game can read (2026-11-04): textures
re-saved as PNG (the desktop build has no JPEG decoder), everything else as
downloaded. For the lab's row of full models (`KIOSK_PIANTE_HI=1`).

    Blender --background --python tools/vegetation/to_glb.py -- <asset> [...]

Reads vegetation_src/polyhaven/<asset>/*.gltf, writes
vegetation_src/assets/<asset>.glb (BEVY_ASSET_ROOT=vegetation_src).
"""
import bpy
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import alpha  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / 'vegetation_src' / 'polyhaven'
OUT = ROOT / 'vegetation_src' / 'assets'


def convert(asset):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(next((SRC / asset).glob('*.gltf'))))
    alpha.wire(asset)
    png = SRC / asset / 'png'
    png.mkdir(exist_ok=True)
    for img in bpy.data.images:
        if img.source != 'FILE' or img.filepath.lower().endswith('.png'):
            continue
        p = png / (Path(img.filepath).stem + '.png')
        img.filepath_raw = str(p)
        img.file_format = 'PNG'
        img.save()
    OUT.mkdir(parents=True, exist_ok=True)
    bpy.ops.export_scene.gltf(filepath=str(OUT / f'{asset}.glb'), export_format='GLB', export_image_format='AUTO')
    print('WROTE', asset, flush=True)


for a in sys.argv[sys.argv.index('--') + 1:]:
    convert(a)
