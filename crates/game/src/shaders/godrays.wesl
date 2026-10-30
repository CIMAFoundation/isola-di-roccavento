// God rays (`crate::godrays`): a radial blur of what is bright, toward the
// sun's place on the screen, added to the scene before tonemapping.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;

struct GodRays {
    sun_uv: vec2<f32>,
    strength: f32,
    threshold: f32,
    aspect: f32,
    _pad: vec3<f32>,
}
@group(0) @binding(2) var<uniform> rays: GodRays;

const SAMPLES: i32 = 48;
// how far toward the sun the march goes, as a share of the way
const REACH: f32 = 0.85;
const DECAY: f32 = 0.965;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let base = textureSample(screen_texture, screen_sampler, in.uv);
    // the threshold is relative to the sun's own brightness on this frame:
    // only the disc, its core glow and the cloud rims it lights make rays
    let sun = textureSampleLevel(screen_texture, screen_sampler, clamp(rays.sun_uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
    let ls = max(dot(sun, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-3);
    let lo = ls * rays.threshold;
    // the sky's brightness just around the disc: the rays are measured in it
    // (the disc itself is far brighter in HDR and would wash everything out)
    var ring = 0.0;
    for (var j = 0; j < 8; j++) {
        let a = f32(j) * 0.785398;
        let p = rays.sun_uv + vec2<f32>(cos(a) / rays.aspect, sin(a)) * 0.08;
        let r = textureSampleLevel(screen_texture, screen_sampler, clamp(p, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
        ring += min(dot(r, vec3<f32>(0.2126, 0.7152, 0.0722)), ls);
    }
    ring = ring / 8.0;
    let delta = (in.uv - rays.sun_uv) * (REACH / f32(SAMPLES));
    // a per-pixel offset of the first step hides the banding of the steps
    let jitter = fract(sin(dot(in.position.xy, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    var uv = in.uv - delta * jitter;
    var weight = 1.0;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < SAMPLES; i++) {
        uv -= delta;
        let s = textureSampleLevel(screen_texture, screen_sampler, uv, 0.0).rgb;
        let l = max(dot(s, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4);
        let k = clamp((l - lo) / max(ls - lo, 1e-4), 0.0, 1.0);
        let d = (uv - rays.sun_uv) * vec2<f32>(rays.aspect, 1.0);
        let near = 1.0 - smoothstep(0.02, 0.6, length(d));
        sum += (s / l) * k * near * weight;
        weight *= DECAY;
    }
    // in units of the sky's brightness by the sun, so it never washes out
    let shafts = sum / f32(SAMPLES);
    return vec4<f32>(base.rgb + shafts * (rays.strength * ring), base.a);
}
