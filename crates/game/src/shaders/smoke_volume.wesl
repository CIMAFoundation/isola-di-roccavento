// Volumetric smoke (2026-11-04): the fire's plume as a density field
// instead of hundreds of puffs.
//
// `fire_view::update_smoke_volumes` covers the smoking ground with boxes
// of a block of cells each, sheared downwind. The smoke in a box is the
// smoke that left that block's own ground: a parcel found at height `h`
// above the ground left it `h / rise` seconds ago and has drifted that long
// with the wind, so the ground it came from is the point de-sheared by
// that drift, and the fire map there says how much is smoking. The column
// so drawn leans with the wind, thins and greys with height, and is torn
// by 3D noise advected on the wind. Lit by the sky and the sun, with one
// tap toward the sun through the plume for its self-shadow: dark on the
// far side, bright on top; and orange from below where the ground is
// alight. Blocks partition the source, so overlapping boxes never draw
// the same smoke twice.
//
// Cost: a march of a few steps per box (the boxes are hundreds of metres
// across), each a ground lookup, two fire-map samples and a noise fetch.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::mesh_view_bindings::{view, globals, lights};
import game::shaders::scene_light::apply_scene_fog;

struct Params {
    // 1 / world width, 1 / world height (the fire map's uv), the terrain's
    // posting (m) and its height_m (world y of row 0, the north edge).
    rect: vec4<f32>,
    // the plume's top (m above the ground), its rise (m/s), seconds a
    // cell smokes, metres a step
    plume: vec4<f32>,
    // the wind's drift (Bevy x, z, m/s), its speed, the lateral margin (m)
    wind: vec4<f32>,
    // extinction (/ m at full density), noise period (m), gain, the
    // seconds a cell flames (`FLAMING_S`)
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
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var flame_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var flame_samp: sampler;

// A box: position on its surface; the normal slot carries the centre of
// its block of ground (x, the ground's mean y, z), the uv the block's half
// extents, the colour its box's centre y and half height.
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) foot: vec3<f32>,
    @location(2) half_xz: vec2<f32>,
    @location(5) info: vec4<f32>,
}

struct VolOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) @interpolate(flat) foot: vec3<f32>,
    @location(2) @interpolate(flat) half_xz: vec2<f32>,
    @location(3) @interpolate(flat) box_y: vec2<f32>,
}

@vertex
fn vertex(v: Vertex) -> VolOut {
    var out: VolOut;
    let world = mesh_position_local_to_world(
        get_world_from_local(v.instance_index),
        vec4<f32>(v.position, 1.0),
    ).xyz;
    out.world_position = world;
    out.foot = v.foot;
    out.half_xz = v.half_xz;
    out.box_y = v.info.xy;
    out.clip = position_world_to_clip(world);
    return out;
}

fn terrain_h(ix: i32, iz: i32, dims: vec2<i32>) -> f32 {
    let t = textureLoad(height_tex, clamp(vec2<i32>(ix, iz), vec2<i32>(0), dims - 1), 0);
    return params.elev.x + (t.r * 255.0 * 256.0 + t.g * 255.0) * params.elev.y;
}

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

// What the ground at `xz` puts into the plume: x the smoke, 1 for a cell
// the front just crossed down to a smoulder at the end of the smoking
// time, weighted by the fire's intensity; y how alight it is (the flames'
// light from below).
fn source(xz: vec2<f32>) -> vec2<f32> {
    let uv = vec2<f32>(xz.x, -xz.y) * params.rect.xy;
    let f = textureSampleLevel(fire_tex, fire_samp, uv, 0.0);
    let since = -f.b / max(f.g, 1e-3);
    let burnt = smoothstep(0.2, 0.6, f.g);
    let smoking = clamp(1.0 - since / params.plume.z, 0.0, 1.0);
    let flaming = clamp(1.0 - since / params.look.w, 0.0, 1.0);
    // the plume's strength: the flames' height stands for the intensity
    let strength = clamp(textureSampleLevel(flame_tex, flame_samp, uv, 0.0).r / 10.0, 0.3, 1.2);
    return vec2<f32>(burnt * strength * smoking * (0.35 + 0.65 * flaming), burnt * flaming);
}

// The smoke's density at `p`, `h` above the ground, from the source
// de-sheared by the drift since it left the ground; `with_noise` tears it.
// Zero outside the box's own block of ground.
fn density(p: vec3<f32>, h: f32, foot: vec3<f32>, half_xz: vec2<f32>, with_noise: bool) -> vec2<f32> {
    let top = params.plume.x;
    if h <= 0.0 || h >= top {
        return vec2<f32>(0.0);
    }
    let tt = h / params.plume.y;
    let t = globals.time;
    let k = h / top;
    // the column wanders as it rises: a slow noise sways the de-shear
    let sway = (textureSampleLevel(noise_tex, noise_samp, vec3<f32>(p.x, h * 1.5, p.z) / 260.0 + vec3<f32>(0.0, -t * 0.04, 0.0), 0.0).gb - 0.5) * 2.0 * params.wind.w * (0.15 + 0.85 * k);
    let src = p.xz - params.wind.xy * tt - sway;
    let d = abs(src - foot.xz) - half_xz;
    if max(d.x, d.y) > 0.0 {
        return vec2<f32>(0.0);
    }
    let s = source(src);
    if s.x < 0.01 {
        return vec2<f32>(0.0);
    }
    // thins with height, more so once the smoke has spread
    var dens = s.x * pow(1.0 - k, 1.4);
    if with_noise {
        // billows, advected on the wind and rising with the plume
        let adv = vec3<f32>(params.wind.x, params.plume.y * 0.6, params.wind.y) * t;
        let q = (p - adv) / params.look.y;
        let n1 = textureSampleLevel(noise_tex, noise_samp, q, 0.0).r;
        let n2 = textureSampleLevel(noise_tex, noise_samp, q * 2.7 + vec3<f32>(0.0, -t * 0.05, 0.0), 0.0).g;
        let n = 0.65 * n1 + 0.35 * n2;
        // dense at the source, torn to shreds at the top
        dens *= smoothstep(0.33 + 0.3 * k, 0.68 + 0.3 * k, n + 0.15 * (1.0 - k));
    } else {
        dens *= 0.55;
    }
    return vec2<f32>(dens, s.y);
}

fn jitter(frag: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(0.06711056 * frag.x + 0.00583715 * frag.y));
}

@fragment
fn fragment(in: VolOut) -> @location(0) vec4<f32> {
    let eye = view.world_position;
    var dir = in.world_position - eye;
    dir = dir / max(length(dir), 1e-3);
    // the box: the block, sheared by the drift to the plume's top, and a
    // margin for the sway (as `fire_view::smoke_box`)
    let shift = params.wind.xy * (params.plume.x / params.plume.y);
    let m = params.wind.w;
    let lo_xz = min(in.foot.xz, in.foot.xz + shift) - in.half_xz - m;
    let hi_xz = max(in.foot.xz, in.foot.xz + shift) + in.half_xz + m;
    let box_lo = vec3<f32>(lo_xz.x, in.box_y.x - in.box_y.y, lo_xz.y);
    let box_hi = vec3<f32>(hi_xz.x, in.box_y.x + in.box_y.y, hi_xz.y);
    let safe = select(dir, vec3<f32>(1e-5), abs(dir) < vec3<f32>(1e-5));
    let inv = 1.0 / safe;
    let lo = (box_lo - eye) * inv;
    let hi = (box_hi - eye) * inv;
    let tmin = min(lo, hi);
    let tmax = max(lo, hi);
    let t0 = max(max(tmin.x, tmin.y), max(tmin.z, 0.0));
    let t1 = min(min(tmax.x, tmax.y), tmax.z);
    if t1 <= t0 {
        discard;
    }
    let len = t1 - t0;
    let steps = clamp(i32(len / params.plume.w), 6, 32);
    let ds = len / f32(steps);
    let sigma = params.look.x;
    let gain = params.look.z;

    var sun_dir = vec3<f32>(0.0, 1.0, 0.0);
    var sun_col = vec3<f32>(0.0);
    if lights.n_directional_lights > 0u {
        let light = lights.directional_lights[0];
        sun_dir = light.direction_to_light;
        sun_col = light.color.rgb * view.exposure * 0.3183;
    }
    let ambient = lights.ambient_color.rgb * view.exposure;
    // forward scattering: the plume glows at its rim against the sun
    let phase = 0.75 + 0.6 * pow(max(dot(dir, sun_dir), 0.0), 8.0);
    let brown = vec3<f32>(0.12, 0.09, 0.07);
    let grey = vec3<f32>(0.50, 0.50, 0.51);
    let glow = vec3<f32>(1.2, 0.42, 0.09);

    var col = vec3<f32>(0.0);
    var trans = 1.0;
    var mid = eye + dir * (t0 + len * 0.5);
    var s = t0 + ds * 0.5 * jitter(in.clip.xy);
    for (var i = 0; i < steps; i++) {
        let p = eye + dir * s;
        let ground = ground_h(p.x, p.z);
        let h = p.y - ground;
        // into the hill: nothing behind it
        if h < 0.0 {
            break;
        }
        let dn = density(p, h, in.foot, in.half_xz, true);
        if dn.x > 0.002 {
            let k = h / params.plume.x;
            // one tap toward the sun: the plume shades itself
            let ps = p + sun_dir * 45.0;
            let shade = exp(-density(ps, h + sun_dir.y * 45.0, in.foot, in.half_xz, false).x * 45.0 * sigma);
            let albedo = mix(brown, grey, smoothstep(0.0, 0.55, k));
            let lit = albedo * (ambient * (0.45 + 0.55 * k) + sun_col * shade * phase * 0.7);
            // lit orange from below by the flames, low in the column
            let fire_lit = glow * dn.y * exp(-h / 18.0) * 0.6;
            let od = dn.x * ds * sigma;
            let absorbed = 1.0 - exp(-od);
            col += (lit + fire_lit) * gain * absorbed * trans;
            trans *= 1.0 - absorbed;
            if trans < 0.02 {
                mid = p;
                break;
            }
        }
        s += ds;
    }
    let alpha = 1.0 - trans;
    if alpha < 0.004 {
        discard;
    }
    // the scene's fog, at the body's middle
    let fogged = apply_scene_fog(vec4<f32>(col / alpha, alpha), mid);
    return vec4<f32>(fogged.rgb, alpha);
}
