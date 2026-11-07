"""Run with:
    /Applications/Blender.app/Contents/MacOS/Blender -b --factory-startup --python tools/buildings/build_real_houses.py

The houses of the towns in the realistic style (`KIOSK_SCANS=1`, 2026-11-07):
a library of whole house models, baked to `assets/models/houses_real.json`
(the index) and `assets/models/houses_real.bin` (the vertices), which
`crates/game/src/house_models.rs` fits onto the scenario's footprints.

Sources (downloaded into `buildings_src/`, out of git, by
`tools/buildings/fetch_houses.sh`; read only through Blender's importers):

- **Poly Haven, "Modular Urban Apartments Facade"** (James Ray Cock, CC0,
  https://polyhaven.com/a/modular_urban_apartments_facade): walls with their
  window and door openings, sash windows with their frames, sills and
  lintels, panelled doors, string courses, dentilled crowns, pilasters. The
  houses are put together here from those modules, a storey of 3 m and a
  bay of 3 m, with what the kit has not and a Ligurian house has: louvred
  shutters, a hipped or two-slope roof in tiles, chimneys.
Whole downloaded houses (the Sketchfab CC-BY ones need an account) are not
read yet: they would join the library the same way, each set on its
footprint, its materials mapped to the roles below by name, reduced to the
two levels.

Every model is in metres, origin at the middle of its footprint on the
ground, `x` along its long side, the front facing +z at runtime (Blender
-y), Y up. Two levels of detail: `lod0` with the kit's geometry (reduced),
`lod1` from the same layout in a few hundred triangles (flat walls, glass and
shutters as quads, the roof as it is), for beyond a few hundred metres.

What each vertex is, its *role* (the game gives it its part and colour,
`house_models::Role`): 0 plaster (the house's wash), 1 trim (string
courses, cornices, sills: the house's trim colour), 2 roof (clay or slate,
the house's), 3 glass, 4 frame (window frames, painted), 5 shutter (the
house's shutter colour), 6 door (wood), 7 metal, 8 stone (bare), 9 as baked
(a whole model's own colour), 10 reveal (inside an opening). The colour of
a vertex is the texture's under it, sRGB: the game scales the house's tint
by its brightness, so the kit's panels and mouldings read through.

`REAL_PREVIEW=<dir>` renders the library there.
"""
import json
import math
import os
import struct
import sys
from pathlib import Path

import bmesh
import bpy
from mathutils import Matrix, Vector

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "buildings_src"
OUT = ROOT / "assets/models"
KIT = SRC / "polyhaven/modular_urban_apartments_facade/modular_urban_apartments_facade_1k.gltf"

R_PLASTER, R_TRIM, R_ROOF, R_GLASS, R_FRAME, R_SHUTTER, R_DOOR, R_METAL, R_STONE, R_BAKED, R_REVEAL = range(11)

BAY = 3.0  # the kit's module: 3 m wide, a storey 3 m tall

bpy.ops.wm.read_factory_settings(use_empty=True)


# --- the kit --------------------------------------------------------------------------

def load_kit():
    if not KIT.exists():
        sys.exit(f"missing {KIT}: run tools/buildings/fetch_houses.sh")
    bpy.ops.import_scene.gltf(filepath=str(KIT))
    images = {}
    for img in bpy.data.images:
        if img.size[0] == 0:
            continue
        w, h = img.size
        px = list(img.pixels[:])
        images[img.name] = (w, h, px)
    mods = {}
    for o in bpy.data.objects:
        if o.type != "MESH":
            continue
        me = o.data.copy()
        me.transform(o.matrix_world)
        bb = [Vector(v.co) for v in me.vertices]
        mn = Vector((min(v.x for v in bb), min(v.y for v in bb), min(v.z for v in bb)))
        mx = Vector((max(v.x for v in bb), max(v.y for v in bb), max(v.z for v in bb)))
        mods[o.name] = dict(mesh=me, min=mn, max=mx)
    return mods, images


def material_role(name, module):
    n = name.lower()
    if "glass" in n:
        return R_GLASS
    if "plaster" in n:
        return R_PLASTER
    if "trim" in n:
        return R_TRIM
    if "objects" in n:
        return R_DOOR if module.startswith("door_") and not module.startswith("door_window") else R_FRAME
    return R_BAKED


def tex_for(mat):
    """The base colour image of a glTF material, if any."""
    if not mat or not mat.use_nodes:
        return None
    for nd in mat.node_tree.nodes:
        if nd.type == "TEX_IMAGE" and nd.image is not None and "nor" not in nd.image.name and "arm" not in nd.image.name:
            return nd.image.name
    return None


def sample(images, name, uv):
    if name not in images:
        return (0.8, 0.8, 0.8)
    w, h, px = images[name]
    x = int((uv[0] % 1.0) * (w - 1))
    y = int((uv[1] % 1.0) * (h - 1))
    i = (y * w + x) * 4
    return (px[i], px[i + 1], px[i + 2])  # Blender pixels are linear for colour images


class Geo:
    """Triangles with a role and a colour (linear) each, flat."""

    def __init__(self):
        self.tris = []  # (a, b, c, role, colour)

    def add(self, a, b, c, role, col):
        self.tris.append((Vector(a), Vector(b), Vector(c), role, tuple(col)))

    def quad(self, a, b, c, d, role, col):
        self.add(a, b, c, role, col)
        self.add(a, c, d, role, col)

    def box(self, lo, hi, role, col, faces="xyzXYZ"):
        x0, y0, z0 = lo
        x1, y1, z1 = hi
        if "x" in faces:
            self.quad((x0, y1, z0), (x0, y0, z0), (x0, y0, z1), (x0, y1, z1), role, col)
        if "X" in faces:
            self.quad((x1, y0, z0), (x1, y1, z0), (x1, y1, z1), (x1, y0, z1), role, col)
        if "y" in faces:
            self.quad((x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1), role, col)
        if "Y" in faces:
            self.quad((x1, y1, z0), (x0, y1, z0), (x0, y1, z1), (x1, y1, z1), role, col)
        if "z" in faces:
            self.quad((x0, y1, z0), (x1, y1, z0), (x1, y0, z0), (x0, y0, z0), role, col)
        if "Z" in faces:
            self.quad((x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1), role, col)

    def extend(self, other, m=Matrix.Identity(4)):
        for a, b, c, r, col in other.tris:
            self.tris.append((m @ a, m @ b, m @ c, r, col))

    def count(self):
        return len(self.tris)


def module_geo(mods, images, name, origin, ratio):
    """A kit module as `Geo`, moved so `origin` (its wall's lower front
    corner) is at 0, reduced: coplanar faces merged, then collapsed to
    `ratio` of its triangles."""
    src = mods[name]["mesh"]
    bm = bmesh.new()
    bm.from_mesh(src)
    bmesh.ops.translate(bm, verts=bm.verts, vec=-origin)
    uv = bm.loops.layers.uv.active
    # one material at a time, so a reduction never merges two
    out = Geo()
    by_mat = {}
    for f in bm.faces:
        by_mat.setdefault(f.material_index, []).append(f)
    for mi, faces in by_mat.items():
        mat = src.materials[mi] if mi < len(src.materials) else None
        role = material_role(mat.name if mat else "", name)
        tex = tex_for(mat)
        sub = bmesh.new()
        vmap = {}
        uvl = sub.loops.layers.uv.new()
        for f in faces:
            vs = []
            for v in f.verts:
                if v.index not in vmap:
                    vmap[v.index] = sub.verts.new(v.co)
                vs.append(vmap[v.index])
            try:
                nf = sub.faces.new(vs)
            except ValueError:
                continue
            for l_new, l_old in zip(nf.loops, f.loops):
                l_new[uvl].uv = l_old[uv].uv if uv else (0, 0)
        sub.verts.ensure_lookup_table()
        bmesh.ops.remove_doubles(sub, verts=sub.verts, dist=1e-4)
        bmesh.ops.dissolve_limit(sub, angle_limit=math.radians(2.0 if role == R_PLASTER else 1.0), verts=sub.verts, edges=sub.edges, delimit={"NORMAL"})
        bmesh.ops.triangulate(sub, faces=sub.faces)
        me = bpy.data.meshes.new("tmp")
        sub.to_mesh(me)
        sub.free()
        if ratio < 1.0 and role not in (R_GLASS, R_PLASTER) and len(me.polygons) > 60:
            ob = bpy.data.objects.new("tmp", me)
            bpy.context.scene.collection.objects.link(ob)
            md = ob.modifiers.new("d", "DECIMATE")
            md.ratio = ratio
            md.use_collapse_triangulate = True
            dg = bpy.context.evaluated_depsgraph_get()
            ev = ob.evaluated_get(dg)
            me2 = bpy.data.meshes.new_from_object(ev)
            bpy.data.objects.remove(ob)
            bpy.data.meshes.remove(me)
            me = me2
        me.calc_loop_triangles()
        uvd = me.uv_layers.active.data if me.uv_layers.active else None
        for t in me.loop_triangles:
            a, b, c = (me.vertices[i].co.copy() for i in t.vertices)
            if (b - a).cross(c - a).length < 1e-7:
                continue
            if tex and uvd:
                u = sum((uvd[li].uv for li in t.loops), Vector((0, 0))) / 3
                col = sample(images, tex, u)
            else:
                col = (0.7, 0.7, 0.7)
            out.add(a, b, c, role, col)
        bpy.data.meshes.remove(me)
    bm.free()
    return out


class Library:
    """The kit's modules, each placed with its lower left front corner at 0,
    the wall in the plane y = 0, the outside towards -y."""

    def __init__(self, mods, images):
        self.mods, self.images, self.cache = mods, images, {}

    def wall_origin(self, wall):
        m = self.mods[wall]
        return Vector((m["min"].x, m["min"].y, m["min"].z))

    def get(self, name, wall=None, ratio=0.2):
        key = (name, wall, ratio)
        if key not in self.cache:
            m = self.mods[name]
            if wall is not None:
                origin = self.wall_origin(wall)
            elif "_pier_corner_01" in name:
                # the showcase's corner is at x = -20, y = 0: the pier stands
                # out of both walls by its own margin
                origin = Vector((-20.0, 0.0, {"wall": 0.0, "crown": 4.0, "cornice": 3.4, "dado": -1.0}[name.split("_")[0]]))
            elif name.startswith(("cornice", "dado", "base")):
                # straight bands: the wall plane is their back face
                origin = Vector((m["min"].x, m["max"].y, m["min"].z))
            elif name.startswith("crown"):
                origin = Vector((m["min"].x, 0.0, m["min"].z))
            else:
                origin = self.wall_origin(name)
            self.cache[key] = module_geo(self.mods, self.images, name, origin, ratio)
            if os.environ.get("REAL_COUNT"):
                print(f"   module {name} ({ratio}): {self.cache[key].count()} triangles (from {len(m['mesh'].polygons)} faces)")
        return self.cache[key]

    def size(self, name):
        m = self.mods[name]
        return m["max"] - m["min"]


# --- a house from the kit ---------------------------------------------------------------

LIGHT = (0.75, 0.72, 0.66)


def side_matrix(k, W, D):
    """Module space (along +x, outside -y) to the house, side `k`:
    0 front (y = -D/2), 1 right, 2 back, 3 left."""
    starts = [(-W / 2, -D / 2), (W / 2, -D / 2), (W / 2, D / 2), (-W / 2, D / 2)]
    ang = [0, 90, 180, 270][k]
    m = Matrix.Translation((starts[k][0], starts[k][1], 0)) @ Matrix.Rotation(math.radians(ang), 4, "Z")
    return m


def shutters(g, x0, x1, z0, z1, state, lod):
    """Louvred shutters at a window from x0 to x1, z0 to z1 (module space):
    open against the wall, half closed, or closed."""
    w = (x1 - x0) * 0.5
    t = 0.05
    leaves = []
    if state == "open":
        leaves = [(x0 - w - 0.02, x0 - 0.02, -0.02 - t), (x1 + 0.02, x1 + w + 0.02, -0.02 - t)]
    elif state == "half":
        leaves = [(x0 - w - 0.02, x0 - 0.02, -0.02 - t), (x0 + w + 0.01, x1, -0.18 - t)]
    else:
        leaves = [(x0, x0 + w - 0.01, -0.18 - t), (x0 + w + 0.01, x1, -0.18 - t)]
    for a, b, y in leaves:
        if lod:
            g.quad((a, y, z0), (b, y, z0), (b, y, z1), (a, y, z1), R_SHUTTER, LIGHT)
        else:
            g.box((a, y, z0), (b, y + t, z1), R_SHUTTER, LIGHT, faces="yxXzZ")


FAMILIES = {
    # window module, its door, shutters
    "A": dict(win="window_centered_small", wall="wall_window_centered_small", door="door_centered_small_01", dwall="wall_door_centered_small_01", shut=True, ratio=0.2),
    "B": dict(win="window_offset_small", wall="wall_window_offset_small", door="door_centered_large_01", dwall="wall_door_centered_large_01", shut=True, ratio=0.2),
    "C": dict(win="window_centered_large", wall="wall_window_centered_large", door="door_centered_large_01", dwall="wall_door_centered_large_01", shut=False, ratio=0.12),
    "D": dict(win="window_centered_double", wall="wall_window_centered_double", door="door_centered_small_01", dwall="wall_door_centered_small_01", shut=False, ratio=0.12),
}


def row_suffix(r, storeys):
    """The kit has three rows of windows: _01 tall, _02 a little lower, _03
    the attic's, with its band."""
    if r == 0 or r == 1:
        return "_01"
    if r == storeys - 1 and storeys >= 4:
        return "_03"
    return "_02"


def kit_house(lib, spec, lod):
    nx, nz, S = spec["nx"], spec["nz"], spec["storeys"]
    fam = FAMILIES[spec["family"]]
    W, D = nx * BAY, nz * BAY
    g = Geo()
    top = S * BAY
    rnd = spec["seed"]

    def h(k):
        x = math.sin(rnd * 12.9898 + k * 78.233) * 43758.5453
        return x - math.floor(x)

    for k in range(4):
        n = nx if k % 2 == 0 else nz
        m = side_matrix(k, W, D)
        blind = spec.get("blind_sides") and k in (1, 3)
        door_at = n // 2 if k == 0 else (n // 2 if k == 2 and spec.get("back_door") else None)
        for r in range(S):
            for i in range(n):
                mm = m @ Matrix.Translation((i * BAY, 0, r * BAY))
                kind = "wall"
                if r == 0 and i == door_at:
                    kind = "door"
                elif blind or (k != 0 and h(k * 100 + r * 10 + i) < 0.15):
                    kind = "blank"
                elif r == 0 and k != 0 and h(k * 77 + i) < 0.4:
                    kind = "blank"
                else:
                    kind = "window"
                if lod:
                    # flat wall, a dark pane and its shutters
                    if kind == "window" or kind == "door":
                        if kind == "window":
                            win = fam["win"] + row_suffix(r, S)
                            sz = lib.size(win)
                            x0, x1 = 1.5 - sz.x * 0.5 + 0.08, 1.5 + sz.x * 0.5 - 0.08
                            z0, z1 = 0.62, 0.62 + sz.z - 0.15
                        else:
                            sz = lib.size(fam["door"])
                            x0, x1, z0, z1 = 1.5 - sz.x * 0.5 + 0.1, 1.5 + sz.x * 0.5 - 0.1, 0.0, sz.z - 0.1
                        sub = Geo()
                        for (a, b, c, d) in (((0, 0, 0), (x0, 0, 0), (x0, 0, BAY), (0, 0, BAY)), ((x1, 0, 0), (BAY, 0, 0), (BAY, 0, BAY), (x1, 0, BAY)),
                                             ((x0, 0, 0), (x1, 0, 0), (x1, 0, z0), (x0, 0, z0)), ((x0, 0, z1), (x1, 0, z1), (x1, 0, BAY), (x0, 0, BAY))):
                            if abs(a[0] - b[0]) > 1e-3 and abs(a[2] - d[2]) > 1e-3:
                                sub.quad(a, b, c, d, R_PLASTER, LIGHT)
                        sub.quad((x0, 0.0, z0), (x1, 0.0, z0), (x1, 0.0, z1), (x0, 0.0, z1), R_GLASS if kind == "window" else R_DOOR, (0.05, 0.05, 0.06) if kind == "window" else (0.3, 0.2, 0.12))
                        if kind == "window" and fam["shut"]:
                            shutters(sub, x0, x1, z0, z1, spec["shutters"], True)
                        g.extend(sub, mm)
                    else:
                        sub = Geo()
                        sub.quad((0, 0, 0), (BAY, 0, 0), (BAY, 0, BAY), (0, 0, BAY), R_PLASTER, LIGHT)
                        g.extend(sub, mm)
                    continue
                if kind == "door":
                    g.extend(lib.get(fam["dwall"], ratio=1.0), mm)
                    g.extend(lib.get(fam["door"], wall=fam["dwall"]), mm)
                elif kind == "window":
                    suf = row_suffix(r, S)
                    wall, win = fam["wall"] + suf, fam["win"] + suf
                    g.extend(lib.get(wall, ratio=1.0), mm)
                    wg = lib.get(win, wall=wall, ratio=fam["ratio"])
                    g.extend(wg, mm)
                    if fam["shut"]:
                        sz = lib.size(win)
                        o = lib.mods[win]["min"] - lib.wall_origin(wall)
                        sub = Geo()
                        shutters(sub, o.x + 0.06, o.x + sz.x - 0.06, o.z + 0.1, o.z + sz.z - 0.12, spec["shutters"], False)
                        g.extend(sub, mm)
                else:
                    g.extend(lib.get("wall_standard_standard_01", ratio=1.0), mm)
            # a string course between the floors
            if r > 0 and not lod and spec.get("courses", True):
                for i in range(n):
                    g.extend(lib.get("cornice_standard_standard_01"), m @ Matrix.Translation((i * BAY, 0, r * BAY - 0.2)))
        # the plinth band and the crown
        for i in range(n):
            mm = m @ Matrix.Translation((i * BAY, 0, 0))
            if lod:
                sub = Geo()
                sub.box((0, -0.33, top), (BAY, 0.0, top + 0.75), R_TRIM, LIGHT, faces="yzZ")
                g.extend(sub, mm)
            else:
                if not (i == door_at):
                    g.extend(lib.get("dado_standard_standard_01"), mm @ Matrix.Translation((0, 0, 0.0)))
                g.extend(lib.get("crown_standard_standard_01", ratio=0.15), mm @ Matrix.Translation((0, 0, top)))
    # the corners: pilasters and the crown's corner blocks
    if not lod:
        for k in range(4):
            m = side_matrix(k, W, D)
            for r in range(S):
                g.extend(lib.get("wall_pier_corner_01", ratio=1.0), m @ Matrix.Translation((0, 0, r * BAY)))
            g.extend(lib.get("crown_pier_corner_01", ratio=0.15), m @ Matrix.Translation((0, 0, top)))
    roof(g, W, D, top + 0.75, spec, lod)
    return g, W, D, top + 0.75


def roof(g, W, D, z, spec, lod):
    e = 0.45
    pitch = 0.42
    a, b = W / 2 + e, D / 2 + e
    col = (0.5, 0.25, 0.15)
    kind = spec["roof"]
    th = 0.25
    if kind == "hip":
        r = min(a, b)
        hz = z + r * pitch
        # ridge along the long side
        if a >= b:
            g.quad((-a, -b, z), (a, -b, z), (a - b, 0, hz), (-a + b, 0, hz), R_ROOF, col)
            g.quad((a, b, z), (-a, b, z), (-a + b, 0, hz), (a - b, 0, hz), R_ROOF, col)
            g.add((a, -b, z), (a, b, z), (a - b, 0, hz), R_ROOF, col)
            g.add((-a, b, z), (-a, -b, z), (-a + b, 0, hz), R_ROOF, col)
        else:
            g.quad((a, -b, z), (a, b, z), (0, b - a, hz), (0, -b + a, hz), R_ROOF, col)
            g.quad((-a, b, z), (-a, -b, z), (0, -b + a, hz), (0, b - a, hz), R_ROOF, col)
            g.add((-a, -b, z), (a, -b, z), (0, -b + a, hz), R_ROOF, col)
            g.add((a, b, z), (-a, b, z), (0, b - a, hz), R_ROOF, col)
    else:
        # two slopes, the ridge along x; the gable ends in plaster
        hz = z + b * pitch
        g.quad((-a, -b, z), (a, -b, z), (a, 0, hz), (-a, 0, hz), R_ROOF, col)
        g.quad((a, b, z), (-a, b, z), (-a, 0, hz), (a, 0, hz), R_ROOF, col)
        for s in (-1, 1):
            x = s * W / 2
            pts = [(x, -D / 2, z), (x, D / 2, z), (x, 0, z + D / 2 * pitch)]
            if s < 0:
                g.add(pts[1], pts[0], pts[2], R_PLASTER, LIGHT)
            else:
                g.add(pts[0], pts[1], pts[2], R_PLASTER, LIGHT)
    # the eave's fascia and soffit
    g.quad((-a, -b, z - th), (a, -b, z - th), (a, -b, z), (-a, -b, z), R_ROOF, col)
    g.quad((a, b, z - th), (-a, b, z - th), (-a, b, z), (a, b, z), R_ROOF, col)
    g.quad((a, -b, z - th), (a, b, z - th), (a, b, z), (a, -b, z), R_ROOF, col)
    g.quad((-a, b, z - th), (-a, -b, z - th), (-a, -b, z), (-a, b, z), R_ROOF, col)
    g.quad((-a, b, z - th), (a, b, z - th), (a, -b, z - th), (-a, -b, z - th), R_TRIM, LIGHT)
    # chimneys
    for (cx, cy) in spec.get("chimneys", []):
        x, y = cx * W / 2, cy * D / 2
        ridge = hz
        g.box((x - 0.45, y - 0.45, z - 0.5), (x + 0.45, y + 0.45, ridge + 1.6), R_PLASTER, LIGHT, faces="xXyYZ")
        g.box((x - 0.6, y - 0.6, ridge + 1.6), (x + 0.6, y + 0.6, ridge + 1.85), R_ROOF, col)


# --- the library ------------------------------------------------------------------------

SPECS = [
    dict(name="kit_3x3_s2_hip", nx=3, nz=3, storeys=2, family="A", roof="hip", shutters="open", chimneys=[(0.3, 0.3)]),
    dict(name="kit_3x3_s3_gable", nx=3, nz=3, storeys=3, family="A", roof="gable", shutters="half", chimneys=[(0.5, -0.2)]),
    dict(name="kit_3x3_s4_hip", nx=3, nz=3, storeys=4, family="A", roof="hip", shutters="open", blind_sides=True, chimneys=[(-0.3, 0.2)]),
    dict(name="kit_4x3_s2_hip", nx=4, nz=3, storeys=2, family="B", roof="hip", shutters="open", chimneys=[(0.5, 0.3)]),
    dict(name="kit_4x3_s3_hip", nx=4, nz=3, storeys=3, family="A", roof="hip", shutters="open", chimneys=[(-0.4, 0.3)]),
    dict(name="kit_4x3_s3_gable", nx=4, nz=3, storeys=3, family="D", roof="gable", shutters="open", chimneys=[(0.6, 0.2)]),
    dict(name="kit_4x3_s4_hip", nx=4, nz=3, storeys=4, family="C", roof="hip", shutters="open", chimneys=[(0.4, -0.3), (-0.4, 0.3)]),
    dict(name="kit_4x3_s2_gable", nx=4, nz=3, storeys=2, family="A", roof="gable", shutters="closed", chimneys=[(-0.6, 0.0)]),
    dict(name="kit_5x3_s3_gable", nx=5, nz=3, storeys=3, family="A", roof="gable", shutters="open", blind_sides=True, chimneys=[(0.5, 0.25)]),
    dict(name="kit_5x3_s2_hip", nx=5, nz=3, storeys=2, family="B", roof="hip", shutters="half", chimneys=[(-0.5, 0.3)]),
    dict(name="kit_4x4_s3_hip", nx=4, nz=4, storeys=3, family="A", roof="hip", shutters="open", chimneys=[(0.3, 0.4)]),
    dict(name="kit_5x4_s3_hip", nx=5, nz=4, storeys=3, family="D", roof="hip", shutters="open", chimneys=[(0.5, 0.3)]),
    dict(name="kit_5x4_s4_hip", nx=5, nz=4, storeys=4, family="C", roof="hip", shutters="open", chimneys=[(-0.5, 0.3), (0.5, -0.3)]),
    dict(name="kit_6x3_s3_gable", nx=6, nz=3, storeys=3, family="A", roof="gable", shutters="half", blind_sides=True, chimneys=[(0.6, 0.3), (-0.2, -0.3)]),
    dict(name="kit_6x4_s4_hip", nx=6, nz=4, storeys=4, family="C", roof="hip", shutters="open", chimneys=[(0.6, 0.3), (-0.6, -0.3)]),
    dict(name="kit_7x4_s3_hip", nx=7, nz=4, storeys=3, family="A", roof="hip", shutters="open", chimneys=[(0.5, 0.3), (-0.5, -0.3)]),
]
for i, s in enumerate(SPECS):
    s["seed"] = i * 1.37 + 0.5


TINTED = (R_PLASTER, R_TRIM, R_FRAME, R_DOOR, R_REVEAL, R_ROOF, R_SHUTTER, R_GLASS, R_STONE, R_METAL)


def luma(c):
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]


def bake(g):
    """Blender (Z up, front -y) to runtime (Y up, front +z): x, z, -y.

    The colour of a tinted role is how much lighter or darker than its
    role's mean the texture is under it, a grey of `factor / 2` (the game
    multiplies the house's tint by it); a baked role keeps its sRGB colour.
    Glass comes last (the lit windows' run). Welded by position, normal,
    colour and role."""
    sums = {}
    for a, b, c, role, col in g.tris:
        area = (b - a).cross(c - a).length
        s = sums.setdefault(role, [0.0, 0.0])
        s[0] += luma(col) * area
        s[1] += area
    mean = {r: (s[0] / s[1] if s[1] > 0 else 1.0) for r, s in sums.items()}
    lookup, verts, idx = {}, [], []
    for a, b, c, role, col in sorted(g.tris, key=lambda t: t[3] == R_GLASS):
        n = (b - a).cross(c - a)
        if n.length < 1e-9:
            continue
        n.normalize()
        nr = (round(n.x * 127), round(n.z * 127), round(-n.y * 127))
        if role in TINTED:
            f = 1.0 if role in (R_ROOF, R_SHUTTER, R_GLASS) else min(1.45, max(0.55, luma(col) / max(mean[role], 1e-4)))
            q = min(255, round(f * 32) * 4)
            rgb = (q, q, q)
        else:
            rgb = tuple(min(255, max(0, round(255 * (c_ ** (1 / 2.2))))) for c_ in col)
        for p in (a, b, c):
            pr = (round(p.x, 3), round(p.z, 3), round(-p.y, 3))
            key = (pr, nr, rgb, role)
            at = lookup.get(key)
            if at is None:
                at = lookup[key] = len(verts)
                verts.append(key)
            idx.append(at)
    return verts, idx


def main():
    mods, images = load_kit()
    lib = Library(mods, images)
    blob = bytearray()
    index = dict(
        version=1,
        credits=["Poly Haven, Modular Urban Apartments Facade, James Ray Cock, CC0 (https://polyhaven.com/a/modular_urban_apartments_facade)"],
        layout="per vertex 13 bytes: 3 i16 position in mm, 3 i8 normal x 127, 4 u8 (r, g, b, role): a tinted role's grey is its tint factor x 127.5, a baked role's colour sRGB; then u16 indices, from an even offset",
        houses=[],
    )
    for spec in SPECS:
        entry = dict(name=spec["name"], source="kit", storeys=spec["storeys"], roof=spec["roof"], family=spec["family"], lods=[])
        for lod in (0, 1):
            g, W, D, eave = kit_house(lib, spec, lod)
            verts, idx = bake(g)
            vo = len(blob)
            for (p, n, c, r) in verts:
                blob += struct.pack("<3h3b4B", round(p[0] * 1000), round(p[1] * 1000), round(p[2] * 1000), n[0], n[1], n[2], c[0], c[1], c[2], r)
            io = len(blob)
            assert len(verts) < 65536
            blob += b"\0" * (len(blob) % 2)
            io = len(blob)
            blob += struct.pack(f"<{len(idx)}H", *idx)
            entry["lods"].append(dict(vertices=len(verts), indices=len(idx), vertex_offset=vo, index_offset=io))
            if lod == 0:
                ys = [v[0][1] for v in verts]
                entry["size"] = [round(W, 3), round(eave, 3), round(D, 3)]
                entry["height"] = round(max(ys), 3)
            print(f"== {spec['name']} lod{lod}: {len(idx) // 3} triangles, {len(verts)} vertices")
        index["houses"].append(entry)
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "houses_real.bin").write_bytes(bytes(blob))
    (OUT / "houses_real.json").write_text(json.dumps(index, indent=1) + "\n")
    print(f"houses_real: {len(index['houses'])} houses, {len(blob) / 1e6:.2f} MB")
    prev = os.environ.get("REAL_PREVIEW")
    if prev:
        preview(lib, Path(prev))


def preview(lib, outdir):
    outdir.mkdir(parents=True, exist_ok=True)
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o)
    tint = {R_PLASTER: (0.80, 0.55, 0.38), R_TRIM: (0.85, 0.82, 0.75), R_ROOF: (0.55, 0.22, 0.12), R_GLASS: (0.03, 0.04, 0.06),
            R_FRAME: (0.85, 0.85, 0.8), R_SHUTTER: (0.10, 0.30, 0.18), R_DOOR: (0.30, 0.18, 0.10), R_METAL: (0.2, 0.2, 0.2),
            R_STONE: (0.5, 0.5, 0.45), R_BAKED: (0.6, 0.6, 0.6), R_REVEAL: (0.7, 0.5, 0.35)}
    mats = {}
    for r, c in tint.items():
        m = bpy.data.materials.new(f"r{r}")
        m.diffuse_color = (*c, 1)
        m.use_nodes = True
        m.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (*c, 1)
        mats[r] = m
    sc = bpy.context.scene
    sc.render.engine = "BLENDER_EEVEE"
    sc.render.resolution_x, sc.render.resolution_y = 800, 600
    sc.world = bpy.data.worlds.new("w")
    sc.world.color = (0.8, 0.85, 0.9)
    bpy.ops.object.light_add(type="SUN", location=(0, 0, 30))
    bpy.context.object.data.energy = 4.0
    bpy.context.object.rotation_euler = (math.radians(45), 0, math.radians(30))
    bpy.ops.object.camera_add()
    cam = bpy.context.object
    sc.camera = cam
    for spec in SPECS[:: max(1, len(SPECS) // 6)] if not os.environ.get("REAL_PREVIEW_ALL") else SPECS:
        for lod in (0, 1):
            g, W, D, eave = kit_house(lib, spec, lod)
            me = bpy.data.meshes.new(spec["name"])
            verts, faces, mi = [], [], []
            for a, b, c, role, col in g.tris:
                k = len(verts)
                verts += [a, b, c]
                faces.append((k, k + 1, k + 2))
                mi.append(role)
            me.from_pydata(verts, [], faces)
            for r in range(11):
                me.materials.append(mats[r])
            for p, r in zip(me.polygons, mi):
                p.material_index = r
            ob = bpy.data.objects.new(spec["name"], me)
            sc.collection.objects.link(ob)
            target = Vector((0, 0, eave * 0.5))
            d = Vector((0.75, -1.0, 0.45)).normalized() * (max(W, D) * 2.2 + eave)
            cam.location = target + d
            cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
            sc.render.filepath = str(outdir / f"{spec['name']}_lod{lod}.png")
            bpy.ops.render.render(write_still=True)
            bpy.data.objects.remove(ob)


main()
