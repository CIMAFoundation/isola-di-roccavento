//! Rocks along the shore, from their Blender models
//! (`scripts/build_coast_models.py` -> `assets/models/coast.json`).
//!
//! Boulders and scree at the foot of the cliffs, a few stones on the beaches,
//! reefs awash and sea stacks (faraglioni) off the rocky stretches. They are
//! scenery, scattered deterministically from the terrain alone: the same coast
//! on every run, nothing for the fire to see. Towns keep a clear margin so the
//! seafront, the lido and the harbour stay as the town kit draws them.
//! Instances are merged per 640 m tile, so the cost is a handful of draw calls
//! and the usual culling.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use scenario::{Pos, Scenario};

use crate::models::{self, Kit};
use crate::materials::{self, Finish, StructureMaterial};
use crate::sea;
use crate::sim::Sim;
use crate::terrain_mesh::{cliff_elev, ShoreField};

pub struct CoastPlugin;

impl Plugin for CoastPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(crate::AppState::Playing), spawn_coast);
    }
}

/// Scatter lattice, metres.
const STEP_M: f32 = 12.0;
/// Merge tile edge, metres.
const TILE_M: f32 = 640.0;
/// Town cells (side `TOWN_CELL_M`) and their neighbours are left alone.
const TOWN_CELL_M: f32 = 50.0;
/// Hard cap on instances.
const MAX_ROCKS: usize = 8000;

fn hash01(x: u64) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u32 << 24) as f32
}

fn smoothstep01(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// One rock to draw: model, ground point, height of its base, scale, yaw.
struct Rock {
    model: &'static str,
    at: Pos,
    base: f32,
    scale: f32,
    yaw: f32,
}

/// The direction (unit, world) and distance to the nearest water within
/// `reach` metres of `p`, if any.
fn water_near(scn: &Scenario, p: Pos, reach: f32) -> Option<(Vec2, f32)> {
    let mut r = 6.0;
    while r <= reach {
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let q = Pos { x: p.x + a.cos() * r, y: p.y + a.sin() * r };
            if scn.world.contains(q) && sea::is_water(scn, q) {
                return Some((Vec2::new(a.cos(), a.sin()), r));
            }
        }
        r += 6.0;
    }
    None
}

/// Whether land rises to a cliff within `reach` metres of `p`.
fn cliff_near(scn: &Scenario, p: Pos, reach: f32) -> bool {
    (0..8).any(|k| {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let q = Pos { x: p.x + a.cos() * reach, y: p.y + a.sin() * reach };
        scn.world.contains(q) && scn.terrain.height_at(q) > 9.0
    })
}

fn scatter(scn: &Scenario) -> Vec<Rock> {
    let shore = ShoreField::new(scn);
    let towns: HashSet<(i32, i32)> = scn
        .vectors
        .buildings
        .iter()
        .map(|b| ((b.centroid[0] / TOWN_CELL_M).floor() as i32, (b.centroid[1] / TOWN_CELL_M).floor() as i32))
        .collect();
    let in_town = |p: Pos| {
        let (cx, cy) = ((p.x / TOWN_CELL_M).floor() as i32, (p.y / TOWN_CELL_M).floor() as i32);
        (-1..=1).any(|dx| (-1..=1).any(|dy| towns.contains(&(cx + dx, cy + dy))))
    };
    let (w, h) = (scn.world.width_m, scn.world.height_m);
    let mut rocks = Vec::new();
    let (nx, ny) = ((w / STEP_M) as u64, (h / STEP_M) as u64);
    for j in 0..ny {
        for i in 0..nx {
            if rocks.len() >= MAX_ROCKS {
                return rocks;
            }
            let id = j * nx + i;
            let jx = hash01(id ^ 0x11) - 0.5;
            let jy = hash01(id ^ 0x22) - 0.5;
            let p = Pos { x: (i as f32 + 0.5 + jx) * STEP_M, y: (j as f32 + 0.5 + jy) * STEP_M };
            let pick = hash01(id ^ 0x33);
            let elev = scn.terrain.height_at(p);
            let yaw = hash01(id ^ 0x44) * std::f32::consts::TAU;
            let size = hash01(id ^ 0x55);

            if elev > 0.6 {
                // Land: only the low strip by the sea.
                if elev > 14.0 || pick > 0.34 || shore.at(p) > 40.0 {
                    continue;
                }
                let Some((dir, dist)) = water_near(scn, p, 24.0) else { continue };
                if in_town(p) {
                    continue;
                }
                let steep = scn.terrain.normal_at(p)[1] < 0.93;
                // Toward the water, where the fallen blocks lie.
                let at = Pos { x: p.x + dir.x * dist * 0.5, y: p.y + dir.y * dist * 0.5 };
                let ground = cliff_elev(scn, &shore, at, scn.terrain.height_at(at)).max(0.0);
                let (model, scale) = if steep {
                    match (size * 4.0) as u32 {
                        0 => ("scree", 1.5 + size * 2.0),
                        1 => ("rock_b", 1.6 + size * 1.6),
                        2 => ("rock_a", 1.8 + size * 1.8),
                        _ => ("rock_c", 1.8 + size * 1.6),
                    }
                } else if pick < 0.04 {
                    // a stone or two on a beach
                    ("rock_a", 0.8 + size * 0.8)
                } else {
                    continue;
                };
                rocks.push(Rock { model, at, base: ground - 0.25 * scale, scale, yaw });
            } else {
                // Sea: awash reefs and stacks, only off rocky shores.
                if pick > 0.045 {
                    continue;
                }
                let d = sea::shore_distance(scn, p);
                if !(12.0..=70.0).contains(&d) || in_town(p) || !cliff_near(scn, p, d + 40.0) {
                    continue;
                }
                let (model, scale, base) = if pick < 0.004 {
                    ("sea_stack", 1.6 + size * 1.4, -1.0)
                } else if pick < 0.02 {
                    ("rock_b", 1.4 + size * 1.4, -0.9)
                } else {
                    ("reef", 1.4 + size * 1.6, 0.0)
                };
                rocks.push(Rock { model, at: p, base, scale, yaw });
            }
        }
    }
    rocks
}

/// A coast model as it is drawn: the bake, or with the photographed rock
/// (`real`) the bake refined (`landmarks::rock::refine`: welded, split,
/// rounded and broken up; a boulder of forty flat facets reads as a cut gem
/// next to the scanned plants). Made once per model.
struct Shape {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

fn shape(name: &'static str, real: bool) -> &'static Shape {
    use std::sync::OnceLock;
    static PLAIN: OnceLock<HashMap<&'static str, Shape>> = OnceLock::new();
    static REAL: OnceLock<HashMap<&'static str, Shape>> = OnceLock::new();
    const NAMES: [&str; 6] = ["rock_a", "rock_b", "rock_c", "scree", "sea_stack", "reef"];
    let of = |k: &Kit| Shape { positions: k.positions.clone(), normals: k.normals.clone(), colors: k.colors.clone(), indices: k.indices.clone() };
    let set = if real {
        REAL.get_or_init(|| {
            use crate::landmarks::rock;
            NAMES
                .iter()
                .map(|&n| {
                    let k = models::coast(n);
                    // scree is a heap of chips, small on screen: left as it is
                    let how = match n {
                        "scree" => return (n, of(k)),
                        "sea_stack" => &rock::SEA_STACK,
                        _ => &rock::COAST_BOULDER,
                    };
                    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
                    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, k.positions.clone());
                    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, k.colors.iter().map(|c| [c[0], c[1], c[2], 1.0]).collect::<Vec<_>>());
                    mesh.insert_indices(Indices::U32(k.indices.clone()));
                    rock::refine(&mut mesh, how);
                    use bevy::mesh::VertexAttributeValues as V;
                    let (Some(V::Float32x3(p)), Some(V::Float32x3(nr)), Some(V::Float32x4(c))) =
                        (mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.attribute(Mesh::ATTRIBUTE_NORMAL), mesh.attribute(Mesh::ATTRIBUTE_COLOR))
                    else {
                        return (n, of(k));
                    };
                    let indices = match mesh.indices() {
                        Some(Indices::U16(i)) => i.iter().map(|&x| x as u32).collect(),
                        Some(Indices::U32(i)) => i.clone(),
                        None => (0..p.len() as u32).collect(),
                    };
                    (n, Shape { positions: p.clone(), normals: nr.clone(), colors: c.iter().map(|c| [c[0], c[1], c[2]]).collect(), indices })
                })
                .collect()
        })
    } else {
        PLAIN.get_or_init(|| NAMES.iter().map(|&n| (n, of(models::coast(n)))).collect())
    };
    &set[name]
}

/// Merge `rocks` of one tile into a mesh (`real`: see [`shape`]).
fn tile_mesh(rocks: &[&Rock], real: bool) -> Mesh {
    let (mut pos, mut nor, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for r in rocks {
        let m: &Shape = shape(r.model, real);
        let rot = Quat::from_rotation_y(r.yaw);
        let origin = crate::frame::to_bevy(r.at, r.base);
        let start = pos.len() as u32;
        for ((p, n), c) in m.positions.iter().zip(&m.normals).zip(&m.colors) {
            pos.push((origin + rot * (Vec3::from(*p) * r.scale)).to_array());
            nor.push((rot * Vec3::from(*n)).to_array());
            // Wet at the foot: dark and cool where the spray reaches, bleaching
            // to the model's own limestone higher up. Sunken blocks and reefs
            // (base at or under the water) are wet all the way to the top.
            let height = (p[1] * r.scale).max(0.0);
            let dry = if r.base < 0.5 { smoothstep01((height + 0.9 + r.base) / 1.8) } else { smoothstep01(height / 1.4) };
            let wet = [0.26, 0.27, 0.28];
            let k = 0.35 + 0.65 * dry;
            let mut c = *c;
            if r.model == "sea_stack" && c[0] > 0.5 {
                // beds of ochre and pale with a grey seam, by height
                let t = (p[1] * r.scale / 3.6 + 0.3 * (p[0] * 0.9 + p[2] * 0.4).sin()).rem_euclid(1.0);
                let ochre = smoothstep01((t - 0.30) / 0.1) - smoothstep01((t - 0.62) / 0.1);
                let seam = smoothstep01((t - 0.88) / 0.06) - smoothstep01((t - 0.96) / 0.04);
                for (i, (o, g)) in [(0.62, 0.50), (0.46, 0.49), (0.29, 0.46)].into_iter().enumerate() {
                    c[i] = c[i] * 0.86 * (1.0 - ochre) + o * ochre + (g - c[i] * 0.86) * seam * 0.6;
                }
            }
            col.push([c[0] * k + wet[0] * (1.0 - k) * 0.5, c[1] * k + wet[1] * (1.0 - k) * 0.5, c[2] * k + wet[2] * (1.0 - k) * 0.5, 1.0]);
        }
        idx.extend(m.indices.iter().map(|i| start + i));
    }
    let n = pos.len();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; n]);
    mesh.insert_indices(Indices::U32(idx));
    mesh.compress_indices();
    mesh
}

fn spawn_coast(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StructureMaterial>>,
    mut masonry_materials: ResMut<Assets<crate::materials::masonry::MasonryMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut done: Local<bool>,
) {
    if std::mem::replace(&mut *done, true) {
        return;
    }
    let rocks = scatter(&sim.scn);
    let material = materials.add(materials::structure(
        StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            reflectance: 0.04,
            ..default()
        },
        Finish::Rock,
    ));
    let mut tiles: HashMap<(i32, i32), Vec<&Rock>> = HashMap::new();
    for r in &rocks {
        tiles.entry(((r.at.x / TILE_M) as i32, (r.at.y / TILE_M) as i32)).or_default().push(r);
    }
    info!("coast: {} rocks in {} tiles", rocks.len(), tiles.len());
    // With the scanned plants, the terrain's photographed limestone, lit
    // smooth over the boulders' facets (`materials::masonry`).
    use crate::materials::masonry;
    let real = masonry::textures(&mut images).map(|t| {
        masonry_materials.add(masonry::material(
            StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.95, reflectance: 0.04, ..default() },
            &t,
            masonry::Masonry::Rock,
        ))
    });
    for (_, tile) in tiles {
        let mesh = tile_mesh(&tile, real.is_some());
        match &real {
            Some(m) => {
                commands.spawn(((Mesh3d(meshes.add(mesh)), MeshMaterial3d(m.clone())), crate::terrain_mesh::PauseSolid));
            }
            None => {
                commands.spawn(((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.clone())), crate::terrain_mesh::PauseSolid));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_coast_gets_rocks_of_every_kind_and_the_models_are_sound() {
        for name in ["rock_a", "rock_b", "rock_c", "scree", "sea_stack", "reef"] {
            let m = models::coast(name);
            assert!(!m.positions.is_empty() && m.indices.len() % 3 == 0, "{name}");
            assert!(m.positions.iter().flatten().all(|v| v.is_finite()), "{name}");
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let Ok(scn) = Scenario::load(&dir) else { return };
        let rocks = scatter(&scn);
        let count = |m: &str| rocks.iter().filter(|r| r.model == m).count();
        eprintln!(
            "coast: {} rocks (scree {}, a {}, b {}, c {}, reef {}, stack {})",
            rocks.len(), count("scree"), count("rock_a"), count("rock_b"), count("rock_c"), count("reef"), count("sea_stack")
        );
        assert!(rocks.len() > 50 && rocks.len() <= MAX_ROCKS);
        assert!(count("sea_stack") > 0 && count("reef") > 0);
    }
}
