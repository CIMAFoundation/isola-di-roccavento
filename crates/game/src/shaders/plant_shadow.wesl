// The plants' shadow pass (`vegetation/render.rs`): depth only, into the
// sun's cascades, with the same placement, sway and fire state as the main
// pass (`plant_common.wesl`), so a plant that is gone casts nothing. Where
// the leaf cards are cut out (`PLANT_CUTOUT`) a card casts the shadow of its
// leaves, not of its quad. Without the GPU's unclipped depth (WebGL2) the
// depth of what stands nearer the sun than the cascade is written by hand,
// as Bevy's prepass does.

import bevy_pbr::render::view_transformations::{position_world_to_clip, direction_view_to_world};
import game::shaders::plant_common::{atlas, atlas_sampler, PlantVertex, place, card_hidden, GONE, ALPHA_CUT, kind_leaf, hole_at, burn_gone};

struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) kind: f32,
    // the fire at the vertex: where it is, the plant's burn and the climb
    // (`Placed`), so the leaves the front ate cast no shadow
    @location(3) world: vec3<f32>,
    @location(4) fire: vec2<f32>,
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
    // the view's backward axis: toward the sun in its cascades
    let toward = normalize(direction_view_to_world(vec3<f32>(0.0, 0.0, 1.0)));
    out.clip = select(clip, GONE, p.gone || card_hidden(v, toward));
    out.uv = v.uv;
    out.kind = v.normal.w;
    out.world = p.world;
    out.fire = vec2<f32>(p.burn, p.climb);
    return out;
}

fn cut(in: ShadowOut) {
    @if(PLANT_CUTOUT) {
        let a = textureSample(atlas, atlas_sampler, in.uv).a;
        if kind_leaf(in.kind) && a < ALPHA_CUT {
            discard;
        }
        // as the main pass: a burning crown's eaten leaves are holes
        // (`plants.wesl`), at the scale of a shadow texel
        let burning = in.fire.x >= 0.0 && in.fire.x <= 1.0;
        if kind_leaf(in.kind) && burning && hole_at(in.world, 0.5) < burn_gone(in.world, clamp(in.fire.y, 0.0, 1.0)) {
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
