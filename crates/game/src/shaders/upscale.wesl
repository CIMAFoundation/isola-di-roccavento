// Adaptive resolution (`crate::render_scale`): the main pass drew the scene
// into the top-left corner of the view; this stretches it over the whole
// view, bilinear, before the post effects.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
// the pass's sampler does not filter: the four texels are read by hand
@group(0) @binding(1) var screen_sampler: sampler;

struct Upscale {
    ratio: vec2<f32>,
    last: vec2<f32>,
    size: vec2<f32>,
    _pad: vec2<f32>,
}
@group(0) @binding(2) var<uniform> upscale: Upscale;

fn texel(p: vec2<f32>) -> vec4<f32> {
    return textureLoad(screen_texture, vec2<i32>(clamp(p, vec2<f32>(0.0), upscale.last)), 0);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // the screen pixel's centre in main-pass texels, then the four around it
    let p = in.position.xy * upscale.ratio - 0.5;
    let i = floor(p);
    let f = p - i;
    let top = mix(texel(i), texel(i + vec2<f32>(1.0, 0.0)), f.x);
    let bottom = mix(texel(i + vec2<f32>(0.0, 1.0)), texel(i + vec2<f32>(1.0, 1.0)), f.x);
    return mix(top, bottom, f.y);
}
