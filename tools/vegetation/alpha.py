"""The leaves' cut-out (2026-11-04). Poly Haven ships it as a map of its
own (`Alpha`, `leaves_alpha`, `twig_alpha`), which its glTF does not
reference: imported as is, leaf cards are opaque squares. `wire(asset)`
plugs the map into each foliage material's alpha through a Math «Round»,
which Blender's glTF exporter writes as alphaMode MASK (cutoff 0.5).
"""
from pathlib import Path

import bpy

SRC = Path(__file__).resolve().parents[2] / 'vegetation_src' / 'polyhaven'


# a lower cut-out for the needles, a few texels wide in the 1k atlas
CUTOFF = {'pine_tree_01': .2, 'fir_sapling_medium': .2}


def maps(asset):
    """material-name test -> alpha map path"""
    out = []
    for p in sorted((SRC / asset / 'textures').glob(f'{asset}_*alpha*.png')):
        key = p.stem[len(asset) + 1:].rsplit('_', 1)[0]  # 'alpha', 'leaves_alpha', 'twig_alpha'
        part = key[:-len('_alpha')] if key.endswith('_alpha') else ''
        out.append((part, p))
    return out


def base_image(m):
    bsdf = next((n for n in m.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
    links = bsdf.inputs['Base Color'].links if bsdf else []
    n = links[0].from_node if links else None
    return n.image.name if n is not None and n.type == 'TEX_IMAGE' and n.image else ''


def wire(asset):
    found = maps(asset)
    wired = []
    for m in bpy.data.materials:
        if not m.use_nodes:
            continue
        name = m.name.lower()
        for part, path in found:
            # 'leaves' -> <asset>_leaves; 'twig' -> <asset>_twig; '' -> every
            # material painted from the asset's one atlas (<asset>_diff...)
            if (part and name.endswith('_' + part)) or (not part and base_image(m).startswith(f'{asset}_diff')):
                nodes, links = m.node_tree.nodes, m.node_tree.links
                bsdf = next(n for n in nodes if n.type == 'BSDF_PRINCIPLED')
                img = nodes.new('ShaderNodeTexImage')
                img.image = bpy.data.images.load(str(path), check_existing=True)
                img.image.colorspace_settings.name = 'Non-Color'
                # the same uv as the colour
                colour = next((n for n in nodes if n.type == 'TEX_IMAGE' and n.outputs['Color'].is_linked
                               and any(l.to_socket == bsdf.inputs['Base Color'] for l in n.outputs['Color'].links)), None)
                if colour and colour.inputs['Vector'].is_linked:
                    links.new(colour.inputs['Vector'].links[0].from_socket, img.inputs['Vector'])
                cut = CUTOFF.get(asset, .5)
                if cut == .5:
                    rnd = nodes.new('ShaderNodeMath')
                    rnd.operation = 'ROUND'
                    links.new(img.outputs['Color'], rnd.inputs[0])
                    links.new(rnd.outputs[0], bsdf.inputs['Alpha'])
                else:
                    # 1 - (alpha < cut): the exporter's other clip pattern
                    lt = nodes.new('ShaderNodeMath')
                    lt.operation = 'LESS_THAN'
                    lt.inputs[1].default_value = cut
                    links.new(img.outputs['Color'], lt.inputs[0])
                    sub = nodes.new('ShaderNodeMath')
                    sub.operation = 'SUBTRACT'
                    sub.inputs[0].default_value = 1.0
                    links.new(lt.outputs[0], sub.inputs[1])
                    links.new(sub.outputs[0], bsdf.inputs['Alpha'])
                m.blend_method = 'CLIP' if hasattr(m, 'blend_method') else m.blend_method
                if hasattr(m, 'surface_render_method'):
                    m.surface_render_method = 'DITHERED'
                wired.append(m.name)
    print('ALPHA', asset, wired, flush=True)
    return wired
