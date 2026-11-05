"""One picture of each normalised plant (2026-11-04), for the contact sheet.

    Blender --background --python tools/vegetation/render_sheet.py -- [asset ...]

Opens vegetation_src/normalized/<asset>.blend, renders each plant alone
(Eevee, a sun and a sky, a camera framing it from the side and a little
above) into vegetation_src/normalized/shots/<plant>.png, 512 px.
`tools/vegetation/sheet.py` puts them together with their names and sizes.
"""
import bpy
import math
import sys
from pathlib import Path

from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
NORM = ROOT / 'vegetation_src' / 'normalized'
SHOTS = NORM / 'shots'
SHOTS.mkdir(parents=True, exist_ok=True)


def setup(scene):
    scene.render.engine = 'BLENDER_EEVEE'
    scene.render.resolution_x = scene.render.resolution_y = 512
    scene.render.film_transparent = False
    scene.view_settings.view_transform = 'AgX'
    world = bpy.data.worlds.new('sky')
    world.use_nodes = True
    bg = world.node_tree.nodes['Background']
    bg.inputs['Color'].default_value = (0.62, 0.74, 0.88, 1)
    bg.inputs['Strength'].default_value = 0.9
    scene.world = world
    sun = bpy.data.objects.new('sun', bpy.data.lights.new('sun', 'SUN'))
    sun.data.energy = 4.0
    sun.rotation_euler = (math.radians(50), 0, math.radians(35))
    scene.collection.objects.link(sun)
    ground = bpy.data.meshes.new('ground')
    ground.from_pydata([(-200, -200, 0), (200, -200, 0), (200, 200, 0), (-200, 200, 0)], [], [(0, 1, 2, 3)])
    g = bpy.data.objects.new('ground', ground)
    mat = bpy.data.materials.new('ground')
    mat.use_nodes = True
    mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value = (0.32, 0.27, 0.18, 1)
    ground.materials.append(mat)
    scene.collection.objects.link(g)
    cam = bpy.data.objects.new('cam', bpy.data.cameras.new('cam'))
    cam.data.lens = 50
    scene.collection.objects.link(cam)
    scene.camera = cam
    return cam


def frame(cam, obj):
    pts = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
    lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
    hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
    centre = (lo + hi) / 2
    size = max(hi.x - lo.x, hi.y - lo.y, hi.z - lo.z)
    d = size * 1.15 / math.tan(cam.data.angle / 2) / 2 + size * .5
    direction = Vector((0.0, -1.0, 0.28)).normalized()
    cam.location = centre + direction * d
    cam.rotation_euler = (-direction).to_track_quat('-Z', 'Y').to_euler()
    cam.data.clip_end = d * 4


def main():
    args = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    assets = args or sorted(p.stem for p in NORM.glob('*.blend') if not p.stem[-1].isdigit())
    for asset in assets:
        bpy.ops.wm.open_mainfile(filepath=str(NORM / f'{asset}.blend'))
        scene = bpy.context.scene
        plants = sorted((o for o in bpy.data.objects if o.type == 'MESH'), key=lambda o: o.name)
        cam = setup(scene)
        for o in plants:
            for other in plants:
                other.hide_render = other is not o
            frame(cam, o)
            scene.render.filepath = str(SHOTS / f'{o.name}.png')
            bpy.ops.render.render(write_still=True)
            print('SHOT', o.name, flush=True)


if __name__ == '__main__':
    main()
