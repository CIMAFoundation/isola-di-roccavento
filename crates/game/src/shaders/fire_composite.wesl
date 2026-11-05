// The ray-marched flames and smoke (`fire_march.wesl`), from their small
// target over the scene (`fire_volumetric.rs`). Each screen pixel reads the
// 3x3 march texels round it, weighted by distance (a blur that hides the
// march's jitter) and by how close their depth is to its own (so the smoke
// behind a tree does not bleed over it). Where no texel has a depth like
// its own, the closest in depth stands for it. Blended premultiplied.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

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
    world_from_clip: mat4x4<f32>,
    eye: vec4<f32>,
    more: vec4<f32>,
}

@group(0) @binding(0) var<uniform> p: Params;
@if(MULTISAMPLED)
@group(0) @binding(1) var depth_tex: texture_depth_multisampled_2d;
@else
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var vol_tex: texture_2d<f32>;
@group(0) @binding(3) var vol_depth: texture_2d<f32>;
@group(0) @binding(4) var vol_mid: texture_2d<f32>;
@group(0) @binding(5) var noise_tex: texture_3d<f32>;
@group(0) @binding(6) var noise_samp: sampler;

// Detail finer than the march, carved into the flames at the screen's
// resolution (2026-11-05): their opacity, upsampled soft from the march,
// is pushed through a sharp step by a rising noise anchored in the world
// where the flames are, so their edges break into licks a few pixels wide.
// Torn more toward the tips. The smoke is left as it is.
fn carve(col: vec4<f32>, fl: vec3<f32>, pix: vec2<f32>) -> vec4<f32> {
    let amount = p.more.y;
    let fa = fl.x;
    if amount <= 0.0 || fa < 0.004 || fl.y <= 0.0 {
        return col;
    }
    // where the flames are, in the world
    let f = (pix - p.origin.xy) / p.size.zw;
    let ndc = vec2<f32>(f.x * 2.0 - 1.0, 1.0 - f.y * 2.0);
    let far = p.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - p.eye.xyz);
    let pos = p.eye.xyz + dir * fl.y;
    // a screen pixel there (m), and licks never finer than a few of them
    let foot = fl.y * p.frame.y / p.origin.z;
    let size_m = max(p.more.z, foot * 4.0);
    let lod = log2(max(foot * 48.0 / size_m, 1.0));
    let t = p.more.w;
    // tall and narrow, rising
    let q = vec3<f32>(pos.x / size_m, pos.y / (size_m * 4.0) - t * 1.3, pos.z / size_m);
    let w = noise_tex_at(q * 0.43 + vec3<f32>(0.0, -t * 0.4, 0.0), lod).gb - 0.5;
    var n = 0.75 * noise_tex_at(q + vec3<f32>(w.x, 0.0, w.y) * 0.8, lod).r;
    n += 0.25 * noise_tex_at(q * vec3<f32>(2.3, 1.9, 2.3) + vec3<f32>(w.y, -t * 1.1, w.x), lod + 1.2).g;
    // a soft fringe keeps some of itself: the step is taken relative to
    // what is there, torn harder up the tongue
    let yr = clamp(fl.z, 0.0, 1.0);
    let tear = amount * (0.55 + 0.9 * yr);
    let v = fa + (n - 0.5) * tear * (0.35 + fa);
    let edge = 0.30 * fa + 0.02;
    let carved = smoothstep(edge - 0.04, edge + 0.04, v) * min(fa * 1.3 + 0.08, 1.0);
    // and the light inside: hot streaks rising through it
    let streak = smoothstep(0.25, 0.85, n);
    let k = clamp(carved / fa, 0.0, 2.2) * mix(1.0, 0.45 + 1.1 * streak, min(amount, 1.0));
    // the flames' part of the light: their share of the opacity
    let share = clamp(fa / max(col.a, 1e-4), 0.0, 1.0);
    let rgb = col.rgb * (1.0 + share * (k - 1.0));
    let a = clamp(col.a + fa * (clamp(carved / fa, 0.0, 2.2) - 1.0), 0.0, 1.0);
    return vec4<f32>(rgb, a);
}

fn noise_tex_at(q: vec3<f32>, lod: f32) -> vec4<f32> {
    return textureSampleLevel(noise_tex, noise_samp, q, lod);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pix = in.position.xy;
    let d = textureLoad(depth_tex, vec2<i32>(pix), 0);
    var z = 1.0e7;
    if d > 0.0 {
        z = p.origin.w / d;
    }
    // this pixel's place on the march's grid
    let q = (pix - p.origin.xy) / p.origin.z - 0.5;
    let c = vec2<i32>(round(q));
    let last = vec2<i32>(p.size.xy) - 1;
    var sum = vec4<f32>(0.0);
    var fsum = vec3<f32>(0.0);
    var wsum = 0.0;
    var best = vec4<f32>(0.0);
    var best_f = vec3<f32>(0.0);
    var best_dz = 1.0e30;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let at = clamp(c + vec2<i32>(i, j), vec2<i32>(0), last);
            let v = textureLoad(vol_tex, at, 0);
            let zl = textureLoad(vol_depth, at, 0).r;
            let fm = textureLoad(vol_mid, at, 0).yzw;
            let off = vec2<f32>(at) - q;
            let ws = exp(-dot(off, off) * 1.4);
            let dz = abs(zl - z) / max(min(z, zl), 1.0);
            let wd = exp(-dz * dz * 400.0);
            sum += v * ws * wd;
            wsum += ws * wd;
            // the flames' distance and height, weighted by their opacity
            fsum += vec3<f32>(fm.x, fm.x * fm.y, fm.x * fm.z) * ws * wd;
            if dz < best_dz {
                best_dz = dz;
                best = v;
                best_f = vec3<f32>(fm.x, fm.x * fm.y, fm.x * fm.z);
            }
        }
    }
    var col = best;
    var fl = best_f;
    if wsum > 0.05 {
        col = sum / wsum;
        fl = fsum / wsum;
    }
    if fl.x > 1e-4 {
        col = carve(col, vec3<f32>(fl.x, fl.y / fl.x, fl.z / fl.x), pix);
    }
    if col.a < 0.002 {
        discard;
    }
    return col;
}
