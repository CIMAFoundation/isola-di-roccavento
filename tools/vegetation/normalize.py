"""Normalise the downloaded plant models (2026-11-04, docs/TODO.md
«Vegetazione realistica», step 1).

    /Applications/Blender.app/Contents/MacOS/Blender --background \
        --python tools/vegetation/normalize.py -- [asset ...]

For each vegetation_src/polyhaven/<asset>/:
- imports the glTF and applies its transforms;
- tells wood from leaves by material name (`leaves`/`leaf` -> leaf, the
  rest wood) and counts triangles of each;
- one plant per mesh object (Poly Haven scans several of a species,
  `_a`, `_b`, ...);
- puts each plant's foot at the origin, Z up, and records its height;
- saves vegetation_src/normalized/<asset>.blend and a summary
  vegetation_src/normalized/summary.json.

Untrusted data: glTF only (never the .blend downloads, which can carry
scripts).
"""
import bpy
import numpy as np
import json
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import alpha  # noqa: E402

from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / 'vegetation_src' / 'polyhaven'
OUT = ROOT / 'vegetation_src' / 'normalized'
OUT.mkdir(parents=True, exist_ok=True)


# materials that are foliage although their name does not say so
FOLIAGE = {'pine_tree_01': ('twig',)}


def is_leaf(mat_name, asset=''):
    n = mat_name.lower()
    extra = FOLIAGE.get(asset, ())
    return 'leaf' in n or 'leaves' in n or 'needle' in n or 'flower' in n or any(e in n for e in extra)


def import_asset(asset):
    """The asset's plants: Poly Haven scans several of one species, one
    mesh object each (`_a`, `_b`, ...), at their own sizes."""
    bpy.ops.wm.read_factory_settings(use_empty=True)
    gltf = next((SRC / asset).glob('*.gltf'))
    bpy.ops.import_scene.gltf(filepath=str(gltf))
    alpha.wire(asset)
    meshes = [o for o in bpy.data.objects if o.type == 'MESH']
    for o in bpy.data.objects:
        o.select_set(o.type == 'MESH')
    bpy.context.view_layer.objects.active = meshes[0]
    bpy.ops.object.parent_clear(type='CLEAR_KEEP_TRANSFORM')
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    for o in [o for o in bpy.data.objects if o.type != 'MESH']:
        bpy.data.objects.remove(o)
    return sorted(meshes, key=lambda o: o.name)


def arrays(obj, asset=''):
    me = obj.data
    nv, nf = len(me.vertices), len(me.polygons)
    co = np.empty(nv * 3, np.float32)
    me.vertices.foreach_get('co', co)
    centre = np.empty(nf * 3, np.float32)
    me.polygons.foreach_get('center', centre)
    mat = np.empty(nf, np.int32)
    me.polygons.foreach_get('material_index', mat)
    loops = np.empty(nf, np.int32)
    me.polygons.foreach_get('loop_total', loops)
    leaf_slots = np.array([bool(s.material and is_leaf(s.material.name, asset)) for s in obj.material_slots] or [False])
    leaf = leaf_slots[np.clip(mat, 0, len(leaf_slots) - 1)]
    return co.reshape(-1, 3), centre.reshape(-1, 3), leaf, loops - 2


def summary_of(obj, asset):
    co, centre, leaf, ntri = arrays(obj, asset)
    base = centre[(~leaf) & (centre[:, 2] < co[:, 2].min() + .15)][:, :2]
    foot = base.mean(0) if len(base) else co[:, :2].mean(0)
    off = np.array([foot[0], foot[1], co[:, 2].min()], np.float32)
    co = co - off
    obj.data.vertices.foreach_set('co', co.ravel())
    obj.data.update()
    obj.location = (0, 0, 0)
    h = float(co[:, 2].max())
    r = float(np.hypot(co[:, 0], co[:, 1]).max())
    return h, r, int(ntri[leaf].sum()), int(ntri[~leaf].sum())


def main():
    args = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    assets = args or sorted(p.name for p in SRC.iterdir() if p.is_dir())
    summary_path = OUT / 'summary.json'
    summary = json.loads(summary_path.read_text()) if summary_path.exists() else {}
    for asset in assets:
        try:
            plants = import_asset(asset)
        except StopIteration:
            print(f'{asset}: no glTF')
            continue
        for k, o in enumerate(plants):
            name = f'{asset}_{k}'
            o.name = name
            h, r, leaf_t, wood_t = summary_of(o, asset)
            mats = [s.material.name for s in o.material_slots if s.material]
            summary[name] = dict(asset=asset, height_m=round(h, 2), radius_m=round(r, 2), leaf_tris=leaf_t, wood_tris=wood_t,
                                 leaf_materials=[m for m in mats if is_leaf(m, asset)], wood_materials=[m for m in mats if not is_leaf(m, asset)])
            print(f'PLANT {name:26s} h {h:5.2f} m  r {r:4.2f} m  leaf {leaf_t:>9,}  wood {wood_t:>9,}', flush=True)
        bpy.ops.wm.save_as_mainfile(filepath=str(OUT / f'{asset}.blend'), compress=True)
        summary_path.write_text(json.dumps(summary, indent=1))


if __name__ == '__main__':
    main()
