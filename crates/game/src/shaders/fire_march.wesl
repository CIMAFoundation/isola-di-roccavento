// Flames and smoke, ray-marched at a fraction of the screen's resolution
// (`fire_volumetric.rs`, which explains the field). One ray per pixel of
// the small target: through the plume's box, to the scene's depth; out
// come the light the volume sends to the eye (premultiplied), how much of
// the scene it hides, and the depth the ray stopped at (for the
// composite's bilateral upsample) and the volume's distance (for the
// resolve, `fire_resolve.wesl`, which blends in the frames before).

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_pbr::render::mesh_view_bindings::{view, globals, lights};
import game::shaders::scene_light::apply_scene_fog;

struct Params {
    map: vec4<f32>,
    lo: vec4<f32>,
    hi: vec4<f32>,
    wind: vec4<f32>,
    smoke: vec4<f32>,
    flame: vec4<f32>,
    look: vec4<f32>,
    look2: vec4<f32>,
    size: vec4<f32>,
    origin: vec4<f32>,
    frame: vec4<f32>,
    drift: array<vec4<f32>, 4>,
    shear: array<vec4<f32>, 2>,
    prev: mat4x4<f32>,
    rays: mat4x4<f32>,
    eye: vec4<f32>,
    more: vec4<f32>,
    stride: vec4<f32>,
    front: vec4<f32>,
    fluid: vec4<f32>,
    fluid2: vec4<f32>,
}

@group(1) @binding(0) var<uniform> p: Params;
@if(MULTISAMPLED)
@group(1) @binding(1) var depth_tex: texture_depth_multisampled_2d;
@else
@group(1) @binding(1) var depth_tex: texture_depth_2d;
@group(1) @binding(2) var plume_tex: texture_2d<f32>;
@group(1) @binding(3) var plume_samp: sampler;
@group(1) @binding(4) var noise_tex: texture_3d<f32>;
@group(1) @binding(5) var noise_samp: sampler;
@group(1) @binding(6) var ground_tex: texture_2d<f32>;
// the front drawn ahead of the game's (`fire_ahead::FRONT_MAP`), read
// texel by texel; the flames' fluid (`fire_fluid`), heat in w
@group(1) @binding(7) var front_tex: texture_2d<f32>;
@group(1) @binding(8) var fluid_tex: texture_3d<f32>;
@group(1) @binding(9) var fluid_samp: sampler;

struct Out {
    @location(0) color: vec4<f32>,
    // for the composite, in half floats: the scene's depth the ray stopped
    // at (m, `FAR_M` the sky; below 0 where the footprint has a nearer
    // surface too, `near_aux`); then the flames' share of the opacity, how
    // far they are, and how high up their tongues (0 foot, 1 tip), which
    // the composite carves finer
    @location(1) aux: vec4<f32>,
    // how far the volume's weighted middle is (the scene's, where none):
    // the resolve reprojects the frames before from there
    @location(2) mid: f32,
    // the same ray stopped at the nearest surface of the pixel's footprint
    // (`footprint`): its light, and that surface's depth (m) with the
    // flames' share of the opacity in front of it. Where the footprint is
    // one surface, as the first.
    @location(3) near: vec4<f32>,
    @location(4) near_aux: vec2<f32>,
}

// The screen pixels a march pixel stands for (`div` a side, all their
// samples), split into a nearer and a farther surface where their depths
// are farther apart than `ONE_SURFACE`. One depth a march pixel, the
// composite had nothing for a pixel of another surface: a gap in a crown
// whose march pixels all fell on leaves was the sky without its smoke, a
// twig thinner than a march pixel had the smoke behind it over it
// (2026-11-06). The ray goes on to the farther surface and notes what it
// held at the nearer: the composite finds both.
//
// Each surface's depth is where the ray stops for it: the centre pixel's,
// as before, for the surface it is of; the front of the other (stopped at
// the back of a sloping footprint, a ray grazing the ground ran on under
// it through the flames' foot, and the burning ground came out in bright
// march pixels).
struct Foot {
    near: f32,
    far: f32,
    split: bool,
}

@if(MULTISAMPLED)
fn depth_at(at: vec2<i32>, s: i32) -> f32 {
    return metres(textureLoad(depth_tex, at, s));
}
@else
fn depth_at(at: vec2<i32>, s: i32) -> f32 {
    return metres(textureLoad(depth_tex, at, 0));
}

@if(MULTISAMPLED)
fn samples() -> i32 {
    return i32(textureNumSamples(depth_tex));
}
@else
fn samples() -> i32 {
    return 1;
}

fn footprint(base: vec2<i32>, d: i32, centre: vec2<i32>) -> Foot {
    let n = samples();
    let last = vec2<i32>(p.size.zw) - 1;
    let o = vec2<i32>(p.origin.xy);
    let zc = depth_at(o + min(centre, last), 0);
    var lo = FAR_M;
    var hi = 0.0;
    for (var j = 0; j < d; j++) {
        for (var i = 0; i < d; i++) {
            let at = o + min(base + vec2<i32>(i, j), last);
            for (var s = 0; s < n; s++) {
                let z = depth_at(at, s);
                lo = min(lo, z);
                hi = max(hi, z);
            }
        }
    }
    if hi - lo <= ONE_SURFACE * lo {
        return Foot(zc, zc, false);
    }
    let mid = sqrt(lo * hi);
    if zc < mid {
        // the front of the farther surface
        var front = hi;
        for (var j = 0; j < d; j++) {
            for (var i = 0; i < d; i++) {
                let at = o + min(base + vec2<i32>(i, j), last);
                for (var s = 0; s < n; s++) {
                    let z = depth_at(at, s);
                    if z >= mid {
                        front = min(front, z);
                    }
                }
            }
        }
        return Foot(zc, front, front - zc > ONE_SURFACE * zc);
    }
    return Foot(lo, zc, zc - lo > ONE_SURFACE * lo);
}

// A depth as the distance along the camera's axis (m; `FAR_M` the sky).
fn metres(d: f32) -> f32 {
    if d > 0.0 {
        return min(p.origin.w / d, FAR_M);
    }
    return FAR_M;
}

// Depths nearer to each other than this share of their distance are one
// surface (as in `fire_composite.wesl`).
const ONE_SURFACE = 0.1;

// The sky's depth, and the farthest written (as `fire_composite.wesl`).
const FAR_M = 6.0e4;

// The plume map at a point of the ground (Bevy x, z): r smoke given off,
// g how alight, b flame height (m).
fn plume(xz: vec2<f32>, lod: f32) -> vec4<f32> {
    let uv = vec2<f32>(xz.x, -xz.y) * p.map.xy;
    return textureSampleLevel(plume_tex, plume_samp, uv, min(lod, p.map.w));
}

// The height of the ground the scene draws at a point (Bevy x, z; m): its
// heightfield (`fire_volumetric::GROUND_MAP`), samples from the world's
// west edge to its east and from its north edge, row 0, to its south. Not
// a height a fire cell: those are 20 m apart, and the ground between them
// is metres off the drawn one (and the fire grid's own heights are tens of
// metres off it: `fire_volumetric::ground_image`).
fn ground(xz: vec2<f32>) -> f32 {
    let n = vec2<f32>(textureDimensions(ground_tex));
    let at = vec2<f32>(xz.x * p.map.x, 1.0 + xz.y * p.map.y) * (n - 1.0);
    return textureSampleLevel(ground_tex, plume_samp, (at + 0.5) / n, 0.0).r;
}

// Emission of the flame by its temperature (as `fire_volume.wesl`).
fn flame_ramp(h: f32) -> vec3<f32> {
    let c0 = vec3<f32>(0.30, 0.05, 0.01);
    let c1 = vec3<f32>(1.4, 0.18, 0.02);
    let c2 = vec3<f32>(2.8, 0.75, 0.06);
    let c3 = vec3<f32>(3.6, 1.7, 0.25);
    let c4 = vec3<f32>(4.2, 3.0, 1.1);
    let a = smoothstep(0.0, 0.3, h);
    let b = smoothstep(0.3, 0.55, h);
    let c = smoothstep(0.55, 0.8, h);
    let d = smoothstep(0.8, 1.0, h);
    return mix(mix(mix(mix(c0, c1, a), c2, b), c3, c), c4, d);
}

// The noise, at the mip its footprint asks for (`textures::noise_volume`
// has mips): a billow finer than a sample stands for is averaged, not
// sampled (sampled, it sparkles as the camera moves).
fn noise(q: vec3<f32>, lod: f32) -> vec4<f32> {
    return textureSampleLevel(noise_tex, noise_samp, q, lod);
}

// A rotation of the noise's space (about 37° round (1, 2, 1.5)).
const NOISE_TURN = mat3x3<f32>(
    vec3<f32>(0.8303, 0.4210, -0.3654),
    vec3<f32>(-0.3058, 0.8885, 0.3420),
    vec3<f32>(0.4686, -0.1824, 0.8644),
);

// How far smoke `age` seconds old has drifted with the wind of its time
// (`fire_volumetric::WindMemory`): the table holds ages 1/8 … 8/8 of the
// plume's, from no drift at age 0.
fn drift_at(age: f32) -> vec2<f32> {
    let full = p.wind.w / p.wind.z;
    let x = clamp(age / full, 0.0, 1.0) * 8.0;
    let k = min(i32(floor(x)), 7);
    let f = x - f32(k);
    var a = vec2<f32>(0.0);
    if k > 0 {
        let v = p.drift[(k - 1) / 2];
        a = select(v.zw, v.xy, (k - 1) % 2 == 0);
    }
    let v = p.drift[k / 2];
    let b = select(v.zw, v.xy, k % 2 == 0);
    return mix(a, b, f);
}

// How many cells wider smoke `age` seconds old has spread for the winds
// it met (`fire_volumetric::WindMemory`): ages 1/8 … 8/8 of the plume's.
fn shear_at(age: f32) -> f32 {
    let x = clamp(age / (p.wind.w / p.wind.z), 0.0, 1.0) * 8.0;
    let k = min(i32(floor(x)), 7);
    let f = x - f32(k);
    var a = 0.0;
    if k > 0 {
        a = p.shear[(k - 1) / 4][(k - 1) % 4];
    }
    return mix(a, p.shear[k / 4][k % 4], f);
}

// The smoke's body at a point, before the billows tear it.
struct Body {
    // density (below 0 where nothing smokes, so the march may stride)
    dens: f32,
    // metres the map was averaged over there: nothing of the body is
    // narrower
    wide: f32,
    // where the parcel left the ground
    src: vec2<f32>,
}

// The smoke's body at `pos`, `h` metres over the ground below it. The
// parcel left the ground `h / rise` seconds ago and drifted since; the map
// is read where it left, at a mip as wide as the plume has grown; `sway`:
// through the column's sway (not for the taps toward the sun).
fn smoke_body(pos: vec3<f32>, h: f32, sway: bool) -> Body {
    let top = p.wind.w;
    let hh = max(h, 0.0);
    let k = hh / top;
    let age = hh / p.wind.z;
    let base = 1.0 + hh / p.smoke.z;
    // the winds the parcel met spread it further, and thin it: the mip
    // averages it over the wider ground, only the plume's own growth is
    // made up below, and the widening thins it once more (seen slanting
    // through a wide layer, a spread that only averaged read as a haze as
    // thick as the column)
    let spread = base + shear_at(age);
    var out: Body;
    out.dens = -1.0;
    out.wide = spread * p.map.z;
    out.src = pos.xz - drift_at(age);
    if h < -8.0 || h > top {
        return out;
    }
    if sway {
        // the column sways as it rises
        out.src -= (noise(vec3<f32>(pos.x, hh * 1.5, pos.z) / 600.0 + vec3<f32>(0.0, -globals.time * 0.02, 0.0), 0.0).gb - 0.5) * (40.0 + 160.0 * k);
    }
    let s = plume(out.src, log2(spread) + 0.5).r;
    if s < 0.002 {
        return out;
    }
    // the mip averages the column over its width: a line of fire would
    // fade as the square of it, the plume spreads mostly across the wind;
    // thin over the flames (the column lifts off the fire, the front shows
    // under it), thickest some tens of metres up, gone at the top
    let lift = 0.06 + 0.94 * smoothstep(p.look.x * 0.07, p.look.x, hh);
    out.dens = min(s * pow(base, 0.85), 1.6) * (base / spread) * lift * (1.0 - smoothstep(top * 0.5, top, hh));
    return out;
}

// What the billows leave of the body's density at `pos` (0 to 1.4): 3D
// noise carried by the wind and rising with the plume, read at mip `lod`.
fn billows(pos: vec3<f32>, h: f32, lod: f32) -> f32 {
    let k = clamp(h / p.wind.w, 0.0, 1.0);
    let t = globals.time;
    let adv = vec3<f32>(p.wind.x, p.wind.z * 0.7, p.wind.y) * t;
    // turned off the axes: the noise repeats every few hundred metres,
    // and a repeat along the vertical reads as horizontal layers
    let q = NOISE_TURN * ((pos - adv) / p.smoke.y);
    // a slow warp first, so they roll instead of sliding
    let warp = (noise(q * 0.37 + vec3<f32>(0.0, -t * 0.01, 0.0), max(lod - 1.4, 0.0)).gb - 0.5) * 1.2;
    let qw = q + vec3<f32>(warp.x, 0.0, warp.y);
    var n = 0.64 * noise(qw, lod).r + 0.36 * noise(qw * 2.7 + vec3<f32>(0.0, -t * 0.03, 0.0), lod + 1.4).g;
    // the finest octave: past the noise's last mip (3 texels a side) it is
    // its mean, and no tap
    var finest = 0.5;
    if lod < 1.2 {
        finest = mix(noise(qw * 7.1, lod + 2.8).b, 0.5, smoothstep(0.7, 1.2, lod));
    }
    n = 0.87 * n + 0.13 * finest;
    // dense at the source, torn into shreds toward the top: a narrow
    // window, so the billows have edges
    let edge = p.look.y + 0.16 * k - 0.14 * (1.0 - k);
    return smoothstep(edge, edge + 0.12, n) * (0.6 + 0.8 * n);
}

// The flames' pattern: `base` is the place in the noise (x and z in tongue
// widths, y up the flame), `tq` how far it has risen.
fn tongues(base: vec3<f32>, tq: f32, flod: f32, near_w: f32) -> f32 {
    let q = base - vec3<f32>(0.0, tq, 0.0);
    let w = noise(q * 0.37 + vec3<f32>(0.0, -tq * 0.3, 0.0), max(flod - 1.4, 0.0)).gb - 0.5;
    var n = noise(q + vec3<f32>(w.x, 0.0, w.y) * 0.9, flod).r;
    // a finer flicker, where it is more than a pixel
    if near_w > 0.0 {
        let fine_n = noise(q * vec3<f32>(2.9, 2.2, 2.9) + vec3<f32>(w.y, -tq, w.x) * 1.4, flod + 1.5).g;
        n = mix(n, 0.62 * n + 0.38 * fine_n * 1.15, near_w);
    }
    return n;
}

// The flames' pattern at `pos`, `yr` up the flame (0 to 1), in tongues
// `wide` metres across, where a march pixel spans `foot` metres; `small` 1
// where the flame is a few march pixels tall.
fn tongues_at(pos: vec3<f32>, yr: f32, wide: f32, foot: f32, small: f32, t: f32) -> f32 {
    // the noise has 48 texels a period
    let flod = log2(max(foot * 48.0 / wide, 1.0));
    // the phase slanted across the ground: with the same one everywhere,
    // every flame read the same level of the noise at once, whose mean
    // changes from level to level, and the whole front rose and sank
    // together each 0.9 s (user, 2026-11-05: «sembra muoversi su e giù»)
    let qx = pos.x / wide;
    let qz = pos.z / wide;
    let base = vec3<f32>(qx, yr * 0.9 + 0.41 * qx + 0.29 * qz, qz);
    let near_w = 1.0 - smoothstep(0.12, 0.35, foot / wide);
    // rising a flame's height in about a second. A flame a few march
    // pixels tall shows no rising: its pattern is a few dots, each going
    // on and off in place as the noise passes through it, 1.1 times a
    // second. There the pattern passes slowly. Two patterns, each at its
    // own speed, faded one into the other: one speed changing with the
    // distance would race as the camera moved
    var n = 0.5;
    if small < 1.0 {
        n = tongues(base, t * 1.1, flod, near_w);
    }
    if small > 0.0 {
        let slow = tongues(base + vec3<f32>(0.37, 0.0, 0.61), t * FAR_PACE, flod, near_w);
        // two patterns mixed are flatter than either
        n = 0.5 + (mix(n, slow, small) - 0.5) / sqrt(small * small + (1.0 - small) * (1.0 - small));
    }
    return n;
}

// How alight the ground is at a point (Bevy x, z), from the front map on
// this frame's clock (`fire_ahead`): each corner's share burnt and how long
// it has flamed, as `fire_volumetric::rebuild` makes the band, bilinear.
// The plume map's own band moves on only as often as it is rewritten.
fn front_band(xz: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(front_tex));
    let at = vec2<f32>(xz.x, -xz.y) * p.map.xy * vec2<f32>(dims) - 0.5;
    let c0 = vec2<i32>(floor(at));
    let fr = at - floor(at);
    let clock = p.front.x;
    var band = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let wk = mix(1.0 - fr.x, fr.x, f32(o.x)) * mix(1.0 - fr.y, fr.y, f32(o.y));
        let l = textureLoad(front_tex, clamp(c0 + o, vec2<i32>(0), dims - 1), 0);
        let v = l.g + l.b * clock;
        let s = select(clamp(v, l.a, 1.0), clamp(v, 0.0, l.a), l.b >= 0.0);
        let flaming = clamp(1.0 - max(clock - l.r, 0.0) / FLAMING_S, 0.0, 1.0);
        let edge = flaming * flaming * flaming;
        band += wk * s * smoothstep(0.05, 0.3, flaming) * (0.45 + 0.55 * edge);
    }
    return band;
}

// As `fire_view::FLAMING_S`.
const FLAMING_S = 540.0;

// How much the fluid has the say at `pos` (0 none): inside its grid, faded
// over the last 40 m of it and up its top, off where the flames are drawn
// larger than true (`grow`) or a march pixel spans more than a voxel.
fn fluid_weight(pos: vec3<f32>, h: f32, grow: f32, foot: f32) -> f32 {
    if p.fluid.w < 0.5 {
        return 0.0;
    }
    let size = p.fluid2.x * p.fluid.z;
    let rel = pos.xz - p.fluid.xy;
    let edge = min(min(rel.x, rel.y), min(size - rel.x, size - rel.y));
    let top = p.fluid2.y * p.fluid2.z;
    return smoothstep(0.0, 40.0, edge) * (1.0 - smoothstep(top * 0.75, top, h)) * (1.0 - smoothstep(1.05, 1.3, grow)) * (1.0 - smoothstep(1.0, 2.5, foot / p.fluid.z));
}

// The fluid's heat at `pos`, `h` over the ground (its grid follows the
// ground; it wraps, and so does the sampler).
fn fluid_heat(pos: vec3<f32>, h: f32) -> f32 {
    let uvw = vec3<f32>(
        (pos.x / p.fluid.z + 0.5) / p.fluid2.x,
        (max(h, 0.0) / p.fluid2.z + 0.5) / p.fluid2.y,
        (pos.z / p.fluid.z + 0.5) / p.fluid2.x,
    );
    return textureSampleLevel(fluid_tex, fluid_samp, uvw, 0.0).w;
}

// The flames at `pos`, `h` over the ground, where a march pixel spans
// `foot` metres: x how far the noise is over the flame's edge there (the
// flame is where it is over 0, soft over `look2.x`), y how high up the
// flame (0 to 1), z 1 where there is a flame to speak of, else 0.
fn flame_at(pos: vec3<f32>, h: f32, foot: f32, grow: f32, t: f32) -> vec3<f32> {
    // the band read through a wobble of a cell or so: no squares
    let wob = noise(vec3<f32>(pos.xz / 90.0, 0.71), 0.0).rg - 0.5;
    var fire = plume(pos.xz + wob * p.map.z * 2.2, 0.0);
    if p.front.y > 0.5 {
        fire.g = front_band(pos.xz + wob * p.map.z * 2.2);
    }
    // the fluid: its heat is a flame, the tongues' noise its grain; it adds
    // to the tongues below, where the stronger of the two is drawn
    let wf = fluid_weight(pos, h, grow, foot);
    var fl = vec3<f32>(0.0);
    if wf > 0.0 && h > -8.0 {
        let heat = fluid_heat(pos, h);
        if heat > 0.02 {
            let wide = max(4.0 * p.look.w / 14.0, foot * 3.0);
            let octave = log2(wide);
            let n = tongues_at(pos, clamp(h / 12.0, 0.0, 1.0), exp2(floor(octave)), foot, 0.0, t);
            let yr = clamp(1.0 - heat / 0.9, 0.0, 0.98);
            // the noise carves the heat into tongues (a low threshold drew a
            // sheet as thick as the heat)
            fl = vec3<f32>(heat * (0.35 + 1.0 * n) - 0.42, yr, 1.0);
        }
    }
    if fire.g <= 0.02 {
        return fl * wf;
    }
    let clumps = noise(vec3<f32>(pos.xz * 0.028, 0.37), 0.0).b;
    let clump = smoothstep(0.3, 0.75, clumps);
    let band = fire.g * (0.25 + 0.75 * clump);
    let hf = max(fire.b * grow * p.look2.y * (0.45 + 0.35 * clump), max(0.5, foot * 3.0));
    // under the ground (the scene's mesh is not its heightfield to the
    // metre): the depth stops the ray there
    let yr = max(h, 0.0) / hf;
    if yr >= 1.0 || h <= -8.0 {
        return fl * wf;
    }
    // a tongue as tall as the flame and a third as wide (`lingue` scales
    // the width), never under a few march pixels: a fixed size cut the
    // flame into slabs where it was lower than the pattern
    let wide = max(hf * 0.32 * p.look.w / 14.0, foot * 3.0);
    // The pattern is read at the place in the world over a width, and the
    // width changes from place to place (the flame's height, the distance
    // from the eye): over the width itself, kilometres from the origin, a
    // metre along the ray moved the reading by whole periods, each sample
    // read the noise at random and the jitter drew other flames each frame
    // (measured with `marca`: a quarter of their opacity from a frame to
    // the next, a fortieth with the jitter still; user, 2026-11-05:
    // «continua a vibrare, si nota maggiormente a grandi distanze»). Over
    // a width that holds still instead, the power of two under it, faded
    // into the next near the top of its span.
    let octave = log2(wide);
    let under = exp2(floor(octave));
    let up = smoothstep(0.6, 1.0, fract(octave));
    // a flame a few march pixels tall shows no rising, see `tongues_at`
    let small = 1.0 - smoothstep(6.0, 24.0, hf / foot);
    var n = tongues_at(pos, yr, under, foot, small, t);
    if up > 0.0 {
        let above = tongues_at(pos, yr, under * 2.0, foot, small, t);
        // two patterns mixed are flatter than either
        n = 0.5 + (mix(n, above, up) - 0.5) / sqrt(up * up + (1.0 - up) * (1.0 - up));
    }
    let noise_fl = vec3<f32>(n - (0.33 + 0.26 * yr + 0.20 * (1.0 - band)), yr, 1.0);
    if wf > 0.0 && fl.z > 0.0 {
        // the fluid adds to the tongues: the stronger flame of the two (the
        // fluid alone left the band behind the front without flames)
        let f = mix(noise_fl, fl, wf);
        return select(noise_fl, f, f.x > noise_fl.x);
    }
    return noise_fl;
}

// The mean of `smoothstep(0, w, f)` while `f` runs straight from `a` to
// `b`: how much of a stretch of the ray is inside an edge that its two
// ends see at `a` and `b`.
fn edge_mean(a: f32, b: f32, w: f32) -> f32 {
    if abs(b - a) < 1.0e-4 {
        return smoothstep(0.0, w, 0.5 * (a + b));
    }
    return (edge_sum(b, w) - edge_sum(a, w)) / (b - a);
}

// The integral of `smoothstep(0, w, f)` up to `f`.
fn edge_sum(f: f32, w: f32) -> f32 {
    let x = clamp(f / w, 0.0, 1.0);
    return w * x * x * x * (1.0 - 0.5 * x) + max(f - w, 0.0);
}

// Flame heights a second the pattern rises where the flame is a few march
// pixels tall (1.1 where it is tall).
const FAR_PACE = 0.25;

// The march pixel's place on the screen, as full-resolution pixels of
// the main pass.
fn ndc_of(full: vec2<f32>) -> vec2<f32> {
    let f = full / p.size.zw;
    return vec2<f32>(f.x * 2.0 - 1.0, 1.0 - f.y * 2.0);
}

// A per-pixel jitter of the first step, moved on each frame when the
// history averages it (golden ratio), still otherwise.
fn jitter(frag: vec2<f32>) -> f32 {
    let ign = fract(52.9829189 * fract(0.06711056 * frag.x + 0.00583715 * frag.y));
    return fract(ign + p.frame.x * 0.6180339);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> Out {
    var out: Out;
    // the scene's depths under this pixel, from the full-resolution buffer
    let full = in.position.xy * p.origin.z;
    let d = i32(p.origin.z);
    let foot_z = footprint(vec2<i32>(floor(in.position.xy)) * d, d, vec2<i32>(clamp(full, vec2<f32>(0.0), p.size.zw - 1.0)));
    let ndc = ndc_of(full);
    let eye = view.world_position;
    // The ray through this pixel, from the camera's turn alone (`rays`,
    // `fire_volumetric::rays_from_clip`): its run along the camera's axis
    // is 1. Not a point of the near plane less the eye: a decimetre apart
    // and kilometres from the world's origin, their difference kept a few
    // hundred steps of direction, and each ray pointed up to a march pixel
    // off its own (user, 2026-11-06: «vibra tutto ciò che è volumetrico …
    // nel lab non lo fa»: the lab is at the origin).
    let ray = (p.rays * vec4<f32>(ndc, 1.0, 1.0)).xyz;
    let dir = normalize(ray);
    // the depth is the distance along the camera's axis
    var t_scene = 1.0e7;
    let scene_z = foot_z.far;
    var hit = eye + dir * 1.0e6;
    if scene_z < FAR_M {
        t_scene = scene_z * length(ray);
        hit = eye + dir * t_scene;
    }
    // the nearest surface, where the footprint has two
    let split = foot_z.split;
    let t_near = select(1.0e7, foot_z.near * length(ray), foot_z.near < FAR_M);
    // what the ray held when it reached it
    var snapped = !split;
    var n_col = vec3<f32>(0.0);
    var n_trans = 1.0;
    var n_fog_at = vec3<f32>(0.0);
    var n_fog_w = 0.0;
    var n_fl_w = 0.0;
    var col = vec3<f32>(0.0);
    var trans = 1.0;
    var fog_at = vec3<f32>(0.0);
    var fog_w = 0.0;
    var fl_w = 0.0;
    var fl_s = 0.0;
    var fl_yr = 0.0;
    // what the steps were spent on (`VolumeSettings::count`): all, on
    // nothing, on smoke too thin to show, on smoke shaded, of which
    // behind smoke that hides seven tenths, in the flames' reach
    var n_all = 0.0;
    var n_empty = 0.0;
    var n_thin = 0.0;
    var n_shaded = 0.0;
    var n_dim = 0.0;
    // samples that found flame (`VolumeSettings::mark`)
    var n_hit = 0.0;
    // steps over a flame's band
    var n_band = 0.0;
    var n_flame = 0.0;
    var in_box = 0.0;

    // the plume's box
    let safe = select(dir, vec3<f32>(1e-6), abs(dir) < vec3<f32>(1e-6));
    let inv = 1.0 / safe;
    let a = (p.lo.xyz - eye) * inv;
    let b = (p.hi.xyz - eye) * inv;
    let tmin = min(a, b);
    let tmax = max(a, b);
    let t0 = max(max(tmin.x, tmin.y), max(tmin.z, 0.0));
    let t1 = min(min(min(tmax.x, tmax.y), tmax.z), t_scene);
    if t1 > t0 {
        in_box = 1.0;
        // first to the nearest surface, then on from it to the farthest
        var t_end = t1;
        if !snapped {
            if t_near <= t0 {
                snapped = true;
            } else {
                t_end = min(t_near, t1);
            }
        }
        var sun_dir = vec3<f32>(0.0, 1.0, 0.0);
        var sun_col = vec3<f32>(0.0);
        if lights.n_directional_lights > 0u {
            let light = lights.directional_lights[0];
            sun_dir = light.direction_to_light;
            sun_col = light.color.rgb * view.exposure * 0.3183;
        }
        let ambient = lights.ambient_color.rgb * view.exposure;
        // forward scattering: the plume glows at its rim against the sun
        let mu = dot(dir, sun_dir);
        let phase = 0.7 + 0.9 * pow(max(mu, 0.0), 6.0);
        let brown = vec3<f32>(0.17, 0.14, 0.11);
        let grey = vec3<f32>(0.52, 0.52, 0.53);
        let glow_col = vec3<f32>(1.4, 0.48, 0.10);
        let sigma = p.smoke.x;
        let zone = p.hi.w;
        let t = globals.time;
        let len = t1 - t0;
        let coarse0 = max(len / (p.lo.w * 0.7), 4.0);
        var s = t0;
        var first = true;
        let max_steps = i32(p.lo.w);
        for (var i = 0; i < max_steps; i++) {
            if s >= t_end && !snapped {
                // at the nearest surface: what the ray holds is its, and
                // the ray goes on from there (a stride that went past it
                // only counted the smoke up to it)
                snapped = true;
                n_col = col;
                n_trans = trans;
                n_fog_at = fog_at;
                n_fog_w = fog_w;
                n_fl_w = fl_w;
                s = min(s, t_near);
                t_end = t1;
            }
            if s >= t_end || trans < 0.02 {
                break;
            }
            let pos = eye + dir * s;
            let h = pos.y - ground(pos.xz);
            // steps: coarse through the smoke, fine through the flames,
            // never jumping over the flames' zone on the way down. The
            // coarse step is the ray's length shared among the steps, but
            // no shorter than `stride.x`: the billows have nothing finer,
            // and a short ray near the ground took as many steps as one
            // through the whole plume
            let coarse = clamp(max(coarse0, s * 0.02), p.stride.x, 90.0);
            // the flames grow toward the toy scale with distance, as the
            // tongues and the houses do: a true flame from the play camera
            // is a few pixels (`VolumeSettings::flame_toy`)
            let grow = 1.0 + (p.more.x - 1.0) * smoothstep(p.flame.z, p.flame.w, s);
            let fine = clamp(s * 0.005, 0.8, 6.0) * (0.5 + 0.5 * grow);
            var ds = coarse;
            var in_flames = false;
            // the tallest flame round here (m over the ground)
            var cap = 0.0;
            if zone > 0.0 && h < zone * grow {
                // the tallest flame round here: the cell's own, or three
                // times the mean over 80 m (the mip averages the band with
                // the ground beside it)
                let here = plume(pos.xz, 0.0);
                let round = plume(pos.xz, 2.0);
                cap = max(here.b, round.b * 3.0) * grow * p.look2.y + 2.0;
                if h < cap && round.g > 0.0005 {
                    in_flames = true;
                    // fine steps only over a cell alight (and its edge),
                    // half a cell elsewhere in the band's reach
                    ds = select(max(fine, p.map.z * 0.5), fine, here.g > 0.005);
                } else if dir.y < 0.0 {
                    ds = clamp((h - cap) / -dir.y, fine, coarse);
                }
            } else if zone > 0.0 && dir.y < 0.0 {
                ds = min(coarse, max((h - zone * grow) / -dir.y, fine));
            }
            if first {
                // a jittered start: no bands
                first = false;
                s += ds * jitter(in.position.xy);
                continue;
            }
            // metres a march pixel spans here
            let foot = s * p.frame.y;

            // the flames
            let show = u32(p.frame.z);
            if in_flames && (show & 1u) != 0u {
                let sharp = p.look2.x;
                // A long step through the flames (from far it is 12 m)
                // found flame in one or two samples of a ray: the jitter
                // added one or took one away, and the history could not
                // hold what changed so much (measured with `marca`, the
                // world still, from 2.5 km: a third of the flames'
                // opacity from each frame to the next with one sample a
                // step, a tenth so; both only once the pattern's width
                // holds still, `flame_at`). The step is cut in parts of
                // 3.5 m at most, and the flame's edge is not read at a
                // point but averaged along each part, as if what it is
                // cut from ran straight from one end to the other: what a
                // part holds changes smoothly as the ray slides.
                //
                // The step that reaches the scene ends half a fine step
                // behind it, wherever it began. Counted whole, it ran into
                // the ground as far as the jitter had it, through the
                // flame's foot, its brightest part: with the flames
                // standing on the ground (`ground`) they changed half as
                // much again from frame to frame, from 2.5 km. Stopped at
                // the scene, the far flames lost most of their light: a
                // flame a few march pixels tall shows little else.
                var run = ds;
                if s + ds >= t_end {
                    run = t_end - s + 0.5 * fine;
                }
                let parts = clamp(i32(ceil(run / 3.5)), 1, 4);
                let dpart = run / f32(parts);
                var a = flame_at(pos, h, foot, grow, t);
                n_band += a.z;
                if parts == 1 {
                    if a.z > 0.0 {
                        let dens = smoothstep(0.0, sharp, a.x) * (1.0 - a.y * a.y * a.y);
                        if dens > 0.002 {
                            n_hit += 1.0;
                            let temp = clamp((1.0 - 0.75 * a.y) * (0.25 + 0.55 * dens) * p.look2.z, 0.0, 1.0);
                            let absorbed = 1.0 - exp(-dens * run * p.flame.y);
                            col += flame_ramp(temp) * p.flame.x * absorbed * trans;
                            fog_at += pos * absorbed * trans;
                            fog_w += absorbed * trans;
                            fl_w += absorbed * trans;
                            fl_s += s * absorbed * trans;
                            fl_yr += a.y * absorbed * trans;
                            trans *= 1.0 - absorbed;
                        }
                    }
                } else {
                    for (var k = 1; k <= parts; k++) {
                        let sk = s + dpart * f32(k);
                        let pk = eye + dir * sk;
                        let b = flame_at(pk, pk.y - ground(pk.xz), foot, grow, t);
                        if a.z > 0.0 || b.z > 0.0 {
                            // no flame at an end: under the edge there
                            let fa = select(-sharp, a.x, a.z > 0.0);
                            let fb = select(-sharp, b.x, b.z > 0.0);
                            let yr = (a.y * a.z + b.y * b.z) / (a.z + b.z);
                            let dens = edge_mean(fa, fb, sharp) * (1.0 - yr * yr * yr);
                            if dens > 0.002 {
                                n_hit += 1.0;
                                let temp = clamp((1.0 - 0.75 * yr) * (0.25 + 0.55 * dens) * p.look2.z, 0.0, 1.0);
                                let absorbed = 1.0 - exp(-dens * dpart * p.flame.y);
                                let sm = sk - dpart * 0.5;
                                col += flame_ramp(temp) * p.flame.x * absorbed * trans;
                                fog_at += (eye + dir * sm) * absorbed * trans;
                                fog_w += absorbed * trans;
                                fl_w += absorbed * trans;
                                fl_s += sm * absorbed * trans;
                                fl_yr += yr * absorbed * trans;
                                trans *= 1.0 - absorbed;
                            }
                        }
                        a = b;
                    }
                }
            }

            // the smoke, thinned round the eye
            let clear = p.frame.w;
            let near_k = select(1.0, smoothstep(clear * 0.25, clear, s), clear > 1.0);
            n_all += 1.0;
            n_flame += f32(in_flames);
            // How far a stride longer than the step may go: not into
            // the flames. A stride that lands past their top skips their
            // tips, by as much as the jitter has it, and they flicker
            // (2026-11-05: the flames changed half as much again from each
            // frame to the next). Above the zone, down to its top; in it,
            // where no flame is in reach (`round`, the map over 80 m: none
            // within two cells), down to the tallest flame's height and
            // two cells along the ground.
            let free = !in_flames;
            var reach = 1.0e9;
            if zone > 0.0 {
                if h >= zone * grow {
                    if dir.y < 0.0 {
                        reach = max((h - zone * grow) / -dir.y, ds);
                    }
                } else {
                    reach = max(2.0 * p.map.z / max(length(dir.xz), 0.05), ds);
                    if dir.y < 0.0 {
                        reach = min(reach, max((h - cap) / -dir.y, ds));
                    }
                }
            }
            if near_k <= 0.0 || (show & 2u) == 0u {
                // none drawn here: nothing read, only the stride (not cut
                // to end where the smoke begins: every ray would sample
                // the same shells round the eye from there, and they show)
                if free {
                    ds = min(ds * 1.8, reach);
                }
                n_empty += 1.0;
            } else {
                let body = smoke_body(pos, h, true);
                if body.dens < 0.0 {
                    n_empty += 1.0;
                    if free {
                        // empty: a longer stride. Nothing of the smoke is
                        // narrower than what the map was averaged over at
                        // this height, so a stride of a part of that
                        // cannot step over it.
                        var far = max(ds * 1.8, min(body.wide * p.stride.y, 300.0));
                        // Near the ground that is a cell or two; there, if
                        // the map averaged eight times wider holds nothing
                        // either, nothing smokes for 2.5 times that width
                        // round here (less the sway and the drift's slant)
                        let c = ceil(log2(body.wide / p.map.z) + 0.5);
                        if p.stride.y > 0.0 && far < 60.0 && c + 3.0 < p.map.w {
                            if plume(body.src, c + 3.0).r < 8.0e-6 {
                                far = max(far, 1.4 * exp2(c) * p.map.z - 30.0);
                            }
                        }
                        ds = min(far, reach);
                    }
                } else {
                    // thin smoke in longer steps, up to three times: each
                    // step takes its share of the light whatever its
                    // length, and what a step of thin smoke adds is under
                    // the eye's notice (2026-11-05: inside the plume three
                    // steps in four shaded a haze)
                    if free && p.stride.z > 0.0 {
                        ds = min(ds * clamp(p.stride.z / max(body.dens * sigma * ds, 1.0e-5), 1.0, p.stride.w), reach);
                    }
                    // the billows averaged over what one sample stands for:
                    // the pixel's width and part of the stride (the jitter
                    // and the history average the rest); sampled finer, the
                    // smoke sparkles as the camera moves
                    let slod = log2(max(max(foot, ds * 0.35) * 48.0 / p.smoke.y, 1.0));
                    let dens = body.dens * billows(pos, h, slod);
                    if dens <= 0.002 {
                        n_thin += 1.0;
                    } else {
                        n_shaded += 1.0;
                        n_dim += f32(trans < 0.3);
                        let hh = max(h, 0.0);
                        let k = clamp(h / p.wind.w, 0.0, 1.0);
                        // two taps toward the sun: the plume shades itself
                        var shade = 1.0;
                        if p.look.z > 0.0 {
                            let d1 = max(smoke_body(pos + sun_dir * 35.0, h + sun_dir.y * 35.0, false).dens, 0.0);
                            let d2 = max(smoke_body(pos + sun_dir * 110.0, h + sun_dir.y * 110.0, false).dens, 0.0);
                            shade = exp(-sigma * (d1 * 48.0 + d2 * 78.0) * p.look.z);
                        }
                        let albedo = mix(brown, grey, smoothstep(0.0, 0.5, k));
                        let lit = albedo * (ambient * (0.45 + 0.4 * k) * (0.5 + 0.5 * shade) + sun_col * shade * phase * 0.9);
                        // lit from below by the alight ground under it,
                        // widening with height
                        var glow = 0.0;
                        if hh < 300.0 {
                            glow = plume(pos.xz, 1.0 + hh / 40.0).g * exp(-hh / 70.0);
                        }
                        let fire_lit = glow_col * glow * 0.9;
                        // no farther than the scene: a long stride of thin
                        // smoke ends in the ground
                        let absorbed = 1.0 - exp(-dens * near_k * min(ds, t_end - s) * sigma);
                        col += (lit + fire_lit) * p.smoke.w * absorbed * trans;
                        fog_at += pos * absorbed * trans;
                        fog_w += absorbed * trans;
                        trans *= 1.0 - absorbed;
                    }
                }
            }
            s += ds;
        }
    }
    if !snapped {
        // the ray ended before the nearest surface (or never met the box)
        n_col = col;
        n_trans = trans;
        n_fog_at = fog_at;
        n_fog_w = fog_w;
        n_fl_w = fl_w;
    }
    let alpha = 1.0 - trans;
    var now = vec4<f32>(0.0);
    var mid = hit;
    if alpha >= 0.002 {
        // the scene's fog, at the volume's weighted middle
        mid = fog_at / max(fog_w, 1e-4);
        let fogged = apply_scene_fog(vec4<f32>(col / alpha, alpha), mid);
        now = vec4<f32>(fogged.rgb * alpha, alpha);
    }
    var near = now;
    if split {
        near = vec4<f32>(0.0);
        let n_alpha = 1.0 - n_trans;
        if n_alpha >= 0.002 {
            let n_mid = n_fog_at / max(n_fog_w, 1e-4);
            let fogged = apply_scene_fog(vec4<f32>(n_col / n_alpha, n_alpha), n_mid);
            near = vec4<f32>(fogged.rgb * n_alpha, n_alpha);
        }
    }

    out.color = now;
    out.near = near;
    out.near_aux = vec2<f32>(foot_z.near, select(fl_w, n_fl_w, split));
    let count = (u32(p.frame.z) >> 2u) & 3u;
    if count != 0u {
        var c = vec2<f32>(n_all, n_empty);
        if count == 2u {
            c = vec2<f32>(n_shaded, n_dim);
        } else if count == 3u {
            c = vec2<f32>(n_thin, n_flame);
        }
        out.color = vec4<f32>(c / 128.0, 1.0, 1.0) * in_box;
        out.near = out.color;
    }
    // markers (`VolumeSettings::mark`): the flames' opacity, the samples
    // that found flame, the smoke's opacity
    let mark = (u32(p.frame.z) >> 5u) & 3u;
    if mark != 0u {
        out.color = vec4<f32>(fl_w, n_hit / 8.0, max(fog_w - fl_w, 0.0), alpha);
        out.near = select(out.color, vec4<f32>(n_fl_w, 0.0, max(n_fog_w - n_fl_w, 0.0), 1.0 - n_trans), split);
    }
    // 3: the steps in the flames' reach (each a 32nd), those of them over
    // a flame's band (each a 16th)
    if mark == 3u {
        out.color = vec4<f32>(fl_w, n_flame / 32.0, n_band / 16.0, 1.0) * in_box;
        out.near = out.color;
    }
    let fl_d = select(0.0, fl_s / max(fl_w, 1e-4), fl_w > 1e-4);
    // the scene's depth below 0 where the pixel has a nearer surface
    out.aux = vec4<f32>(select(scene_z, -scene_z, split), fl_w, min(fl_d, FAR_M), select(0.0, fl_yr / max(fl_w, 1e-4), fl_w > 1e-4));
    out.mid = min(distance(mid, eye), 1.0e6);
    return out;
}
