// Main-pass vertex stage of the vegetation (`rocca::veg_state`): Bevy 0.14's
// `mesh.wgsl` vertex without morph targets and skinning (plants have neither),
// plus the per-plant fire state. Only vegetation materials take this path
// (`materials/foliage.rs`).

import bevy_pbr::render::mesh_functions;
import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::view_transformations::position_world_to_clip;
import game::shaders::veg_state::plant_apply;

struct VegVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @if(VERTEX_NORMALS)
    @location(1) normal: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
    @if(VERTEX_COLORS)
    @location(5) color: vec4<f32>,
    @if(!PLANT_TAG)
    @location(14) plant: f32,
}

@vertex
fn vertex(vertex: VegVertex) -> VertexOutput {
    var out: VertexOutput;
    var position = vertex.position;
    @if(VERTEX_COLORS) {
        // the plant: from the vertex (merged chunk meshes) or the mesh
        // tag (instances, `vegetation::instanced`)
        @if(PLANT_TAG)
        let plant = f32(mesh_functions::get_tag(vertex.instance_index));
        @else
        let plant = vertex.plant;
        let planted = plant_apply(plant, vertex.position, vertex.color);
        position = planted.position;
        out.color = planted.color;
    }
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    @if(VERTEX_NORMALS) {
        out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    }
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    @if(VERTEX_UVS_A)
    out.uv = vertex.uv;
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex.instance_index;
    @if(VISIBILITY_RANGE_DITHER) {
        out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
            vertex.instance_index, world_from_local[3]);
    }
    return out;
}
