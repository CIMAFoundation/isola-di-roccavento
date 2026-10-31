// Depth, normal and shadow pass vertex stage of the vegetation: Bevy 0.14's
// `prepass.wgsl` vertex without morph targets and skinning, plus the per-plant
// fire state (see `veg_vertex.wgsl`), so a plant that has collapsed leaves no
// shadow. Where the material cuts leaves out of the atlas (`AlphaMode::Mask`
// on the desktop) the fragment stage below cuts them out of the depth and the
// shadows too: a leaf card casts the shadow of its leaves, not of its quad.

import bevy_pbr::render::mesh_functions;
import bevy_pbr::prepass::io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import game::shaders::veg_state::{plant_apply, atlas, atlas_sampler, ALPHA_CUT};

struct VegVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(1) uv: vec2<f32>,
    @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS)
    @location(3) normal: vec3<f32>,
    @if(VERTEX_COLORS)
    @location(7) color: vec4<f32>,
    @location(14) plant: f32,
}

@vertex
fn vertex(vertex: VegVertex) -> VertexOutput {
    var out: VertexOutput;
    var position = vertex.position;
    @if(VERTEX_COLORS) {
        let planted = plant_apply(vertex.plant, vertex.position, vertex.color);
        position = planted.position;
        out.color = planted.color;
    }
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    @if(VERTEX_UVS_A)
    out.uv = vertex.uv;
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = out.position.z;
        out.position.z = min(out.position.z, 1.0);
    }
    @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
        out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    }
    @if(MOTION_VECTOR_PREPASS) {
        // the plants do not move, collapsed or not
        let prev_model = mesh_functions::get_previous_world_from_local(vertex.instance_index);
        out.previous_world_position = mesh_functions::mesh_position_local_to_world(prev_model, vec4<f32>(position, 1.0));
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex.instance_index;
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
