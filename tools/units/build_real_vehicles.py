"""Run with: Blender --background --python tools/units/build_real_vehicles.py

The realistic style (`KIOSK_SCANS=1`, `models::scans()`): the teams' AIB
engines and the boats of the port and the open sea, in place of the toy-like
models of `scripts/build_unit_models.py` and `scripts/build_island_models.py`.
Modelled here from primitives (no downloaded meshes: Poly Haven has no modern
vehicles or boats, Kenney and Quaternius are toy-styled). Generic liveries:
no real emblem, logo or plate.

Bakes `assets/models/vehicles_real.json` in the layout of `units.json`
(positions, normals, linear colours, indices, surfaces per vertex, rigs for
the animated parts), welded and indexed, and saves the source in
`assets/models/vehicles_real_assets.blend`. `REAL_PREVIEW=<dir>` renders
close-ups there.

Engines, three bodies of the same truck (a 4x4 crew-cab AIB tender), told
apart from above by the team letter on the cab roof:
  engine_rosso   red cab and closed red body, white bands
  engine_bianco  white cab with a red band, red body
  engine_modulo  red cab, flat bed with a stainless tank and the pump module
Each has the parts `wheel_fl/fr/rl/rr` (wheel) and `beacon_l/r` (blink), and
`letter_<A..H>`: the roof plates, drawn as children.

Boats (bow +X, waterline at z = 0.1 as the island's sloops):
  gozzo_<n>, peschereccio_<n>, gommone_<n>, motoscafo_<n>, vela_<n> (furled),
  vela_<n>_vele (under sail). Three colourways each, with their own hull number.

Blender: metres, Z up, front -Y (vehicles) / bow +X (boats). Runtime: Y up.
"""
import json
import math
import os
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector, Matrix

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "assets/models"

bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)

# absolute sRGB; baked linear
COLOURS = {
    "red": (.72, .04, .035), "red_dark": (.42, .03, .03), "white": (.92, .93, .94), "offwhite": (.86, .86, .83),
    "dark": (.04, .042, .05), "chassis": (.07, .07, .075), "metal": (.55, .57, .6), "alu": (.74, .76, .79),
    "inox": (.80, .82, .84), "glass": (.06, .09, .12), "light": (.98, .95, .80), "beacon": (.08, .30, 1.0),
    "tail": (.85, .06, .04), "amber": (1.0, .55, .06), "yellow": (.97, .80, .06), "tyre": (.05, .05, .055),
    "rim": (.78, .79, .80), "grille": (.06, .065, .08), "plate": (.95, .95, .92), "black": (.02, .02, .025),
    "hose": (.80, .10, .06), "reflect": (.95, .95, .90), "reflect_red": (.90, .08, .06),
    # boats
    "teak": (.55, .40, .24), "teak_d": (.36, .25, .15), "antifoul": (.45, .10, .07), "antifoul_b": (.10, .14, .26),
    "canvas": (.93, .92, .88), "sail": (.96, .95, .92), "navy": (.06, .12, .30), "blue": (.10, .32, .62),
    "sky": (.38, .62, .80), "green": (.10, .40, .26), "teal": (.05, .45, .48), "ochre": (.82, .60, .22),
    "brick": (.62, .20, .12), "grey": (.42, .44, .46), "orange": (.95, .38, .05), "cream": (.90, .86, .74),
    "rope": (.80, .74, .58), "fender": (.08, .14, .30), "net": (.20, .45, .30), "yellow_b": (.95, .75, .10),
}
SURFACES = {"paint": 1, "metal": 2, "rubber": 3, "glass": 4, "lamp": 5, "cloth": 7, "reflective": 8, "wood": 10}
TONE_SURFACE = {
    "red": "paint", "red_dark": "paint", "white": "paint", "offwhite": "paint", "yellow": "paint", "plate": "paint",
    "dark": "rubber", "chassis": "paint", "tyre": "rubber", "black": "rubber", "hose": "rubber", "fender": "rubber",
    "metal": "metal", "alu": "metal", "rim": "metal", "inox": "metal", "grille": "metal",
    "glass": "glass", "light": "lamp", "beacon": "lamp", "tail": "lamp", "amber": "lamp",
    "reflect": "reflective", "reflect_red": "reflective",
    "teak": "wood", "teak_d": "wood", "antifoul": "paint", "antifoul_b": "paint", "canvas": "cloth", "sail": "cloth",
    "navy": "paint", "blue": "paint", "sky": "paint", "green": "paint", "teal": "paint", "ochre": "paint",
    "brick": "paint", "grey": "rubber", "orange": "rubber", "cream": "paint", "rope": "cloth", "net": "cloth",
    "yellow_b": "paint",
}
SHARP = math.radians(48)
materials = {}
current = None
PART = None
RIGS = {}


def srgb_to_linear(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def mat(tone):
    if tone not in materials:
        rgb = COLOURS[tone]
        m = bpy.data.materials.new(tone)
        m.diffuse_color = (*[srgb_to_linear(c) for c in rgb], 1)
        m["srgb"] = list(rgb)
        m["surface"] = SURFACES.get(TONE_SURFACE.get(tone), 0)
        materials[tone] = m
    return materials[tone]


class part:
    """Everything built inside is one animated part about `pivot`."""

    def __init__(self, name, pivot, axis, kind, phase=0.0):
        self.args = (name, pivot, axis, kind, phase)

    def __enter__(self):
        global PART
        name, pivot, axis, kind, phase = self.args
        PART = name
        RIGS.setdefault(current.name, {})[name] = dict(pivot=pivot, axis=axis, kind=kind, phase=phase)

    def __exit__(self, *a):
        global PART
        PART = None


def activate(o):
    for s in bpy.context.selected_objects:
        s.select_set(False)
    bpy.context.view_layer.objects.active = o
    o.select_set(True)


def bevelled(o, width, seg):
    activate(o)
    mod = o.modifiers.new("bevel", "BEVEL")
    mod.width = width
    mod.segments = seg
    mod.limit_method = "ANGLE"
    bpy.ops.object.modifier_apply(modifier=mod.name)


def finish(o, name, tone, smooth_all=False):
    o.name = name
    for c in list(o.users_collection):
        c.objects.unlink(o)
    current.objects.link(o)
    me = o.data
    bm = bmesh.new()
    bm.from_mesh(me)
    for e in bm.edges:
        e.smooth = smooth_all or not (len(e.link_faces) == 2 and e.calc_face_angle(0.0) > SHARP)
    for f in bm.faces:
        f.smooth = True
    bm.to_mesh(me)
    bm.free()
    activate(o)
    wn = o.modifiers.new("wn", "WEIGHTED_NORMAL")
    wn.keep_sharp = True
    bpy.ops.object.modifier_apply(modifier=wn.name)
    me.materials.clear()
    me.materials.append(mat(tone))
    o["part"] = PART or ""
    return o


def box(name, pos, size, tone, bevel=0.0, seg=1, rot=None):
    bpy.ops.mesh.primitive_cube_add(size=1, location=pos)
    o = bpy.context.object
    o.scale = size
    if rot:
        o.rotation_euler = rot
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
    if bevel and min(size) >= .06:
        bevelled(o, min(bevel, min(size) * .4), seg)
    return finish(o, name, tone)


def rod(name, a, b, r1, tone, r2=None, sides=8, caps=True):
    a, b = Vector(a), Vector(b)
    bpy.ops.mesh.primitive_cone_add(vertices=sides, radius1=r1, radius2=r1 if r2 is None else r2,
                                    depth=(b - a).length, location=(a + b) / 2,
                                    end_fill_type="NGON" if caps else "NOTHING")
    o = bpy.context.object
    o.rotation_euler = (b - a).to_track_quat("Z", "Y").to_euler()
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=False)
    return finish(o, name, tone)


def tube(name, pts, r, tone, sides=6):
    """A pipe through the points (one rod per segment, joints hidden by spheres)."""
    for a, b in zip(pts[:-1], pts[1:]):
        rod(name, a, b, r, tone, sides=sides, caps=False)
    for p in pts[1:-1]:
        ico(name, p, (r, r, r), tone, 1)


def ico(name, pos, size, tone, sub=2):
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=sub, radius=1, location=pos)
    o = bpy.context.object
    o.scale = size
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return finish(o, name, tone, smooth_all=True)


def solid(name, verts, faces, tone, bevel=0.0, seg=2, outward=True, smooth_all=False):
    me = bpy.data.meshes.new(name)
    me.from_pydata([tuple(v) for v in verts], [], faces)
    me.update()
    if outward:
        bm = bmesh.new()
        bm.from_mesh(me)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        bm.to_mesh(me)
        bm.free()
    o = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(o)
    if bevel:
        bevelled(o, bevel, seg)
    return finish(o, name, tone, smooth_all)


def extrude(name, pts, axis, a, b, tone, bevel=0.0, seg=2):
    """A 2D profile pushed along an axis. 'x': profile in (y, z); 'y': in (x, z); 'z': in (x, y)."""
    n = len(pts)
    verts = []
    for c in (a, b):
        for u, v in pts:
            verts.append((c, u, v) if axis == "x" else (u, c, v) if axis == "y" else (u, v, c))
    faces = [list(range(n)), list(range(2 * n - 1, n - 1, -1))]
    for i in range(n):
        j = (i + 1) % n
        faces.append([i, j, n + j, n + i])
    return solid(name, verts, faces, tone, bevel, seg)


def text(name, s, pos, size, tone, rot=(0, 0, 0), depth=.012, bold=.0):
    """Letters as geometry (Blender's own font), centred on `pos`; `rot` turns
    them from the XY plane (reading +X, up +Y)."""
    cu = bpy.data.curves.new(name, "FONT")
    cu.body = s
    cu.size = size
    cu.extrude = depth
    cu.offset = bold * size
    cu.align_x = "CENTER"
    cu.align_y = "CENTER"
    cu.resolution_u = 3
    o = bpy.data.objects.new(name, cu)
    bpy.context.scene.collection.objects.link(o)
    o.location = pos
    o.rotation_euler = rot
    bpy.context.view_layer.update()
    dg = bpy.context.evaluated_depsgraph_get()
    me = bpy.data.meshes.new_from_object(o.evaluated_get(dg))
    me.transform(o.matrix_world)
    bpy.data.objects.remove(o)
    m = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(m)
    return finish(m, name, tone)


# =====================================================================================
#  the AIB engine: a 4x4 crew-cab tender (front -Y, origin on the ground)
# =====================================================================================
WR = .56
FRONT_Y, REAR_Y, TRACK = -2.05, 1.75, 1.04


def wheel(s, cx, y):
    out = lambda d: cx + s * d
    rod("Tyre", (out(-.19), y, WR), (out(.19), y, WR), WR, "tyre", sides=20)
    rod("Tyre wall", (out(.19), y, WR), (out(.2), y, WR), WR * .9, "tyre", sides=20)
    # tread lugs, staggered: what reads as an off-road tyre and shows it turning
    for k in range(16):
        a = k * 2 * math.pi / 16
        off = .1 if k % 2 else -.1
        c = (out(off), y + math.cos(a) * (WR + .015), WR + math.sin(a) * (WR + .015))
        box("Lug", c, (.17, .12, .05), "tyre", rot=(-a + math.pi / 2, 0, 0))
    rod("Rim", (out(.19), y, WR), (out(.215), y, WR), .33, "rim", sides=14)
    rod("Rim dish", (out(.2), y, WR), (out(.22), y, WR), .23, "chassis", sides=12)
    rod("Hub", (out(.2), y, WR), (out(.27), y, WR), .12, "metal", r2=.09, sides=10)
    for k in range(6):
        a = k * math.pi / 3
        rod("Nut", (out(.22), y + math.cos(a) * .17, WR + math.sin(a) * .17),
            (out(.24), y + math.cos(a) * .17, WR + math.sin(a) * .17), .025, "metal", sides=6)


def cab(paint, band):
    # crew cab, raked windscreen, high on the 4x4 chassis
    prof = [(-3.28, 1.12), (-3.30, 2.02), (-3.12, 2.92), (-2.96, 3.06), (-.78, 3.06), (-.74, 1.12)]
    extrude("Cab", prof, "x", -1.2, 1.2, paint, .09, 2)
    rake = math.atan2(.18, .90)
    for nm, sz, dy, tone in (("Windscreen frame", (2.18, .03, .9), 0, "black"),
                             ("Windscreen", (2.06, .03, .78), -.008, "glass")):
        g = box(nm, (0, -3.215 + dy, 2.47), sz, tone, .01)
        g.rotation_euler = (-rake, 0, 0)
    box("Wiper", (-.4, -3.33, 2.06), (.7, .02, .025), "black", rot=(0, .25, 0))
    box("Wiper", (.4, -3.33, 2.06), (.7, .02, .025), "black", rot=(0, .25, 0))
    box("Sun visor", (0, -3.06, 3.0), (2.25, .3, .04), "black", .015)
    box("Grille", (0, -3.32, 1.62), (1.5, .04, .52), "grille", .015)
    for z in (1.42, 1.54, 1.66, 1.78):
        box("Grille slat", (0, -3.345, z), (1.4, .02, .045), "black")
    for s in (-1, 1):
        box("Headlamp", (s * .86, -3.33, 1.38), (.36, .05, .2), "light", .03)
        box("Headlamp bezel", (s * .86, -3.315, 1.38), (.44, .03, .27), "black", .02)
        box("Indicator", (s * .86, -3.33, 1.2), (.32, .04, .08), "amber", .015)
    box("Bumper", (0, -3.45, .98), (2.42, .3, .32), "chassis", .05)
    box("Bumper top", (0, -3.45, 1.15), (2.38, .26, .03), "metal")
    box("Number plate", (0, -3.61, .98), (.52, .02, .11), "plate")
    for s in (-1, 1):
        box("Fog lamp", (s * .78, -3.61, .92), (.18, .03, .1), "light", .02)
    # protection frame and winch: what an AIB truck has and a city engine has not
    for s in (-1, 1):
        tube("Bull bar", [(s * 1.05, -3.42, 1.12), (s * 1.05, -3.78, 1.25), (s * 1.0, -3.78, 2.0),
                          (s * .78, -3.46, 2.1)], .045, "chassis")
        box("Brush guard post", (s * .45, -3.72, 1.6), (.06, .06, .78), "chassis")
    rod("Bull bar top", (-1.0, -3.78, 2.0), (1.0, -3.78, 2.0), .045, "chassis", sides=6, caps=False)
    rod("Bull bar low", (-1.05, -3.78, 1.25), (1.05, -3.78, 1.25), .045, "chassis", sides=6, caps=False)
    box("Winch", (0, -3.66, 1.0), (.62, .22, .24), "chassis", .03)
    rod("Winch drum", (-.22, -3.73, 1.0), (.22, -3.73, 1.0), .085, "metal", sides=10)
    box("Winch hook", (0, -3.82, .93), (.06, .06, .1), "red_dark")
    for s in (-1, 1):
        # four doors: two rows of windows, door lines, handles, a band
        for y0, y1 in ((-3.0, -2.08), (-1.98, -.98)):
            yc, yl = (y0 + y1) / 2, y1 - y0
            box("Window frame", (s * 1.205, yc, 2.53), (.02, yl, .68), "black", .01)
            box("Side window", (s * 1.212, yc, 2.53), (.02, yl - .1, .58), "glass")
            box("Door handle", (s * 1.215, y1 - .18, 2.05), (.03, .16, .04), "black")
        for y in (-3.06, -2.03, -.9):
            box("Door line", (s * 1.204, y, 1.75), (.02, .022, 1.26), "black")
        box("Door sill", (s * 1.204, -1.96, 1.16), (.02, 2.2, .02), "black")
        box("Cab band", (s * 1.206, -1.98, 1.72), (.02, 2.38, .3), band)
        box("Cab band edge", (s * 1.207, -1.98, 1.56), (.02, 2.38, .045), "reflect")
        text("AIB", "AIB", (s * 1.215, -2.55, 1.73), .26, "white" if band != "white" else "red",
             rot=(math.pi / 2, 0, s * math.pi / 2), bold=.04)
        # steps up to the high cab
        for z, yc in ((.62, -2.55), (.62, -1.5), (.92, -2.55), (.92, -1.5)):
            box("Cab step", (s * 1.12, yc, z), (.18, .55, .05), "alu", .01)
        box("Step hanger", (s * 1.12, -2.03, .78), (.05, 1.6, .36), "chassis")
        rod("Mirror arm", (s * 1.2, -3.0, 2.5), (s * 1.48, -3.08, 2.55), .02, "black", sides=6)
        box("Mirror", (s * 1.52, -3.08, 2.45), (.06, .2, .46), "black", .02)
        box("Mirror glass", (s * 1.496, -3.08, 2.45), (.01, .16, .4), "glass")
        rod("Grab handle", (s * 1.23, -3.05, 1.5), (s * 1.23, -3.05, 2.3), .02, "alu", sides=5)
    # roof: light bar with the beacons, an antenna, the roof plate (separate part)
    box("Light bar", (0, -2.75, 3.12), (1.8, .26, .1), "black", .03)
    for s, nm in ((-1, "l"), (1, "r")):
        with part(f"beacon_{nm}", (s * .42, -2.75, 3.2), (0, 0, 1), "blink", 0.0 if s < 0 else 1.0):
            box("Beacon", (s * .42, -2.75, 3.22), (.62, .22, .14), "beacon", .05)
    for s in (-1, 1):
        box("Corner light", (s * .9, -2.75, 3.2), (.2, .22, .1), "beacon", .03)
    rod("Antenna", (.75, -1.1, 3.06), (.75, -1.1, 3.9), .012, "black", sides=4)


def chassis():
    box("Frame", (0, -.2, .92), (1.0, 7.0, .22), "chassis", .03)
    for y in (FRONT_Y, REAR_Y):
        box("Axle", (0, y, WR), (2.0, .18, .18), "chassis")
        box("Diff", (0, y, WR - .02), (.42, .42, .34), "chassis", .08)
        for s in (-1, 1):
            box("Leaf spring", (s * .55, y, WR + .22), (.12, 1.2, .08), "chassis")
    box("Fuel tank", (-.98, -.3, .95), (.36, .9, .42), "metal", .05)
    box("Tool box", (.98, -.3, .95), (.36, .9, .42), "chassis", .03)
    rod("Exhaust", (1.05, -.55, 1.2), (1.05, -.55, 3.25), .055, "metal", sides=8)
    for y, end in ((FRONT_Y, "f"), (REAR_Y, "r")):
        for s, side in ((-1, "l"), (1, "r")):
            cx = s * TRACK
            with part(f"wheel_{end}{side}", (cx, y, WR), (1, 0, 0), "wheel"):
                wheel(s, cx, y)
            # mudguards over the wheels
            pts = [(y + math.cos(a) * (WR + .14), WR + .05 + math.sin(a) * (WR + .14))
                   for a in [math.radians(d) for d in (10, 45, 90, 135, 170)]]
            pts += [(y + math.cos(a) * (WR + .07), WR + .05 + math.sin(a) * (WR + .07))
                    for a in [math.radians(d) for d in (170, 135, 90, 45, 10)]]
            extrude("Mudguard", pts, "x", cx - .26, cx + .26, "black")


def closed_body(paint, band):
    y0, y1 = -.55, 3.25
    yc, yl = (y0 + y1) / 2, y1 - y0
    box("Body", (0, yc, 2.02), (2.42, yl, 1.72), paint, .08, 2)
    box("Subframe", (0, yc, 1.1), (2.3, yl, .14), "chassis")
    comps = [(-.1, .82), (.82, .9), (1.8, .9), (2.75, .78)]
    for s in (-1, 1):
        for cy, cl in comps:
            box("Locker frame", (s * 1.212, cy, 1.88), (.02, cl + .06, 1.12), "black")
            box("Roller shutter", (s * 1.222, cy, 1.88), (.02, cl, 1.04), "alu")
            for k in range(7):
                box("Shutter rib", (s * 1.232, cy, 1.42 + k * .15), (.015, cl * .96, .018), "metal")
            box("Shutter bar", (s * 1.24, cy, 1.4), (.03, cl * .9, .045), "black")
        box("Body band", (s * 1.214, yc, 2.6), (.02, yl - .1, .2), band)
        box("Reflective band", (s * 1.216, yc, 2.48), (.02, yl - .1, .05), "reflect")
        for k in range(5):  # red and white blocks along the sill, as on real tenders
            box("Sill block", (s * 1.215, y0 + .4 + k * .72, 1.24), (.02, .36, .1), "reflect_red" if k % 2 else "reflect")
        text("AIB", "AIB", (s * 1.226, 1.3, 2.81), .3, "white" if band != "white" else "red",
             rot=(math.pi / 2, 0, s * math.pi / 2), bold=.04)
    # tank top: a walkway with a rail, the manhole, a hose basket
    box("Roof walkway", (0, yc, 2.9), (2.2, yl - .2, .04), "alu")
    for s in (-1, 1):
        rod("Roof rail", (s * 1.08, y0 + .1, 3.12), (s * 1.08, y1 - .1, 3.12), .025, "alu", sides=5)
        for y in (y0 + .2, yc, y1 - .2):
            rod("Rail post", (s * 1.08, y, 2.9), (s * 1.08, y, 3.12), .02, "alu", sides=4)
    rod("Manhole", (.4, 1.9, 2.92), (.4, 1.9, 2.99), .25, "metal", sides=14)
    box("Hose basket", (-.35, .6, 3.02), (1.1, 1.6, .2), "alu", .02)
    for k in range(3):
        rod("Hose coil", (-.35 - .3 + k * .3, .1, 3.08), (-.35 - .3 + k * .3, 1.1, 3.08), .09, "hose", sides=8)
    rear(paint, y1)


def rear(paint, y1):
    # rear: hose reel in its bay, pump panel, lamps, chevrons
    box("Rear bay", (0, y1 + .01, 1.95), (1.7, .04, 1.3), "chassis")
    rod("Hose reel", (-.62, y1 + .2, 2.1), (.62, y1 + .2, 2.1), .3, "hose", sides=16)
    for s in (-1, 1):
        rod("Reel flange", (s * .62, y1 + .2, 2.1), (s * .67, y1 + .2, 2.1), .42, "metal", sides=16)
    box("Pump panel", (0, y1 + .05, 1.45), (1.5, .08, .38), "alu", .02)
    for k in range(4):
        rod("Gauge", (-.5 + k * .33, y1 + .09, 1.5), (-.5 + k * .33, y1 + .11, 1.5), .06, "white", sides=10)
    for s in (-1, 1):
        box("Tail lamp", (s * .98, y1 + .03, 1.32), (.22, .04, .36), "tail", .015)
        box("Indicator", (s * .98, y1 + .03, 1.08), (.22, .04, .1), "amber", .015)
        rod("Ladder rail", (s * .25, y1 + .06, .72), (s * .25, y1 + .06, 2.7), .022, "alu", sides=5)
    for k in range(5):
        box("Ladder rung", (0, y1 + .06, .95 + k * .4), (.5, .03, .03), "alu")
    box("Rear bumper", (0, y1 + .05, .88), (2.3, .16, .2), "chassis", .03)
    for i in range(8):
        box("Chevron", (-1.05 + i * .3, y1 + .14, .88), (.24, .02, .17), "reflect_red" if i % 2 else "reflect")
    box("Rear plate", (0, y1 + .14, 1.08), (.5, .02, .11), "plate")


def modulo_body():
    # flat bed with drop sides; on it the skid unit: stainless tank, red pump
    # module with the hose reel; a roll bar behind the cab
    y0, y1 = -.55, 3.25
    yc, yl = (y0 + y1) / 2, y1 - y0
    box("Bed", (0, yc, 1.18), (2.4, yl, .12), "chassis", .02)
    for s in (-1, 1):
        box("Drop side", (s * 1.18, yc, 1.42), (.05, yl, .38), "alu", .015)
        for k in range(6):
            box("Side rib", (s * 1.205, y0 + .3 + k * .65, 1.42), (.02, .04, .36), "metal")
        for k in range(5):
            box("Sill block", (s * 1.21, y0 + .4 + k * .72, 1.17), (.02, .36, .09), "reflect_red" if k % 2 else "reflect")
    box("Tail gate", (0, y1 - .02, 1.42), (2.4, .05, .38), "alu", .015)
    # roll bar
    tube("Roll bar", [(-1.05, -.45, 1.25), (-1.05, -.45, 3.1), (1.05, -.45, 3.1), (1.05, -.45, 1.25)], .07, "red")
    rod("Roll bar brace", (-1.05, -.45, 3.1), (1.05, -.45, 1.3), .05, "red", sides=6, caps=False)
    # the tank: an elliptic stainless drum with bands
    n, tl = 20, 2.3
    ty0 = -.2
    sec = [(math.cos(2 * math.pi * k / n) * 1.0, math.sin(2 * math.pi * k / n) * .72 + 2.08) for k in range(n)]
    extrude("Tank", sec, "y", ty0, ty0 + tl, "inox")
    for y in (ty0 + .15, ty0 + tl / 2, ty0 + tl - .15):
        extrude("Tank band", [(x * 1.02, (z - 2.08) * 1.03 + 2.08) for x, z in sec], "y", y - .04, y + .04, "red")
    rod("Tank lid", (0, ty0 + .7, 2.78), (0, ty0 + .7, 2.86), .22, "metal", sides=12)
    for s in (-1, 1):
        box("Tank cradle", (s * .7, ty0 + tl / 2, 1.33), (.3, 1.9, .2), "chassis")
        text("AIB", "AIB", (s * 1.03, ty0 + tl / 2, 2.12), .34, "red",
             rot=(math.pi / 2, 0, s * math.pi / 2), bold=.05)
    # pump module at the back, red, with the reel on top
    py0 = ty0 + tl + .08
    box("Pump module", (0, (py0 + y1) / 2, 1.78), (2.1, y1 - py0 - .05, 1.1), "red", .06, 2)
    for s in (-1, 1):
        box("Module louvre", (s * 1.055, (py0 + y1) / 2, 1.85), (.02, .5, .5), "black")
        box("Module band", (s * 1.056, (py0 + y1) / 2, 1.42), (.02, y1 - py0 - .1, .08), "reflect")
    rod("Hose reel", (-.6, (py0 + y1) / 2, 2.6), (.6, (py0 + y1) / 2, 2.6), .3, "hose", sides=16)
    for s in (-1, 1):
        rod("Reel flange", (s * .6, (py0 + y1) / 2, 2.6), (s * .65, (py0 + y1) / 2, 2.6), .42, "metal", sides=16)
    box("Pump panel", (0, y1 - .02, 1.75), (1.5, .06, .55), "alu", .02)
    for k in range(4):
        rod("Gauge", (-.5 + k * .33, y1 + .01, 1.85), (-.5 + k * .33, y1 + .04, 1.85), .06, "white", sides=10)
    for s in (-1, 1):
        box("Tail lamp", (s * .98, y1 + .03, 1.0), (.22, .04, .3), "tail", .015)
    box("Rear bumper", (0, y1 + .05, .82), (2.3, .16, .2), "chassis", .03)
    for i in range(8):
        box("Chevron", (-1.05 + i * .3, y1 + .14, .82), (.24, .02, .16), "reflect_red" if i % 2 else "reflect")
    box("Rear plate", (0, y1 + .14, 1.02), (.5, .02, .11), "plate")


def engine_rosso():
    chassis()
    cab("red", "white")
    closed_body("red", "white")


def engine_bianco():
    chassis()
    cab("white", "red")
    closed_body("red", "white")


def engine_modulo():
    chassis()
    cab("red", "white")
    modulo_body()


def letters():
    # the team letter on the cab roof, on a white disc with a black rim: the
    # top-down camera reads it before anything else. The game turns the disc
    # about its centre so the letter is always upright on screen.
    for i in range(8):
        ch = chr(ord("A") + i)
        with part(f"letter_{ch}", (0, -1.72, 3.1), (0, 0, 1), "plate"):
            rod("Roof plate rim", (0, -1.72, 3.07), (0, -1.72, 3.1), .84, "black", sides=28)
            rod("Roof plate", (0, -1.72, 3.1), (0, -1.72, 3.12), .74, "white", sides=28)
            text("Roof letter", ch, (0, -1.72, 3.125), 1.15, "black", rot=(0, 0, math.pi), depth=.006, bold=.06)


# =====================================================================================
#  boats (bow +X, waterline z = WL)
# =====================================================================================
WL = .1


LAST_HULL = []


def hull_shell(L, B, free, draft, *, transom=True, sheer_bow=.3, sheer_stern=.1, vee=.35, round_=2.0,
               n_st=14, n_side=6, transom_w=.85):
    """Stations from stern (u=0) to bow (u=1): half width, sheer and keel
    height. Returns verts, faces (the shell from keel to sheer, ends closed)
    and the sheer line per station."""
    rows, sheer, keels = [], [], []
    LAST_HULL[:] = [keels, round_]
    for i in range(n_st + 1):
        u = i / n_st
        if transom:
            if u < .55:
                w = transom_w + (1 - transom_w) * math.sin(math.pi / 2 * u / .55)
            else:
                w = math.cos(math.pi / 2 * (u - .55) / .45) ** .75
            u_x = u
        else:
            w = math.sin(math.pi * u) ** .62
            u_x = u
        w = max(w, .02) * B / 2
        x = -L / 2 + u_x * L
        zs = WL + free + sheer_bow * u ** 2.2 + sheer_stern * (1 - u) ** 2.5
        # the keel rises to the waterline at the stem (and at a canoe stern)
        rise = max(0, (u - .8) / .2) ** 1.6
        if not transom:
            rise = max(rise, max(0, (.2 - u) / .2) ** 1.6)
        zk = WL - draft * (1 - .95 * rise)
        row = []
        for k in range(n_side + 1):
            a = k / n_side * math.pi / 2
            yy = w * math.sin(a) ** (2 / round_)
            zz = zk + (zs - zk) * (1 - math.cos(a) ** (2 / round_))
            # a vee bottom: straighten the lower part
            if k < n_side / 2:
                t = k / (n_side / 2)
                zz = zk + (zz - zk) * (1 - vee) + vee * t * (zk + (zs - zk) * .35 - zk)
            row.append((x, yy, zz))
        rows.append(row)
        sheer.append((x, w, zs))
        keels.append(zk)
    m = n_side + 1
    verts = []
    for row in rows:
        verts += [(x, -y, z) for (x, y, z) in reversed(row)] + [(x, y, z) for (x, y, z) in row[1:]]
    w2 = 2 * m - 1
    faces = []
    for i in range(n_st):
        for k in range(w2 - 1):
            a, b = i * w2 + k, (i + 1) * w2 + k
            faces.append([a, b, b + 1, a + 1])
    faces.append(list(range(w2)))  # stern
    last = n_st * w2
    faces.append(list(range(last + w2 - 1, last - 1, -1)))  # bow
    return verts, faces, sheer


def banded_hull(name, verts, faces, bands):
    """The shell cut at fixed heights: `bands` = [(z_top, tone), ...] from the bottom; the last to the sheer."""
    bm = bmesh.new()
    for v in verts:
        bm.verts.new(v)
    bm.verts.ensure_lookup_table()
    for f in faces:
        try:
            bm.faces.new([bm.verts[i] for i in f])
        except ValueError:
            pass
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    for z, _ in bands[:-1]:
        geom = bm.verts[:] + bm.edges[:] + bm.faces[:]
        bmesh.ops.bisect_plane(bm, geom=geom, plane_co=(0, 0, z), plane_no=(0, 0, 1))
    lo = -1e9
    for z, tone in bands:
        hi = z if z is not None else 1e9
        b2 = bm.copy()
        kill = [f for f in b2.faces if not (lo - 1e-4 <= f.calc_center_median().z <= hi + 1e-4)]
        bmesh.ops.delete(b2, geom=kill, context="FACES")
        me = bpy.data.meshes.new(name)
        b2.to_mesh(me)
        b2.free()
        o = bpy.data.objects.new(name, me)
        bpy.context.scene.collection.objects.link(o)
        finish(o, name, tone)
        lo = hi
    bm.free()


def deck_from_sheer(name, sheer, drop, inset, tone):
    pts_l = [(x, -w * inset, z - drop) for x, w, z in sheer]
    pts_r = [(x, w * inset, z - drop) for x, w, z in sheer]
    verts = pts_l + pts_r
    n = len(sheer)
    faces = [[i, i + 1, n + i + 1, n + i] for i in range(n - 1)]
    o = solid(name, verts, faces, tone, outward=False)
    # face up
    me = o.data
    if me.polygons[0].normal.z < 0:
        bm = bmesh.new()
        bm.from_mesh(me)
        bmesh.ops.reverse_faces(bm, faces=bm.faces)
        bm.to_mesh(me)
        bm.free()
    return o


def bulwark(name, sheer, drop, tone, inset=.94):
    """The inside of the topsides above the deck, both sides, and the capping rail."""
    for s in (-1, 1):
        verts = []
        for x, w, z in sheer:
            verts += [(x, s * w * inset, z - drop), (x, s * w * inset, z + .01)]
        faces = [[2 * i, 2 * i + 2, 2 * i + 3, 2 * i + 1] for i in range(len(sheer) - 1)]
        o = solid(name, verts, faces, tone, outward=False)
        # facing inboard
        bm = bmesh.new()
        bm.from_mesh(o.data)
        bm.normal_update()
        bm.faces.ensure_lookup_table()
        if bm.faces and bm.faces[len(bm.faces) // 2].normal.y * s > 0:
            bmesh.ops.reverse_faces(bm, faces=bm.faces)
        bm.to_mesh(o.data)
        bm.free()


def rail(name, sheer, tone, r=.045, lift=.02):
    for s in (-1, 1):
        tube(name, [(x, s * w, z + lift) for x, w, z in sheer], r, tone, sides=5)


def hull_number(code, x, z, size, tone, B_at=None, sheer=None):
    """The number on both bows, on the hull's own surface at height `z`."""
    if sheer is not None:
        keels, r = LAST_HULL
        i = min(range(len(sheer)), key=lambda k: abs(sheer[k][0] - x))
        _, w, zs = sheer[i]
        f = min(max((z - keels[i]) / (zs - keels[i]), 0), 1)
        ca = (1 - f) ** (r / 2)
        B_at = w * max(1 - ca * ca, 0) ** (1 / r) + .015
    for s in (-1, 1):
        text("Hull number", code, (x, s * B_at, z), size, tone,
             rot=(math.pi / 2, 0, 0 if s < 0 else math.pi), depth=.004, bold=.03)


def width_at(sheer, x):
    best = min(sheer, key=lambda p: abs(p[0] - x))
    return best[1]


def gozzo(top, band, tend, code):
    L, B = 6.4, 2.25
    v, f, sh = hull_shell(L, B, .62, .45, transom=False, sheer_bow=.32, sheer_stern=.22, vee=.15, round_=2.3)
    banded_hull("Hull", v, f, [(WL - .02, "antifoul"), (WL + .08, "white"), (sh[len(sh) // 2][2] - .2, top),
                               (None, band)])
    deck_from_sheer("Floor", sh, .42, .9, "teak")
    bulwark("Inner side", sh, .42, "offwhite")
    rail("Gunwale", sh, "teak_d", .05)
    # the stem rising above the bow, as on the Ligurian gozzi
    tube("Stem", [(L / 2 - .05, 0, sh[-1][2] - .2), (L / 2 + .02, 0, sh[-1][2] + .22)], .06, "teak_d")
    for x in (-1.6, -.2, 1.2):
        w = width_at(sh, x) * .9
        box("Thwart", (x, 0, sh[0][2] - .32 if x < 0 else sh[-1][2] - .55), (.22, 2 * w, .05), "teak")
    box("Engine box", (-.6, 0, WL + .55), (.9, .7, .45), "offwhite", .05)
    # the small forward cuddy (tuga) and the stern awning (tendalino)
    box("Tuga", (1.75, 0, WL + .72), (1.1, 1.2, .4), "offwhite", .1, 2)
    box("Tuga hatch", (1.75, 0, WL + .93), (.5, .5, .04), "teak")
    for x in (-2.4, -1.0):
        for s in (-1, 1):
            rod("Awning post", (x, s * .8, WL + .5), (x, s * .8, WL + 1.75), .025, "alu", sides=5)
    box("Tendalino", (-1.7, 0, WL + 1.78), (1.65, 1.8, .05), tend, .02)
    box("Tendalino valance", (-1.7, 0, WL + 1.7), (1.7, 1.82, .1), tend)
    rod("Rudder", (-L / 2 - .05, 0, WL - .35), (-L / 2 - .05, 0, sh[0][2]), .05, "teak_d", sides=4)
    rod("Tiller", (-L / 2 - .05, 0, sh[0][2]), (-L / 2 + .9, 0, sh[0][2] + .05), .03, "teak_d", sides=4)
    for s in (-1, 1):
        for x in (-1.0, .9):
            rod("Fender", (x, s * (width_at(sh, x) + .06), sh[3][2] - .1), (x, s * (width_at(sh, x) + .06), sh[3][2] - .45),
                .09, "fender", sides=8)
    hull_number(code, 1.9, sh[-4][2] - .2, .2, "black", sheer=sh)


def peschereccio(top, wheel_c, code):
    L, B = 11.5, 3.9
    v, f, sh = hull_shell(L, B, 1.1, 1.15, transom=True, sheer_bow=.75, sheer_stern=.05, vee=.25, round_=2.2,
                          transom_w=.8)
    mid = sh[len(sh) // 2][2]
    banded_hull("Hull", v, f, [(WL - .02, "antifoul"), (WL + .12, "white"), (mid - .25, top), (None, "white")])
    deck_from_sheer("Deck", sh, .5, .93, "teak")
    bulwark("Bulwark", sh, .5, "white")
    rail("Rail", sh, "white", .05)
    # wheelhouse forward of midships
    wx = 1.2
    box("Wheelhouse", (wx, 0, mid + .55), (2.6, 2.4, 1.6), wheel_c, .06, 2)
    box("Wheelhouse roof", (wx, 0, mid + 1.4), (2.9, 2.6, .1), "white", .03)
    for s in (-1, 1):
        for k in range(3):
            box("Window", (wx - .8 + k * .8, s * 1.205, mid + .95), (.55, .02, .45), "glass")
    for k in range(3):
        box("Front window", (wx + 1.305, -.75 + k * .75, mid + .95), (.02, .6, .45), "glass")
    box("Back door", (wx - 1.305, .5, mid + .5), (.02, .6, 1.2), "teak_d")
    # mast with radar and lights, the trawl gantry at the stern, the net drum
    rod("Mast", (wx + .5, 0, mid + 1.45), (wx + .5, 0, mid + 4.4), .07, "white", sides=8)
    box("Radar", (wx + .5, 0, mid + 3.2), (1.2, .15, .1), "white", .02)
    box("Mast light", (wx + .5, 0, mid + 4.45), (.12, .12, .12), "light")
    for s in (-1, 1):
        box("Side light", (wx + .9, s * 1.32, mid + 1.5), (.2, .06, .12), "tail" if s < 0 else "light")
        tube("Gantry", [(-L / 2 + .6, s * 1.5, mid - .45), (-L / 2 + .4, s * 1.3, mid + 2.8)], .08, "yellow_b")
        tube("Boom", [(wx - .4, s * 1.1, mid + 1.45), (-2.0, s * 2.6, mid + 2.5)], .05, "yellow_b")
        for x in (-2.5, 0.0, 3.0):
            rod("Tyre fender", (x, s * (width_at(sh, x) + .08), mid - .35), (x, s * (width_at(sh, x) + .2), mid - .35),
                .22, "black", sides=10)
    rod("Gantry beam", (-L / 2 + .4, -1.3, mid + 2.8), (-L / 2 + .4, 1.3, mid + 2.8), .08, "yellow_b", sides=6)
    rod("Net drum", (-3.3, -1.1, mid + .1), (-3.3, 1.1, mid + .1), .45, "net", sides=14)
    for s in (-1, 1):
        rod("Drum flange", (-3.3, s * 1.1, mid + .1), (-3.3, s * 1.18, mid + .1), .6, "metal", sides=14)
    box("Fish boxes", (-1.6, .6, mid - .3), (.8, .6, .35), "sky", .03)
    box("Fish boxes", (-1.6, -.5, mid - .3), (.8, .6, .5), "orange", .03)
    hull_number(code, 3.4, sh[-4][2] - .4, .4, "white" if top in ("navy", "green", "brick", "blue") else "black",
                sheer=sh)


def gommone(tubes, code):
    L, B = 6.0, 2.5
    v, f, sh = hull_shell(L * .95, B * .78, .5, .45, transom=True, sheer_bow=.25, sheer_stern=0, vee=.6, round_=1.6,
                          transom_w=.8)
    banded_hull("Hull", v, f, [(WL + .05, "grey"), (None, "white")])
    deck_from_sheer("Floor", sh, .25, .85, "grey")
    # the tube: a U from the transom round the bow
    pts = []
    for x, w, z in sh:
        pts.append((x, w + .12, z + .1))
    ring = [(x, -y, z) for x, y, z in reversed(pts)] + pts[1:]
    n = 10
    verts, faces = [], []
    for i, p in enumerate(ring):
        a = ring[max(i - 1, 0)]
        b = ring[min(i + 1, len(ring) - 1)]
        t = (Vector(b) - Vector(a)).normalized()
        side = t.cross(Vector((0, 0, 1))).normalized()
        r = .26 * (1 - .2 * max(0, (p[0] - L * .35) / (L * .15)))
        for k in range(n):
            ang = 2 * math.pi * k / n
            q = Vector(p) + side * math.cos(ang) * r + Vector((0, 0, 1)) * math.sin(ang) * r
            verts.append(tuple(q))
    for i in range(len(ring) - 1):
        for k in range(n):
            k2 = (k + 1) % n
            faces.append([i * n + k, i * n + k2, (i + 1) * n + k2, (i + 1) * n + k])
    faces.append(list(range(n)))
    last = (len(ring) - 1) * n
    faces.append(list(range(last + n - 1, last - 1, -1)))
    solid("Tube", verts, faces, tubes, smooth_all=True)
    # console, seat, outboard
    box("Console", (.2, 0, WL + .75), (.7, .75, .9), "white", .08, 2)
    box("Windscreen", (.55, 0, WL + 1.3), (.04, .7, .35), "glass", rot=(0, -.4, 0))
    box("Seat", (-.75, 0, WL + .6), (.8, .8, .5), "navy", .1, 2)
    rod("Grab rail", (-1.15, -.4, WL + 1.05), (-1.15, .4, WL + 1.05), .025, "inox", sides=5)
    box("Outboard", (-L / 2 * .95 - .25, 0, WL + .75), (.45, .4, .7), "black", .08, 2)
    box("Outboard leg", (-L / 2 * .95 - .2, 0, WL - .1), (.18, .12, .9), "black", .03)
    text("RIB", code, (1.6, -(B * .39 + .38 + .265), sh[-6][2] + .12), .18, "white" if tubes in ("black", "grey", "orange") else "black",
         rot=(math.pi / 2, 0, 0), depth=.004)


def motoscafo(stripe, code):
    L, B = 7.6, 2.6
    v, f, sh = hull_shell(L, B, .85, .55, transom=True, sheer_bow=.35, sheer_stern=0, vee=.55, round_=1.8,
                          transom_w=.85)
    mid = sh[len(sh) // 2][2]
    banded_hull("Hull", v, f, [(WL - .02, "antifoul_b"), (WL + .14, stripe), (mid - .3, "white"),
                               (mid - .18, stripe), (None, "white")])
    deck_from_sheer("Deck", sh, .05, .97, "white")
    box("Cockpit floor", (-1.6, 0, mid - .35), (3.2, 2.0, .05), "teak")
    # cabin forward with a raked tinted screen, cockpit seats aft
    box("Cabin", (1.2, 0, mid + .25), (2.4, 2.0, .55), "white", .2, 3)
    box("Screen", (.15, 0, mid + .62), (.06, 2.1, .5), "glass", rot=(0, -.6, 0))
    for s in (-1, 1):
        box("Port light", (1.4, s * 1.005, mid + .25), (.9, .02, .14), "glass")
    box("Seat", (-2.6, 0, mid - .1), (.6, 2.1, .45), "cream", .1, 2)
    box("Helm seat", (-.6, -.5, mid - .05), (.5, .6, .55), "cream", .1, 2)
    rod("Rail", (2.4, -.9, mid + .35), (3.6, 0, mid + .35), .02, "inox", sides=4)
    rod("Rail", (2.4, .9, mid + .35), (3.6, 0, mid + .35), .02, "inox", sides=4)
    tube("Bimini frame", [(-2.9, -1.1, mid), (-2.4, -1.1, mid + 1.35), (-2.4, 1.1, mid + 1.35), (-2.9, 1.1, mid)], .025, "inox")
    box("Bimini", (-1.9, 0, mid + 1.38), (1.3, 2.3, .05), "navy" if stripe != "navy" else "canvas", .02)
    for s in (-1, 1):
        box("Outboard", (-L / 2 - .3, s * .45, WL + .7), (.5, .45, .8), "white", .1, 2)
        box("Outboard leg", (-L / 2 - .25, s * .45, WL - .1), (.2, .14, .9), "grey", .03)
    hull_number(code, 2.2, sh[-5][2] - .38, .22, stripe if stripe != "white" else "navy", sheer=sh)


def vela(top, code, sails, sail_c):
    L, B = 9.6, 3.1
    v, f, sh = hull_shell(L, B, .9, .6, transom=True, sheer_bow=.25, sheer_stern=.05, vee=.3, round_=1.9,
                          transom_w=.75)
    mid = sh[len(sh) // 2][2]
    banded_hull("Hull", v, f, [(WL - .02, "antifoul_b"), (WL + .1, "white"), (None, top)])
    rod("Keel", (-.3, 0, WL - .5), (.4, 0, WL - 1.6), .12, "antifoul_b", sides=6)
    deck_from_sheer("Deck", sh, .02, .97, "teak")
    rail("Toe rail", sh, "teak_d", .03)
    box("Coachroof", (.1, 0, mid + .3), (3.2, 2.1, .5), "white", .15, 2)
    for s in (-1, 1):
        box("Coachroof window", (.1, s * 1.055, mid + .33), (1.8, .02, .18), "glass")
        rod("Lifeline", (-4.4, s * 1.3, mid + .6), (3.5, s * 1.05, mid + .6), .012, "inox", sides=4)
    box("Cockpit", (-2.7, 0, mid - .05), (2.4, 1.6, .1), "teak_d")
    rod("Wheel", (-3.9, -.0, mid + .8), (-3.92, 0, mid + .8), .45, "inox", sides=14, caps=False)
    rod("Mast", (.6, 0, mid), (.6, 0, mid + 12.5), .1, "alu", sides=8)
    rod("Boom", (.5, 0, mid + 1.4), (-3.8, 0, mid + 1.5), .07, "alu", sides=6)
    rod("Forestay", (.6, 0, mid + 12.3), (4.75, 0, sh[-1][2]), .02, "inox", sides=4, caps=False)
    rod("Backstay", (.6, 0, mid + 12.3), (-4.7, 0, sh[0][2]), .02, "inox", sides=4, caps=False)
    for s in (-1, 1):
        rod("Shroud", (.6, 0, mid + 11.0), (.4, s * 1.4, sh[7][2]), .015, "inox", sides=4, caps=False)
        rod("Spreader", (.6, 0, mid + 6.5), (.6, s * 1.0, mid + 6.4), .03, "alu", sides=4)
    if not sails:
        rod("Sail cover", (.4, 0, mid + 1.62), (-3.6, 0, mid + 1.68), .19, "navy" if top != "navy" else "canvas", sides=8)
        rod("Furled jib", (4.55, 0, sh[-1][2] + .3), (.75, 0, mid + 11.6), .1, sail_c, r2=.05, sides=6)
    else:
        def sail(name, top_p, tack, clew, belly, tone, rows=7, cols=5):
            verts, faces = [], []
            for i in range(rows + 1):
                t = i / rows
                luff = Vector(top_p).lerp(Vector(tack), t)
                leech = Vector(top_p).lerp(Vector(clew), t)
                for j in range(cols + 1):
                    u = j / cols
                    p = luff.lerp(leech, u)
                    p.y = belly * math.sin(math.pi * u) * math.sin(math.pi * min(1.0, t * .9 + .1))
                    verts.append(tuple(p))
            for i in range(rows):
                for j in range(cols):
                    a = i * (cols + 1) + j
                    faces.append((a, a + 1, a + cols + 2, a + cols + 1))
            o = solid(name, verts, faces, tone, outward=False, smooth_all=True)
            return o
        sail("Mainsail", (.48, 0, mid + 12.1), (.48, 0, mid + 1.5), (-3.75, 0, mid + 1.55), .7, sail_c)
        sail("Genoa", (.7, 0, mid + 11.3), (4.6, 0, sh[-1][2] + .3), (-.6, 0, mid + .6), .6, sail_c)
        text("Sail number", code.split()[-1], (-.9, .45, mid + 6.5), .9, "navy", rot=(math.pi / 2, 0, 0), depth=.004)
    hull_number(code, 2.9, sh[-4][2] - .3, .3, "white" if top in ("navy", "blue", "green") else "navy", sheer=sh)


BUILDERS = {
    "engine_rosso": engine_rosso,
    "engine_bianco": engine_bianco,
    "engine_modulo": engine_modulo,
    "engine_letters": letters,
    "gozzo_1": lambda: gozzo("blue", "white", "canvas", "RV 1087"),
    "gozzo_2": lambda: gozzo("white", "green", "green", "RV 412"),
    "gozzo_3": lambda: gozzo("ochre", "brick", "blue", "RV 2261"),
    "peschereccio_1": lambda: peschereccio("navy", "white", "RV 214"),
    "peschereccio_2": lambda: peschereccio("brick", "white", "RV 87"),
    "peschereccio_3": lambda: peschereccio("green", "cream", "RV 351"),
    "gommone_1": lambda: gommone("grey", "RV 3305"),
    "gommone_2": lambda: gommone("orange", "RV 518"),
    "gommone_3": lambda: gommone("black", "RV 1194"),
    "motoscafo_1": lambda: motoscafo("navy", "RV 4471"),
    "motoscafo_2": lambda: motoscafo("brick", "RV 905"),
    "motoscafo_3": lambda: motoscafo("teal", "RV 2730"),
    "vela_1": lambda: vela("white", "ITA 14", False, "sail"),
    "vela_2": lambda: vela("navy", "ITA 233", False, "sail"),
    "vela_3": lambda: vela("green", "ITA 71", False, "cream"),
    "vela_1_vele": lambda: vela("white", "ITA 14", True, "sail"),
    "vela_2_vele": lambda: vela("navy", "ITA 233", True, "sail"),
    "vela_3_vele": lambda: vela("green", "ITA 71", True, "cream"),
}


def rt(v):
    """Blender (Z up) to runtime (Y up): x, z, -y."""
    return (round(v[0], 4), round(v[2], 4), round(-v[1], 4))


only = os.environ.get("REAL_ONLY")
baked = {}
for name, build in BUILDERS.items():
    if only and not name.startswith(only):
        continue
    current = bpy.data.collections.new(name)
    bpy.context.scene.collection.children.link(current)
    build()
    bpy.context.view_layer.update()
    groups = {}
    for o in current.objects:
        g = groups.setdefault(o["part"], ({}, [], [], [], [], []))
        lookup, pos, nrm, col, srf, idx = g
        mesh = o.to_mesh()
        mesh.calc_loop_triangles()
        corner = mesh.corner_normals
        colour = tuple(round(srgb_to_linear(c), 4) for c in o.data.materials[0]["srgb"])
        surface = o.data.materials[0]["surface"]
        nm = o.matrix_world.to_3x3().inverted().transposed()
        for tri in mesh.loop_triangles:
            for vi, li in zip(tri.vertices, tri.loops):
                p = rt(o.matrix_world @ mesh.vertices[vi].co)
                n = (nm @ Vector(corner[li].vector)).normalized()
                n = (round(n.x, 3), round(n.z, 3), round(-n.y, 3))
                key = (p, n, colour, surface)
                at = lookup.get(key)
                if at is None:
                    at = lookup[key] = len(pos)
                    pos.append(p)
                    nrm.append(n)
                    col.append(colour)
                    srf.append(surface)
                idx.append(at)
        o.to_mesh_clear()
    total = 0
    for part_name, (_, pos, nrm, col, srf, idx) in groups.items():
        if name == "engine_letters":
            key = part_name.replace("letter_", "engine_letter_")
        else:
            key = name if not part_name else f"{name}.{part_name}"
        entry = dict(positions=pos, normals=nrm, colors=col, indices=idx, surfaces=srf)
        if part_name:
            r = RIGS[name][part_name]
            entry["rig"] = dict(pivot=rt(r["pivot"]), axis=rt(r["axis"]), kind=r["kind"], phase=round(r["phase"], 4))
        baked[key] = entry
        total += len(idx) // 3
    print(f"== {name}: {total} triangles, {sum(len(g[1]) for g in groups.values())} vertices")

if not only:
    (OUT / "vehicles_real.json").write_text(json.dumps(baked, separators=(",", ":")) + "\n")
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(OUT / "vehicles_real_assets.blend"))

# --- close-ups ------------------------------------------------------------------------
prev = os.environ.get("REAL_PREVIEW")
if prev:
    outdir = Path(prev)
    outdir.mkdir(parents=True, exist_ok=True)
    for tone, m in materials.items():
        m.use_nodes = True
        sh = m.node_tree.nodes.get("Principled BSDF")
        sh.inputs["Base Color"].default_value = m.diffuse_color
        srf = m["surface"]
        sh.inputs["Roughness"].default_value = {1: .3, 2: .3, 4: .05}.get(srf, .7)
        sh.inputs["Metallic"].default_value = 1.0 if srf == 2 else 0.0
        if srf == 5:
            sh.inputs["Emission Color"].default_value = m.diffuse_color
            sh.inputs["Emission Strength"].default_value = 3.0
    names = [c.name for c in bpy.data.collections]
    current = bpy.data.collections.new("studio")
    bpy.context.scene.collection.children.link(current)
    PART = None
    COLOURS["ground"] = (.45, .42, .36)
    COLOURS["sea"] = (.10, .30, .40)
    bpy.ops.object.light_add(type="SUN", location=(0, 0, 20))
    sun = bpy.context.object
    sun.data.energy = 4.0
    sun.rotation_euler = (math.radians(40), 0, math.radians(35))
    bpy.ops.object.camera_add(location=(14, -16, 14))
    cam = bpy.context.object
    cam.data.type = "ORTHO"
    sc = bpy.context.scene
    sc.camera = cam
    sc.render.engine = "BLENDER_EEVEE"
    sc.world = bpy.data.worlds.new("w")
    sc.world.color = (.55, .6, .65)
    sc.render.resolution_x, sc.render.resolution_y = 900, 640
    sc.view_settings.exposure = 1.3
    sc.world.color = (.9, .92, .95)
    for nm in names:
        for c in bpy.data.collections:
            c.hide_render = c.name not in (nm, "studio")
        if nm == "engine_letters":
            continue
        boat = not nm.startswith("engine")
        g = box("Ground", (0, 0, -.02 if not boat else WL - .01), (60, 60, .02), "ground" if not boat else "sea")
        centre = Vector((0, .0, 1.6)) if not boat else Vector((0, 0, 1.5 if "vele" not in nm else 5.0))
        scale = 9.5 if not boat else (14 if "vele" in nm or nm.startswith("vela") else 12.5)
        for tag, ang, el in (("a", -35, .55), ("b", 145, .55), ("c", 60, 2.5)):
            d = Vector((math.sin(math.radians(ang)), -math.cos(math.radians(ang)), el)).normalized() * 40
            cam.location = centre + d
            cam.rotation_euler = (centre - cam.location).to_track_quat("-Z", "Y").to_euler()
            cam.data.ortho_scale = scale
            sc.render.filepath = str(outdir / f"{nm}_{tag}.png")
            bpy.ops.render.render(write_still=True)
        bpy.data.objects.remove(g)
