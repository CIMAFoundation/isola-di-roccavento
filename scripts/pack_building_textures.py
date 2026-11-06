"""Pack Poly Haven 1K maps into the buildings' textures (assets/buildings/).

    python3 scripts/pack_building_textures.py <dir with the downloaded jpgs> assets/buildings

From https://polyhaven.com (CC0): painted_plaster_wall, clay_roof_tiles_02,
sandstone_blocks_05, marble_cliff_05 (`materials/masonry.rs`), and for the
houses (`materials/house.rs`, 2026-11-06) roof_slates_02, rustic_stone_wall_02,
rough_plaster_brick_02, white_rough_plaster, clay_roof_tiles_03; each
`<id>_{diff,nor_gl,arm}_1k.jpg`. With `--only <name>...` just those. Writes per material
`<name>_color.png` (RGB = diffuse x ambient occlusion, sRGB; A = roughness)
and `<name>_normal.png` (OpenGL normal), and prints the mean linear colour
that `shaders/masonry.wesl` divides by (MEAN_*).
"""
import os
import sys

from PIL import Image, ImageChops, ImageStat

src, dst = sys.argv[1], sys.argv[2]
os.makedirs(dst, exist_ok=True)
ALL = [("painted_plaster_wall", "plaster"), ("clay_roof_tiles_02", "roof"), ("sandstone_blocks_05", "stone"), ("marble_cliff_05", "rock"),
       ("roof_slates_02", "slate"), ("rustic_stone_wall_02", "rustic"), ("rough_plaster_brick_02", "brick"),
       ("white_rough_plaster", "plaster2"), ("clay_roof_tiles_03", "roof2")]
only = sys.argv[sys.argv.index("--only") + 1:] if "--only" in sys.argv else None
for ph, name in ALL:
    if only is not None and name not in only:
        continue
    diff = Image.open(f"{src}/{ph}_diff_1k.jpg").convert("RGB")
    ao, rough, _ = Image.open(f"{src}/{ph}_arm_1k.jpg").convert("RGB").split()
    # occlusion only partly: the sun's shadows do the rest
    ao = ao.point(lambda v: int(255 - (255 - v) * 0.7))
    r, g, b = [ImageChops.multiply(ch, ao) for ch in diff.split()]
    Image.merge("RGBA", (r, g, b, rough)).save(f"{dst}/{name}_color.png", optimize=True)
    Image.open(f"{src}/{ph}_nor_gl_1k.jpg").convert("RGB").save(f"{dst}/{name}_normal.png", optimize=True)
    n = r.width * r.height
    mean = [sum(((v / 255) ** 2.2) * k for v, k in enumerate(ch.histogram())) / n for ch in (r, g, b)]
    print(name, "mean linear rgb", [round(x, 4) for x in mean], "mean roughness", round(ImageStat.Stat(rough).mean[0] / 255, 3))
