// Depth, normal and shadow pass vertex stage of the vegetation: Bevy's
// `prepass.wgsl` vertex for a plant drawn as an instance, with its fire state
// (see `veg_vertex.wgsl`), so a plant that has collapsed leaves no shadow.
// Where the material cuts leaves out of the atlas (`AlphaMode::Mask` on the
// desktop) the fragment stage below cuts them out of the depth and the
// shadows too: a leaf card casts the shadow of its leaves, not of its quad.

import bevy_pbr::prepass::io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import game::shaders::veg_state::{plant_apply, place, place_normal, atlas, atlas_sampler, ALPHA_CUT};

struct VegVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(1) uv: vec2<f32>,
    @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS)
    @location(3) normal: vec3<f32>,
    @if(VERTEX_COLORS)
    @location(7) color: vec4<f32>,
    // `veg_instancing::instance_layout`
    @location(10) foot: vec3<f32>,
    @location(11) yaw: f32,
    @location(12) scale: vec3<f32>,
    @location(13) tint: vec4<f32>,
}

@vertex
fn vertex(vertex: VegVertex) -> VertexOutput {
    var out: VertexOutput;
    var world = place(vertex.position, vertex.foot, vertex.yaw, vertex.scale);
    @if(VERTEX_COLORS) {
        let planted = plant_apply(vertex.foot, world, vertex.color, vertex.tint);
        world = planted.position;
        out.color = planted.color;
    }
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    @if(VERTEX_UVS_A)
    out.uv = vertex.uv;
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = out.position.z;
        out.position.z = min(out.position.z, 1.0);
    }
    @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS)
    out.world_normal = place_normal(vertex.normal, vertex.yaw, vertex.scale);
    @if(MOTION_VECTOR_PREPASS)
    // the plants do not move
    out.previous_world_position = out.world_position;
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = 0u;
    return out;
}

// Only the leaves' cut-out; the burn holes of `foliage.wgsl` stay out of the
// depth (as before the atlas).
fn cut(in: VertexOutput) {
    @if(VERTEX_UVS_A) {
        if textureSample(atlas, atlas_sampler, in.uv).a < ALPHA_CUT {
            discard;
        }
    }
}

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    cut(in);
    var out: FragmentOutput;
    @if(NORMAL_PREPASS)
    out.normal = vec4<f32>(normalize(in.world_normal) * 0.5 + vec3<f32>(0.5), 1.0);
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
    out.frag_depth = in.unclipped_depth;
    @if(MOTION_VECTOR_PREPASS)
    out.motion_vector = vec2<f32>(0.0);
    return out;
}
@else
@fragment
fn fragment(in: VertexOutput) {
    cut(in);
}
