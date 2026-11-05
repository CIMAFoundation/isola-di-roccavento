// The frames before, blended into this frame's march (`fire_volumetric.rs`).
// Each march pixel's volume is reprojected to where it was last frame and
// the history read there; the history is clamped to the spread of this
// frame's 3x3 neighbourhood first (mean ± deviation, inside its min-max),
// so it can neither ghost nor pulse: a reprojection that missed lands on
// something like what is here. The jitter of the march's first step, new
// each frame, averages out.

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
@group(0) @binding(1) var raw_tex: texture_2d<f32>;
@group(0) @binding(2) var mid_tex: texture_2d<f32>;
@group(0) @binding(3) var hist_tex: texture_2d<f32>;
@group(0) @binding(4) var hist_samp: sampler;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pix = vec2<i32>(in.position.xy);
    let last = vec2<i32>(p.size.xy) - 1;
    let now = textureLoad(raw_tex, pix, 0);
    let keep = p.look2.w;
    if keep <= 0.0 {
        return now;
    }
    // the neighbourhood's spread
    var m1 = vec4<f32>(0.0);
    var m2 = vec4<f32>(0.0);
    var lo = now;
    var hi = now;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let v = textureLoad(raw_tex, clamp(pix + vec2<i32>(i, j), vec2<i32>(0), last), 0);
            m1 += v;
            m2 += v * v;
            lo = min(lo, v);
            hi = max(hi, v);
        }
    }
    let mean = m1 / 9.0;
    let dev = sqrt(max(m2 / 9.0 - mean * mean, vec4<f32>(0.0)));
    let box_lo = max(lo, mean - dev * 1.25);
    let box_hi = min(hi, mean + dev * 1.25);

    // where this pixel's volume was last frame
    let full = in.position.xy * p.origin.z;
    let f = full / p.size.zw;
    let ndc = vec2<f32>(f.x * 2.0 - 1.0, 1.0 - f.y * 2.0);
    let far = p.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - p.eye.xyz);
    let pos = p.eye.xyz + dir * textureLoad(mid_tex, pix, 0).r;
    let pc = p.prev * vec4<f32>(pos, 1.0);
    if pc.w <= 0.0 {
        return now;
    }
    let ndc_p = pc.xy / pc.w;
    let uv = vec2<f32>(ndc_p.x + 1.0, 1.0 - ndc_p.y) * 0.5 * p.size.zw / (p.size.xy * p.origin.z);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return now;
    }
    let past = clamp(textureSampleLevel(hist_tex, hist_samp, uv, 0.0), box_lo, box_hi);
    return mix(now, past, keep);
}
