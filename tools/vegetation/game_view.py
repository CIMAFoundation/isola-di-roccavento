"""The plants as the game's camera would show them (2026-11-04): 45° of
vertical field, 1600x1000, a low camera over a meadow, each plant at
several distances, real on-screen size (no enlargement).

    Blender --background --python tools/vegetation/game_view.py -- <plant> [...]

Two pictures, vegetation_src/lod/shots/game_original.png (the full models)
and game_lod.png (the level `lod.LEVELS` picks at each distance: the full
model nearer than the first level's distance). Rendered by Eevee, not by
the game: the cards are drawn with `lod.facing`, as the game is to.
"""
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lod  # noqa: E402
import render_sheet  # noqa: E402

DIST = [25, 50, 100, 200, 400]
W, H = 1600, 1000


def append(blend, name):
    with bpy.data.libraries.load(str(blend)) as (src, dst):
        dst.objects = [name]
    o = dst.objects[0]
    bpy.context.scene.collection.objects.link(o)
    return o


def level_for(d):
    chosen = None
    for level, start in lod.LEVELS.items():
        if d >= start:
            chosen = level
    return chosen


def main():
    plants = sys.argv[sys.argv.index('--') + 1:]
    for mode in ('original', 'lod'):
        bpy.ops.wm.read_factory_settings(use_empty=True)
        scene = bpy.context.scene
        cam = render_sheet.setup(scene)
        g = bpy.data.objects['ground']
        g.scale = (20, 20, 1)
        g.data.materials[0].node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value = (0.20, 0.21, 0.10, 1)
        scene.eevee.taa_render_samples = 32
        scene.render.resolution_x, scene.render.resolution_y = W, H
        cam.data.sensor_fit = 'VERTICAL'
        cam.data.angle = math.radians(45)
        cam.data.clip_end = 5000
        cam.location = (0, 0, 12)
        pitch = math.radians(-6)
        cam.rotation_euler = (math.pi / 2 + pitch, 0, 0)
        for d in DIST:
            for k, plant in enumerate(plants):
                asset = plant.rsplit('_', 1)[0]
                level = level_for(d) if mode == 'lod' else None
                if level is None:
                    o = append(lod.NORM / f'{asset}.blend', plant)
                else:
                    o = append(lod.OUT / f'{plant}_{level}.blend', f'{plant}_{level}')
                # spread sideways in proportion to the distance, so that
                # every row stays in the frame
                side = (k - (len(plants) - 1) / 2) * 0.12 * d
                o.location = (side + (0.05 * d if d in DIST[1::2] else -0.05 * d), d, 0)
        scene.render.filepath = str(lod.OUT / 'shots' / f'game_{mode}.png')
        bpy.ops.render.render(write_still=True)
        print('SHOT', mode, flush=True)


if __name__ == '__main__':
    main()
