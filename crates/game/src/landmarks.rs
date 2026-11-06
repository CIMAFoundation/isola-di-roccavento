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

pub mod rock;

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
        let mut mesh = match (real.is_some(), model) {
            // the rocca made again for the photographed stone
            // (`scripts/build_rocca_real.py`): stepped broken tops, wall-walks
            // and parapets, openings cut through the walls with dressed jambs
            // and arches. The crag under it is the island bake's own.
            (true, "rocca") => rocca_scans().unwrap_or_else(|| models::island_mesh(model)),
            _ => models::island_mesh(model),
        };
        if real.is_some() {
            without_greenery(&mut mesh);
        }
        if real.is_some() && model == "torre_pietra" {
            // the boulders at its foot: a mesh of their own, refined as rock
            let mut boulders = boulders_of_the_tower(&mut mesh);
            rock::refine(&mut boulders, &rock::BOULDERS);
            if let Some(m) = &masonry_material {
                commands.spawn(((Mesh3d(meshes.add(boulders)), MeshMaterial3d(m.clone()), transform), crate::terrain_mesh::PauseSolid));
            }
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
                    // a heightfield of 3 m cells and stacked prisms: refined,
                    // rounded and broken into a rock face (`rock::refine`)
                    rock::refine(&mut crag, &rock::CRAG);
                    ground_under(&mut crag, &transform, &sim.scn.terrain);
                    commands.spawn(((Mesh3d(meshes.add(crag)), MeshMaterial3d(m.clone()), transform), crate::terrain_mesh::PauseSolid));
                }
                None => {
                    commands.spawn(((Mesh3d(meshes.add(crag)), MeshMaterial3d(rock.clone()), transform), crate::terrain_mesh::PauseSolid));
                }
            }
        }
    }
}

/// The terrain's height under each vertex of a landmark, in world metres, in
/// its first uv (`masonry.wesl`: the crag gathers scree and soil at its foot,
/// so it does not stop on the ground in a clean line).
fn ground_under(mesh: &mut Mesh, transform: &Transform, terrain: &scenario::Terrain) {
    use bevy::mesh::VertexAttributeValues;
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return };
    let uv: Vec<[f32; 2]> = pos
        .iter()
        .map(|&v| {
            let w = transform.transform_point(Vec3::from(v));
            [terrain.height_at(crate::frame::to_world(w)), 0.0]
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
}

/// The rocca of `assets/models/rocca_scans.json` (`scripts/build_rocca_real.py`),
/// in the layout of `island.json`. Compiled into the desktop binary; in the
/// browser it is not in the page's data (`scripts/build_web.sh` leaves out
/// what is named `_scans`), and the island bake's rocca is drawn instead.
/// Like every landmark it keeps `MAIN_WORLD`: Mario walks on what is drawn.
fn rocca_scans() -> Option<Mesh> {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    #[cfg(not(target_arch = "wasm32"))]
    let text: std::borrow::Cow<'static, str> = include_str!("../../../assets/models/rocca_scans.json").into();
    #[cfg(target_arch = "wasm32")]
    let text: std::borrow::Cow<'static, str> = String::from_utf8(datafs::take("assets/models/rocca_scans.json").ok()?).ok()?.into();
    let mut kits: std::collections::HashMap<String, models::Kit> =
        serde_json::from_str(&text).map_err(|e| warn!("rocca_scans.json: {e}")).ok()?;
    let m = kits.remove("rocca")?;
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
    let n = m.positions.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, m.colors.iter().map(|c| [c[0], c[1], c[2], 1.0]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; n]);
    mesh.insert_indices(Indices::U32(m.indices));
    mesh.compress_indices();
    Some(mesh)
}

/// With the photographed masonry: the ivy and scrub of the rocca's bake
/// (flattened icospheres in greens, `scripts/build_island_models.py`) read
/// as green gems next to the photographed stone and the scanned plants, and
/// are left out; the shader grows lichen and moss on the stone instead
/// (`masonry.wesl`, `overgrown`). Green is what the shader took for ivy
/// before (`MASONRY_LANDMARK`).
fn without_greenery(mesh: &mut Mesh) {
    use bevy::mesh::{Indices, VertexAttributeValues};
    let Some(VertexAttributeValues::Float32x4(col)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { return };
    let green: Vec<bool> = col.iter().map(|c| c[1] > c[0] * 1.05 && c[1] > c[2] && c[1] > 0.04).collect();
    let idx: Vec<u32> = match mesh.indices() {
        Some(Indices::U16(i)) => i.iter().map(|&k| k as u32).collect(),
        Some(Indices::U32(i)) => i.clone(),
        None => return,
    };
    let keep: Vec<u32> = idx.chunks_exact(3).filter(|t| !t.iter().all(|&i| green[i as usize])).flatten().copied().collect();
    mesh.insert_indices(Indices::U32(keep));
    mesh.compress_indices();
}

/// With the photographed masonry: the boulders heaped at the foot of the
/// stone tower are rock, not masonry (`masonry::Part::Rock`, the terrain's
/// limestone). They are taken out of `mesh` (what of the bake lies low and
/// outside the tower's shaft, in model metres: the shaft is 6.5 m in radius at
/// its foot, the heap reaches 11 m) and returned as a mesh of their own, for
/// `rock::refine`: a few dozen flat facets read as cut gems.
fn boulders_of_the_tower(mesh: &mut Mesh) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()) };
    let pos = pos.clone();
    let Some(VertexAttributeValues::Float32x4(col)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { return Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()) };
    let col = col.clone();
    let idx: Vec<u32> = match mesh.indices() {
        Some(Indices::U16(i)) => i.iter().map(|&k| k as u32).collect(),
        Some(Indices::U32(i)) => i.clone(),
        None => (0..pos.len() as u32).collect(),
    };
    let rock = |p: &[f32; 3]| p[1] < 2.6 && (p[0] * p[0] + p[2] * p[2]).sqrt() > 7.2;
    let (mut keep, mut out_p, mut out_c) = (Vec::new(), Vec::new(), Vec::new());
    for t in idx.chunks_exact(3) {
        if t.iter().all(|&i| rock(&pos[i as usize])) {
            for &i in t {
                out_p.push(pos[i as usize]);
                let mut c = col[i as usize];
                c[3] = materials::surface_alpha(masonry::Part::Rock as u8);
                out_c.push(c);
            }
        } else {
            keep.extend_from_slice(t);
        }
    }
    mesh.insert_indices(Indices::U32(keep));
    mesh.compress_indices();
    let n = out_p.len();
    let mut b = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
    b.insert_attribute(Mesh::ATTRIBUTE_POSITION, out_p);
    b.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    b.insert_attribute(Mesh::ATTRIBUTE_COLOR, out_c);
    b.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; n]);
    b
}
