"""Levels of detail of the normalised plants, driven by screen-space error
(2026-11-04, docs/TODO.md «Vegetazione realistica», step 3; the user: «non
mi convince l'idea di ingrandire le foglie e togliere rami a caso, fai un
proper lod»).

    Blender --background --python tools/vegetation/lod.py -- <plant> [...]

Each level is drawn from a distance `D` on (`LEVELS`). There a pixel spans
`e = D * PIXEL_RAD` metres, and the level removes or merges only what is
smaller than that. Nothing is chosen at random and nothing is scaled:

- **wood**: each loose part of the wood (a branch, a twig) thinner than
  `TWIG_PX * e` leaves the geometry (its thickness: the two smaller
  principal spreads of its vertices). What stays is simplified with
  Blender's quadric collapse to a triangle per `WOOD_PX` pixels of
  wood on screen (its surface over pi);
- **leaves**: grouped in clusters (k-means on the leaves' centres; k from
  the crown's size over a card of `CARD_PX * e`), each cluster three
  crossed cards on its principal planes (PCA), with normals out of the
  crown's centre. The card's texture is a
  render of the cluster's true leaves *and* of the thin twigs nearest it,
  seen square to the card (Eevee, the materials' own colour and alpha,
  unlit), so what leaves the geometry is still painted on a card.

Writes vegetation_src/lod/<plant>_<level>.glb, its card atlas
<plant>_<level>_cards.png and summary.json; `render_lods.py` compares each
level with the original at the level's distance.
"""
import bpy
import json
import math
import os
import sys
import zlib
from pathlib import Path

import numpy as np
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from normalize import is_leaf  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
NORM = ROOT / 'vegetation_src' / 'normalized'
OUT = ROOT / 'vegetation_src' / 'lod'
OUT.mkdir(parents=True, exist_ok=True)

# a pixel of a 1080-line view with a ~45° vertical field
PIXEL_RAD = math.radians(45) / 1080
# level -> the distance it is built for (m); the game draws each from a
# little farther on, so it never shows coarser than this (2026-11-04, the
# user: «vorrei dei modelli dettagliati da vicino»; `lod3` takes the
# place of the game's proxies: «non voglio vedere i modelli vecchi»).
# `LOD_LEVELS=lod0,lod3` builds only those.
LEVELS = {'lod0': 15.0, 'lod1': 40.0, 'lod2': 150.0, 'lod3': 600.0}
TWIG_PX = 1.0
# screen pixels of wood per triangle left by the collapse
WOOD_PX = 2.0
# a card spans this many pixels at its level's distance: the last level,
# a few dozen pixels tall, needs more of them to keep its outline
CARD_PX = {'lod0': 12.0, 'lod1': 10.0, 'lod2': 6.0, 'lod3': 5.0}
# at most this many leaf clusters (three cards each): each card is a render
MAX_CLUSTERS = {'lod0': 1200, 'lod1': 800, 'lod2': 800, 'lod3': 800}
TILE = 64


# --- mesh access --------------------------------------------------------------

def mesh_arrays(obj, asset):
    me = obj.data
    nf = len(me.polygons)
    g = lambda coll, attr, n, dt, w=1: (lambda a: (coll.foreach_get(attr, a), a)[1])(np.empty(n * w, dt))
    mat = g(me.polygons, 'material_index', nf, np.int32)
    start = g(me.polygons, 'loop_start', nf, np.int32)
    total = g(me.polygons, 'loop_total', nf, np.int32)
    centre = g(me.polygons, 'center', nf, np.float32, 3).reshape(-1, 3)
    lv = g(me.loops, 'vertex_index', len(me.loops), np.int32)
    co = g(me.vertices, 'co', len(me.vertices), np.float32, 3).reshape(-1, 3)
    slots = np.array([bool(s.material and is_leaf(s.material.name, asset)) for s in obj.material_slots] or [False])
    leaf = slots[np.clip(mat, 0, len(slots) - 1)]
    loop_face = np.repeat(np.arange(nf), total)
    return dict(nf=nf, start=start, total=total, centre=centre, lv=lv, co=co, leaf=leaf, loop_face=loop_face)


def islands(m, faces):
    """Loose parts among `faces`: a label per face (the smallest vertex
    index of its part). Min-label propagation through the faces, with
    pointer jumping (a label is always a vertex of the same part, not
    larger than the vertex itself)."""
    mask = np.zeros(m['nf'], bool)
    mask[faces] = True
    loops = np.nonzero(mask[m['loop_face']])[0]
    lf = m['loop_face'][loops]
    verts = m['lv'][loops]
    label = np.arange(len(m['co']), dtype=np.int64)
    while True:
        face_min = np.full(m['nf'], np.iinfo(np.int64).max)
        np.minimum.at(face_min, lf, label[verts])
        new = label.copy()
        np.minimum.at(new, verts, face_min[lf])
        while True:
            jump = new[new]
            if np.array_equal(jump, new):
                break
            new = jump
        if np.array_equal(new, label):
            break
        label = new
    return label[m['lv'][m['start'][faces]]]


def part_stats(m, faces, face_label):
    """Per loose part (sorted unique labels): centre and thickness (the
    diameter of a cylinder with the two smaller principal spreads), from
    the parts' face centres weighted by nothing."""
    ids, inv = np.unique(face_label, return_inverse=True)
    p = m['centre'][faces].astype(np.float64)
    n = np.bincount(inv).astype(np.float64)
    mean = np.stack([np.bincount(inv, p[:, k]) / n for k in range(3)], 1)
    d = p - mean[inv]
    cov = np.empty((len(ids), 3, 3))
    for a in range(3):
        for b in range(a, 3):
            cov[:, a, b] = cov[:, b, a] = np.bincount(inv, d[:, a] * d[:, b]) / n
    lam = np.clip(np.linalg.eigvalsh(cov), 0, None)  # ascending
    # face centres of a tube of radius r sit at r from the axis: sigma = r/sqrt2
    diam = 2 * math.sqrt(2) * (lam[:, 0] * lam[:, 1]) ** 0.25
    return ids, inv, mean, diam


def select_faces(obj, mask):
    """Exactly the faces of `mask` selected, `obj` alone in edit mode."""
    me = obj.data
    me.vertices.foreach_set('select', np.zeros(len(me.vertices), bool))
    me.edges.foreach_set('select', np.zeros(len(me.edges), bool))
    me.polygons.foreach_set('select', mask)
    lv = np.empty(len(me.loops), np.int32)
    me.loops.foreach_get('vertex_index', lv)
    total = np.empty(len(me.polygons), np.int32)
    me.polygons.foreach_get('loop_total', total)
    vsel = np.zeros(len(me.vertices), bool)
    vsel[lv[np.repeat(mask, total)]] = True
    me.vertices.foreach_set('select', vsel)
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_mode(type='FACE')


def delete_faces(obj, mask):
    select_faces(obj, mask)
    bpy.ops.mesh.delete(type='FACE')
    bpy.ops.object.mode_set(mode='OBJECT')


def pngs():
    """The JPEG textures as PNG (no JPEG decoder in the game)."""
    for img in bpy.data.images:
        if img.source == 'FILE' and not img.filepath.lower().endswith('.png'):
            p = Path(bpy.path.abspath(img.filepath))
            png = p.parent / 'png' / (p.stem + '.png')
            png.parent.mkdir(exist_ok=True)
            img.filepath_raw = str(png)
            img.file_format = 'PNG'
            img.save()


# --- clusters and cards -------------------------------------------------------

def kmeans(x, k, seed, iters=15):
    rng = np.random.default_rng(seed)
    k = max(1, min(k, len(x)))
    c = x[rng.choice(len(x), k, replace=False)].copy()
    lab = np.zeros(len(x), np.int64)
    for _ in range(iters):
        lab = np.concatenate([((x[i:i + 8192, None, :] - c[None]) ** 2).sum(-1).argmin(1) for i in range(0, len(x), 8192)])
        n = np.bincount(lab, minlength=k)
        for a in range(3):
            s = np.bincount(lab, x[:, a], minlength=k)
            c[n > 0, a] = s[n > 0] / n[n > 0]
    return lab, c


def card_frames(points, plant_centre):
    """Three crossed cards on the cluster's principal planes: (u, v), the
    mean plane, then (u, n) and (v, n), so that no view sees all of them
    edge-on. Each is (centre, right, up, normal, half side), right-handed,
    the first normal leaning out of the crown."""
    c = points.mean(0)
    _, _, vt = np.linalg.svd(points - c, full_matrices=False)
    u, v = vt[0], vt[1]
    n = np.cross(u, v)
    if np.dot(n, c - plant_centre) < 0:
        v = -v
        n = -n
    out = []
    for a, b in ((u, v), (u, n), (v, n)):
        pa = (points - c) @ a
        pb = (points - c) @ b
        mid = c + a * (pa.max() + pa.min()) / 2 + b * (pb.max() + pb.min()) / 2
        h = max(pa.max() - pa.min(), pb.max() - pb.min(), 1e-3) / 2 * 1.04
        out.append((mid, a, b, np.cross(a, b), h))
    return out


def unlit(mat, current):
    """A copy of `mat` that shows its base colour and alpha as they are,
    only on the faces of the cluster in `current` (a Value node set before
    each render)."""
    m = mat.copy()
    nt = m.node_tree
    bsdf = next(n for n in nt.nodes if n.type == 'BSDF_PRINCIPLED')
    out = next(n for n in nt.nodes if n.type == 'OUTPUT_MATERIAL')
    em = nt.nodes.new('ShaderNodeEmission')
    tr = nt.nodes.new('ShaderNodeBsdfTransparent')
    mix = nt.nodes.new('ShaderNodeMixShader')
    col = bsdf.inputs['Base Color']
    if col.is_linked:
        nt.links.new(col.links[0].from_socket, em.inputs['Color'])
    else:
        em.inputs['Color'].default_value = col.default_value
    attr = nt.nodes.new('ShaderNodeAttribute')
    attr.attribute_type = 'GEOMETRY'
    attr.attribute_name = 'cluster'
    cur = nt.nodes.new('ShaderNodeValue')
    cur.name = 'current'
    eq = nt.nodes.new('ShaderNodeMath')
    eq.operation = 'COMPARE'
    eq.inputs[2].default_value = 0.25
    nt.links.new(attr.outputs['Fac'], eq.inputs[0])
    nt.links.new(cur.outputs[0], eq.inputs[1])
    a = bsdf.inputs['Alpha']
    fac = nt.nodes.new('ShaderNodeMath')
    fac.operation = 'MULTIPLY'
    if a.is_linked:
        nt.links.new(a.links[0].from_socket, fac.inputs[0])
    else:
        fac.inputs[0].default_value = a.default_value
    nt.links.new(eq.outputs[0], fac.inputs[1])
    nt.links.new(fac.outputs[0], mix.inputs['Fac'])
    nt.links.new(tr.outputs[0], mix.inputs[1])
    nt.links.new(em.outputs[0], mix.inputs[2])
    nt.links.new(mix.outputs[0], out.inputs['Surface'])
    m.surface_render_method = 'DITHERED'
    current.append(cur)
    return m


def dilate(px, steps=6):
    """Spread the colour of the opaque texels into the transparent ones,
    so that mipmaps and filtering do not pull in black."""
    a = px[..., 3] > 0.5
    rgb = px[..., :3] * a[..., None]
    w = a.astype(np.float32)
    for _ in range(steps):
        s = np.zeros_like(rgb)
        ws = np.zeros_like(w)
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                s += np.roll(np.roll(rgb, dy, 0), dx, 1)
                ws += np.roll(np.roll(w, dy, 0), dx, 1)
        fill = (w == 0) & (ws > 0)
        rgb[fill] = s[fill] / ws[fill, None]
        w[fill] = 1
    px[..., :3] = rgb
    return px


def bake_cards(obj, frames, path):
    """Render each card's cluster square to the card into one atlas."""
    scene = bpy.context.scene
    scene.render.engine = 'BLENDER_EEVEE'
    scene.eevee.taa_render_samples = 16
    scene.render.film_transparent = True
    scene.view_settings.view_transform = 'Standard'
    scene.render.resolution_x = scene.render.resolution_y = TILE
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = 'PNG'
    scene.render.image_settings.color_mode = 'RGBA'
    cam = bpy.data.objects.new('bake_cam', bpy.data.cameras.new('bake_cam'))
    cam.data.type = 'ORTHO'
    scene.collection.objects.link(cam)
    scene.camera = cam
    current = []
    originals = [s.material for s in obj.material_slots]
    for s in obj.material_slots:
        if s.material is not None:
            s.material = unlit(s.material, current)
    cols = math.ceil(math.sqrt(len(frames)))
    atlas = np.zeros((cols * TILE, cols * TILE, 4), np.float32)
    tmp = OUT / f'_card_{os.getpid()}.png'
    for i, (j, c, u, v, n, h) in enumerate(frames):
        for node in current:
            node.outputs[0].default_value = float(j)
        cam.data.ortho_scale = 2 * h
        depth = 50.0
        rot = Matrix((tuple(u), tuple(v), tuple(n))).transposed().to_4x4()
        cam.matrix_world = Matrix.Translation(Vector(tuple(c + n * depth))) @ rot
        cam.data.clip_start, cam.data.clip_end = 0.01, depth * 2
        scene.render.filepath = str(tmp)
        bpy.ops.render.render(write_still=True)
        img = bpy.data.images.load(str(tmp), check_existing=False)
        px = np.array(img.pixels[:], np.float32).reshape(TILE, TILE, 4)[::-1]
        bpy.data.images.remove(img)
        r, k = divmod(i, cols)
        atlas[r * TILE:(r + 1) * TILE, k * TILE:(k + 1) * TILE] = px
    tmp.unlink(missing_ok=True)
    bpy.data.objects.remove(cam)
    for s, mat in zip(obj.material_slots, originals):
        s.material = mat
    atlas = dilate(atlas)
    img = bpy.data.images.new(path.stem, cols * TILE, cols * TILE, alpha=True)
    img.pixels = atlas[::-1].ravel()
    img.filepath_raw = str(path)
    img.file_format = 'PNG'
    img.save()
    return cols


def facing(mat):
    """A copy of the card material for the Blender previews that shows each
    card only where it is, of its cluster's three, the most square to the
    view: |n.i| >= |a.i| and |n.i| >= |b.i| (`a`, `b` its axes, `i` the
    view). The game's plant shader is to do the same."""
    m = mat.copy()
    nt = m.node_tree
    bsdf = nt.nodes['Principled BSDF']
    rnd = bsdf.inputs['Alpha'].links[0].from_node
    geo = nt.nodes.new('ShaderNodeNewGeometry')
    att = {}
    for nm in ('_CARD_A', '_CARD_B'):
        n = nt.nodes.new('ShaderNodeAttribute')
        n.attribute_type = 'GEOMETRY'
        n.attribute_name = nm
        att[nm] = n.outputs['Vector']
    cross = nt.nodes.new('ShaderNodeVectorMath')
    cross.operation = 'CROSS_PRODUCT'
    nt.links.new(att['_CARD_A'], cross.inputs[0])
    nt.links.new(att['_CARD_B'], cross.inputs[1])

    def absdot(sock):
        d = nt.nodes.new('ShaderNodeVectorMath')
        d.operation = 'DOT_PRODUCT'
        nt.links.new(sock, d.inputs[0])
        nt.links.new(geo.outputs['Incoming'], d.inputs[1])
        a = nt.nodes.new('ShaderNodeMath')
        a.operation = 'ABSOLUTE'
        nt.links.new(d.outputs['Value'], a.inputs[0])
        return a.outputs[0]

    dn, da, db = absdot(cross.outputs['Vector']), absdot(att['_CARD_A']), absdot(att['_CARD_B'])
    mx = nt.nodes.new('ShaderNodeMath')
    mx.operation = 'MAXIMUM'
    nt.links.new(da, mx.inputs[0])
    nt.links.new(db, mx.inputs[1])
    sub = nt.nodes.new('ShaderNodeMath')
    sub.operation = 'SUBTRACT'
    nt.links.new(dn, sub.inputs[0])
    nt.links.new(mx.outputs[0], sub.inputs[1])
    gt = nt.nodes.new('ShaderNodeMath')
    gt.operation = 'GREATER_THAN'
    gt.inputs[1].default_value = -1e-4
    nt.links.new(sub.outputs[0], gt.inputs[0])
    mul = nt.nodes.new('ShaderNodeMath')
    mul.operation = 'MULTIPLY'
    nt.links.new(rnd.outputs[0], mul.inputs[0])
    nt.links.new(gt.outputs[0], mul.inputs[1])
    nt.links.new(mul.outputs[0], bsdf.inputs['Alpha'])
    return m


def alpha_image(bsdf):
    """The image feeding a Principled BSDF's alpha, through math nodes."""
    sock = bsdf.inputs['Alpha']
    for _ in range(4):
        if not sock.is_linked:
            return None
        node = sock.links[0].from_node
        if node.type == 'TEX_IMAGE':
            return node.image
        sock = next((i for i in node.inputs if i.is_linked), None)
        if sock is None:
            return None
    return None


def leaf_roughness(obj, asset):
    """The mean roughness of the plant's foliage materials, over their
    texels: the cards' own (the leaves' sheen of the sky makes most of
    their colour from afar)."""
    vals = []
    for sl in obj.material_slots:
        m = sl.material
        if m is None or not is_leaf(m.name, asset) or not m.use_nodes:
            continue
        b = next((n for n in m.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
        if b is None:
            continue
        r = b.inputs['Roughness']
        if not r.is_linked:
            vals.append(r.default_value)
            continue
        link = r.links[0]
        node, sock = link.from_node, link.from_socket.name
        if node.type == 'SEPARATE_COLOR' and node.inputs[0].is_linked:
            node = node.inputs[0].links[0].from_node
        if node.type != 'TEX_IMAGE' or node.image is None:
            continue
        img = node.image
        px = np.array(img.pixels[:], np.float32).reshape(-1, img.channels)
        ch = {'Red': 0, 'Green': 1, 'Blue': 2}.get(sock, 0)
        # only where there is a leaf: the alpha map (alpha.wire) when the
        # same size, else the texture's own alpha
        mask = None
        al = alpha_image(b)
        if al is not None and tuple(al.size) == tuple(img.size):
            mask = np.array(al.pixels[:], np.float32).reshape(-1, al.channels)[:, 0] > 0.5
        elif img.channels == 4:
            mask = px[:, 3] > 0.5
        v = px[mask, ch] if mask is not None and mask.any() else px[:, ch]
        vals.append(float(np.median(v)))
    return float(np.mean(vals)) if vals else 0.6


def card_object(name, frames, cols, atlas_path, crown_centre, roughness):
    verts, faces, uvs = [], [], []
    for i, (j, c, u, v, n, h) in enumerate(frames):
        r, k = divmod(i, cols)
        u0, u1 = k / cols, (k + 1) / cols
        v0, v1 = 1 - (r + 1) / cols, 1 - r / cols
        base = len(verts)
        for a, b, uv in ((-1, -1, (u0, v0)), (1, -1, (u1, v0)), (1, 1, (u1, v1)), (-1, 1, (u0, v1))):
            verts.append(tuple(c + u * a * h + v * b * h))
            uvs.append(uv)
        faces.append((base, base + 1, base + 2, base + 3))
    me = bpy.data.meshes.new(name)
    me.from_pydata(verts, [], faces)
    uvl = me.uv_layers.new(name='UVMap')
    for li, loop in enumerate(me.loops):
        uvl.data[li].uv = uvs[loop.vertex_index]
    # normals out of the crown's centre (a sphere's), not the cards' own:
    # the crown is shaded as a volume, and a card has no dark side
    p = np.array(verts)
    d = p - crown_centre
    d /= np.linalg.norm(d, axis=1, keepdims=True) + 1e-6
    # half way to the sky: a crown's underside still sees the sky through
    # its gaps, and lights up as the scanned leaves do
    d = d + np.array([0, 0, 1.0])
    d /= np.linalg.norm(d, axis=1, keepdims=True) + 1e-6
    me.normals_split_custom_set_from_vertices([tuple(x) for x in d])
    # each card's right and up axes: with its normal they are the three
    # normals of its cluster's cards, so a card can tell, alone, whether it
    # is the one of the three most square to the view (`facing`)
    axes_a = np.repeat(np.array([f[2] for f in frames]), 4, 0)
    axes_b = np.repeat(np.array([f[3] for f in frames]), 4, 0)
    for nm, arr in (('_CARD_A', axes_a), ('_CARD_B', axes_b)):
        at = me.attributes.new(nm, 'FLOAT_VECTOR', 'POINT')
        at.data.foreach_set('vector', arr.astype(np.float32).ravel())
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = nt.nodes['Principled BSDF']
    tex = nt.nodes.new('ShaderNodeTexImage')
    tex.image = bpy.data.images.load(str(atlas_path), check_existing=True)
    nt.links.new(tex.outputs['Color'], bsdf.inputs['Base Color'])
    rnd = nt.nodes.new('ShaderNodeMath')
    rnd.operation = 'ROUND'
    nt.links.new(tex.outputs['Alpha'], rnd.inputs[0])
    nt.links.new(rnd.outputs[0], bsdf.inputs['Alpha'])
    bsdf.inputs['Roughness'].default_value = roughness
    mat.use_backface_culling = False
    mat.surface_render_method = 'DITHERED'
    me.materials.append(mat)
    o = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(o)
    return o


# --- one level ----------------------------------------------------------------

def build_level(plant, asset, level, dist):
    bpy.ops.wm.open_mainfile(filepath=str(NORM / f'{asset}.blend'))
    pngs()
    for o in [o for o in bpy.data.objects if o.type == 'MESH' and o.name != plant]:
        bpy.data.objects.remove(o)
    obj = bpy.data.objects[plant]
    e = dist * PIXEL_RAD
    m = mesh_arrays(obj, asset)
    rough = leaf_roughness(obj, asset)
    print(f'ROUGHNESS {plant} {rough:.2f}', flush=True)
    co = m['co']
    lo, hi = co.min(0), co.max(0)
    size = float((hi - lo).max())
    plant_centre = (lo + hi) / 2
    wood_f = np.nonzero(~m['leaf'])[0]
    leaf_f = np.nonzero(m['leaf'])[0]

    # wood parts thinner than a pixel
    thin_f = np.zeros(0, np.int64)
    n_twigs = n_thin = 0
    if len(wood_f):
        ids, inv, wmean, diam = part_stats(m, wood_f, islands(m, wood_f))
        thin = diam < TWIG_PX * e
        n_twigs, n_thin = len(ids), int(thin.sum())
        thin_f = wood_f[thin[inv]]
        thin_part = inv[thin[inv]]  # part of each thin face

    # leaf clusters
    face_cluster = np.full(m['nf'], -1, np.int64)
    frames = []
    if len(leaf_f):
        lids, linv, lmean, _ = part_stats(m, leaf_f, islands(m, leaf_f))
        card = CARD_PX[level] * e
        k = int(np.clip(np.prod(np.maximum((hi - lo) / card, 1.0)) ** (2 / 3), 4, MAX_CLUSTERS[level]))
        clab, cent = kmeans(lmean, k, seed=zlib.crc32(f'{plant}/{level}'.encode()))
        face_cluster[leaf_f] = clab[linv]
        if len(thin_f):
            near = np.array([((cent - wmean[p]) ** 2).sum(1).argmin() for p in range(len(wmean))])
            face_cluster[thin_f] = near[thin_part]
        # each cluster's vertices: loops sorted by cluster
        loop_cl = face_cluster[m['loop_face']]
        order = np.argsort(loop_cl, kind='stable')
        sorted_cl = loop_cl[order]
        bounds = np.searchsorted(sorted_cl, np.arange(k + 1))
        for j in range(k):
            pts = co[m['lv'][order[bounds[j]:bounds[j + 1]]]].astype(np.float64)
            if len(pts) >= 3:
                frames.extend((j, *f) for f in card_frames(pts, plant_centre))
        # the crown's centre: of the leaves, a little low (the light reaches
        # the top of a crown from above)
        crown_centre = lmean.mean(0) - np.array([0, 0, 0.15 * (hi[2] - lo[2])])
        attr = obj.data.attributes.new('cluster', 'INT', 'FACE')
        attr.data.foreach_set('value', face_cluster.astype(np.int32))
        atlas_path = OUT / f'{plant}_{level}_cards.png'
        cols = bake_cards(obj, frames, atlas_path)
        obj.data.attributes.remove(obj.data.attributes['cluster'])

    # the level's mesh: no leaves, no thin twigs, the wood collapsed
    delete_faces(obj, face_cluster >= 0)
    wood_tris = sum(len(p.vertices) - 2 for p in obj.data.polygons)
    # the wood's budget from what it covers on screen at the level's
    # distance: its surface over pi (a branch seen from any side), in
    # pixels, a triangle per WOOD_PX of them
    area = np.empty(len(obj.data.polygons), np.float32)
    obj.data.polygons.foreach_get('area', area)
    pixels = float(area.sum()) / math.pi / (e * e)
    target = int(np.clip(pixels / WOOD_PX, 200, wood_tris)) if wood_tris else 0
    if wood_tris and target < wood_tris:
        mod = obj.modifiers.new('decimate', 'DECIMATE')
        mod.decimate_type = 'COLLAPSE'
        mod.ratio = target / wood_tris
        mod.use_collapse_triangulate = True
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.modifier_apply(modifier=mod.name)
        wood_tris = sum(len(p.vertices) - 2 for p in obj.data.polygons)
    if frames:
        cards = card_object(f'{plant}_{level}_cards', frames, cols, atlas_path, crown_centre, rough)
        bpy.ops.object.select_all(action='DESELECT')
        cards.select_set(True)
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.join()
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.export_scene.gltf(filepath=str(OUT / f'{plant}_{level}.glb'), export_format='GLB', use_selection=True,
                              export_image_format='AUTO', export_attributes=True)
    # the preview: the same level, the cards drawn as the game is to draw them
    if frames:
        for sl in obj.material_slots:
            if sl.material is not None and sl.material.name.startswith(f'{plant}_{level}_cards'):
                sl.material = facing(sl.material)
    obj.name = f'{plant}_{level}'
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT / f'{plant}_{level}.blend'), compress=True)
    tris = sum(len(p.vertices) - 2 for p in obj.data.polygons)
    return dict(distance_m=dist, pixel_m=round(e, 4), tris=tris, wood_tris=wood_tris, cards=len(frames),
                twigs_dropped=n_thin, wood_parts=n_twigs, original_tris=int((m['total'] - 2).sum()))


def main():
    plants = sys.argv[sys.argv.index('--') + 1:]
    path = OUT / 'summary.json'
    summary = json.loads(path.read_text()) if path.exists() else {}
    summary = {k: v for k, v in summary.items() if 'distance_m' in v}
    for plant in plants:
        asset = plant.rsplit('_', 1)[0]
        only = [l for l in os.environ.get('LOD_LEVELS', '').split(',') if l]
        for level, dist in LEVELS.items():
            if only and level not in only:
                continue
            r = build_level(plant, asset, level, dist)
            summary[f'{plant}_{level}'] = r
            print(f'LOD {plant:24s} {level} from {dist:4.0f} m (pixel {r["pixel_m"] * 100:.1f} cm): '
                  f'{r["original_tris"]:>9,} -> {r["tris"]:>7,} tris ({r["wood_tris"]:,} wood, {r["cards"]} cards), '
                  f'wood parts to cards {r["twigs_dropped"]}/{r["wood_parts"]}', flush=True)
            path.write_text(json.dumps(summary, indent=1))


if __name__ == '__main__':
    main()
