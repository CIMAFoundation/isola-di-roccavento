// Main-pass vertex stage of the vegetation: Bevy's `mesh.wgsl` vertex for a
// plant drawn as an instance (`veg_instancing.rs`). The instance attributes
// place it (no per-mesh transform: the tile entity's is the identity), and
// its fire state comes from the fire grid at its foot (`veg_state.wgsl`).
// Only vegetation materials take this path (`materials/foliage.rs`).

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::view_transformations::position_world_to_clip;
import game::shaders::veg_state::{plant_apply, place, place_normal};

struct VegVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @if(VERTEX_NORMALS)
    @location(1) normal: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
    @if(VERTEX_COLORS)
    @location(5) color: vec4<f32>,
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
    @if(VERTEX_NORMALS)
    out.world_normal = place_normal(vertex.normal, vertex.yaw, vertex.scale);
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    @if(VERTEX_UVS_A)
    out.uv = vertex.uv;
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = 0u;
    @if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = 0;
    return out;
}
