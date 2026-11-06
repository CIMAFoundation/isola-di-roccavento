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
use crate::materials::masonry::{self, Masonry, MasonryMaterial};
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
    mut masonry_materials: ResMut<Assets<MasonryMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let path = sim.data.join("scenarios").join(sim.scn.id.as_str()).join("island.json");
    let Some(island) = datafs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else {
        return; // a scenario that is not an island has none
    };
    let stone = StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.9,
        reflectance: 0.08,
        // stone and sails seen from either side; the models are authored for it
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    // With the scanned plants, photographed sandstone masonry and the
    // terrain's own limestone for the crag (`materials::masonry`).
    let real = masonry::textures(&mut images);
    let masonry_material = real.as_ref().map(|t| masonry_materials.add(masonry::material(stone.clone(), t, Masonry::Landmark)));
    let crag_material = real.as_ref().map(|t| {
        masonry_materials.add(masonry::material(
            StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.95, reflectance: 0.04, ..default() },
            t,
            Masonry::Crag,
        ))
    });
    let material = materials.add(materials::structure(stone, Finish::Stone));
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
        let mut mesh = models::island_mesh(model);
        if real.is_some() && model == "torre_pietra" {
            boulders_of_the_tower(&mut mesh);
        }
        let mesh = meshes.add(mesh);
        match &masonry_material {
            Some(m) => commands.spawn(((Mesh3d(mesh), MeshMaterial3d(m.clone()), transform), crate::terrain_mesh::PauseSolid)),
            None => commands.spawn(((Mesh3d(mesh), MeshMaterial3d(material.clone()), transform), crate::terrain_mesh::PauseSolid)),
        };
        if let Some(name) = rock_model {
            let mut crag = models::island_mesh(name);
            match &crag_material {
                Some(m) => {
                    // a few hundred flat facets: lit as the rounded rock it is
                    masonry::smooth_normals(&mut crag, 55.0, None);
                    commands.spawn(((Mesh3d(meshes.add(crag)), MeshMaterial3d(m.clone()), transform), crate::terrain_mesh::PauseSolid));
                }
                None => {
                    commands.spawn(((Mesh3d(meshes.add(crag)), MeshMaterial3d(rock.clone()), transform), crate::terrain_mesh::PauseSolid));
                }
            }
        }
    }
}

/// With the photographed masonry: the boulders heaped at the foot of the
/// stone tower are rock, not masonry (`masonry::Part::Rock`, the terrain's
/// limestone), their facets lit smooth (the masonry keeps its flat faces: its
/// photograph is laid on them). The boulders are what of the bake lies low and outside the
/// tower's shaft (model metres: the shaft is 6.5 m in radius at its foot,
/// the heap reaches 11 m).
fn boulders_of_the_tower(mesh: &mut Mesh) {
    use bevy::mesh::VertexAttributeValues;
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return };
    let rock: Vec<bool> = pos.iter().map(|p| p[1] < 2.6 && (p[0] * p[0] + p[2] * p[2]).sqrt() > 7.2).collect();
    if let Some(VertexAttributeValues::Float32x4(col)) = mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR) {
        for (c, &r) in col.iter_mut().zip(&rock) {
            if r {
                c[3] = materials::surface_alpha(masonry::Part::Rock as u8);
            }
        }
    }
    masonry::smooth_normals(mesh, 55.0, Some(&rock));
}
