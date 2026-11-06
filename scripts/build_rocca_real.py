"""Run with:
    /Applications/Blender.app/Contents/MacOS/Blender --background --python scripts/build_rocca_real.py

The rocca for the realistic look (`KIOSK_SCANS=1`, 2026-11-06), baked to
`assets/models/rocca_scans.json` in the layout of `island.json`. It is the
rocca of `scripts/build_island_models.py`, built by the same code with the
same random draws (so the crag under it, `rocca_roccia`, is the very same and
is not baked again; the script checks it), with what read as low-poly next to
photographed stone and scanned plants made again:

- **wall tops**: a broken wall stood to heights interpolated along it, and
  its top was one sloping quad per 5 m, sheared again over every bed of the
  crag: big pale triangles in the sun. Now a wall is built of columns about
  1.25 m long and three across (two faces and the rubble core between them),
  each with a flat top dropped course by course where the wall has fallen, and
  a whole stretch of wall (each 5 m) is lifted onto the crag in one piece: it
  steps where the rock steps, as walls do;
- **the whole reaches** of curtain and keep have a wall-walk, a parapet out
  on corbels and swallow-tail merlons with flat horns, instead of merlons
  standing on a slope;
- **openings**: the windows of keep and palazzo were black boxes stuck on
  the face, the tower windows were buried in the masonry, the gate was a
  black box between two stubs. Now they are cut through the wall (Blender's
  exact boolean), so you see the reveals' stone and what lies behind, with
  dressed jambs, a ring of voussoirs and a sill; the gate is a round arch
  through the curtain with the wall standing over it;
- the inside of a tower is a floor of rubble low down, not a black plug; a
  fallen drum is hollow stone, not black.

Only with the scans (`landmarks.rs`); without them the game draws the bake of
`island.json`, unchanged.
"""
import json
import math
import random
import types
from pathlib import Path

import bpy
from mathutils import Vector

SRC = Path(__file__).resolve().with_name("build_island_models.py")
ns = {"__file__": str(SRC), "__name__": "island_models"}
# everything up to the bake: the palette, the primitives, rocca() and the crag
exec(compile(SRC.read_text().split("\nBUILDERS = {")[0], str(SRC), "exec"), ns)

ROOT = SRC.parents[1]
OUT = ROOT / "assets/models/rocca_scans.json"

# Draws of our own: the island's RNG must see exactly the calls it saw in
# build_island_models.py, or the crag would come out different.
ns["RNG2"] = random.Random(29)
ns["ANCHOR"] = None
ns["CUTS"] = []
ns["COURSE"] = .45                        # a course of masonry, model metres
ns["DRESSED"] = (.76, .69, .56)           # jambs, voussoirs, corbels: dressed stone, a little lighter
ns["WALK"] = (.50, .45, .37)              # the wall-walk and the broken tops: weathered, darker
ns["HOLLOW"] = (.34, .31, .27)            # rubble on the floor inside a tower, the inside of a drum
ns["_link0"] = ns["link"]


def inject(f):
    ns[f.__name__] = types.FunctionType(f.__code__, ns, f.__name__, f.__defaults__)
    return f


@inject
def link(o, color):
    o = _link0(o, color)  # noqa: F821
    if ANCHOR is not None:  # noqa: F821
        o["anchor"] = list(ANCHOR)  # noqa: F821
    return o


@inject
def solid(boxes, color):
    """Boxes as one object: each (centre, (ax, ay, az) unit axes, (hx, hy, hz)
    half sizes), wound outward."""
    verts, faces = [], []
    quads = ((0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3))
    for c, (ax, ay, az), (hx, hy, hz) in boxes:
        c, ax, ay, az = Vector(c), Vector(ax), Vector(ay), Vector(az)
        base = len(verts)
        for sx in (-1, 1):
            for sy in (-1, 1):
                for sz in (-1, 1):
                    verts.append(tuple(c + ax * hx * sx + ay * hy * sy + az * hz * sz))
        for q in quads:
            f = [base + i for i in q]
            a, b, d = (Vector(verts[i]) for i in f[:3])
            mid = sum((Vector(verts[i]) for i in f), Vector()) / 4
            if (b - a).cross(d - a).dot(mid - c) < 0:
                f.reverse()
            faces.append(f)
    me = bpy.data.meshes.new("m")
    me.from_pydata(verts, [], faces)
    me.update()
    o = bpy.data.objects.new("o", me)
    bpy.context.scene.collection.objects.link(o)
    return link(o, color)  # noqa: F821


@inject
def _axes(yaw):
    return (math.cos(yaw), math.sin(yaw), 0.0), (-math.sin(yaw), math.cos(yaw), 0.0), (0.0, 0.0, 1.0)


@inject
def obox(c, size, color, yaw=0.0):
    """An upright box, `c` its centre, `size` (along, across, high)."""
    return solid([(c, _axes(yaw), (size[0] / 2, size[1] / 2, size[2] / 2))], color)


@inject
def wall_block(cx, cy, l, t, heights, yaw, color, z0=0.0, walk=None):
    """`rough_block` as masonry: columns 0.6-1.5 m long, three across (the two
    faces and the core), each with a flat top; where the wall is broken the
    tops fall course by course and the core lies lower than the faces. With
    `walk`, a whole wall: every column stops at the wall-walk."""
    skin = min(.85, t * .3)
    layers = ((-t / 2, -t / 2 + skin), (-t / 2 + skin, t / 2 - skin), (t / 2 - skin, t / 2))
    ax, ay, az = _axes(yaw)
    # columns of uneven length (a whole wall: even), cut afresh for each layer
    # so the faces and the core do not break along the same lines
    for m, (v0, v1) in enumerate(layers):
        cuts = [0.0]
        while walk is None and l - cuts[-1] > 1.6:
            cuts.append(cuts[-1] + RNG2.uniform(.6, 1.5))  # noqa: F821
        if len(cuts) > 1 and l - cuts[-1] < .45:
            cuts.pop()
        if walk is not None:
            cuts = [l * j / max(2, round(l / 1.25)) for j in range(max(2, round(l / 1.25)))]
        cuts.append(l)
        for j in range(len(cuts) - 1):
            u0, u1 = -l / 2 + cuts[j], -l / 2 + cuts[j + 1]
            fu = ((cuts[j] + cuts[j + 1]) / 2) / l
            fv = ((v0 + v1) / 2 + t / 2) / t
            # the corners: (-l/-t), (+l/-t), (+l/+t), (-l/+t)
            h = (heights[0] * (1 - fu) * (1 - fv) + heights[1] * fu * (1 - fv)
                 + heights[2] * fu * fv + heights[3] * (1 - fu) * fv)
            if walk is not None:
                h = walk
            else:
                h = math.floor(h / COURSE) * COURSE  # noqa: F821
                r = RNG2.random()  # noqa: F821
                h -= COURSE * (1 if r < .3 else 2 if r < .42 else 3 if r < .48 else 0)  # noqa: F821
                if m == 1:
                    h -= COURSE * RNG2.choice((1, 2, 2, 3))  # noqa: F821
                h = max(.5, h)
            uc, vc = (u0 + u1) / 2, (v0 + v1) / 2
            c = (cx + ax[0] * uc + ay[0] * vc, cy + ax[1] * uc + ay[1] * vc, z0 + (h - z0) / 2)
            solid([(c, (ax, ay, az), ((u1 - u0) / 2, (v1 - v0) / 2, (h - z0) / 2))], color if m != 1 or walk is not None else STONE_M)  # noqa: F821
            if walk is None and m != 1 and h > 3 and RNG2.random() < .22:  # noqa: F821
                # a loose stone left on the broken top
                s = RNG2.uniform(.35, .6)  # noqa: F821
                q = (c[0] + RNG2.uniform(-.3, .3), c[1] + RNG2.uniform(-.2, .2), h + s * .3)  # noqa: F821
                obox(q, (s * 1.4, s, s * .7), color, yaw + RNG2.uniform(-.4, .4))  # noqa: F821


@inject
def rough_block(cx, cy, l, t, heights, yaw, color, z0=0.0):
    return wall_block(cx, cy, l, t, heights, yaw, color, z0)  # noqa: F821


@inject
def ghibelline(cx, cy, z, yaw, w=2.0, t=1.1, h=2.4, color=None):
    """A swallow-tail merlon with flat horns (the stone's own width on top)."""
    color = color or STONE  # noqa: F821
    c, s_ = math.cos(yaw), math.sin(yaw)
    k = .2 * w
    prof = [(-w / 2, 0), (w / 2, 0), (w / 2, h), (w / 2 - k, h), (0, h * .6), (-w / 2 + k, h), (-w / 2, h)]
    verts = []
    for ty in (-t / 2, t / 2):
        for u, v in prof:
            verts.append((cx + u * c - ty * s_, cy + u * s_ + ty * c, z + v))
    n = len(prof)
    faces = [tuple(range(n)), tuple(range(2 * n - 1, n - 1, -1))]
    faces += [(i, (i + 1) % n, n + (i + 1) % n, n + i) for i in range(n)]
    return surface(verts, faces, color, centre=(cx, cy, z + h / 2))  # noqa: F821


@inject
def cutter(c, along, depth, w, h, z0):
    """What an opening takes out of the wall: a slot `w` wide from `z0` to
    `z0 + h` and the half-round head on it, `depth` through, across `along`."""
    a = Vector((along[0], along[1], 0)).normalized()
    nrm = Vector((-a.y, a.x, 0))
    yaw = math.atan2(a.y, a.x)
    zb = z0 - .6 if z0 < .5 else z0          # a gate: through the foot too
    box_c = (c[0], c[1], (zb + z0 + h) / 2)
    slot = solid([(box_c, (tuple(a), tuple(nrm), (0, 0, 1)), (w / 2, depth / 2, (z0 + h - zb) / 2))], DARK)  # noqa: F821
    bpy.ops.mesh.primitive_cylinder_add(vertices=24, radius=w / 2, depth=depth, location=(c[0], c[1], z0 + h))
    head = bpy.context.object
    head.rotation_euler = (math.pi / 2, 0, yaw)
    for o in (slot, head):
        CUTS.append(o)  # noqa: F821
        o["cutter"] = True


@inject
def frame(c, along, face, w, h, z0, sill=True):
    """Dressed stone round an opening on one face of the wall: quoined
    jambs, a ring of voussoirs with its keystone, a sill. `face`: offset of
    the face from `c` across the wall (signed)."""
    a = Vector((along[0], along[1], 0)).normalized()
    nrm = Vector((-a.y, a.x, 0))
    out = nrm * (1 if face > 0 else -1)
    yaw = math.atan2(a.y, a.x)
    D, P = .4, .1                             # into the wall, out of it
    base = Vector((c[0], c[1], 0)) + nrm * face + out * (P - D / 2)
    z = z0
    k = 0
    while z < z0 + h - .1:
        ch = min(.48, z0 + h - z)
        bw = .45 if k % 2 else .75
        for s in (-1, 1):
            p = base + a * s * (w / 2 - .03 + bw / 2)
            solid([((p.x, p.y, z + ch / 2), _axes(yaw), (bw / 2, D / 2, ch / 2))], DRESSED)["frame"] = True  # noqa: F821
        z += ch
        k += 1
    r, nv = w / 2, 9 if w > 1.6 else 7
    for i in range(nv):
        t = math.pi * (i + .5) / nv
        key = i == nv // 2
        rad = .6 if not key else .78
        radial = Vector((math.cos(t) * a.x, math.cos(t) * a.y, math.sin(t)))
        tang = Vector((-math.sin(t) * a.x, -math.sin(t) * a.y, math.cos(t)))
        p = base + a * 0 + radial * (r - .03 + rad / 2) + Vector((0, 0, z0 + h))
        half_t = math.pi * (r + rad / 2) / nv / 2 * .94
        solid([(tuple(p), (tuple(radial), tuple(out), tuple(tang)), (rad / 2, D / 2 + (.04 if key else 0), half_t))], DRESSED)["frame"] = True  # noqa: F821
    if sill:
        p = Vector((c[0], c[1], 0)) + nrm * face + out * .08
        solid([((p.x, p.y, z0 + .03 - .11), _axes(yaw), (w / 2 + .3, .3, .11))], DRESSED)["frame"] = True  # noqa: F821


@inject
def opening(c, along, T, z0, w, h, faces=(1, -1)):
    cutter(c, along, T + 1.6, w, h, z0)  # noqa: F821
    for s in faces:
        frame(c, along, s * T / 2, w, h, z0, sill=z0 > .5)  # noqa: F821


@inject
def ruined_wall(a, b, ctrl, T, color=None, scarp=True, crenel=True, step=5.0, window=False, centre=(0.0, 0.0)):
    """`ruined_wall` of build_island_models.py, with the island's random draws
    in the same order; see the module for what is built differently."""
    global ANCHOR
    color = color or STONE  # noqa: F821
    a, b = Vector((a[0], a[1], 0)), Vector((b[0], b[1], 0))
    d = b - a
    L = d.length
    n = max(2, int(round(L / step)))
    yaw = math.atan2(d.y, d.x)
    nrm = Vector((-d.y, d.x, 0)).normalized()
    out = nrm if nrm.dot((a + b) / 2 - Vector((centre[0], centre[1], 0))) > 0 else -nrm
    xs = [i / n for i in range(n + 1)]
    cs = [i / (len(ctrl) - 1) for i in range(len(ctrl))]
    hv = []
    for u in xs:
        k = max(0, min(len(cs) - 2, int(u * (len(cs) - 1))))
        f = u * (len(cs) - 1) - k
        hv.append(ctrl[k] * (1 - f) + ctrl[k + 1] * f)
    top_ref = max(ctrl)
    for i in range(n):
        p0, p1 = a + d * xs[i], a + d * xs[i + 1]
        mid = (p0 + p1) / 2
        l = (p1 - p0).length
        h0, h1 = hv[i], hv[i + 1]
        jit = [RNG.uniform(-.5, .5) for _ in range(2)]  # noqa: F821
        heights = [max(.5, h0 + jit[0]), max(.5, h1 + jit[1]), max(.5, h1 - jit[1]), max(.5, h0 - jit[0])]
        ANCHOR = (mid.x, mid.y)
        whole = crenel and h0 >= .93 * top_ref and h1 >= .93 * top_ref and RNG.random() < CREN_P  # noqa: F821
        low = min(h0, h1)
        walk = math.floor(min(h0, h1) / COURSE) * COURSE if whole else None  # noqa: F821
        wall_block(mid.x, mid.y, l, T, heights, yaw, color, walk=walk)  # noqa: F821
        if scarp and low > 3.0:
            frustum(mid.x, mid.y, 0, min(low * .4, 4.0), (l, T + 2.6), (l, T), yaw, STONE_M)  # noqa: F821
        if whole:
            # the parapet, out on corbels, and its merlons
            pc = mid + out * (T / 2 - .45 + .3)
            obox((pc.x, pc.y, walk + .45), (l, .9, .9), color, yaw)  # noqa: F821
            obox((mid.x + out.x * T / 2, mid.y + out.y * T / 2, walk - .1), (l, .6, .2), DRESSED, yaw)  # noqa: F821
            nc = max(2, round(l / 1.25))
            for j in range(nc):
                q = p0 + (p1 - p0) * ((j + .5) / nc) + out * (T / 2 + .05)
                obox((q.x, q.y, walk - .65), (.42, .5, .9), DRESSED, yaw)  # noqa: F821
            for k in (.25, .75):
                q = p0 + (p1 - p0) * k + out * (T / 2 - .45 + .3)
                ghibelline(q.x, q.y, walk + .85, yaw, w=1.6, t=.86, h=1.7)  # noqa: F821
        if low < .5 * top_ref:
            ANCHOR = None
            rubble(mid.x + nrm.x * 2.0, mid.y + nrm.y * 2.0, 3.2, 4)  # noqa: F821
            rubble(mid.x - nrm.x * 2.2, mid.y - nrm.y * 2.2, 2.6, 2)  # noqa: F821
            ANCHOR = (mid.x, mid.y)
        if RNG.random() < IVY_P and low > 3:  # noqa: F821
            ivy(mid.x + nrm.x * (T / 2), mid.y + nrm.y * (T / 2), low * RNG.uniform(.5, .95))  # noqa: F821
        if window and low > 7.0 and i % 2 == 1:
            opening((mid.x, mid.y), (d.x, d.y), T, 3.0, 1.4, 2.8)  # noqa: F821
    ANCHOR = None


@inject
def torrione(c, r, ctrl, scarp=8.0, windows=(), moss=True):
    """`torrione` of build_island_models.py, same draws: a hollow drum whose
    floor is rubble low down, its windows cut through the wall."""
    global ANCHOR
    x, y = c
    ANCHOR = (x, y)
    ctrl = [ctrl[int(k * len(ctrl) / 12) % len(ctrl)] * RNG.uniform(.8, 1.08) for k in range(12)]  # noqa: F821
    n = len(ctrl)
    cyl((x, y, 0), r * 1.32, scarp * 1.4, STONE_M, 20, top=r * 1.12)  # noqa: F821
    cyl((x, y, 0), r * .66, max(1.0, min(ctrl) * .3), HOLLOW, 14)  # noqa: F821
    for i in range(n):
        tm = 2 * math.pi * (i + .5) / n
        l = 2 * r * math.sin(math.pi / n) * 1.06
        h = ctrl[i]
        heights = [max(1.5, h + RNG.uniform(-1.0, .8)) for _ in range(4)]  # noqa: F821
        rough_block(x + math.cos(tm) * r, y + math.sin(tm) * r, l, 4.4, heights, tm + math.pi / 2, STONE)  # noqa: F821
        if h < 6:
            ANCHOR = None
            rubble(x + math.cos(tm) * (r + 3.2), y + math.sin(tm) * (r + 3.2), 3.0, 3, big=2.2)  # noqa: F821
            ANCHOR = (x, y)
        if RNG.random() < .55 and h > 4:  # noqa: F821
            ivy(x + math.cos(tm) * (r + 1.1), y + math.sin(tm) * (r + 1.1), h * RNG.uniform(.35, .85), 1.7)  # noqa: F821
    for k in windows:
        # only where the drum still stands over the window's head
        i = int((k % (2 * math.pi)) / (2 * math.pi) * n) % n
        if min(ctrl[i], ctrl[(i + 1) % n], ctrl[i - 1]) > 13.5:
            p = (x + math.cos(k) * r, y + math.sin(k) * r)
            # along the ring; its normal points in, so the outer face is -1
            opening(p, (-math.sin(k), math.cos(k)), 4.4, 9.4, 1.0, 2.2, faces=(-1,))  # noqa: F821
    if moss:
        scrub(x, y, min(ctrl) * .62, r * .5)  # noqa: F821
    ANCHOR = None


@inject
def rubble(cx, cy, radius, count, color=None, big=2.0):
    """`rubble` of build_island_models.py, same draws: fallen blocks a
    little out of square and tumbled, each set on the rock where it lies (they
    were sheared over the beds into slivers)."""
    color = color or STONE_M  # noqa: F821
    for _ in range(count):
        a, r = RNG.uniform(0, 2 * math.pi), radius * math.sqrt(RNG.random())  # noqa: F821
        sz = RNG.uniform(.6, big)  # noqa: F821
        size = (sz * RNG.uniform(.8, 1.5), sz, sz * .7)  # noqa: F821
        col = RNG.choice((color, STONE, STONE_D, TUFA))  # noqa: F821
        yaw = RNG.uniform(0, 3.14)  # noqa: F821
        x, y = cx + math.cos(a) * r, cy + math.sin(a) * r
        o = obox((0.0, 0.0, 0.0), size, col, yaw)  # noqa: F821
        tilt = Matrix.Rotation(RNG2.uniform(-.35, .35), 4, "X") @ Matrix.Rotation(RNG2.uniform(-.25, .25), 4, "Y")  # noqa: F821
        for v in o.data.vertices:
            v.co = tilt @ (v.co * Vector((RNG2.uniform(.85, 1.15), RNG2.uniform(.85, 1.15), RNG2.uniform(.8, 1.2))))  # noqa: F821
            v.co.x += x
            v.co.y += y
        o["lift"] = -sz * .25                 # its foot a tenth of it into the rock
        if "anchor" in o:
            del o["anchor"]


@inject
def fallen_drum(x, y, r, length, yaw, tilt=.0):
    bpy.ops.mesh.primitive_cylinder_add(vertices=14, radius=r, depth=length, location=(x, y, r * .85))
    o = bpy.context.object
    o.rotation_euler = (math.pi / 2 + tilt, 0, yaw)
    apply(o, rot=True, scale=False)  # noqa: F821
    link(o, CRAG)  # noqa: F821
    bpy.ops.mesh.primitive_cylinder_add(vertices=14, radius=r * .62, depth=length * 1.02, location=(x, y, r * .85))
    o = bpy.context.object
    o.rotation_euler = (math.pi / 2 + tilt, 0, yaw)
    apply(o, rot=True, scale=False)  # noqa: F821
    link(o, HOLLOW)  # noqa: F821


@inject
def apply_cuts():
    """Cut every opening out of the masonry it crosses, then drop the cutters."""
    bpy.context.view_layer.update()
    solids = [o for o in current.objects if not o.get("cutter") and not o.get("rock") and o.type == "MESH"]  # noqa: F821

    def bounds(o):
        ps = [o.matrix_world @ Vector(c) for c in o.bound_box]
        return Vector([min(p[i] for p in ps) for i in range(3)]), Vector([max(p[i] for p in ps) for i in range(3)])
    boxes = {o.name: bounds(o) for o in solids}
    for cut in CUTS:  # noqa: F821
        c0, c1 = bounds(cut)
        for o in solids:
            if o.get("frame"):
                continue
            b0, b1 = boxes[o.name]
            if any(b1[i] < c0[i] or b0[i] > c1[i] for i in range(3)):
                continue
            m = o.modifiers.new("cut", "BOOLEAN")
            m.operation = "DIFFERENCE"
            m.solver = "EXACT"
            m.object = cut
            bpy.context.view_layer.objects.active = o
            bpy.ops.object.modifier_apply(modifier=m.name)
    for cut in CUTS:  # noqa: F821
        bpy.data.objects.remove(cut, do_unlink=True)
    CUTS.clear()  # noqa: F821


@inject
def conform_to_peak():
    """As in build_island_models.py, except what was built with an anchor (a
    stretch of wall and all on it, a tower): lifted whole by the crag's height
    at the anchor, its foot sunk to the lowest rock under it."""
    apply_cuts()  # noqa: F821
    for o in list(current.objects):  # noqa: F821
        if o.get("rock"):
            continue
        me = o.data
        me.transform(o.matrix_world)
        o.matrix_world = Matrix.Identity(4)  # noqa: F821
        zs = [v.co.z for v in me.vertices]
        if not zs:
            continue
        zmin, zmax = min(zs), max(zs)
        grounded = zmin < .05 and zmax - zmin > 3.0
        if o.get("lift") is not None:
            # a fallen block: set on the rock under its middle, a little sunk
            cx = sum(v.co.x for v in me.vertices) / len(me.vertices)
            cy = sum(v.co.y for v in me.vertices) / len(me.vertices)
            off = max(ground_h(cx, cy), -1.0) - o["lift"]  # noqa: F821
            for v in me.vertices:
                v.co.z += off
            me.update()
            continue
        anchor = o.get("anchor")
        if grounded or (anchor is not None and zmin < .05):
            foot = [v.co for v in me.vertices if v.co.z - zmin < .05]
            x0, x1 = min(c.x for c in foot), max(c.x for c in foot)
            y0, y1 = min(c.y for c in foot), max(c.y for c in foot)
            low = min(max(ground_h(x0 + (x1 - x0) * i / 4, y0 + (y1 - y0) * j / 4), -1.0) for i in range(5) for j in range(5))  # noqa: F821
        if anchor is not None:
            off = max(ground_h(anchor[0], anchor[1]), -1.0)  # noqa: F821
            for v in me.vertices:
                if zmin < .05 and v.co.z - zmin < .05:
                    v.co.z = min(low - 1.8, off - .3)
                else:
                    v.co.z += off
            me.update()
            continue
        extent = max(max(v.co.x for v in me.vertices) - min(v.co.x for v in me.vertices), max(v.co.y for v in me.vertices) - min(v.co.y for v in me.vertices))
        rigid = grounded and extent >= 9.0
        for v in me.vertices:
            if rigid:
                v.co.z += low - 1.8
            elif grounded and v.co.z - zmin < .05:
                v.co.z += low - 1.8
            else:
                v.co.z += max(ground_h(v.co.x, v.co.y), -1.0) + (.15 if not grounded else 0.0)
        me.update()


@inject
def rocca():
    """`rocca` of build_island_models.py, same draws in the same order: the
    keep's walls know their centre, the gate is an arch through the curtain."""
    global ROCK, _PEAK_EDGE, STONE, STONE_L, STONE_M, STONE_D, TUFA, IVY_P, CREN_P, ANCHOR
    saved = (STONE, STONE_L, STONE_M, STONE_D, TUFA, IVY_P, CREN_P)  # noqa: F821
    _PEAK_EDGE = fractal_edge(RNG)  # noqa: F821
    STONE, STONE_L, STONE_M, STONE_D, TUFA = (.68, .60, .47), (.78, .70, .55), (.56, .49, .39), (.45, .39, .32), (.44, .42, .38)
    IVY_P, CREN_P = .5, .55
    T = WALL_T  # noqa: F821
    H = HALF  # noqa: F821
    gate_w = 7.0
    ruined_wall((-H, -H), (-H, H), [12, 11.6, 10.4, 8.0, 6.0, 8.5, 11.5, 12], T)  # noqa: F821
    ruined_wall((-H, H), (H, H), [11, 9.0, 6.0, 3.6, 2.6, 3.4, 6.0, 9.0, 11, 11.5], T)  # noqa: F821
    ruined_wall((-H, -H), (H, -H), [12, 11, 8.5, 5.0, 2.4, 3.0, 6.5, 10.0, 11.5, 11.5], T)  # noqa: F821
    ruined_wall((H, -H), (H, -gate_w / 2), [11, 9.5, 6.0, 8.0, 10.6], T)  # noqa: F821
    ruined_wall((H, gate_w / 2), (H, H), [11, 9.0, 5.0, 3.0, 3.6], T)  # noqa: F821
    # --- the gate: a round arch through the curtain, the wall broken over it
    R = gate_w / 2
    ANCHOR = (H, 0.0)
    wall_block(H, 0.0, gate_w, T, [10.4, 8.6, 8.2, 10.0], math.pi / 2, STONE)  # noqa: F821
    opening((H, 0.0), (0.0, 1.0), T, 0.0, gate_w - 1.6, 4.4, faces=(1, -1))  # noqa: F821
    ANCHOR = None
    rubble(H + 1.5, 1.6, 3.4, 9, big=2.4)  # noqa: F821
    rubble(H + 5.0, -1.0, 3.0, 6, big=1.6)  # noqa: F821
    scrub(H + 1.0, -1.0, 1.0, 1.4)  # noqa: F821
    torrione((-H, -H), 7.2, [23, 22, 16, 8, 5, 7, 16, 22], 7.5, windows=(.6, 3.6))  # noqa: F821
    torrione((-H, H), 6.8, [27, 26, 24, 19, 21, 25, 27, 27], 8.0, windows=(1.0, 2.6, 5.0))  # noqa: F821
    torrione((H, H), 6.6, [4.0, 3.0, 2.0, 1.5, 2.4, 3.6, 4.4, 4.0], 4.0, moss=False)  # noqa: F821
    torrione((H + .4, -gate_w / 2 - 6.0), 5.6, [16, 12, 6, 4, 9, 15, 17, 17], 5.5, windows=(2.0,))  # noqa: F821
    ANCHOR = (H, -H)                      # the bastion, lifted whole: it was sheared over the beds
    cyl((H, -H, 0), 9.4, 5.5, STONE_M, 5, top=7.2)  # noqa: F821
    cyl((H, -H, 5.4), 7.0, 3.0, STONE, 5, top=6.8)  # noqa: F821
    for k in (0, 1, 4):
        t = 2 * math.pi * (k + .5) / 5 + math.pi / 10
        ghibelline(H + math.cos(t) * 6.2, -H + math.sin(t) * 6.2, 8.2, t + math.pi / 2, w=2.4, h=2.2)  # noqa: F821
    ANCHOR = None
    fallen_drum(H + 6, H + 6, 3.2, 7.5, .5)  # noqa: F821
    fallen_drum(H + 11, H + 1, 3.0, 6.0, 1.4, .1)  # noqa: F821
    fallen_drum(H - 4, H + 12, 2.6, 5.0, -.4)  # noqa: F821
    rubble(H + 5, H + 5, 9, 16, big=3.0)  # noqa: F821
    kx, ky, ks = -10.0, 3.0, 8.2
    keep = (kx, ky)
    ruined_wall((kx - ks, ky - ks), (kx + ks, ky - ks), [30, 30, 28, 23, 17, 9, 6], 3.8, scarp=False, window=True, centre=keep)  # noqa: F821
    ruined_wall((kx + ks, ky - ks), (kx + ks, ky + ks), [6, 5, 12, 20, 16, 9], 3.8, scarp=False, window=True, centre=keep)  # noqa: F821
    ruined_wall((kx + ks, ky + ks), (kx - ks, ky + ks), [9, 18, 27, 31, 30, 29, 27], 3.8, scarp=False, window=True, centre=keep)  # noqa: F821
    ruined_wall((kx - ks, ky + ks), (kx - ks, ky - ks), [27, 30, 31, 29, 30, 30], 3.8, scarp=False, window=True, centre=keep)  # noqa: F821
    frustum(kx, ky, 0, 4.2, (2 * ks + 7.0, 2 * ks + 7.0), (2 * ks + 1.4, 2 * ks + 1.4), 0.0, CRAG_D)  # noqa: F821
    rubble(kx + ks + 4, ky - 3, 8.5, 16, big=3.2)  # noqa: F821
    scrub(kx, ky, 3.0, 3.5)  # noqa: F821
    ruined_wall((-27.0, 22.0), (-7.0, 22.0), [10.5, 11.0, 9.0, 6.0, 3.0, 2.2], 3.2, scarp=False, crenel=False, window=True, centre=(-17.0, 0.0))  # noqa: F821
    # the arcade, the well, the stumps and the dead trees: each lifted whole,
    # they too were sheared over the beds (the well's mouth stood on a slant)
    ax, ay = 14.0, -17.0
    ANCHOR = (ax + 6.0, ay)
    for i in range(3):
        cyl((ax + i * 6.0, ay, 0), .95, 7.2 if i != 2 else 3.6, CRAG_L, 10)  # noqa: F821
    for i in range(1):
        for k in range(7):
            t = math.pi * (k + .5) / 7
            box((ax + i * 6.0 + 3.0 - math.cos(t) * 2.6, ay, 7.2 + math.sin(t) * 2.6), (1.0, 1.9, 1.3), CRAG if k % 2 else CRAG_L, -t + math.pi / 2)  # noqa: F821
    ANCHOR = None
    rubble(ax + 10, ay - 1, 3.5, 6, big=1.8)  # noqa: F821
    ANCHOR = (8.0, 10.0)
    cyl((8.0, 10.0, 0), 1.9, 1.0, CRAG, 12)  # noqa: F821
    cyl((8.0, 10.0, 1.0), 1.3, .15, DARK, 12)  # noqa: F821
    ANCHOR = None
    scrub(8.0, 10.0, 1.0, 1.5)  # noqa: F821
    for (x, y, h) in ((-2.0, -14.0, 1.2), (3.0, -4.0, 2.0), (-4.0, 14.0, .8)):
        ANCHOR = (x, y)
        cyl((x, y, 0), .9, h, CRAG_L, 10)  # noqa: F821
    ANCHOR = (-18.0, -10.0)
    dead_tree(-18.0, -10.0, 0.0, 7.5)  # noqa: F821
    ANCHOR = (16.0, 12.0)
    dead_tree(16.0, 12.0, 0.0, 5.0)  # noqa: F821
    ANCHOR = None
    rubble(0, 0, 20, 30, big=1.8)  # noqa: F821
    for _ in range(14):
        a, rr = RNG.uniform(0, 6.28), RNG.uniform(3, 27)  # noqa: F821
        scrub(math.cos(a) * rr, math.sin(a) * rr, 0, RNG.uniform(.9, 1.8))  # noqa: F821
    conform_to_peak()  # noqa: F821
    crag_mound()  # noqa: F821
    STONE, STONE_L, STONE_M, STONE_D, TUFA, IVY_P, CREN_P = saved


# --- build and bake --------------------------------------------------------------------
current = bpy.data.collections.new("rocca")
bpy.context.scene.collection.children.link(current)
ns["current"] = current
ns["rocca"]()
bpy.context.view_layer.update()


def lin_green(c):
    # what `landmarks::without_greenery` leaves out: ivy and scrub
    return c[1] > c[0] * 1.05 and c[1] > c[2] and c[1] > .04


parts = {False: ([], [], [], []), True: ([], [], [], [])}
deps = bpy.context.evaluated_depsgraph_get()
for o in current.objects:
    if not o.data.materials:
        print("no material:", o.name, len(o.data.vertices), o.get("anchor"), o.get("cutter"))
        continue
    color = o.data.materials[0].diffuse_color[:]
    is_rock = bool(o.get("rock"))
    if not is_rock and lin_green(color):
        continue
    positions, normals, colors, indices = parts[is_rock]
    eo = o.evaluated_get(deps)
    mesh = eo.to_mesh()
    mesh.calc_loop_triangles()
    mw = o.matrix_world
    nm = mw.to_3x3().inverted().transposed()
    for tri in mesh.loop_triangles:
        if tri.area < 1e-7:
            continue
        n = (nm @ tri.normal).normalized()
        for vi in tri.vertices:
            p = mw @ mesh.vertices[vi].co
            positions.append([round(p.x, 4), round(p.z, 4), round(-p.y, 4)])
            normals.append([round(n.x, 4), round(n.z, 4), round(-n.y, 4)])
            colors.append([round(c, 4) for c in color[:3]])
            indices.append(len(indices))
    eo.to_mesh_clear()

# the crag must be the island bake's own: the same draws, in the same order
old = json.loads((ROOT / "assets/models/island.json").read_text())["rocca_roccia"]["positions"]
new = parts[True][0]
same = len(old) == len(new) and max(max(abs(a - b) for a, b in zip(p, q)) for p, q in zip(old, new)) < 1e-3
print(f"crag: {len(new) // 3} triangles, {'the same as' if same else 'DIFFERENT FROM'} island.json's")
if not same:
    raise SystemExit("the island's random draws went out of step: the crag would not match")

positions, normals, colors, indices = parts[False]
baked = {"rocca": dict(positions=positions, normals=normals, colors=colors, indices=indices, footprint=[62.0, 62.0])}
OUT.write_text(json.dumps(baked, separators=(",", ":")) + "\n")
print(f"rocca: {len(indices) // 3} triangles -> {OUT}")
