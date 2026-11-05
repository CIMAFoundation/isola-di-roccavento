// Volumetric flames (2026-11-04, the user: «fiamme e fumo più volumetrici»).
//
// Near the camera the fire front is drawn as volumes instead of billboard
// tongues: `fire_view::update_volumes` tiles the flaming band with boxes of
// a few cells a side, and this shader ray-marches each box. The flame is a
// density field: where the fire map says the band is alight (the same map
// the terrain draws the glowing band from), rising 3D noise, curled by a
// slower noise, is thresholded by the height above the ground, so the
// flames are solid sheets at the foot that break into licks and wisps at
// the top. Emission goes white-yellow at the base, orange, then deep red at
// the tips, and the body absorbs a little of what is behind it. Everything
// is sampled in world space, so neighbouring boxes join without a seam and
// the fire keeps its shape as the camera turns.
//
// Cost: the fire map, the height map (twice per fragment) and two fetches
// of a 3D noise texture per step, ~12 steps, with an early exit once the
// body is opaque. Beyond `fade.z` metres the vertex stage drops the box.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::mesh_view_bindings::{view, globals};

struct Params {
    // 1 / world width, 1 / world height (the fire map's uv), the terrain's
    // posting (m) and its height_m (world y of row 0, the north edge).
    rect: vec4<f32>,
    // seconds a cell flames (`FLAMING_S`), fade start and end (m from the
    // eye), metres a step up close.
    fade: vec4<f32>,
    // emission gain, noise period (m), rise (periods / s), absorption (/ m).
    look: vec4<f32>,
    // the height texture's offset (m) and scale (m per code), spare.
    elev: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: Params;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var noise_tex: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var noise_samp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var fire_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var fire_samp: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var height_tex: texture_2d<f32>;
// the flame length (m) of every cell (`fire_view::FLAME_MAP`), filtered
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var flame_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var flame_samp: sampler;

// A box: position on its surface; the normal slot carries its centre, the
// uv its half extents in x and z, the colour its flame height (m), its heat,
// its half height and the kind (0 a box, 1 a quad across the eye).
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) centre: vec3<f32>,
    @location(2) half_xz: vec2<f32>,
    @location(5) info: vec4<f32>,
}

struct VolOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) @interpolate(flat) centre: vec3<f32>,
    @location(2) @interpolate(flat) half: vec3<f32>,
    // flame height (m), heat, distance fade, height scale
    @location(3) @interpolate(flat) info: vec4<f32>,
}

@vertex
fn vertex(v: Vertex) -> VolOut {
    var out: VolOut;
    let world = mesh_position_local_to_world(
        get_world_from_local(v.instance_index),
        vec4<f32>(v.position, 1.0),
    ).xyz;
    out.world_position = world;
    out.centre = v.centre;
    out.half = vec3<f32>(v.half_xz.x, v.info.z, v.half_xz.y);
    let d = distance(view.world_position, v.centre);
    let fade = 1.0 - smoothstep(params.fade.y, params.fade.z, d);
    // the flames grow with distance toward the tongues' toy scale, so the
    // far fire is not smaller than the near (`fire_view::TOY_SCALE_FAR_M`)
    let grow = 1.0 + 1.5 * smoothstep(150.0, params.fade.z, d);
    out.info = vec4<f32>(v.info.x * grow, v.info.y, fade, grow);
    // past the fade: no fragments at all
    if fade <= 0.0 {
        out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    } else {
        out.clip = position_world_to_clip(world);
    }
    return out;
}

fn terrain_h(ix: i32, iz: i32, dims: vec2<i32>) -> f32 {
    let t = textureLoad(height_tex, clamp(vec2<i32>(ix, iz), vec2<i32>(0), dims - 1), 0);
    return params.elev.x + (t.r * 255.0 * 256.0 + t.g * 255.0) * params.elev.y;
}

// The ground at a world position: bilinear over the terrain's own grid
// (row 0 at the north edge, as `scenario::Terrain::bilinear`).
fn ground_h(x: f32, z: f32) -> f32 {
    let dims = vec2<i32>(textureDimensions(height_tex));
    let fx = x / params.rect.z;
    let fz = (params.rect.w + z) / params.rect.z;
    let x0 = i32(floor(fx));
    let z0 = i32(floor(fz));
    let tx = fx - floor(fx);
    let tz = fz - floor(fz);
    let a = mix(terrain_h(x0, z0, dims), terrain_h(x0 + 1, z0, dims), tx);
    let b = mix(terrain_h(x0, z0 + 1, dims), terrain_h(x0 + 1, z0 + 1, dims), tx);
    return mix(a, b, tz);
}

// Emission of the flame by its temperature `h` (0 cold ... 1 the core):
// a dark red-brown that is already smoke, deep red, orange, yellow, and a
// small white-yellow core. Above 1 for the bloom, as the tongues' ramp.
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

// The fire at `xz` from the fire map: x how alight the ground is (1 just
// reached by the front, 0 once the flaming time is over or ahead of the
// front), y the flames' height there (m).
fn alight(xz: vec2<f32>) -> vec2<f32> {
    let uv = vec2<f32>(xz.x, -xz.y) * params.rect.xy;
    let f = textureSampleLevel(fire_tex, fire_samp, uv, 0.0);
    // b is the arrival against now, times g (`fire_view::update_fire_map`)
    let since = -f.b / max(f.g, 1e-3);
    let flaming = clamp(1.0 - since / params.fade.x, 0.0, 1.0) * smoothstep(0.2, 0.6, f.g);
    // the tongues stop at a fifth of the flaming time left; the flames of
    // a cell shrink as it burns out
    let band = smoothstep(0.12, 0.45, flaming);
    let h = textureSampleLevel(flame_tex, flame_samp, uv, 0.0).r * (0.5 + 0.5 * flaming);
    return vec2<f32>(band, max(h, 0.5));
}

// The noise's vertical scale against its horizontal: a flame's licks are
// about three times as tall as wide.
const STRETCH: f32 = 0.3;

// A stable per-pixel jitter (not per frame: a march that jitters every
// frame boils).
fn jitter(frag: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(0.06711056 * frag.x + 0.00583715 * frag.y));
}

@fragment
fn fragment(in: VolOut) -> @location(0) vec4<f32> {
    let eye = view.world_position;
    var dir = in.world_position - eye;
    dir = dir / max(length(dir), 1e-3);
    // the ray through the box (slabs)
    let safe = select(dir, vec3<f32>(1e-5), abs(dir) < vec3<f32>(1e-5));
    let inv = 1.0 / safe;
    let lo = (in.centre - in.half - eye) * inv;
    let hi = (in.centre + in.half - eye) * inv;
    let tmin = min(lo, hi);
    let tmax = max(lo, hi);
    var t0 = max(max(tmin.x, tmin.y), max(tmin.z, 0.0));
    var t1 = min(min(tmax.x, tmax.y), tmax.z);
    if t1 <= t0 {
        discard;
    }
    // the ground along the ray, from its height where the ray enters and
    // leaves the box (a straight line between: the box is a few cells)
    let p0 = eye + dir * t0;
    let p1 = eye + dir * t1;
    let g0 = ground_h(p0.x, p0.z);
    let g1 = ground_h(p1.x, p1.z);
    let a0 = p0.y - g0;
    let a1 = p1.y - g1;
    if a0 <= 0.0 && a1 <= 0.0 {
        discard;
    }
    // the march stops at the ground
    if a1 < 0.0 {
        t1 = t0 + (t1 - t0) * a0 / (a0 - a1);
    }
    if a0 < 0.0 {
        t0 = t0 + (t1 - t0) * (-a0) / (a1 - a0);
    }
    let len = t1 - t0;
    if len <= 0.05 {
        discard;
    }
    // nothing alight along this ray (the box's empty half, the ring round
    // the band): the common case, out before the march
    let mid = alight(mix(p0, p1, 0.5).xz).x;
    if mid + alight(p0.xz).x + alight(p1.xz).x < 0.02 {
        discard;
    }
    // a metre a step close by, coarser with distance: far off the licks
    // are a pixel wide anyway
    let step_m = max(params.fade.w, t0 * 0.012 * params.fade.w);
    let steps = clamp(i32(len / step_m), 4, 32);
    let ds = len / f32(steps);
    let gain = params.look.x * in.info.z;
    let inv_period = 1.0 / params.look.y;
    let t = globals.time;
    // flames are taller than wide: the noise is stretched upward, and it
    // rises at `look.z` periods a second (in world metres, not stretched)
    let stretch = vec3<f32>(1.0, STRETCH, 1.0);
    let rise = vec3<f32>(0.0, -t * params.look.z * STRETCH, 0.0);
    let heat = in.info.y;
    let h_box = in.info.x;
    // the fire comes in clumps along the band: a slow 2D noise over the
    // ground, read where the ray enters and leaves the box
    let patch0 = textureSampleLevel(noise_tex, noise_samp, vec3<f32>(p0.xz * 0.028, 0.37), 0.0).b;
    let patch1 = textureSampleLevel(noise_tex, noise_samp, vec3<f32>(p1.xz * 0.028, 0.37), 0.0).b;

    var col = vec3<f32>(0.0);
    var trans = 1.0;
    // `clip` is the fragment's position in this stage
    // half a step of jitter: enough against banding, and the dither does
    // not show as a screen of dots
    var s = t0 + ds * 0.5 * jitter(in.clip.xy);
    for (var i = 0; i < steps; i++) {
        let p = eye + dir * s;
        let k = (s - t0) / max(t1 - t0, 1e-3);
        let ground = mix(g0, g1, k);
        // above the tallest flame of the box: nothing to fetch
        if p.y - ground > h_box {
            s += ds;
            continue;
        }
        let fire = alight(p.xz);
        let clump = smoothstep(0.3, 0.75, mix(patch0, patch1, k));
        let band = fire.x * (0.25 + 0.75 * clump);
        let h_here = min(fire.y * in.info.w, h_box) * (0.6 + 0.4 * clump);
        let yr = (p.y - ground) / h_here;
        if band > 0.02 && yr > -0.02 && yr < 1.0 {
            let q = p * stretch * inv_period + rise;
            // a slow curl, then the body's own noise
            let w = textureSampleLevel(noise_tex, noise_samp, q * 0.31 + rise * 0.2, 0.0).gb - 0.5;
            let n = textureSampleLevel(noise_tex, noise_samp, q + vec3<f32>(w.x, 0.0, w.y) * 0.9, 0.0).r;
            // the threshold climbs with height: a wall at the foot, licks
            // and wisps at the tips; the tail of the flaming time thins it
            let thresh = 0.40 + 0.40 * yr + 0.25 * (1.0 - band);
            let dens = smoothstep(thresh, thresh + 0.3, n) * (1.0 - yr * yr * yr);
            if dens > 0.002 {
                // hotter low and inside the body; white only in the core
                let temp = clamp((1.0 - 0.7 * yr) * (0.3 + 0.6 * dens) * heat, 0.0, 1.0);
                // emission per unit of optical depth: the body saturates to
                // the ramp's colour, never past it
                let od = dens * ds * params.look.w;
                let absorbed = 1.0 - exp(-od);
                col += flame_ramp(temp) * gain * absorbed * trans;
                trans *= 1.0 - absorbed;
                if trans < 0.03 {
                    break;
                }
            }
        }
        s += ds;
    }
    if trans > 0.999 {
        discard;
    }
    return vec4<f32>(col, 1.0 - trans);
}
