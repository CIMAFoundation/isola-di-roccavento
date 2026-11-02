//! The island's landmarks: the rocca on the summit and the stone tower on the
//! headland, from their Blender models (`scripts/build_island_models.py`).
//!
//! Where they stand is the factory's decision, not the game's
//! (`tools/factory/island.py` picks the summit and the headland, levels the
//! ground, takes the fuel off it and writes `island.json`): this only draws what
//! the data says. They are scenery: the fire never sees them, and they are
//! drawn at diorama size (`scale` in the data).

use bevy::prelude::*;
use scenario::Pos;

use crate::models;
use crate::materials::{self, Finish, StructureMaterial};
use crate::sim::Sim;

pub struct LandmarksPlugin;

impl Plugin for LandmarksPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(crate::AppState::Playing), spawn_landmarks);
    }
}

fn spawn_landmarks(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StructureMaterial>>,
) {
    let path = sim.data.join("scenarios").join(sim.scn.id.as_str()).join("island.json");
    let Some(island) = datafs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else {
        return; // a scenario that is not an island has none
    };
    let material = materials.add(materials::structure(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.9,
        reflectance: 0.08,
        // stone and sails seen from either side; the models are authored for it
        cull_mode: None,
        double_sided: true,
        ..default()
    }, Finish::Stone));
    // the bedrock under the rocca is drawn like the mountain's rock, not as masonry
    let rock = materials.add(materials::structure(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.95,
        reflectance: 0.04,
        ..default()
    }, Finish::Rock));
    for (key, model, rock_model) in [("rocca", "rocca", Some("rocca_roccia")), ("torre", "torre_pietra", None)] {
        let l = &island["landmarks"][key];
        let (Some(x), Some(y), Some(ground)) = (l["pos"][0].as_f64(), l["pos"][1].as_f64(), l["ground_m"].as_f64()) else { continue };
        let scale = l["scale"].as_f64().unwrap_or(1.0) as f32;
        // the gate faces +X in the model; `yaw_deg` is the map bearing of the gate
        // counter-clockwise from east, as Bevy's rotation about Y
        let yaw = (l["yaw_deg"].as_f64().unwrap_or(0.0) as f32).to_radians();
        // sit on the level the factory made, a little into it so no sliver shows
        let ground = ground as f32 - 0.3;
        let at = crate::frame::to_bevy(Pos { x: x as f32, y: y as f32 }, ground);
        info!("landmark {key}: {model} at ({x:.0}, {y:.0}), ground {ground:.1} m, scale {scale}, yaw {:.0} deg", yaw.to_degrees());
        let transform = Transform { translation: at, rotation: Quat::from_rotation_y(yaw), scale: Vec3::splat(scale) };
        for (name, material) in std::iter::once((model, material.clone())).chain(rock_model.map(|m| (m, rock.clone()))) {
            commands.spawn(((
                Mesh3d(meshes.add(models::island_mesh(name))),
                MeshMaterial3d(material),
                transform,
            ),
                crate::terrain_mesh::PauseSolid,
            ));
        }
    }
}
