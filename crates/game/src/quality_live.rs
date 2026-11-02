//! The graphics level changed in the operator bar, put on screen (see `quality`).

use bevy::prelude::*;

use crate::quality::Graphics;

/// Put a changed [`Graphics`] on screen: the sample count, the shadows, the
/// post effects on the camera and the plants' LOD distance.
pub fn apply_live(
    gfx: Res<Graphics>,
    mut commands: Commands,
    mut msaa: Query<&mut Msaa>,
    mut shadow_map: ResMut<bevy::light::DirectionalLightShadowMap>,
    mut suns: Query<(&mut DirectionalLight, &mut bevy::light::CascadeShadowConfig), With<crate::sky::Sun>>,
    cameras: Query<(
        Entity,
        Has<bevy::post_process::bloom::Bloom>,
        Has<bevy::post_process::dof::DepthOfField>,
        Has<bevy::anti_alias::smaa::Smaa>,
        &crate::camera::OrbitCamera,
    )>,
    veg: Option<ResMut<crate::vegetation::Vegetation>>,
    terrain: Query<(Entity, Has<bevy::light::NotShadowCaster>), With<crate::terrain_mesh::TerrainChunk>>,
) {
    use bevy::{post_process::{bloom::Bloom, dof::*}, core_pipeline::prepass::DepthPrepass};
    for mut m in &mut msaa {
        *m = gfx.msaa();
    }
    if shadow_map.size != gfx.shadow_map {
        shadow_map.size = gfx.shadow_map;
    }
    for (mut light, mut cascades) in &mut suns {
        light.shadow_maps_enabled = gfx.shadows;
        *cascades = bevy::light::CascadeShadowConfigBuilder {
            num_cascades: gfx.cascades,
            minimum_distance: 2.0,
            maximum_distance: gfx.shadow_distance,
            first_cascade_far_bound: 80.0,
            overlap_proportion: 0.3,
        }
        .build();
    }
    for (e, has_bloom, has_dof, has_smaa, orbit) in &cameras {
        let mut cam = commands.entity(e);
        match (gfx.smaa, has_smaa) {
            (true, false) => { cam.insert(bevy::anti_alias::smaa::Smaa::default()); }
            (false, true) => { cam.remove::<bevy::anti_alias::smaa::Smaa>(); }
            _ => {}
        }
        match (gfx.bloom, has_bloom) {
            (true, false) => { cam.insert(Bloom { intensity: 0.20, ..Bloom::NATURAL }); }
            (false, true) => { cam.remove::<Bloom>(); }
            _ => {}
        }
        // the depth of field reads the depth at screen resolution: not while
        // the scene is drawn smaller (`render_scale`)
        match (gfx.dof && crate::render_scale::whole(gfx.render_scale), has_dof) {
            (true, false) => {
                cam.insert((
                    DepthPrepass,
                    DepthOfField {
                        mode: DepthOfFieldMode::Gaussian,
                        focal_distance: orbit.distance,
                        sensor_height: 0.01866,
                        aperture_f_stops: 0.012,
                        max_circle_of_confusion_diameter: 48.0,
                        max_depth: 12_000.0,
                    },
                ));
            }
            (false, true) => { cam.remove::<(DepthPrepass, DepthOfField)>(); }
            _ => {}
        }
    }
    // the terrain's shadow casting (its stride needs a restart, like the plant density)
    for (e, no_cast) in &terrain {
        match (gfx.terrain_shadows, no_cast) {
            (false, false) => { commands.entity(e).insert(bevy::light::NotShadowCaster); }
            (true, true) => { commands.entity(e).remove::<bevy::light::NotShadowCaster>(); }
            _ => {}
        }
    }
    if let Some(mut veg) = veg {
        veg.set_lod_m(gfx.lod_m);
        veg.set_keep(gfx.veg_keep);
    }
}

/// MSAA is a camera component since Bevy 0.15 (it was one resource for all):
/// every new camera starts with the profile's sample count.
pub fn msaa_on_new_cameras(gfx: Res<Graphics>, mut cams: Query<&mut Msaa, Added<Camera>>) {
    for mut m in &mut cams {
        *m = gfx.msaa();
    }
}
