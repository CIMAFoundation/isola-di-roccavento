// The scene's own light and fog, for the materials that draw on the ground
// themselves (the burn scar, the hazard tape). `StandardMaterial` gets both
// for free; a custom `Material` does not, and without them these surfaces
// stay flat and sunlit while the terrain around them is shaded, shadowed and
// hazed. Imported as `rocca::scene_light` (registered in `shader_lib.rs`).
//
// Map signs (flat, reach_zone, route_line, forecast_zone) deliberately do NOT
// use this: they are interface and must stay legible in shadow and distance.



// Brightness multiplier for a surface with unit normal `n` at `world_pos`:
// ambient plus the sun (Lambert, with the cascaded shadow map). `frag` is the
// `@builtin(position)` of the fragment. Multiply an albedo by it.

import bevy_pbr::render::mesh_view_bindings::{view, fog, lights};
import bevy_pbr::render::mesh_view_types;
import bevy_pbr::render::fog as scene_fog;
import bevy_pbr::render::shadows;

fn scene_shade(world_pos: vec3<f32>, n: vec3<f32>, frag: vec4<f32>) -> vec3<f32> {
    var lit = lights.ambient_color.rgb * view.exposure;
    if lights.n_directional_lights > 0u {
        let light = lights.directional_lights[0];
        let ndl = max(dot(n, light.direction_to_light), 0.0);
        let view_z = view_depth(world_pos);
        var shadow = 1.0;
        if (light.flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            shadow = shadows::fetch_directional_shadow(0u, vec4<f32>(world_pos, 1.0), n, view_z, frag.xy);
            shadow = mix(1.0, shadow, shadow_reach(view_z));
        }
        lit += light.color.rgb * view.exposure * ndl * shadow * 0.3183;
    }
    return lit;
}

// The view-space depth of `world_pos` as Bevy measures it for the cascades
// (negative in front of the camera).
fn view_depth(world_pos: vec3<f32>) -> f32 {
    return dot(vec4<f32>(
        view.view_from_world[0].z,
        view.view_from_world[1].z,
        view.view_from_world[2].z,
        view.view_from_world[3].z,
    ), vec4<f32>(world_pos, 1.0));
}

// How much of the sun's shadow is kept at view depth `view_z`: all of it
// until the last cascade's overlap, then less and less to none at its far
// bound. Bevy blends each cascade into the next over that overlap and stops
// the last one dead, a line across the land beyond which nothing casts a
// shadow (user, 2026-11-02); here the last one hands over to no shadow the
// same way. Every surface that takes the sun's shadow reads this: the
// terrain and the structures (`apply_pbr_lighting` twice in the band), the
// plants, the ground overlays (`scene_shade`).
fn shadow_reach(view_z: f32) -> f32 {
    // no sun, or a sun without shadows: nothing to fade
    if lights.n_directional_lights == 0u {
        return 1.0;
    }
    let light = &lights.directional_lights[0];
    let n = (*light).num_cascades;
    if ((*light).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u || n == 0u {
        return 1.0;
    }
    let far = (*light).cascades[n - 1u].far_bound;
    let near = (1.0 - (*light).cascades_overlap_proportion) * far;
    return clamp((far + view_z) / max(far - near, 1.0), 0.0, 1.0);
}

// The scene's distance fog, applied by hand: `apply_fog` transcribed, because
// importing `pbr_functions` would pull in `StandardMaterial`'s own bindings.
fn apply_scene_fog(input_color: vec4<f32>, world_position: vec3<f32>) -> vec4<f32> {
    let view_to_world = world_position - view.world_position.xyz;
    let distance = length(view_to_world);

    var scattering = vec3<f32>(0.0);
    if fog.directional_light_color.a > 0.0 {
        let dir = view_to_world / max(distance, 1e-3);
        for (var i: u32 = 0u; i < lights.n_directional_lights; i = i + 1u) {
            let light = lights.directional_lights[i];
            scattering += pow(
                max(dot(dir, light.direction_to_light), 0.0),
                fog.directional_light_exponent,
            ) * light.color.rgb * view.exposure;
        }
    }

    if fog.mode == mesh_view_types::FOG_MODE_LINEAR {
        return scene_fog::linear_fog(fog, input_color, distance, scattering);
    } else if fog.mode == mesh_view_types::FOG_MODE_EXPONENTIAL {
        return scene_fog::exponential_fog(fog, input_color, distance, scattering);
    } else if fog.mode == mesh_view_types::FOG_MODE_EXPONENTIAL_SQUARED {
        return scene_fog::exponential_squared_fog(fog, input_color, distance, scattering);
    } else if fog.mode == mesh_view_types::FOG_MODE_ATMOSPHERIC {
        return scene_fog::atmospheric_fog(fog, input_color, distance, scattering);
    }
    return input_color;
}
