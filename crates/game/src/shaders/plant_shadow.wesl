// The plants' shadow pass (`vegetation/render.rs`): depth only, into the
// sun's cascades, with the same placement, sway and fire state as the main
// pass (`plant_common.wesl`), so a plant that is gone casts nothing. Where
// the leaf cards are cut out (`PLANT_CUTOUT`) a card casts the shadow of its
// leaves, not of its quad. Without the GPU's unclipped depth (WebGL2) the
// depth of what stands nearer the sun than the cascade is written by hand,
// as Bevy's prepass does.

import bevy_pbr::render::view_transformations::position_world_to_clip;
import game::shaders::plant_common::{atlas, atlas_sampler, PlantVertex, place, GONE, ALPHA_CUT, kind_leaf};

struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) kind: f32,
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
    @location(2) unclipped_depth: f32,
}

@vertex
fn vertex(v: PlantVertex) -> ShadowOut {
    var out: ShadowOut;
    let p = place(v);
    var clip = position_world_to_clip(p.world);
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = clip.z;
        clip.z = min(clip.z, 1.0);
    }
    out.clip = select(clip, GONE, p.gone);
    out.uv = v.uv;
    out.kind = v.normal.w;
    return out;
}

fn cut(in: ShadowOut) {
    @if(PLANT_CUTOUT) {
        let a = textureSample(atlas, atlas_sampler, in.uv).a;
        if kind_leaf(in.kind) && a < ALPHA_CUT {
            discard;
        }
    }
}

@if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
@fragment
fn fragment(in: ShadowOut) -> @builtin(frag_depth) f32 {
    cut(in);
    return in.unclipped_depth;
}
@else
@fragment
fn fragment(in: ShadowOut) {
    cut(in);
}
