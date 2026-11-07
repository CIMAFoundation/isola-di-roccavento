// The ray-marched flames and smoke (`fire_march.wesl`), from their small
// target over the scene (`fire_volumetric.rs`). Each screen pixel reads the
// 3x3 march texels round it, weighted by distance (a blur that hides the
// march's jitter) and by how close their depth is to its own (so the smoke
// behind a tree does not bleed over it). Each march texel holds two
// surfaces where its footprint has two, the nearer and the farther
// (`footprint` in `fire_march.wesl`), so a pixel always finds its own
// surface in its own texel: a gap in a crown is no longer the sky without
// its smoke, nor a twig thinner than a march texel the smoke behind it
// (2026-11-06; asking the sixteen texels round the nine, as was done
// before, still left gaps in crowns wider than them). Where none is of its
// surface (a third one in a footprint), the closest in depth stands for
// it. Blended premultiplied.
//
// Under multisampling a pixel at the rim of a leaf holds two surfaces, the
// leaf and what is behind it, and each has its own volume in front of it:
// the smoke over the sky must neither veil the leaf nor be missing from
// the sky. Composed over the resolved picture with one depth for the
// pixel, such a rim kept the sky without its smoke: every leaf against the
// smoke wore a pale outline (user, 2026-11-06). So there the pass draws
// into the samples, before they are resolved, and twice: the farther
// surface of each pixel, then (`NEAR`) the nearer one of the pixels that
// have two, each over its own samples (`surface`).
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
    front: vec4<f32>,
    fluid: vec4<f32>,
    fluid2: vec4<f32>,
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
// the same march stopped at the nearest surface of each march pixel's
// footprint (`fire_march.wesl`, `footprint`): its light, and that
// surface's depth (m) with the flames' share in front of it
@group(0) @binding(7) var vol_near: texture_2d<f32>;
@group(0) @binding(8) var near_aux: texture_2d<f32>;

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

// What the pass writes: under multisampling, also into which of the
// pixel's samples.
@if(MULTISAMPLED)
struct Out {
    @location(0) colour: vec4<f32>,
    @builtin(sample_mask) mask: u32,
}
@else
struct Out {
    @location(0) colour: vec4<f32>,
}

@if(MULTISAMPLED)
fn out(colour: vec4<f32>, mask: u32) -> Out {
    return Out(colour, mask);
}
@else
fn out(colour: vec4<f32>, mask: u32) -> Out {
    return Out(colour);
}

// The second draw: the nearer surface of the pixels that hold two.
@if(NEAR)
const NEAR_DRAW = true;
@else
const NEAR_DRAW = false;

// Samples nearer to each other than this share of their distance are one
// surface: the volume in front of them is the same (the blur below takes
// depths a twentieth apart for alike).
const ONE_SURFACE = 0.1;

// A surface of the pixel: how far it is (m) and which of the pixel's
// samples are of it (none: nothing for this draw).
struct Surface {
    z: f32,
    mask: u32,
}

fn metres(d: f32) -> f32 {
    if d > 0.0 {
        return min(p.origin.w / d, FAR_M);
    }
    return FAR_M;
}

// The pixel's samples, split where they are farthest apart (in ratio): the
// farther ones for the first draw, the nearer ones for the second. A pixel
// of one surface is all the first draw's.
@if(MULTISAMPLED)
fn surface(pix: vec2<i32>) -> Surface {
    let n = i32(textureNumSamples(depth_tex));
    var zs: array<f32, 8>;
    var lo = FAR_M;
    var hi = 0.0;
    for (var s = 0; s < n; s++) {
        let z = metres(textureLoad(depth_tex, pix, s));
        zs[s] = z;
        lo = min(lo, z);
        hi = max(hi, z);
    }
    let all = (1u << u32(n)) - 1u;
    if hi - lo <= ONE_SURFACE * lo {
        if NEAR_DRAW {
            return Surface(lo, 0u);
        }
        return Surface(hi, all);
    }
    let mid = sqrt(lo * hi);
    var near = 0u;
    for (var s = 0; s < n; s++) {
        if zs[s] < mid {
            near |= 1u << u32(s);
        }
    }
    if NEAR_DRAW {
        return Surface(lo, near);
    }
    return Surface(hi, all & ~near);
}
@else
fn surface(pix: vec2<i32>) -> Surface {
    return Surface(metres(textureLoad(depth_tex, pix, 0)), 1u);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> Out {
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
        if solo && !NEAR_DRAW {
            return out(vec4<f32>(0.0, 0.0, 0.0, 1.0), 0xffffffffu);
        }
        discard;
    }
    let on = surface(vec2<i32>(pix));
    if on.mask == 0u {
        discard;
    }
    let z = on.z;
    var sum = vec4<f32>(0.0);
    var fsum = vec3<f32>(0.0);
    var wsum = 0.0;
    var best = vec4<f32>(0.0);
    var best_f = vec3<f32>(0.0);
    var best_dz = 1.0e30;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let at = clamp(c + vec2<i32>(i, j), vec2<i32>(0), last);
            let off = vec2<f32>(at) - q;
            let ws = exp(-dot(off, off) * 1.4);
            // each march pixel holds its footprint's farthest surface and,
            // where it has two, its nearest
            // (the farther's depth below 0 where there are two)
            let aux = textureLoad(vol_aux, at, 0);
            let fm = aux.gba;
            let two = aux.r < 0.0;
            var na = vec2<f32>(0.0);
            if two {
                na = textureLoad(near_aux, at, 0).rg;
            }
            for (var k = 0; k < 2; k++) {
                var zl = abs(aux.r);
                var fa = fm.x;
                if k == 1 {
                    if !two {
                        break;
                    }
                    zl = na.x;
                    fa = na.y;
                }
                let dz = abs(zl - z) / max(min(z, zl), 1.0);
                let wd = exp(-dz * dz * 400.0);
                if wd < 1.0e-4 && dz >= best_dz {
                    continue;
                }
                var v: vec4<f32>;
                if k == 0 {
                    v = textureLoad(vol_tex, at, 0);
                } else {
                    v = textureLoad(vol_near, at, 0);
                }
                // the flames' distance and height, weighted by their
                // opacity (the nearer surface's from the farther's)
                let fw = vec3<f32>(fa, fa * fm.y, fa * fm.z);
                sum += v * ws * wd;
                wsum += ws * wd;
                fsum += fw * ws * wd;
                if dz < best_dz {
                    best_dz = dz;
                    best = v;
                    best_f = fw;
                }
            }
        }
    }
    // none of them is of this surface (a third one, between the two of
    // its footprint): the closest in depth
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
        return out(vec4<f32>(col.rgb, 1.0), on.mask);
    }
    if col.a < 0.002 {
        discard;
    }
    return out(col, on.mask);
}
