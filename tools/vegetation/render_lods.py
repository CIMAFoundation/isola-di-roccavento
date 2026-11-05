"""Each level of detail beside the original, at the level's distance and
at the pixel size it is drawn with (2026-11-04).

    Blender --background --python tools/vegetation/render_lods.py -- <plant> [...]

For each level of lod.LEVELS: the original and the level from the same
camera at `D`, the image as many pixels as the plant spans there on a
1080-line screen (lod.PIXEL_RAD), then enlarged without filtering. A third
picture shows the level from close, to see what it is made of. Writes
vegetation_src/lod/shots/<plant>.png, one row per level:
original at D | level at D | level close up.
"""
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lod  # noqa: E402
import render_sheet  # noqa: E402

SHOTS = lod.OUT / 'shots'
SHOTS.mkdir(parents=True, exist_ok=True)
CELL = 384


def shoot(scene, cam, target, dist, path, pixels):
    """Camera at `dist` from the target's centre, a little above, square
    image of `pixels` px covering `pixels * PIXEL_RAD` radians."""
    lo, hi = target
    centre = (lo + hi) / 2
    direction = Vector((0.0, -1.0, 0.18)).normalized()
    cam.location = centre + direction * dist
    cam.rotation_euler = (-direction).to_track_quat('-Z', 'Y').to_euler()
    cam.data.sensor_fit = 'VERTICAL'
    cam.data.angle = pixels * lod.PIXEL_RAD
    cam.data.clip_start, cam.data.clip_end = 0.1, dist * 4
    scene.render.resolution_x = scene.render.resolution_y = pixels
    scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)


def load(path):
    img = bpy.data.images.load(str(path), check_existing=False)
    w, h = img.size
    px = np.array(img.pixels[:], np.float32).reshape(h, w, 4)[::-1]
    bpy.data.images.remove(img)
    return px


def enlarge(px, size):
    """Nearest-neighbour up to `size` (every on-screen pixel a block)."""
    h = px.shape[0]
    idx = (np.arange(size) * h // size).clip(0, h - 1)
    return px[idx][:, idx]


def main():
    for plant in sys.argv[sys.argv.index('--') + 1:]:
        asset = plant.rsplit('_', 1)[0]
        bpy.ops.wm.open_mainfile(filepath=str(lod.NORM / f'{asset}.blend'))
        for o in [o for o in bpy.data.objects if o.type == 'MESH' and o.name != plant]:
            bpy.data.objects.remove(o)
        orig = bpy.data.objects[plant]
        scene = bpy.context.scene
        cam = render_sheet.setup(scene)
        scene.eevee.taa_render_samples = 32
        pts = np.array([tuple(orig.matrix_world @ Vector(c)) for c in orig.bound_box])
        box = (Vector(pts.min(0)), Vector(pts.max(0)))
        size = max(pts.max(0) - pts.min(0))
        levels = {}
        for level in lod.LEVELS:
            # from the level's .blend: its cards drawn as in the game (lod.facing)
            with bpy.data.libraries.load(str(lod.OUT / f'{plant}_{level}.blend')) as (src, dst):
                dst.objects = [f'{plant}_{level}']
            for o in dst.objects:
                scene.collection.objects.link(o)
            levels[level] = list(dst.objects)
        everything = [orig] + [o for objs in levels.values() for o in objs]
        rows = []
        tmp = SHOTS / '_tmp.png'
        for level, dist in lod.LEVELS.items():
            pixels = max(16, int(math.ceil(size * 1.25 / (dist * lod.PIXEL_RAD))))
            row = []
            for shown, d, px in ((orig, dist, pixels), (level, dist, pixels), (level, size * 1.6 / math.tan(math.radians(20)), CELL)):
                objs = [orig] if shown is orig else levels[shown]
                for o in everything:
                    o.hide_render = o not in objs
                if px == CELL:
                    # close up: a normal lens framing the plant
                    lo, hi = box
                    centre = (lo + hi) / 2
                    direction = Vector((0.0, -1.0, 0.18)).normalized()
                    cam.data.angle = math.radians(40)
                    cam.location = centre + direction * size * 1.6
                    cam.rotation_euler = (-direction).to_track_quat('-Z', 'Y').to_euler()
                    scene.render.resolution_x = scene.render.resolution_y = CELL
                    scene.render.filepath = str(tmp)
                    bpy.ops.render.render(write_still=True)
                else:
                    shoot(scene, cam, box, d, tmp, px)
                row.append(enlarge(load(tmp), CELL))
            rows.append(np.concatenate(row, 1))
            print(f'ROW {plant} {level}: {pixels} px on screen at {dist:.0f} m', flush=True)
        tmp.unlink(missing_ok=True)
        sheet = np.concatenate(rows, 0)
        h, w = sheet.shape[:2]
        img = bpy.data.images.new(plant, w, h, alpha=True)
        img.pixels = sheet[::-1].ravel()
        img.filepath_raw = str(SHOTS / f'{plant}.png')
        img.file_format = 'PNG'
        img.save()
        print('SHOT', plant, flush=True)


if __name__ == '__main__':
    main()
