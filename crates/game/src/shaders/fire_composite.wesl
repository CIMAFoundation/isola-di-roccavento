// The ray-marched flames and smoke (`fire_march.wesl`), from their small
// target over the scene (`fire_volumetric.rs`). Each screen pixel reads the
// 3x3 march texels round it, weighted by distance (a blur that hides the
// march's jitter) and by how close their depth is to its own (so the smoke
// behind a tree does not bleed over it). Where no texel has a depth like
// its own, the closest in depth stands for it. Blended premultiplied.
//
// Most of the screen has no volume over it, and this pass runs at the
// screen's resolution (2026-11-05: at 9x the pixels it was two thirds of
// the volumes' cost): a pixel with nothing in the 3x3 texels round it
// leaves after four filtered taps. The pass still covers the whole screen:
// drawing it only over the plume's box is not done.

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
    rays: mat4x4<f32>,
    eye: vec4<f32>,
    more: vec4<f32>,
    stride: vec4<f32>,
}

@group(0) @binding(0) var<uniform> p: Params;
@if(MULTISAMPLED)
@group(0) @binding(1) var depth_tex: texture_depth_multisampled_2d;
@else
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var vol_tex: texture_2d<f32>;
// the scene's depth the march stopped at (m), then the flames' share of
// the opacity, their distance (m) and how high up their tongues
@group(0) @binding(3) var vol_aux: texture_2d<f32>;
@group(0) @binding(4) var noise_tex: texture_3d<f32>;
@group(0) @binding(5) var noise_samp: sampler;
@group(0) @binding(6) var vol_samp: sampler;

// Detail finer than the march, carved into the flames at the screen's
// resolution (2026-11-05): their opacity, upsampled soft from the march,
// is pushed through a sharp step by a rising noise anchored in the world
// where the flames are, so their edges break into licks a few pixels wide.
// Torn more toward the tips. The smoke is left as it is.
fn carve(col: vec4<f32>, fl: vec3<f32>, pix: vec2<f32>) -> vec4<f32> {
    let fa = fl.x;
    if p.more.y <= 0.0 || fa < 0.004 || fl.y <= 0.0 {
        return col;
    }
    // a screen pixel at the flames (m). From far the licks cannot be their
    // size (`more.z`), only as many pixels: a few pixels wide and rising
    // tens of pixels a second, they made the fire shake (user, 2026-11-05:
    // «continua a vibrare, si nota maggiormente a grandi distanze»; from
    // 2.5 km they were most of what changed in the flames from a frame to
    // the next). Faded out as they outgrow their size.
    let foot = fl.y * p.frame.y / p.origin.z;
    let amount = p.more.y * (1.0 - smoothstep(1.0, 2.0, foot * 4.0 / p.more.z));
    if amount <= 0.0 {
        return col;
    }
    // where the flames are, in the world
    let f = (pix - p.origin.xy) / p.size.zw;
    let ndc = vec2<f32>(f.x * 2.0 - 1.0, 1.0 - f.y * 2.0);
    let dir = normalize((p.rays * vec4<f32>(ndc, 1.0, 1.0)).xyz);
    let pos = p.eye.xyz + dir * fl.y;
    // licks never finer than a few pixels
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

// The sky's depth, and the farthest the march writes (m): its depths are
// half floats (`fire_march.wesl`).
const FAR_M = 6.0e4;

fn noise_tex_at(q: vec3<f32>, lod: f32) -> vec4<f32> {
    return textureSampleLevel(noise_tex, noise_samp, q, lod);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pix = in.position.xy;
    // this pixel's place on the march's grid
    let q = (pix - p.origin.xy) / p.origin.z - 0.5;
    let c = vec2<i32>(round(q));
    let last = vec2<i32>(p.size.xy) - 1;
    // nothing in the 3x3 texels round it: four filtered taps at the centre
    // texel's corners weigh each of the nine a quarter or more, and what
    // the nine can make is discarded below under 0.002
    let corner = vec2<f32>(c) / p.size.xy;
    let texel = 1.0 / p.size.xy;
    let near_a = textureSampleLevel(vol_tex, vol_samp, corner, 0.0).a
        + textureSampleLevel(vol_tex, vol_samp, corner + vec2<f32>(texel.x, 0.0), 0.0).a
        + textureSampleLevel(vol_tex, vol_samp, corner + vec2<f32>(0.0, texel.y), 0.0).a
        + textureSampleLevel(vol_tex, vol_samp, corner + texel, 0.0).a;
    // `solo`: the volumes over black, not over the scene
    let solo = (u32(p.frame.z) & 16u) != 0u;
    if near_a < 0.0005 {
        if solo {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
        discard;
    }
    let d = textureLoad(depth_tex, vec2<i32>(pix), 0);
    var z = FAR_M;
    if d > 0.0 {
        z = min(p.origin.w / d, FAR_M);
    }
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
            let aux = textureLoad(vol_aux, at, 0);
            let zl = aux.r;
            let fm = aux.gba;
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
    // no carving of the markers, nor of the history's offset (`scarto`)
    if fl.x > 1e-4 && ((u32(p.frame.z) >> 5u) & 7u) == 0u {
        col = carve(col, vec3<f32>(fl.x, fl.y / fl.x, fl.z / fl.x), pix);
    }
    if solo {
        return vec4<f32>(col.rgb, 1.0);
    }
    if col.a < 0.002 {
        discard;
    }
    return col;
}
