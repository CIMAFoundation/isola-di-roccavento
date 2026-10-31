"""Run with Blender --background --python scripts/build_vegetation_models.py.

The plants of the landscape (2026-10-31 remodel): Mediterranean species with
several varieties each, leaf cards cut out of a texture atlas, bark mapped on
the trunks, and the burnt companion built from the SAME structure.

What it writes (assets/models/):

* foliage_atlas.png: one 1024 x 512 sRGB atlas for every plant. Leaf sprays
  with alpha (128 px tiles, rendered here with Workbench from leaf geometry
  built in this script), opaque canopy tiles for the distant proxies (64 px),
  bark strips (32 x 512 px, Blender's procedural noise sampled per texel) and
  one flat grey texel block for what is untextured (remains, grass). The
  colours are the species' own, read from reference photos (CREDITS next to
  the atlas). The game multiplies them by a per-plant tint
  (`vegetation.rs`: vertex colour x 2, grey = linear 0.5 = no change).
* vegetation.json: {"atlas": ..., "varieties": {...}}. A variety has
  - live: trunk and boughs (`wood`), a lumpy crown core and leaf cards, with
    normals (crowns light as volumes, not as cards), uvs into the atlas and
    a per-vertex shade in `colors`;
  - remains: what the fire adds or uncovers (twig skeleton `skel`, debris),
    untextured, as before; low: 12 triangles of the same for far chunks;
  - canopy / bark: the atlas rects of the variety's proxy (`vegetation.rs`).
* vegetation_assets.blend and vegetation_preview/ (index.html, atlas and an
  overview render of every variety).

Species follow the fuel groups (`vegetation.rs`, `species_for`): conifers
(stone pine, Aleppo pine, cypress), broadleaves (holm oak, cork oak, carob,
olive), macchia (lentisk, myrtle, strawberry tree, broom, tree heath,
rosemary, cistus). Varieties of one species share its silhouette family.

Blender: metres, Z up, front -Y. Runtime: metres, Y up, front +Z.
UVs here are in the game's convention: (0, 0) is the top-left texel.
"""
import bpy
import json
import math
import random
import struct
import zlib
from pathlib import Path

import numpy as np
from mathutils import Matrix, Vector, noise

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'assets/models'
PREVIEW = OUT / 'vegetation_preview'
PREVIEW.mkdir(parents=True, exist_ok=True)
TMP = Path(bpy.app.tempdir)

# --- atlas layout ---------------------------------------------------------------
AW, AH = 1024, 512
LEAF_PX = 128      # leaf spray tiles, 6 x 3 grid at the top left
CANOPY_PX = 64     # canopy tiles, four per leaf slot after the leaf tiles
BARK_W = 32        # bark strips, full height, from x = 768
BARK_X = 768
SUPER = 2          # tiles are rendered this many times larger, then averaged

LEAVES = ['stone', 'aleppo', 'cypress', 'holm', 'cork', 'carob', 'olive',
          'lentisk', 'myrtle', 'arbutus', 'broom', 'heather', 'rosemary', 'cistus']
BARKS = ['pine', 'aleppo', 'oak', 'cork', 'olive', 'smooth', 'cypress', 'stem']


def leaf_px(name):
    i = LEAVES.index(name)
    return (i % 6) * LEAF_PX, (i // 6) * LEAF_PX


def canopy_px(slot):
    """Canopy tiles (one per leaf kind, then the grey) fill the leaf slots
    after the last leaf tile, four 64 px tiles per slot."""
    base = len(LEAVES) + slot // 4
    q = slot % 4
    return (base % 6) * LEAF_PX + (q % 2) * CANOPY_PX, (base // 6) * LEAF_PX + (q // 2) * CANOPY_PX


GREY_SLOT = len(LEAVES)


def rect(x, y, w, h, inset=0.0):
    return [(x + inset) / AW, (y + inset) / AH, (x + w - inset) / AW, (y + h - inset) / AH]


def leaf_rect(name):
    return rect(*leaf_px(name), LEAF_PX, LEAF_PX, .5)


def canopy_rect(name):
    return rect(*canopy_px(LEAVES.index(name)), CANOPY_PX, CANOPY_PX, 1.5)


def bark_rect(name):
    return rect(BARK_X + BARKS.index(name) * BARK_W, 0, BARK_W, AH, 1.0)


GREY_UV = [(canopy_px(GREY_SLOT)[0] + CANOPY_PX / 2) / AW, (canopy_px(GREY_SLOT)[1] + CANOPY_PX / 2) / AH]


def lin(c):
    return [x ** 2.2 for x in c]


# --- leaf sprays: geometry rendered into the atlas -------------------------------
# A spray is a list of flat polygons, painted bottom to top (later ones on top),
# in tile coordinates [-1, 1]. Colours are sRGB. Each kind follows its
# reference photo: needle tufts for the pines, scale sprays for the cypress,
# leaves of the right size, shape and two-tone for the broadleaves and the
# macchia, rods for the broom, needle-leaved twigs for heath and rosemary,
# with flowers and berries where the plant shows them.
SPECIES_LEAF = {
    #            kind      top               underside         size(L, W)    count  extra
    'stone':   ('needle', (.16, .28, .12), (.27, .38, .17), (.30, .016), 26, None),
    'aleppo':  ('needle', (.30, .40, .16), (.40, .47, .22), (.26, .012), 18, None),
    'cypress': ('scale', (.13, .23, .10), (.20, .31, .13), (.10, .05), 0, None),
    'holm':    ('leaf', (.15, .24, .10), (.42, .44, .36), (.11, .055), 0, None),
    'cork':    ('leaf', (.24, .35, .14), (.43, .45, .32), (.13, .07), 0, None),
    'carob':   ('pinnate', (.13, .25, .08), (.22, .34, .12), (.10, .08), 0, None),
    'olive':   ('leaf', (.30, .36, .23), (.60, .63, .55), (.16, .03), 0, None),
    'lentisk': ('pinnate', (.14, .24, .08), (.25, .33, .12), (.07, .03), 0, ((.62, .10, .08), .06)),
    'myrtle':  ('leaf', (.20, .37, .11), (.30, .44, .17), (.085, .042), 0, ((.95, .94, .90), .05)),
    'arbutus': ('leaf', (.17, .31, .10), (.30, .40, .16), (.15, .055), 0, ((.80, .22, .06), .07)),
    'broom':   ('rod', (.30, .43, .14), (.40, .50, .18), (.55, .018), 0, ((.97, .80, .10), .14)),
    'heather': ('heath', (.24, .32, .16), (.33, .38, .22), (.035, .008), 0, ((.90, .84, .84), .18)),
    'rosemary': ('heath', (.30, .38, .27), (.44, .50, .40), (.05, .010), 0, ((.48, .55, .85), .05)),
    'cistus':  ('leaf', (.33, .40, .25), (.48, .52, .40), (.11, .055), 0, ((.96, .92, .92), .06)),
}
TWIG = (.26, .19, .13)


def leaf_outline(L, W, pointed=True, n=7):
    """A leaf from its base at (0, 0) along +x: a lens, a little fuller near
    the base, pointed or blunt at the tip."""
    pts = [(0, 0)]
    for i in range(1, n):
        t = i / n
        w = W * math.sin(math.pi * t ** (.85 if pointed else .7)) * (1 - .25 * t)
        pts.append((L * t, w))
    pts.append((L, 0))
    for i in range(n - 1, 0, -1):
        t = i / n
        w = W * math.sin(math.pi * t ** (.85 if pointed else .7)) * (1 - .25 * t)
        pts.append((L * t, -w))
    return pts


def place(pts, origin, angle, scale=1.0):
    c, s = math.cos(angle), math.sin(angle)
    return [(origin[0] + (x * c - y * s) * scale, origin[1] + (x * s + y * c) * scale) for x, y in pts]


def quad_line(a, b, wa, wb):
    dx, dy = b[0] - a[0], b[1] - a[1]
    ln = math.hypot(dx, dy) or 1
    nx, ny = -dy / ln, dx / ln
    return [(a[0] + nx * wa, a[1] + ny * wa), (b[0] + nx * wb, b[1] + ny * wb),
            (b[0] - nx * wb, b[1] - ny * wb), (a[0] - nx * wa, a[1] - ny * wa)]


def disc(c, r, n=8):
    return [(c[0] + r * math.cos(i * math.tau / n), c[1] + r * math.sin(i * math.tau / n)) for i in range(n)]


def jitter(col, rng, k=.10):
    f = 1 + rng.uniform(-k, k)
    h = rng.uniform(-k, k) * .5
    return (col[0] * f * (1 + h), col[1] * f, col[2] * f * (1 - h))


def branches(rng, n, reach, start_r=.0, wander=.35):
    """Twig paths from near the centre outwards (a spray seen from above)."""
    out = []
    for i in range(n):
        a = i * math.tau / n + rng.uniform(-.4, .4)
        r0 = start_r * rng.uniform(.3, 1)
        p = [(math.cos(a) * r0, math.sin(a) * r0)]
        steps = 4
        for k in range(1, steps + 1):
            a += rng.uniform(-wander, wander) * .5
            r = r0 + (reach * rng.uniform(.82, 1.0) - r0) * k / steps
            p.append((math.cos(a) * r, math.sin(a) * r))
        out.append(p)
    return out


def spray(name, dense=False, seed=0):
    """Polygons of one leaf kind. `dense`: the canopy tile, leaves over the
    whole square (the background plane closes it)."""
    kind, top, under, (L, W), count, extra = SPECIES_LEAF[name]
    rng = random.Random(seed * 101 + LEAVES.index(name))
    polys = []   # (points, colour)
    reach = 1.35 if dense else .86

    def leaf_along(path, every, L, W, pointed=True, under_share=.25, angle=.75, pair=False):
        for (a, b) in zip(path, path[1:]):
            seg = math.hypot(b[0] - a[0], b[1] - a[1])
            k = max(1, int(seg / every))
            base_a = math.atan2(b[1] - a[1], b[0] - a[0])
            for j in range(k):
                t = (j + rng.random()) / k
                o = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
                for side in ((-1, 1) if pair else (rng.choice((-1, 1)),)):
                    ang = base_a + side * (angle + rng.uniform(-.25, .25))
                    col = under if rng.random() < under_share else top
                    depth = .78 + .32 * (math.hypot(*o) / reach)
                    polys.append((place(leaf_outline(L * rng.uniform(.8, 1.15), W * rng.uniform(.85, 1.15), pointed), o, ang),
                                  jitter([c * depth for c in col], rng)))

    twig_w = .012
    if kind == 'needle':
        # tufts of needles at the ends of twigs: the pine's brush
        tufts = 26 if dense else count
        for p in branches(rng, 5 if not dense else 8, reach * .75, .05):
            for a, b in zip(p, p[1:]):
                polys.append((quad_line(a, b, twig_w * 1.3, twig_w * .8), TWIG))
        for i in range(tufts):
            r = reach * math.sqrt(rng.uniform(.04, 1)) * .85
            a = rng.uniform(0, math.tau)
            c = (math.cos(a) * r, math.sin(a) * r)
            for k in range(22 if name == 'stone' else 16):
                ang = rng.uniform(0, math.tau)
                ln = L * rng.uniform(.55, 1.0)
                tip = (c[0] + math.cos(ang) * ln, c[1] + math.sin(ang) * ln)
                col = jitter(top if rng.random() < .7 else under, rng, .14)
                polys.append((quad_line(c, tip, W, W * .35), col))
    elif kind == 'scale':
        # flattened fans of scale leaves, overlapping
        for p in branches(rng, 15 if not dense else 22, reach * .95, .0, .5):
            for a, b in zip(p, p[1:]):
                polys.append((quad_line(a, b, .035, .028), jitter(top, rng, .08)))
                for s in (-1, 1):
                    ang = math.atan2(b[1] - a[1], b[0] - a[0]) + s * .9
                    mid = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)
                    tip = (mid[0] + math.cos(ang) * L * 1.4, mid[1] + math.sin(ang) * L * 1.4)
                    polys.append((quad_line(mid, tip, .028, .012), jitter(under if rng.random() < .4 else top, rng, .1)))
    elif kind in ('leaf', 'pinnate'):
        n_br = 11 if not dense else 14
        paths = branches(rng, n_br, reach * .9, .08)
        for p in paths:
            for a, b in zip(p, p[1:]):
                polys.append((quad_line(a, b, twig_w, twig_w * .6), TWIG))
        every = L * (.32 if kind == 'leaf' else .40)
        if dense:
            every *= .75
        for p in paths:
            leaf_along(p, every, L, W, pointed=name not in ('carob', 'cistus'),
                       under_share=.35 if name in ('olive', 'holm') else .2,
                       angle=.6 if kind == 'leaf' else 1.25, pair=True)
        # side shoots fill between the twigs
        for p in branches(rng, n_br + 4, reach * .75, .15, .7):
            leaf_along(p, every * 1.1, L * .9, W * .9, pointed=name not in ('carob', 'cistus'), under_share=.15)
    elif kind == 'rod':
        rods = []
        # broom: green rods fanning up from the base, flowers near their tips
        for i in range(44 if not dense else 70):
            if dense:
                a = (rng.uniform(-1, 1), rng.uniform(-1.2, 1.2))
            else:
                a = (rng.uniform(-.25, .25), -.92)
            ang = math.pi / 2 + rng.uniform(-.55, .55)
            ln = L * rng.uniform(.7, 1.45)
            b = (a[0] + math.cos(ang) * ln * 1.6, a[1] + math.sin(ang) * ln * 1.6)
            polys.append((quad_line(a, b, W, W * .5), jitter(top if rng.random() < .6 else under, rng, .12)))
            rods.append(((a, b), None))
    elif kind == 'heath':
        # fine twigs covered in needle leaves
        for p in branches(rng, 15 if not dense else 20, reach * .92, .0, .5):
            for a, b in zip(p, p[1:]):
                polys.append((quad_line(a, b, .008, .005), TWIG))
                seg = math.hypot(b[0] - a[0], b[1] - a[1])
                ang0 = math.atan2(b[1] - a[1], b[0] - a[0])
                for j in range(int(seg / (L * .18)) + 1):
                    t = rng.random()
                    o = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
                    for s in (-1, 1):
                        ang = ang0 + s * rng.uniform(.5, 1.2)
                        tip = (o[0] + math.cos(ang) * L, o[1] + math.sin(ang) * L)
                        polys.append((quad_line(o, tip, W, W * .4), jitter(top if rng.random() < .65 else under, rng, .12)))
    if extra:
        col, share = extra
        flowers = int(share * (len(polys) if kind != 'rod' else 160) * (.25 if kind in ('leaf', 'pinnate') else .12))
        for i in range(flowers):
            r = reach * math.sqrt(rng.uniform(.02, 1)) * .9
            a = rng.uniform(0, math.tau)
            c = (math.cos(a) * r, math.sin(a) * r)
            if kind == 'rod':
                # on the upper half of a rod
                rp, rc = rng.choice(rods)
                t = rng.uniform(.5, 1.0)
                c = (rp[0][0] + (rp[1][0] - rp[0][0]) * t, rp[0][1] + (rp[1][1] - rp[0][1]) * t)
            size = {'lentisk': .022, 'arbutus': .035, 'broom': .028, 'cistus': .05, 'myrtle': .03}.get(name, .018)
            polys.append((disc(c, size * rng.uniform(.8, 1.2), 7), jitter(col, rng, .08)))
    if not dense:
        # keep the spray off the tile's edge (mip and bilinear bleed)
        polys = [(p, c) for p, c in polys if all(abs(x) < .97 and abs(y) < .97 for x, y in p)]
    return polys, top


def reset_scene():
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o)
    for c in list(bpy.data.collections):
        bpy.data.collections.remove(c)
    for m in list(bpy.data.meshes):
        bpy.data.meshes.remove(m)


def render_polys(polys, px, background=None):
    """Workbench, flat light, vertex colours, top view: the albedo of the
    spray, with alpha. Returns float sRGB RGBA, row 0 at the top."""
    reset_scene()
    sc = bpy.context.scene
    sc.render.engine = 'BLENDER_WORKBENCH'
    sc.display.shading.light = 'FLAT'
    sc.display.shading.color_type = 'VERTEX'
    sc.display.render_aa = '16'
    sc.render.film_transparent = background is None
    sc.view_settings.view_transform = 'Standard'
    sc.view_settings.look = 'None'
    sc.render.resolution_x = sc.render.resolution_y = px * SUPER
    sc.render.resolution_percentage = 100
    sc.render.image_settings.file_format = 'PNG'
    sc.render.image_settings.color_mode = 'RGBA'
    verts, faces, cols = [], [], []
    if background is not None:
        polys = [([(-1.2, -1.2), (1.2, -1.2), (1.2, 1.2), (-1.2, 1.2)], background)] + polys
    for z, (pts, col) in enumerate(polys):
        i0 = len(verts)
        verts += [(x, y, z * 1e-3) for x, y in pts]
        faces.append(list(range(i0, len(verts))))
        cols.append(col)
    me = bpy.data.meshes.new('spray')
    me.from_pydata(verts, [], faces)
    ca = me.color_attributes.new('Col', 'FLOAT_COLOR', 'CORNER')
    k = 0
    for f, col in zip(me.polygons, cols):
        c = (*lin(col), 1.0)
        for li in f.loop_indices:
            ca.data[li].color = c
        k += 1
    me.color_attributes.active_color = ca
    ob = bpy.data.objects.new('spray', me)
    sc.collection.objects.link(ob)
    cam = bpy.data.objects.new('cam', bpy.data.cameras.new('cam'))
    sc.collection.objects.link(cam)
    sc.camera = cam
    cam.data.type = 'ORTHO'
    cam.data.ortho_scale = 2.0
    cam.data.clip_end = 100
    cam.location = (0, 0, 10)
    path = TMP / 'tile.png'
    sc.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)
    im = bpy.data.images.load(str(path), check_existing=False)
    im.colorspace_settings.name = 'Non-Color'
    a = np.array(im.pixels[:], dtype=np.float32).reshape(px * SUPER, px * SUPER, 4)[::-1]
    bpy.data.images.remove(im)
    return downsample(a, SUPER)


def downsample(a, k):
    """Box filter in linear light with premultiplied alpha."""
    h, w = a.shape[0] // k, a.shape[1] // k
    rgb = a[..., :3] ** 2.2 * a[..., 3:4]
    al = a[..., 3:4]
    rgb = rgb.reshape(h, k, w, k, 3).mean((1, 3))
    al = al.reshape(h, k, w, k, 1).mean((1, 3))
    rgb = np.where(al > 1e-4, rgb / np.maximum(al, 1e-4), 0) ** (1 / 2.2)
    return np.concatenate([rgb, al], -1)


def dilate(tile, passes=12):
    """Spread the colour of the leaves into the transparent texels around
    them, so filtering at the cut-out edge does not pull in black."""
    rgb, al = tile[..., :3].copy(), tile[..., 3]
    known = al > .02
    for _ in range(passes):
        acc = np.zeros_like(rgb)
        n = np.zeros(al.shape, np.float32)
        for dy, dx in ((0, 1), (0, -1), (1, 0), (-1, 0), (1, 1), (-1, -1), (1, -1), (-1, 1)):
            s = np.roll(np.roll(known, dy, 0), dx, 1)
            acc += np.roll(np.roll(rgb, dy, 0), dx, 1) * s[..., None]
            n += s
        grow = (~known) & (n > 0)
        rgb[grow] = acc[grow] / n[grow][:, None]
        known = known | grow
    if (~known).any():
        rgb[~known] = rgb[known].mean(0) if known.any() else .5
    return np.concatenate([rgb, al[..., None]], -1)


# --- bark: Blender's procedural noise, one sample per texel ----------------------
BARK = {
    # base, crack colour, plate scale (u, v), crack width, streaks
    'pine': ((.46, .26, .15), (.10, .07, .05), (3.0, 1.1), .30, .2),
    'aleppo': ((.40, .36, .32), (.16, .11, .08), (5.0, 1.0), .22, .3),
    'oak': ((.25, .23, .21), (.08, .07, .06), (9.0, 1.6), .26, .3),
    'cork': ((.52, .46, .38), (.20, .15, .11), (3.0, 1.5), .36, .2),
    'olive': ((.44, .42, .38), (.18, .16, .14), (4.0, .45), .16, 1.0),
    'smooth': ((.46, .22, .12), (.30, .14, .08), (1.5, .6), .08, .5),
    'cypress': ((.38, .31, .26), (.15, .12, .10), (10.0, .25), .20, 1.0),
    'stem': ((.27, .21, .16), (.12, .09, .07), (6.0, .8), .12, .4),
}


def bark_strip(name):
    base, crack, (su, sv), width, streak = BARK[name]
    out = np.zeros((AH, BARK_W, 4), np.float32)
    seed = Vector((BARKS.index(name) * 17.3, 3.1, 0))
    for y in range(AH):
        v = y / AH * 16.0          # the strip is 16 times as tall as wide
        for x in range(BARK_W):
            # the strip is mirrored at both edges on a trunk: fold u so the
            # pattern meets itself there
            u = x / BARK_W
            p = Vector((u * su, v * sv, 0)) + seed
            d = noise.voronoi(p, distance_metric='DISTANCE')[0]
            f1, f2 = d[0], d[1]
            edge = f2 - f1
            c = 1 - min(edge / width, 1)     # 1 in the cracks
            fine = noise.fractal(Vector((u * 18, v * 5, 1.7)) + seed, 0.6, 2.0, 4)
            fib = noise.noise(Vector((u * 30 * streak + .3, v * .8, 4.2)) + seed)
            t = min(max(c ** 1.5 + .15 * fib, 0), 1)
            col = [base[i] * (1 + .22 * fine + .10 * fib) * (1 - t) + crack[i] * t for i in range(3)]
            if name == 'cork':
                # patches of the red-orange inner bark where the cork was stripped
                patch = noise.noise(Vector((u * 1.2, v * .35, 8.0)) + seed)
                if patch > .28 and y > AH * .55:
                    col = [(.58, .26, .12)[i] * (1 + .15 * fine) for i in range(3)]
            if name == 'smooth':
                peel = noise.noise(Vector((u * 2.0, v * .7, 2.0)) + seed)
                if peel > .2:
                    col = [(.62, .42, .26)[i] * (1 + .1 * fine) for i in range(3)]
            out[y, x, :3] = [min(max(ch, 0), 1) for ch in col]
            out[y, x, 3] = 1
    return out


def write_png(path, rgba8):
    h, w = rgba8.shape[:2]
    raw = b''.join(b'\x00' + rgba8[y].tobytes() for y in range(h))
    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) \
        + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b'')
    Path(path).write_bytes(png)


def build_atlas():
    atlas = np.zeros((AH, AW, 4), np.float32)
    for name in LEAVES:
        polys, top = spray(name)
        tile = dilate(render_polys(polys, LEAF_PX))
        x, y = leaf_px(name)
        atlas[y:y + LEAF_PX, x:x + LEAF_PX] = tile
        polys, top = spray(name, dense=True, seed=1)
        bg = [c * .42 for c in top]
        tile = render_polys(polys, CANOPY_PX, background=bg)
        tile[..., 3] = 1
        # a crown seen from afar is as light as its leaves, not as the gaps
        # between them: the tile's mean (linear) brought to the leaf colour's
        lum = np.array([.2126, .7152, .0722])
        lin_t = tile[..., :3] ** 2.2
        k = (np.array(lin(top)) @ lum) * 1.6 / max(float((lin_t @ lum).mean()), 1e-4)
        tile[..., :3] = np.clip(lin_t * k, 0, 1) ** (1 / 2.2)
        x, y = canopy_px(LEAVES.index(name))
        atlas[y:y + CANOPY_PX, x:x + CANOPY_PX] = tile
        print(f'atlas: {name}')
    x, y = canopy_px(GREY_SLOT)
    # linear 0.5: the vertex colour as it is (`foliage.wesl`)
    atlas[y:y + CANOPY_PX, x:x + CANOPY_PX] = [.5 ** (1 / 2.2), .5 ** (1 / 2.2), .5 ** (1 / 2.2), 1]
    for i, name in enumerate(BARKS):
        atlas[:, BARK_X + i * BARK_W:BARK_X + (i + 1) * BARK_W] = bark_strip(name)
        print(f'atlas: bark {name}')
    rgba8 = (np.clip(atlas, 0, 1) * 255 + .5).astype(np.uint8)
    # exact sRGB 188 for the grey (linear 0.503)
    write_png(OUT / 'foliage_atlas.png', rgba8)
    return rgba8


# --- plant geometry with uvs ---------------------------------------------------
class Geo:
    """The live half of a variety, built directly as vertex lists."""

    def __init__(self):
        self.P, self.N, self.UV, self.S, self.W, self.T = [], [], [], [], [], []
        self.pads = []      # (centre, size) of each crown part, for the skeleton
        self.wood = []      # polylines of trunk and limbs

    def v(self, p, n, uv, shade, wood):
        self.P.append(Vector(p))
        self.N.append(Vector(n).normalized())
        self.UV.append(uv)
        self.S.append(shade)
        self.W.append(wood)
        return len(self.P) - 1

    def limb(self, pts, r0, r1, bark):
        """A tapering four-sided limb along a polyline, bark strip mirrored
        on alternate faces so the four faces meet without a seam."""
        pts = [Vector(p) for p in pts]
        self.wood.append([tuple(p) for p in pts])
        u0, v0, u1, v1 = bark_rect(bark)
        total = sum((b - a).length for a, b in zip(pts, pts[1:]))
        acc = 0.0
        rings = []
        for i, p in enumerate(pts):
            if i > 0:
                acc += (p - pts[i - 1]).length
            d = (pts[min(i + 1, len(pts) - 1)] - pts[max(i - 1, 0)]).normalized()
            side = d.cross(Vector((0, 0, 1)))
            if side.length < 1e-4:
                side = Vector((1, 0, 0))
            side.normalize()
            fwd = d.cross(side).normalized()
            r = r0 + (r1 - r0) * (acc / total if total else 0)
            v = v0 + (v1 - v0) * min(acc / 1.0, 1.0)
            ring = []
            for k in range(4):
                a = k * math.pi / 2 + math.pi / 4
                n = side * math.cos(a) + fwd * math.sin(a)
                ring.append(self.v(p + n * r, n, (u0 if k % 2 == 0 else u1, v), 1.0, True))
            rings.append(ring)
        for ra, rb in zip(rings, rings[1:]):
            for k in range(4):
                a, b = ra[k], ra[(k + 1) % 4]
                c, d = rb[k], rb[(k + 1) % 4]
                self.T += [(a, b, d), (a, d, c)]

    def core(self, centre, size, canopy, seed, jit=.14, flat=.55):
        """The crown's body: a lumpy 12-vertex icosahedron, its texture the
        canopy tile projected from above, normals of an ellipsoid."""
        c, s = Vector(centre), Vector(size)
        u0, v0, u1, v1 = canopy_rect(canopy)
        t = (1 + 5 ** .5) / 2
        base = [(-1, t, 0), (1, t, 0), (-1, -t, 0), (1, -t, 0), (0, -1, t), (0, 1, t), (0, -1, -t), (0, 1, -t),
                (t, 0, -1), (t, 0, 1), (-t, 0, -1), (-t, 0, 1)]
        faces = [(0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11), (1, 5, 9), (5, 11, 4), (11, 10, 2),
                 (10, 7, 6), (7, 1, 8), (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8), (3, 8, 9), (4, 9, 5),
                 (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1)]
        idx = []
        k = seed * 7.31
        for b in base:
            q = Vector(b).normalized()
            r = 1 + jit * math.sin(k + q.x * 5.1 + q.y * 3.7 + q.z * 4.3 + seed)
            q = q * r
            if q.z < 0:
                q.z *= flat
            p = c + Vector((q.x * s.x, q.y * s.y, q.z * s.z))
            n = Vector((q.x / s.x, q.y / s.y, q.z / s.z))
            h = (q.z + 1) / 2
            uv = (u0 + (u1 - u0) * (q.x * .5 + .5), v0 + (v1 - v0) * (q.y * .5 + .5))
            idx.append(self.v(p, n, uv, .78 + .40 * min(max(h, 0), 1), False))
        for f in faces:
            # icosahedron faces listed counter-clockwise from outside
            self.T.append((idx[f[0]], idx[f[1]], idx[f[2]]))
        self.pads.append((tuple(c), tuple(s)))

    def card(self, centre, normal, spin, half, leaf, pad_c, pad_s, mirror=False):
        """A leaf card: a quad facing `normal`, turned `spin` about it,
        half-extents `half` (w, h). Normals lean out of the crown, so the
        card lights as part of a volume; the material draws both faces."""
        n = Vector(normal).normalized()
        ref = Vector((0, 0, 1)) if abs(n.z) < .95 else Vector((1, 0, 0))
        right = ref.cross(n).normalized()
        up = n.cross(right).normalized()
        rot = Matrix.Rotation(spin, 3, n)
        right, up = rot @ right, rot @ up
        u0, v0, u1, v1 = leaf_rect(leaf)
        if mirror:
            u0, u1 = u1, u0
        c, pc, ps = Vector(centre), Vector(pad_c), Vector(pad_s)
        ids = []
        for (a, b), uv in (((-1, -1), (u0, v1)), ((1, -1), (u1, v1)), ((1, 1), (u1, v0)), ((-1, 1), (u0, v0))):
            p = c + right * (a * half[0]) + up * (b * half[1])
            d = p - pc
            radial = Vector((d.x / ps.x ** 2, d.y / ps.y ** 2, d.z / ps.z ** 2)).normalized()
            nn = (radial * .75 + n * .25 * (1 if n.dot(radial) >= 0 else -1)).normalized()
            h = (d.z / ps.z + 1) / 2
            ids.append(self.v(p, nn, uv, .78 + .40 * min(max(h, 0), 1), False))
        a, b, cc, dd = ids
        self.T += [(a, b, cc), (a, cc, dd)]


def crown(g, centre, size, leaf, seed, style, cards=None, core=.78):
    """A crown part: a core a little smaller than the part and leaf cards
    reaching past it, which make the silhouette."""
    rng = random.Random(seed)
    c, s = Vector(centre), Vector(size)
    # a column is all core (the cypress is dense), rods and heath are mostly cards
    core = {'column': .92, 'upright': .6}.get(style, core)
    g.core(c, s * core, leaf, seed, flat={'umbrella': .35, 'column': 1.0}.get(style, .6))
    g.pads[-1] = (tuple(c), tuple(s))
    if style == 'umbrella':
        # flat pads: cards lying across the top, a little tilted
        n = cards or 2
        for i in range(n):
            a = rng.uniform(0, math.tau)
            tilt = rng.uniform(.10, .30)
            nrm = Vector((math.cos(a) * tilt, math.sin(a) * tilt, 1))
            g.card(c + Vector((0, 0, s.z * (.15 + .3 * i))), nrm, rng.uniform(0, math.tau),
                   (s.x * 1.12, s.y * 1.12), leaf, c, s, i % 2 == 1)
    elif style == 'column':
        n = cards or 2
        a0 = rng.uniform(0, math.pi)
        for i in range(n):
            a = a0 + i * math.pi / n
            g.card(c, Vector((math.cos(a), math.sin(a), .12)), 0, (s.x * 1.25, s.z * 1.02), leaf, c, s, i % 2 == 1)
    elif style == 'upright':
        # rods and heath: cards standing, crossing at the clump's axis
        n = cards or 3
        a0 = rng.uniform(0, math.pi)
        for i in range(n):
            a = a0 + i * math.pi / n
            g.card(c + Vector((0, 0, s.z * .1)), Vector((math.cos(a), math.sin(a), .25)), rng.uniform(-.15, .15),
                   (max(s.x, s.y) * 1.15, s.z * 1.1), leaf, c, s, i % 2 == 1)
    else:
        # round crowns and mounds: one card across the top, the others
        # leaning out around it
        n = cards or 3
        a0 = rng.uniform(0, math.tau)
        for i in range(n):
            if i == 0:
                nrm = Vector((rng.uniform(-.2, .2), rng.uniform(-.2, .2), 1))
                off = Vector((0, 0, s.z * .35))
                half = (s.x * 1.05, s.y * 1.05)
            else:
                a = a0 + i * math.tau / (n - 1)
                nrm = Vector((math.cos(a), math.sin(a), .55))
                off = Vector((math.cos(a), math.sin(a), 0)) * s.x * .25
                half = (s.x * 1.08, s.z * 1.15)
            g.card(c + off, nrm, rng.uniform(0, math.tau), half, leaf, c, s, i % 2 == 1)


# --- varieties ------------------------------------------------------------------
VARIETIES = {}


def variety(species, name, text, leaf, bark, kind='tree'):
    def deco(fn):
        VARIETIES[name] = dict(species=species, text=text, build=fn, leaf=leaf, bark=bark, kind=kind)
        return fn
    return deco


def tree(trunks, pads, leaf, bark, style, limbs=(), seed=0, cards=None):
    """trunks/limbs: (polyline, r0, r1); pads: (centre, size)."""
    g = Geo()
    for pts, r0, r1 in trunks:
        g.limb(pts, r0, r1, bark)
    for pts, r0, r1 in limbs:
        g.limb(pts, r0, r1, bark)
    for i, (pos, size) in enumerate(pads):
        crown(g, pos, size, leaf, seed * 13 + i, style, cards)
    return g


def shrub(blobs, leaf, style='round', seed=0, cards=None, stems=None, bark='stem'):
    g = Geo()
    for pts, r0, r1 in stems or ():
        g.limb(pts, r0, r1, bark)
    for i, (pos, size) in enumerate(blobs):
        crown(g, pos, size, leaf, seed * 13 + i, style, cards)
    return g


# Stone pine (Pinus pinea): tall bare trunk, often leaning or forked, a flat
# dark umbrella.
@variety('pine', 'pine_1', 'Pino domestico: tronco inclinato e ombrello a tre cuscini.', 'stone', 'pine')
def pine_1():
    return tree([([(0, 0, 0), (.04, .01, .45), (.07, .02, .84)], .038, .02)],
                [((.08, .02, .88), (.36, .32, .09)), ((.27, .10, .80), (.20, .17, .07)), ((-.15, .08, .92), (.22, .19, .07))],
                'stone', 'pine', 'umbrella', seed=1)


@variety('pine', 'pine_2', 'Pino domestico gemello: due fusti dalla stessa base.', 'stone', 'pine')
def pine_2():
    return tree([([(0, 0, 0), (.02, 0, .32)], .045, .035)],
                [((.18, .05, .86), (.30, .27, .08)), ((-.17, -.05, .80), (.26, .23, .08)), ((.36, .10, .76), (.15, .13, .06))],
                'stone', 'pine', 'umbrella',
                limbs=[([(.02, 0, .30), (.16, .04, .80)], .03, .016), ([(.02, 0, .30), (-.14, -.04, .74)], .026, .014)], seed=2)


@variety('pine', 'pine_3', 'Pino domestico slanciato: fusto alto e ombrello stretto.', 'stone', 'pine')
def pine_3():
    return tree([([(0, 0, 0), (.03, .01, .5), (.06, .02, .92)], .03, .016)],
                [((.07, .02, .94), (.27, .24, .075)), ((.21, .08, .88), (.15, .13, .055)), ((-.13, -.02, .88), (.16, .14, .055))],
                'stone', 'pine', 'umbrella', seed=3)


@variety('pine', 'pine_4', 'Pino domestico piegato dal vento: chioma sottovento.', 'stone', 'pine')
def pine_4():
    return tree([([(0, 0, 0), (-.02, 0, .30), (.06, .03, .60), (.24, .08, .82)], .042, .02)],
                [((.26, .08, .84), (.34, .28, .08)), ((.46, .14, .78), (.20, .16, .06)), ((.08, .14, .86), (.18, .15, .06))],
                'stone', 'pine', 'umbrella', seed=4)


# Aleppo / maritime pine: lower, irregular open crown of tufts, paler
# yellow-green, grey bark, often leaning.
@variety('aleppo', 'aleppo_1', "Pino d'Aleppo: fusto inclinato e chioma irregolare a ciuffi.", 'aleppo', 'aleppo')
def aleppo_1():
    return tree([([(0, 0, 0), (.06, .02, .40), (.14, .04, .66)], .04, .02)],
                [((.14, .04, .72), (.28, .26, .17)), ((-.04, .10, .64), (.22, .20, .15)), ((.28, -.04, .62), (.20, .18, .14))],
                'aleppo', 'aleppo', 'round', seed=21, cards=2)


@variety('aleppo', 'aleppo_2', "Pino d'Aleppo biforcato: due cime aperte.", 'aleppo', 'aleppo')
def aleppo_2():
    return tree([([(0, 0, 0), (.02, 0, .34)], .045, .032)],
                [((.18, .06, .74), (.24, .22, .16)), ((-.16, -.04, .70), (.25, .23, .16)), ((.02, .02, .84), (.20, .18, .12))],
                'aleppo', 'aleppo', 'round',
                limbs=[([(.02, 0, .32), (.18, .05, .70)], .026, .014), ([(.02, 0, .32), (-.16, -.03, .66)], .024, .013)], seed=22, cards=2)


@variety('aleppo', 'aleppo_3', "Pino marittimo giovane: chioma conica e rada.", 'aleppo', 'aleppo')
def aleppo_3():
    return tree([([(0, 0, 0), (.01, 0, .48), (.0, .01, .92)], .035, .015)],
                [((0, .01, .50), (.26, .24, .16)), ((.01, 0, .66), (.21, .20, .15)), ((0, .01, .82), (.14, .13, .11))],
                'aleppo', 'aleppo', 'round', seed=23, cards=2)


# Mediterranean cypress: a dark green flame.
@variety('cypress', 'cypress_1', 'Cipresso: colonna stretta e scura.', 'cypress', 'cypress')
def cypress_1():
    return tree([([(0, 0, 0), (0, 0, .16)], .035, .03)],
                [((0, 0, .48), (.12, .12, .38)), ((0, 0, .84), (.07, .07, .14))],
                'cypress', 'cypress', 'column', seed=31, cards=2)


@variety('cypress', 'cypress_2', 'Cipresso a fiamma larga: rami aperti in basso.', 'cypress', 'cypress')
def cypress_2():
    return tree([([(0, 0, 0), (0, 0, .14)], .04, .034)],
                [((0, 0, .42), (.16, .15, .32)), ((.01, 0, .78), (.10, .09, .18))],
                'cypress', 'cypress', 'column', seed=32, cards=2)


# Holm oak: dense rounded dark crown on a short dark trunk.
@variety('holm', 'holm_1', 'Leccio: tronco tozzo e chioma densa e tonda.', 'holm', 'oak')
def holm_1():
    return tree([([(0, 0, 0), (0, 0, .50)], .08, .05)],
                [((0, 0, .66), (.46, .44, .32)), ((.30, .16, .54), (.28, .26, .22)), ((-.28, -.18, .58), (.28, .26, .23))],
                'holm', 'oak', 'round', seed=41)


@variety('holm', 'holm_2', 'Leccio alto: tronco lungo e chioma raccolta.', 'holm', 'oak')
def holm_2():
    return tree([([(0, 0, 0), (.02, 0, .60)], .07, .045)],
                [((.02, 0, .74), (.34, .33, .26)), ((.22, .12, .64), (.22, .21, .20)), ((-.20, -.12, .68), (.23, .21, .20))],
                'holm', 'oak', 'round', seed=42)


@variety('holm', 'holm_3', 'Leccio a ceppaia: tre polloni sotto una chioma larga.', 'holm', 'oak')
def holm_3():
    return tree([([(0, 0, 0), (.15, .04, .52)], .05, .03), ([(0, 0, 0), (-.15, .08, .56)], .045, .028)],
                [((.16, .04, .62), (.30, .28, .25)), ((-.16, .08, .66), (.28, .26, .24)), ((0, -.12, .74), (.24, .22, .18))],
                'holm', 'oak', 'round', seed=43)


# Cork oak: thick corky trunk (stripped red at the foot), open spreading crown.
@variety('cork', 'cork_1', 'Sughera: tronco grosso, chioma aperta su rami larghi.', 'cork', 'cork')
def cork_1():
    return tree([([(0, 0, 0), (.02, 0, .34)], .09, .07)],
                [((.24, .06, .62), (.30, .28, .18)), ((-.22, -.04, .60), (.28, .26, .18)), ((0, .14, .74), (.24, .22, .15))],
                'cork', 'cork', 'round',
                limbs=[([(.02, 0, .32), (.22, .06, .58)], .045, .025), ([(.02, 0, .32), (-.20, -.04, .56)], .04, .022)], seed=51)


@variety('cork', 'cork_2', 'Sughera inclinata: un grosso ramo laterale.', 'cork', 'cork')
def cork_2():
    return tree([([(0, 0, 0), (.06, .02, .30), (.10, .03, .50)], .085, .05)],
                [((.12, .04, .64), (.32, .30, .20)), ((.42, .10, .54), (.22, .20, .15)), ((-.14, .06, .60), (.20, .18, .14))],
                'cork', 'cork', 'round',
                limbs=[([(.07, .02, .34), (.38, .09, .50)], .04, .02)], seed=52)


# Carob: short thick trunk, a wide dense dome reaching low, glossy dark green.
@variety('carob', 'carob_1', 'Carrubo: cupola larga e bassa, verde scuro lucido.', 'carob', 'oak')
def carob_1():
    return tree([([(0, 0, 0), (0, 0, .28)], .09, .07)],
                [((0, 0, .46), (.52, .48, .26)), ((.32, .18, .36), (.28, .26, .18)), ((-.30, -.16, .36), (.28, .26, .18))],
                'carob', 'oak', 'round', seed=61)


@variety('carob', 'carob_2', 'Carrubo a due fusti: chioma asimmetrica.', 'carob', 'oak')
def carob_2():
    return tree([([(0, 0, 0), (.10, .02, .30)], .07, .05), ([(0, 0, 0), (-.10, .04, .26)], .06, .045)],
                [((.12, .02, .46), (.36, .34, .24)), ((-.18, .06, .42), (.32, .30, .22)), ((0, -.20, .38), (.24, .22, .16))],
                'carob', 'oak', 'round', seed=62)


# Olive: gnarled grey trunk, open silver-green crown.
@variety('olive', 'olive_1', 'Ulivo: tronco nodoso e tre cime argentate.', 'olive', 'olive')
def olive_1():
    return tree([([(0, 0, 0), (.04, 0, .30), (.06, 0, .56)], .07, .035)],
                [((.06, 0, .64), (.40, .36, .23)), ((.28, .10, .52), (.26, .24, .18)), ((-.20, -.12, .56), (.26, .24, .19))],
                'olive', 'olive', 'round', seed=71)


@variety('olive', 'olive_2', 'Ulivo contorto: tronco a S con un grosso ramo laterale.', 'olive', 'olive')
def olive_2():
    return tree([([(0, 0, 0), (-.06, .04, .20), (.04, 0, .56)], .08, .04)],
                [((.06, 0, .64), (.34, .30, .21)), ((.30, .14, .58), (.24, .22, .17)), ((-.16, .10, .60), (.24, .22, .17))],
                'olive', 'olive', 'round', limbs=[([(0, .02, .34), (.26, .12, .52)], .03, .016)], seed=72)


@variety('olive', 'olive_3', 'Ulivo vecchio a due fusti.', 'olive', 'olive')
def olive_3():
    return tree([([(0, 0, 0), (-.08, 0, .56)], .06, .032), ([(0, 0, 0), (.15, .05, .50)], .055, .03)],
                [((-.08, 0, .64), (.30, .28, .20)), ((.16, .05, .57), (.28, .26, .19)), ((0, -.12, .68), (.22, .20, .16))],
                'olive', 'olive', 'round', seed=73)


# Macchia. Lentisk: dense dark cushions, wide.
@variety('lentisk', 'lentisk_1', 'Lentisco: cuscino largo e denso, verde scuro.', 'lentisk', 'stem', 'shrub')
def lentisk_1():
    return shrub([((0, 0, .32), (.78, .72, .36)), ((.46, .22, .22), (.46, .42, .26)), ((-.40, -.30, .20), (.44, .40, .24))],
                 'lentisk', seed=81, cards=2)


@variety('lentisk', 'lentisk_2', 'Lentisco basso: tappeto a quattro lobi.', 'lentisk', 'stem', 'shrub')
def lentisk_2():
    return shrub([((0, 0, .22), (.82, .78, .24)), ((.52, .30, .16), (.42, .38, .16)), ((-.46, -.26, .15), (.40, .36, .15))],
                 'lentisk', seed=82, cards=2)


# Myrtle: compact rounded, brighter green.
@variety('myrtle', 'myrtle_1', 'Mirto: mazzo compatto e tondo, verde vivo.', 'myrtle', 'stem', 'shrub')
def myrtle_1():
    return shrub([((0, 0, .40), (.52, .50, .40)), ((.32, .12, .28), (.34, .32, .28)), ((-.26, -.12, .30), (.32, .30, .28))],
                 'myrtle', seed=91, cards=2)


@variety('myrtle', 'myrtle_2', 'Mirto rado: tre ciuffi staccati.', 'myrtle', 'stem', 'shrub')
def myrtle_2():
    return shrub([((-.40, .10, .28), (.36, .34, .28)), ((.38, -.10, .26), (.38, .35, .26)), ((.05, .42, .20), (.30, .28, .20))],
                 'myrtle', seed=92, cards=2)


# Strawberry tree: a small tree of the macchia, several red-barked stems.
@variety('arbutus', 'arbutus_1', 'Corbezzolo: alberello a più fusti, chioma tonda.', 'arbutus', 'smooth', 'shrub')
def arbutus_1():
    return shrub([((0, 0, .78), (.42, .40, .30)), ((.24, .10, .58), (.28, .26, .22))], 'arbutus', seed=101,
                 stems=[([(0, 0, 0), (.06, .02, .62)], .035, .02), ([(0, 0, 0), (-.08, .04, .58)], .03, .018)], bark='smooth')


@variety('arbutus', 'arbutus_2', 'Corbezzolo largo: due cime sopra i polloni.', 'arbutus', 'smooth', 'shrub')
def arbutus_2():
    return shrub([((.18, 0, .70), (.34, .32, .26)), ((-.20, .06, .66), (.32, .30, .26))], 'arbutus', seed=102,
                 stems=[([(0, 0, 0), (.16, 0, .56)], .03, .018), ([(0, 0, 0), (-.18, .05, .52)], .03, .018)], bark='smooth')


# Broom (Spartium): upright green rods, yellow in flower.
@variety('broom', 'broom_1', 'Ginestra: ciuffo di verghe diritte, fiori gialli.', 'broom', 'stem', 'shrub')
def broom_1():
    return shrub([((0, 0, .45), (.36, .34, .45))], 'broom', 'upright', seed=111, cards=3)


@variety('broom', 'broom_2', 'Ginestra doppia: due ciuffi accostati.', 'broom', 'stem', 'shrub')
def broom_2():
    return shrub([((-.18, 0, .40), (.28, .26, .40)), ((.22, .08, .34), (.26, .24, .34))], 'broom', 'upright', seed=112, cards=2)


# Tree heath: upright, fine, grey-green, white in flower.
@variety('heather', 'heather_1', 'Erica arborea: fusti diritti e fitti, chioma fine.', 'heather', 'stem', 'shrub')
def heather_1():
    return shrub([((0, 0, .50), (.30, .28, .50)), ((.24, .10, .36), (.22, .20, .36))], 'heather', 'upright', seed=121, cards=2)


@variety('heather', 'heather_2', 'Erica bassa: cespuglio largo.', 'heather', 'stem', 'shrub')
def heather_2():
    return shrub([((0, 0, .36), (.44, .40, .36)), ((-.30, .16, .26), (.26, .24, .26))], 'heather', 'upright', seed=122, cards=2)


# Rosemary and cistus: the low garrigue.
@variety('rosemary', 'rosemary_1', 'Rosmarino: cuscino basso grigio-verde.', 'rosemary', 'stem', 'shrub')
def rosemary_1():
    return shrub([((0, 0, .20), (.50, .46, .22)), ((.36, .20, .14), (.30, .28, .14))], 'rosemary', seed=131, cards=3)


@variety('rosemary', 'rosemary_2', 'Rosmarino sparso: tre cuscini.', 'rosemary', 'stem', 'shrub')
def rosemary_2():
    return shrub([((-.30, .05, .18), (.32, .30, .18)), ((.30, -.08, .16), (.30, .28, .16)), ((0, .34, .14), (.24, .22, .14))],
                 'rosemary', seed=132, cards=2)


@variety('cistus', 'cistus_1', 'Cisto: cupola bassa grigio-verde, fiori bianchi e rosa.', 'cistus', 'stem', 'shrub')
def cistus_1():
    return shrub([((0, 0, .26), (.56, .52, .26)), ((.38, .18, .18), (.32, .30, .18))], 'cistus', seed=141)


@variety('cistus', 'cistus_2', 'Cisto largo: tappeto a tre lobi.', 'cistus', 'stem', 'shrub')
def cistus_2():
    return shrub([((0, 0, .20), (.62, .58, .20)), ((.42, .30, .14), (.34, .30, .14)), ((-.40, -.26, .14), (.32, .30, .14))],
                 'cistus', seed=142, cards=2)


SPECIES = [
    ('pine', 'Pino domestico', 'conifere (classi 10-12)'), ('aleppo', "Pino d'Aleppo e marittimo", 'conifere'),
    ('cypress', 'Cipresso', 'conifere'), ('holm', 'Leccio', 'latifoglie (classi 4-6)'), ('cork', 'Sughera', 'latifoglie'),
    ('carob', 'Carrubo', 'latifoglie'), ('olive', 'Ulivo', 'latifoglie, e alberi isolati nei prati'),
    ('lentisk', 'Lentisco', 'macchia (classi 7-9)'), ('myrtle', 'Mirto', 'macchia'), ('arbutus', 'Corbezzolo', 'macchia alta'),
    ('broom', 'Ginestra', 'macchia e gariga'), ('heather', 'Erica arborea', 'macchia'), ('rosemary', 'Rosmarino', 'gariga'),
    ('cistus', 'Cisto', 'gariga'),
]


# --- what fire adds (untextured, as before) ---------------------------------------
current = None
CHAR = (.045, .038, .034)
CHAR2 = (.11, .085, .07)


def finish(o, name, kind, color):
    o.name = name
    for c in list(o.users_collection):
        c.objects.unlink(o)
    current.objects.link(o)
    o['kind'] = kind
    o['color'] = list(color)
    return o


def rod(name, a, b, r0, r1, color, kind='skel', sides=3):
    a, b = Vector(a), Vector(b)
    bpy.ops.mesh.primitive_cone_add(vertices=sides, radius1=r0, radius2=0, depth=(b - a).length,
                                    end_fill_type='NOTHING', location=(a + b) / 2)
    o = bpy.context.object
    o.rotation_euler = (b - a).to_track_quat('Z', 'Y').to_euler()
    return finish(o, name, kind, color)


def sample_wood(trunks):
    out = []
    for pts in trunks:
        for a, b in zip(pts, pts[1:]):
            for k in range(6):
                out.append(tuple(Vector(a).lerp(Vector(b), k / 6)))
        out.append(tuple(pts[-1]))
    return out


def root_for(trunks, pos):
    best, bd = None, 1e9
    for p in sample_wood(trunks):
        if p[2] > pos[2] - .06:
            continue
        d = (Vector(p) - Vector(pos)).length
        if d < bd:
            best, bd = p, d
    return best or trunks[0][-1]


def crown_skeleton(trunks, pads, seed=0):
    """The fine wood inside each crown part: a bough from the trunk to the
    part's centre and a fan of twigs that stop inside it."""
    rng = random.Random(seed)
    for pos, size in pads:
        root = root_for(trunks, pos)
        mid = Vector(root).lerp(Vector(pos), .5)
        rod('Bough', root, pos, .017, .010, CHAR)
        for i in range(3):
            a = i * math.tau / 3 + rng.uniform(-.4, .4)
            reach = rng.uniform(.62, .90)
            end = (pos[0] + math.cos(a) * size[0] * reach, pos[1] + math.sin(a) * size[1] * reach,
                   pos[2] + size[2] * rng.uniform(-.25, .55))
            rod('Twig', pos, end, .010, .004, CHAR2 if i % 2 else CHAR)
        a = rng.uniform(0, math.tau)
        end = (pos[0] + math.cos(a) * size[0] * 1.02, pos[1] + math.sin(a) * size[1] * 1.02, pos[2] + size[2] * .1)
        rod('Dead bough', tuple(mid), end, .012, .004, CHAR)


def shrub_skeleton(blobs, seed=0):
    rng = random.Random(seed)
    for pos, size in blobs:
        foot = (pos[0] * .35, pos[1] * .35, .02)
        n = 3 if size[0] > .3 else 2
        for i in range(n):
            a = i * math.tau / n + rng.uniform(-.35, .35)
            reach = rng.uniform(.15, .40)
            fork = (pos[0] * .6 + math.cos(a) * size[0] * reach * .6, pos[1] * .6 + math.sin(a) * size[1] * reach * .6,
                    pos[2] * .75 + size[2] * .05)
            rod('Stem', foot, fork, .028, .017, CHAR)
            for k in (-1, 1):
                b = a + k * rng.uniform(.5, .9)
                top = (pos[0] + math.cos(b) * size[0] * rng.uniform(.30, .55), pos[1] + math.sin(b) * size[1] * rng.uniform(.30, .55),
                       pos[2] + size[2] * rng.uniform(.10, .50))
                rod('Branch', fork, top, .015, .005, CHAR2 if k > 0 else CHAR)


def remains_objects(name, g):
    global current
    current = bpy.data.collections.new(name + '_remains')
    bpy.context.scene.collection.children.link(current)
    v = VARIETIES[name]
    seed = sum(map(ord, name))
    if v['kind'] == 'shrub' and not g.wood:
        shrub_skeleton(g.pads, seed)
    else:
        wood = g.wood or [[(0, 0, 0), (0, 0, min(p[0][2] for p in g.pads))]]
        crown_skeleton(wood, g.pads, seed)
        # a fallen limb for the larger trees
        if v['kind'] == 'tree' and seed % 2 == 0:
            rng = random.Random(seed)
            a = rng.uniform(0, math.tau)
            p0 = (math.cos(a) * .2, math.sin(a) * .2, .03)
            p1 = (math.cos(a) * .55, math.sin(a) * .55 + .1, .035)
            rod('Fallen limb', p0, p1, .026, .012, CHAR, 'debris', 4)
    bpy.context.view_layer.update()
    return list(current.objects)


def low_remains(objs):
    skel = [o for o in objs if o['kind'] == 'skel']
    pts = [o.matrix_world @ Vector(c) for o in skel for c in o.bound_box]
    lo = Vector((min(p.x for p in pts), min(p.y for p in pts), max(0.0, min(p.z for p in pts))))
    hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
    centre = (lo + hi) / 2
    reach = max(hi.x - lo.x, hi.y - lo.y) / 2 * .85
    global current
    current = bpy.data.collections.new('low')
    bpy.context.scene.collection.children.link(current)
    rod('Bare crown', (centre.x, centre.y, lo.z), (centre.x, centre.y, hi.z), reach, 0, CHAR2, 'skel', 5)
    out = list(current.objects)
    bpy.context.view_layer.update()
    return out


def triangles(objs):
    lookup, ps, ns, cs, idx, flags = {}, [], [], [], [], []
    for o in objs:
        mesh = o.to_mesh()
        mesh.calc_loop_triangles()
        color = tuple(o['color'])
        nm = o.matrix_world.to_3x3().inverted().transposed()
        flag = o['kind'] == 'debris'
        for tri in mesh.loop_triangles:
            n = (nm @ tri.normal).normalized()
            for vi in tri.vertices:
                p = o.matrix_world @ mesh.vertices[vi].co
                pp = [round(p.x, 5), round(p.z, 5), round(-p.y, 5)]
                nn = [round(n.x, 4), round(n.z, 4), round(-n.y, 4)]
                c = [round(x, 5) for x in color]
                key = (*pp, *c, flag)
                if key not in lookup:
                    lookup[key] = len(ps)
                    ps.append(pp); ns.append(nn); cs.append(c); flags.append(flag)
                idx.append(lookup[key])
        o.to_mesh_clear()
    return dict(positions=ps, normals=ns, colors=cs, indices=idx, debris=flags)


def live_json(g):
    """Blender frame to game frame (x, z, -y), indices wound as built."""
    r = lambda x, k=5: round(x, k)
    return dict(
        positions=[[r(p.x), r(p.z), r(-p.y)] for p in g.P],
        normals=[[r(n.x, 4), r(n.z, 4), r(-n.y, 4)] for n in g.N],
        colors=[[r(s, 4), r(s, 4), r(s, 4), 1] for s in g.S],
        uvs=[[r(u, 6), r(v, 6)] for u, v in g.UV],
        indices=[i for t in g.T for i in t],
        wood=g.W,
    )


LIVE_TRIS, LIVE_VERTS = 120, 96


def export():
    baked = {}
    built = {}
    for name, v in VARIETIES.items():
        g = v['build']()
        built[name] = g
        live = live_json(g)
        nt, nv = len(live['indices']) // 3, len(live['positions'])
        assert nt <= LIVE_TRIS and nv <= LIVE_VERTS, f'{name}: {nt} tris / {nv} verts over budget'
        rem_objs = remains_objects(name, g)
        rem = triangles(rem_objs)
        low = triangles(low_remains(rem_objs))
        baked[name] = dict(live=live, remains=rem, low=low,
                           canopy=canopy_rect(v['leaf']), bark=bark_rect(v['bark']))
        print(f"{name}: live {nt} tris / {nv} verts, remains {len(rem['indices']) // 3} / {len(rem['positions'])}, "
              f"low {len(low['indices']) // 3} / {len(low['positions'])}")
    data = dict(atlas=dict(file='foliage_atlas.png', size=[AW, AH], grey=GREY_UV), varieties=baked)
    (OUT / 'vegetation.json').write_text(json.dumps(data, separators=(',', ':')) + '\n')
    return built


# --- preview ----------------------------------------------------------------------
def preview(built):
    """Every variety live, textured with the atlas (Cycles)."""
    reset_scene()
    sc = bpy.context.scene
    sc.render.engine = 'CYCLES'
    sc.cycles.samples = 24
    sc.cycles.use_denoising = True
    sc.render.film_transparent = False
    sc.view_settings.view_transform = 'Standard'
    sc.render.resolution_x, sc.render.resolution_y = 1800, 1100
    world = sc.world or bpy.data.worlds.new('W')
    sc.world = world
    world.use_nodes = True
    world.node_tree.nodes['Background'].inputs['Color'].default_value = (.46, .52, .60, 1)
    world.node_tree.nodes['Background'].inputs['Strength'].default_value = 1.2
    img = bpy.data.images.load(str(OUT / 'foliage_atlas.png'), check_existing=True)
    mat = bpy.data.materials.new('atlas')
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = nt.nodes['Principled BSDF']
    tex = nt.nodes.new('ShaderNodeTexImage')
    tex.image = img
    tex.interpolation = 'Linear'
    attr = nt.nodes.new('ShaderNodeAttribute')
    attr.attribute_name = 'Shade'
    mix = nt.nodes.new('ShaderNodeMix')
    mix.data_type = 'RGBA'
    mix.blend_type = 'MULTIPLY'
    mix.inputs['Factor'].default_value = 1
    nt.links.new(tex.outputs['Color'], mix.inputs['A'])
    nt.links.new(attr.outputs['Color'], mix.inputs['B'])
    nt.links.new(mix.outputs['Result'], bsdf.inputs['Base Color'])
    nt.links.new(tex.outputs['Alpha'], bsdf.inputs['Alpha'])
    bsdf.inputs['Roughness'].default_value = .8
    ground_m = bpy.data.materials.new('ground')
    ground_m.use_nodes = True
    ground_m.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value = (*lin((.42, .38, .26)), 1)
    cols = 7
    names = list(VARIETIES)
    gap, rowgap = 1.9, 1.9
    for k, name in enumerate(names):
        g = built[name]
        sz = 1.0 if VARIETIES[name]['kind'] == 'tree' else .7
        ox, oy = (k % cols) * gap, -(k // cols) * rowgap
        me = bpy.data.meshes.new(name)
        me.from_pydata([tuple(p * sz) for p in g.P], [], g.T)
        uv = me.uv_layers.new(name='UVMap')
        sh = me.color_attributes.new('Shade', 'FLOAT_COLOR', 'CORNER')
        for poly in me.polygons:
            for li in poly.loop_indices:
                vi = me.loops[li].vertex_index
                u, v = g.UV[vi]
                uv.data[li].uv = (u, 1 - v)
                s = g.S[vi]
                sh.data[li].color = (s, s, s, 1)
        me.materials.append(mat)
        ob = bpy.data.objects.new(name, me)
        ob.location = (ox, oy, 0)
        sc.collection.objects.link(ob)
    bpy.ops.mesh.primitive_plane_add(size=1, location=(gap * (cols - 1) / 2, -rowgap * 2, -.002))
    pl = bpy.context.object
    pl.scale = (80, 80, 1)
    pl.data.materials.append(ground_m)
    bpy.ops.object.light_add(type='SUN', location=(0, 0, 10))
    sun = bpy.context.object
    sun.rotation_euler = (math.radians(48), 0, math.radians(-35))
    sun.data.energy = 4.5
    sun.data.angle = math.radians(6)
    cam = bpy.data.objects.new('cam', bpy.data.cameras.new('cam'))
    sc.collection.objects.link(cam)
    sc.camera = cam
    cam.data.type = 'ORTHO'
    cam.data.ortho_scale = gap * cols + .4
    cam.rotation_euler = (math.radians(62), 0, 0)
    target = Vector((gap * (cols - 1) / 2, -rowgap * ((len(names) - 1) // cols) / 2 - .2, .35))
    view = cam.rotation_euler.to_quaternion() @ Vector((0, 0, -1))
    cam.location = target - view * 14
    sc.render.filepath = str(PREVIEW / 'overview_vivo.png')
    bpy.ops.render.render(write_still=True)


def html():
    rows = []
    for key, title, note in SPECIES:
        names = [n for n, v in VARIETIES.items() if v['species'] == key]
        items = ''.join(f'<li><b>{n}</b>: {VARIETIES[n]["text"]}</li>' for n in names)
        rows.append(f'<section><h2>{title} <small>{note}</small></h2><ul>{items}</ul></section>')
    page = f'''<!doctype html>
<html lang="it"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Vegetazione: specie e varietà</title>
<style>
:root{{--bg:#14171c;--fg:#e8eaed;--mut:#9aa3ad;--line:#2a2f37}}
@media (prefers-color-scheme:light){{:root{{--bg:#f4f1ea;--fg:#1d2024;--mut:#5e6670;--line:#d9d3c6}}}}
body{{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif}}
main{{max-width:1240px;margin:0 auto;padding:24px 16px 64px}}
h1{{font-size:1.6rem;margin:.2em 0}} h2{{margin:1.4em 0 .3em;font-size:1.15rem}} h2 small{{color:var(--mut);font-weight:400;font-size:.8em}}
p,li{{color:var(--mut);max-width:80ch}} li b{{color:var(--fg)}}
img{{width:100%;height:auto;display:block;border-radius:8px;border:1px solid var(--line);image-rendering:auto}}
figure{{margin:0 0 18px}} figcaption{{padding:6px 2px;color:var(--mut)}}
</style></head><body><main>
<h1>Vegetazione: specie e varietà</h1>
<p>Specie mediterranee per gruppo di combustibile: ciò che si vede è ciò che brucia. Ogni pianta ha un corpo della chioma, carte di foglie ritagliate dall&rsquo;atlante e la corteccia sul tronco; lo scheletro dei rametti e i detriti compaiono quando brucia (shader del gioco).</p>
<figure><img src="overview_vivo.png" alt="Tutte le varietà"><figcaption>Le varietà, vive (Cycles, con l&rsquo;atlante del gioco). Righe da sinistra a destra nell&rsquo;ordine dell&rsquo;elenco.</figcaption></figure>
<figure><img src="../foliage_atlas.png" alt="Atlante" style="background:#444"><figcaption>L&rsquo;atlante (1024 &times; 512): rametti con le foglie, tessere delle chiome lontane, strisce di corteccia.</figcaption></figure>
{"".join(rows)}
<p>Generato da <code>scripts/build_vegetation_models.py</code> (Blender). Riferimenti fotografici e licenze: <code>assets/models/CREDITS.txt</code>.</p>
</main></body></html>'''
    (PREVIEW / 'index.html').write_text(page, encoding='utf-8')


if __name__ == '__main__':
    import sys
    only_atlas = '--atlas-only' in sys.argv
    reset_scene()
    build_atlas()
    if not only_atlas:
        reset_scene()
        built = export()
        if '--no-preview' not in sys.argv:
            preview(built)
        html()
        reset_scene()
        for name, v in VARIETIES.items():
            g = v['build']()
            me = bpy.data.meshes.new(name)
            me.from_pydata([tuple(p) for p in g.P], [], g.T)
            uv = me.uv_layers.new(name='UVMap')
            for poly in me.polygons:
                for li in poly.loop_indices:
                    u, vv = g.UV[me.loops[li].vertex_index]
                    uv.data[li].uv = (u, 1 - vv)
            col = bpy.data.collections.new(name)
            bpy.context.scene.collection.children.link(col)
            col.objects.link(bpy.data.objects.new(name, me))
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(OUT / 'vegetation_assets.blend'))
