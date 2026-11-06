// The frames before, blended into this frame's march (`fire_volumetric.rs`).
// Each march pixel's volume is reprojected to where it was last frame and
// the history read there; the history is clamped to the spread of this
// frame's 3x3 neighbourhood first (mean ± deviation, inside its min-max),
// so it can neither ghost nor pulse: a reprojection that missed lands on
// something like what is here. The jitter of the march's first step, new
// each frame, averages out.
//
// The neighbourhood is of this pixel's surface only: the neighbours whose
// scene is about as far as its own. A march pixel at the rim of a leaf that
// sways is now the leaf, now the sky behind, with little smoke in front of
// the one and all of it in front of the other; with every neighbour in the
// spread, both fitted, the history kept a mix of the two, and a pale glow
// stood round each leaf against the smoke (user, 2026-11-06).

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
@group(0) @binding(1) var raw_tex: texture_2d<f32>;
@group(0) @binding(2) var mid_tex: texture_2d<f32>;
@group(0) @binding(3) var hist_tex: texture_2d<f32>;
@group(0) @binding(4) var hist_samp: sampler;
// what the march noted of the scene: how far it is (m), in red
@group(0) @binding(5) var aux_tex: texture_2d<f32>;

// Scenes nearer to each other than this share of their distance are one
// surface (as in `fire_composite.wesl`).
const ONE_SURFACE = 0.1;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pix = vec2<i32>(in.position.xy);
    let last = vec2<i32>(p.size.xy) - 1;
    let now = textureLoad(raw_tex, pix, 0);
    let keep = p.look2.w;
    // `marca=2`: this frame's march as it is
    if keep <= 0.0 || ((u32(p.frame.z) >> 5u) & 3u) >= 2u {
        return now;
    }
    // the spread of the neighbours of this pixel's surface (itself among
    // them)
    let z = textureLoad(aux_tex, pix, 0).r;
    var m1 = vec4<f32>(0.0);
    var m2 = vec4<f32>(0.0);
    var lo = now;
    var hi = now;
    var n = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let at = clamp(pix + vec2<i32>(i, j), vec2<i32>(0), last);
            let zl = textureLoad(aux_tex, at, 0).r;
            if abs(zl - z) > ONE_SURFACE * max(min(zl, z), 1.0) {
                continue;
            }
            let v = textureLoad(raw_tex, at, 0);
            m1 += v;
            m2 += v * v;
            lo = min(lo, v);
            hi = max(hi, v);
            n += 1.0;
        }
    }
    let mean = m1 / n;
    let dev = sqrt(max(m2 / n - mean * mean, vec4<f32>(0.0)));
    let box_lo = max(lo, mean - dev * 1.25);
    let box_hi = min(hi, mean + dev * 1.25);

    // where this pixel's volume was last frame
    let full = in.position.xy * p.origin.z;
    let f = full / p.size.zw;
    let ndc = vec2<f32>(f.x * 2.0 - 1.0, 1.0 - f.y * 2.0);
    // the ray from the camera's turn alone, and `prev` takes a place given
    // from this frame's eye: nothing here is a difference of two places
    // kilometres from the world's origin (as such a difference the ray was
    // up to a march pixel off, and with the camera still each pixel read
    // the history of a neighbour, `fire_march.wesl`)
    let dir = normalize((p.rays * vec4<f32>(ndc, 1.0, 1.0)).xyz);
    let pc = p.prev * vec4<f32>(dir * textureLoad(mid_tex, pix, 0).r, 1.0);
    if pc.w <= 0.0 {
        return now;
    }
    let ndc_p = pc.xy / pc.w;
    let uv = vec2<f32>(ndc_p.x + 1.0, 1.0 - ndc_p.y) * 0.5 * p.size.zw / (p.size.xy * p.origin.z);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return now;
    }
    // `scarto`: how far from itself the pixel was found, in march pixels
    // (red across, green down); black with the camera still
    if (u32(p.frame.z) & 128u) != 0u {
        let off = abs(uv * p.size.xy - in.position.xy);
        return vec4<f32>(off.x, off.y, 0.0, 1.0);
    }
    let past = clamp(textureSampleLevel(hist_tex, hist_samp, uv, 0.0), box_lo, box_hi);
    return mix(now, past, keep);
}
