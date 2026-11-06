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
//
// The march has two layers, the volume up to the farther and up to the
// nearer surface of each march pixel's footprint (`fire_march.wesl`): each
// is held by its own history, in the spread of the neighbours' layers of
// its surface, both reprojected from the farther's volume.

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
// what the march noted of the scene: how far it is (m), in red, below 0
// where the march pixel has a nearer surface too
@group(0) @binding(5) var aux_tex: texture_2d<f32>;
// the same for the nearest surface of each march pixel's footprint
// (`fire_march.wesl`, `footprint`): its light, its frames before, how far
// it is (m, in red)
@group(0) @binding(6) var near_tex: texture_2d<f32>;
@group(0) @binding(7) var near_hist_tex: texture_2d<f32>;
@group(0) @binding(8) var near_aux_tex: texture_2d<f32>;

// Scenes nearer to each other than this share of their distance are one
// surface (as in `fire_composite.wesl`).
const ONE_SURFACE = 0.1;

struct Out {
    @location(0) far: vec4<f32>,
    @location(1) near: vec4<f32>,
}

// The spread of a layer's neighbours of its surface (itself among them):
// the box the history is held in. Both layers' neighbours are asked, the
// nearer and the farther surface of each.
struct Spread {
    m1: vec4<f32>,
    m2: vec4<f32>,
    lo: vec4<f32>,
    hi: vec4<f32>,
    n: f32,
}

fn take(sp: ptr<function, Spread>, z: f32, zl: f32, v: vec4<f32>) {
    if abs(zl - z) > ONE_SURFACE * max(min(zl, z), 1.0) {
        return;
    }
    (*sp).m1 += v;
    (*sp).m2 += v * v;
    (*sp).lo = min((*sp).lo, v);
    (*sp).hi = max((*sp).hi, v);
    (*sp).n += 1.0;
}

fn held(sp: Spread, past: vec4<f32>) -> vec4<f32> {
    let n = max(sp.n, 1.0);
    let mean = sp.m1 / n;
    let dev = sqrt(max(sp.m2 / n - mean * mean, vec4<f32>(0.0)));
    let box_lo = max(sp.lo, mean - dev * 1.25);
    let box_hi = min(sp.hi, mean + dev * 1.25);
    return clamp(past, box_lo, box_hi);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> Out {
    let pix = vec2<i32>(in.position.xy);
    let last = vec2<i32>(p.size.xy) - 1;
    let now = textureLoad(raw_tex, pix, 0);
    let now_n = textureLoad(near_tex, pix, 0);
    let keep = p.look2.w;
    // `marca=2`: this frame's march as it is
    if keep <= 0.0 || ((u32(p.frame.z) >> 5u) & 3u) >= 2u {
        return Out(now, now_n);
    }
    // the scene's depth, below 0 where the pixel has a nearer surface
    let zs = textureLoad(aux_tex, pix, 0).r;
    let z = abs(zs);
    var z_n = z;
    if zs < 0.0 {
        z_n = textureLoad(near_aux_tex, pix, 0).r;
    }
    var far = Spread(vec4<f32>(0.0), vec4<f32>(0.0), now, now, 0.0);
    var nea = Spread(vec4<f32>(0.0), vec4<f32>(0.0), now_n, now_n, 0.0);
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let at = clamp(pix + vec2<i32>(i, j), vec2<i32>(0), last);
            let zls = textureLoad(aux_tex, at, 0).r;
            let zl = abs(zls);
            let v = textureLoad(raw_tex, at, 0);
            take(&far, z, zl, v);
            take(&nea, z_n, zl, v);
            // a neighbour's nearer surface, where it has two
            if zls < 0.0 {
                let zl_n = textureLoad(near_aux_tex, at, 0).r;
                let v_n = textureLoad(near_tex, at, 0);
                take(&far, z, zl_n, v_n);
                take(&nea, z_n, zl_n, v_n);
            }
        }
    }

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
        return Out(now, now_n);
    }
    let ndc_p = pc.xy / pc.w;
    let uv = vec2<f32>(ndc_p.x + 1.0, 1.0 - ndc_p.y) * 0.5 * p.size.zw / (p.size.xy * p.origin.z);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return Out(now, now_n);
    }
    // `scarto`: how far from itself the pixel was found, in march pixels
    // (red across, green down); black with the camera still
    if (u32(p.frame.z) & 128u) != 0u {
        let off = abs(uv * p.size.xy - in.position.xy);
        let o = vec4<f32>(off.x, off.y, 0.0, 1.0);
        return Out(o, o);
    }
    let past = held(far, textureSampleLevel(hist_tex, hist_samp, uv, 0.0));
    let past_n = held(nea, textureSampleLevel(near_hist_tex, hist_samp, uv, 0.0));
    return Out(mix(now, past, keep), mix(now_n, past_n, keep));
}
