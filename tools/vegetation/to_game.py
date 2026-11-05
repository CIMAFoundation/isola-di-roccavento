"""The levels of detail of the scanned plants in the game's form
(2026-11-04, docs/TODO.md «Vegetazione realistica», step 3).

    Blender --background --python tools/vegetation/to_game.py -- [plant ...]

Reads vegetation_src/lod/<plant>_<level>.blend (lod.py) and writes
assets/models/vegetation_scans.json (index), vegetation_scans.bin (vertices
and indices) and the card atlas assets/models/foliage_scans.png:

- each plant's levels as the game's models (`models::Model` fields): Bevy's
  frame (y up), the foot at the origin, one unit tall in every axis (the
  game scales by the plant's height; the model keeps its proportions);
  triangles; per vertex its normal, `wood` (the cards are leaves: they
  burn away, the wood chars), its uv into the card atlas, and a colour:
  the bark's own (linear, sampled from its texture at the vertex) on the
  wood, a neutral 0.5 on the leaves (the atlas paints them);
- each card's right and up axes (`card_a`, `card_b`, zero on the wood),
  for the choice of the card most square to the view;
- the card tiles of every level packed in one atlas, each 64 px tile of
  lod.py brought down to `TILE` px, its alpha the share of the tile's
  texels that were leaf (the game cuts at 0.5 and keeps that share in its
  mips, `vegetation::atlas`).
"""
import bpy
import json
import math
import sys
from pathlib import Path

import bmesh
import numpy as np

ROOT = Path(__file__).resolve().parents[2]
LOD = ROOT / 'vegetation_src' / 'lod'
OUT = ROOT / 'assets' / 'models'
LEVELS = ('lod0', 'lod1', 'lod2', 'lod3')
# texels per card tile in the game, by level: about what a card spans on
# screen where the game starts drawing the level (lod.CARD_PX)
TILE = {'lod0': 16, 'lod1': 16, 'lod2': 8, 'lod3': 8}
SRC_TILE = 64
ATLAS_W = 4096
# position f32x3, normal snorm8x3 and leaf u8, uv f32x2, colour (linear)
# unorm8x3 and a pad, card right and up axes snorm8x3 and a pad each
VERTEX_BYTES = 36


def srgb_to_linear(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def base_image(mat):
    """The image upstream of a material's base colour, if any."""
    if mat is None or not mat.use_nodes:
        return None
    bsdf = next((n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
    if bsdf is None:
        return None
    todo = [bsdf.inputs['Base Color']]
    seen = set()
    while todo:
        sock = todo.pop()
        for link in sock.links:
            n = link.from_node
            if n.name in seen:
                continue
            seen.add(n.name)
            if n.type == 'TEX_IMAGE' and n.image is not None:
                return n.image
            todo.extend(i for i in n.inputs if i.is_linked)
    return None


def image_array(img):
    """`img` as float RGBA rows from the top, sRGB values as stored."""
    w, h = img.size
    px = np.array(img.pixels[:], np.float32).reshape(h, w, img.channels)
    if img.channels == 3:
        px = np.concatenate([px, np.ones((h, w, 1), np.float32)], 2)
    return px[::-1]


def sample(arr, uv):
    """Nearest texel of `arr` (rows from the top) at each uv (wrapped)."""
    h, w = arr.shape[:2]
    x = (np.mod(uv[:, 0], 1.0) * w).astype(int).clip(0, w - 1)
    y = ((1 - np.mod(uv[:, 1], 1.0)) * h).astype(int).clip(0, h - 1)
    return arr[y, x]


def tiles_down(atlas, tile):
    """The level's 64 px card tiles brought down to `tile` px: colour
    averaged over the leaf texels, alpha the share of leaf texels."""
    n = atlas.shape[0] // SRC_TILE
    k = SRC_TILE // tile
    a = atlas[..., 3] > 0.5
    lin = srgb_to_linear(atlas[..., :3])
    side = n * tile
    blocks_a = a.reshape(side, k, side, k).mean((1, 3))
    w = a.astype(np.float32)[..., None]
    num = (lin * w).reshape(side, k, side, k, 3).sum((1, 3))
    den = w.reshape(side, k, side, k, 1).sum((1, 3))
    # where no leaf: the colour of all the block (the dilated colour)
    all_rgb = lin.reshape(side, k, side, k, 3).mean((1, 3))
    rgb = np.where(den > 0, num / np.maximum(den, 1e-6), all_rgb)
    srgb = np.where(rgb <= 0.0031308, rgb * 12.92, 1.055 * np.power(np.clip(rgb, 0, 1), 1 / 2.4) - 0.055)
    # coverage kept: the game cuts at 0.5, so a sparse tile (leaves in
    # every block, none half full) would vanish. Of each tile, as many
    # texels as its leaf share asks pass the cut, the fullest first; the
    # others stay below it, in their order (the mips average them)
    alpha = blocks_a.copy()
    t = tile * tile
    for r in range(n):
        for c in range(n):
            blk = blocks_a[r * tile:(r + 1) * tile, c * tile:(c + 1) * tile].ravel()
            keep = int(round(blk.sum()))
            order = np.argsort(-blk, kind='stable')
            out = np.minimum(blk, 0.49)
            out[order[:keep]] = np.maximum(blk[order[:keep]], 0.5)
            alpha[r * tile:(r + 1) * tile, c * tile:(c + 1) * tile] = out.reshape(tile, tile)
    return np.concatenate([srgb, alpha[..., None]], 2).astype(np.float32)


def level_mesh(plant, level):
    path = LOD / f'{plant}_{level}.blend'
    bpy.ops.wm.open_mainfile(filepath=str(path))
    obj = bpy.data.objects[f'{plant}_{level}']
    me = obj.data
    bm = bmesh.new()
    bm.from_mesh(me)
    bmesh.ops.triangulate(bm, faces=bm.faces[:])
    bm.to_mesh(me)
    bm.free()
    me.update()
    nl = len(me.loops)
    nf = len(me.polygons)
    lv = np.empty(nl, np.int32)
    me.loops.foreach_get('vertex_index', lv)
    co = np.empty(len(me.vertices) * 3, np.float32)
    me.vertices.foreach_get('co', co)
    co = co.reshape(-1, 3)
    nrm = np.empty(nl * 3, np.float32)
    me.corner_normals.foreach_get('vector', nrm)
    nrm = nrm.reshape(-1, 3)
    uv = np.zeros((nl, 2), np.float32)
    if me.uv_layers.active is not None:
        flat = np.empty(nl * 2, np.float32)
        me.uv_layers.active.data.foreach_get('uv', flat)
        uv = flat.reshape(-1, 2)
    mat = np.empty(nf, np.int32)
    me.polygons.foreach_get('material_index', mat)
    loop_face = np.repeat(np.arange(nf), 3)
    slots = obj.material_slots
    is_card = np.array([bool(s.material and '_cards' in s.material.name) for s in slots] or [False])
    card = is_card[np.clip(mat, 0, len(is_card) - 1)][loop_face]
    # the bark's colour at each wood corner
    colour = np.full((nl, 3), 0.5, np.float32)
    for k, s in enumerate(slots):
        if is_card[k]:
            continue
        img = base_image(s.material)
        sel = (mat[loop_face] == k)
        if img is None or not sel.any():
            continue
        colour[sel] = srgb_to_linear(sample(image_array(img), uv[sel])[:, :3])
    # each card's right and up axes (lod.card_object): the game shows, of
    # a cluster's three cards, the one most square to the view
    axes = []
    for name in ('_CARD_A', '_CARD_B'):
        at = me.attributes.get(name)
        v = np.zeros((len(me.vertices), 3), np.float32)
        if at is not None and at.domain == 'POINT':
            flat = np.empty(len(me.vertices) * 3, np.float32)
            at.data.foreach_get('vector', flat)
            v = flat.reshape(-1, 3)
        v = v[lv]
        axes.append(np.stack([v[:, 0], v[:, 2], -v[:, 1]], 1) * card[:, None])
    # Blender z up -> Bevy y up, Blender y -> Bevy -z; one unit tall
    p = co[lv]
    # every level of a plant by the original's height (a level's cards
    # may stand above it)
    height = json.loads((ROOT / 'vegetation_src' / 'normalized' / 'summary.json').read_text())[plant]['height_m']
    pos = np.stack([p[:, 0], p[:, 2], -p[:, 1]], 1) / max(height, 1e-6)
    n = np.stack([nrm[:, 0], nrm[:, 2], -nrm[:, 1]], 1)
    # the card atlas of the level, as a square block of tiles
    atlas = None
    leaf_rgb = [0.5, 0.5, 0.5]
    cards_png = LOD / f'{plant}_{level}_cards.png'
    if card.any() and cards_png.exists():
        img = bpy.data.images.load(str(cards_png), check_existing=False)
        src = image_array(img)
        # the mean colour of the leaves (linear): the game brings it to its
        # own palette's
        a = src[..., 3] > 0.5
        if a.any():
            leaf_rgb = [round(float(c), 4) for c in srgb_to_linear(src[..., :3])[a].mean(0)]
        atlas = tiles_down(src, TILE[level])
    # weld identical corners
    key = np.concatenate([np.round(pos, 5), np.round(n, 3), np.round(uv, 5), np.round(colour, 3), card[:, None],
                          np.round(axes[0], 3), np.round(axes[1], 3)], 1)
    uniq, inv = np.unique(key, axis=0, return_inverse=True)
    first = np.zeros(len(uniq), np.int64)
    first[inv[::-1]] = np.arange(nl)[::-1]
    return dict(pos=pos[first], normal=n[first], uv=uv[first], colour=colour[first], leaf=card[first],
                card_a=axes[0][first], card_b=axes[1][first],
                indices=inv.astype(np.int64).ravel(), atlas=atlas, height_m=height, leaf_rgb=leaf_rgb)


def main():
    args = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    plants = args or sorted({p.stem.rsplit('_', 1)[0] for p in LOD.glob('*_lod3.blend')})
    built = {}
    blocks = []
    for plant in plants:
        built[plant] = {}
        for level in LEVELS:
            if not (LOD / f'{plant}_{level}.blend').exists():
                print(f'GAME {plant:24s} {level}: missing', flush=True)
                continue
            m = level_mesh(plant, level)
            built[plant][level] = m
            if m['atlas'] is not None:
                blocks.append((plant, level, m['atlas']))
            nv, nt = len(m['pos']), len(m['indices']) // 3
            assert nv < 65536, f'{plant} {level}: {nv} vertices'
            print(f'GAME {plant:24s} {level}: {nt:>7,} tris {nv:>7,} verts, {m["height_m"]:.2f} m', flush=True)
    # shelf packing of the level atlases, tallest first
    blocks.sort(key=lambda b: -b[2].shape[0])
    x = y = shelf = 0
    place = {}
    for plant, level, a in blocks:
        s = a.shape[0]
        if x + s > ATLAS_W:
            x, y, shelf = 0, y + shelf, 0
        place[(plant, level)] = (x, y, s)
        x += s
        shelf = max(shelf, s)
    height = 1 << math.ceil(math.log2(max(y + shelf, 1)))
    atlas = np.zeros((height, ATLAS_W, 4), np.float32)
    for plant, level, a in blocks:
        bx, by, s = place[(plant, level)]
        atlas[by:by + s, bx:bx + s] = a
    # the index (json) and the vertex and index data (bin), little-endian:
    # per vertex VERTEX_BYTES (see `models::scans`), then the indices (u16)
    out = {'atlas': {'width': ATLAS_W, 'height': height}, 'vertex_bytes': VERTEX_BYTES, 'plants': {}}
    blob = bytearray()
    snorm = lambda v: np.clip(np.round(v * 127), -127, 127).astype(np.int8)
    for plant, levels in built.items():
        out['plants'][plant] = {}
        for level, m in levels.items():
            uv = m['uv'].copy()
            leaf = m['leaf']
            if (plant, level) in place:
                bx, by, s = place[(plant, level)]
                # uv (v up) within the level's block -> the whole atlas (v up)
                u = (bx + uv[leaf, 0] * s) / ATLAS_W
                v = 1 - (by + (1 - uv[leaf, 1]) * s) / height
                uv[leaf, 0], uv[leaf, 1] = u, v
            nv = len(m['pos'])
            rec = np.zeros(nv, dtype=[('pos', '<f4', 3), ('normal', 'i1', 3), ('leaf', 'u1'), ('uv', '<f4', 2),
                                      ('colour', 'u1', 3), ('pad', 'u1'), ('card_a', 'i1', 3), ('pad_a', 'u1'),
                                      ('card_b', 'i1', 3), ('pad_b', 'u1')])
            assert rec.itemsize == VERTEX_BYTES
            nrm = m['normal'] / np.maximum(np.linalg.norm(m['normal'], axis=1, keepdims=True), 1e-6)
            rec['pos'] = m['pos']
            rec['normal'] = snorm(nrm)
            rec['leaf'] = leaf.astype(np.uint8)
            # the game's uvs count v from the top (Bevy)
            rec['uv'] = np.stack([uv[:, 0], 1 - uv[:, 1]], 1)
            rec['colour'] = np.clip(np.round(m['colour'] * 255), 0, 255).astype(np.uint8)
            rec['card_a'] = snorm(m['card_a'])
            rec['card_b'] = snorm(m['card_b'])
            offset = len(blob)
            blob += rec.tobytes()
            blob += m['indices'].astype('<u2').tobytes()
            while len(blob) % 4:
                blob.append(0)
            out['plants'][plant][level] = {'height_m': m['height_m'], 'leaf_rgb': m['leaf_rgb'], 'offset': offset, 'vertices': nv,
                                           'indices': int(len(m['indices']))}
    (OUT / 'vegetation_scans.json').write_text(json.dumps(out, indent=1) + '\n')
    (OUT / 'vegetation_scans.bin').write_bytes(bytes(blob))
    img = bpy.data.images.new('foliage_scans', ATLAS_W, height, alpha=True)
    img.pixels = atlas[::-1].ravel()
    img.filepath_raw = str(OUT / 'foliage_scans.png')
    img.file_format = 'PNG'
    img.save()
    print(f'ATLAS {ATLAS_W}x{height}, {len(blocks)} blocks; data {len(blob) / 1e6:.1f} MB', flush=True)


if __name__ == '__main__':
    main()
