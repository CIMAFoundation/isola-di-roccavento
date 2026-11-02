// Bevy's PBR light with the sun's shadow faded out toward the end of its
// reach (`scene_light::shadow_reach`), for the materials that light with
// `apply_pbr_lighting` (the terrain, the structures). Bevy keeps the shadow
// to the last cascade's far bound and drops it there; in the band where it
// should fade the light is worked out twice, with the shadow and without,
// and the two mixed: the sun's term is linear in its shadow, so the mix is
// the light of the faded shadow. Elsewhere it costs nothing more.

import bevy_pbr::render::pbr_functions::apply_pbr_lighting;
import bevy_pbr::render::pbr_types::PbrInput;
import bevy_pbr::render::mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT;
import game::shaders::scene_light::{view_depth, shadow_reach};

fn apply_pbr_lighting_faded(pbr: PbrInput) -> vec4<f32> {
    let shadowed = apply_pbr_lighting(pbr);
    let reach = shadow_reach(view_depth(pbr.world_position.xyz));
    // whole before the band; past it Bevy has no shadow to give
    if reach >= 1.0 || reach <= 0.0 || (pbr.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) == 0u {
        return shadowed;
    }
    var bare = pbr;
    bare.flags &= ~MESH_FLAGS_SHADOW_RECEIVER_BIT;
    return mix(apply_pbr_lighting(bare), shadowed, reach);
}
