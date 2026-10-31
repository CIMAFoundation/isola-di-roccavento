//! `?materiale=standard` in the browser (2026-10-31): the plants drawn with
//! Bevy's `StandardMaterial` (their vertex colours, both faces) instead of
//! `FoliageMaterial`, to tell whether the material or the plants themselves
//! kill an iPhone's page. No fire state, burn or thinning on screen then.
//! Inert without the switch, and always on the desktop.

use bevy::prelude::*;

use crate::materials::FoliageMaterial;

pub struct VegMaterialSwitchPlugin;

impl Plugin for VegMaterialSwitchPlugin {
    fn build(&self, app: &mut App) {
        if crate::quality::web_param("materiale").as_deref() == Some("standard") {
            app.add_systems(PostUpdate, swap);
        }
    }
}

fn swap(
    mut commands: Commands,
    added: Query<Entity, Added<MeshMaterial3d<FoliageMaterial>>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut plain: Local<Option<Handle<StandardMaterial>>>,
) {
    for e in &added {
        let handle = plain
            .get_or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    perceptual_roughness: 0.9,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })
            })
            .clone();
        commands.entity(e).remove::<MeshMaterial3d<FoliageMaterial>>().insert(MeshMaterial3d(handle));
    }
}
