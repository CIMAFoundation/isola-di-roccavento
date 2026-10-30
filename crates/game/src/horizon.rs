//! Land on the horizon (user, 2026-10-30): a few other islands and a stretch
//! of mainland far out at sea, so the island is not alone in an empty ocean.
//! Very simple on purpose: unlit silhouettes, no shadows, whose colour is the
//! land's mixed with the horizon's (the sky's, every frame) by how far they
//! are. The distance fog would hide them: it reaches 11 km (`sky`), they lie
//! at 19-35 km.

use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::sim::Sim;

pub struct HorizonPlugin;

impl Plugin for HorizonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(crate::AppState::Playing), spawn)
            .add_systems(Update, tint.after(crate::sky::update_sky).run_if(in_state(crate::AppState::Playing)));
    }
}

/// One silhouette and how much of the horizon's colour it takes.
#[derive(Component)]
struct Distant {
    haze: f32,
    material: Handle<StandardMaterial>,
}

/// The land's own colour, before the haze: dark Mediterranean scrub.
const LAND: [f32; 3] = [0.16, 0.19, 0.15];

/// Islands: bearing from the island's centre (degrees from north), distance,
/// radius and height (metres). One stands against the sunrise.
const ISLANDS: [(f32, f32, f32, f32); 5] = [
    (70.0, 21_000.0, 2_200.0, 380.0),
    (96.0, 26_000.0, 1_000.0, 160.0),
    (150.0, 28_000.0, 600.0, 90.0),
    (205.0, 19_000.0, 3_000.0, 520.0),
    (242.0, 24_000.0, 1_500.0, 240.0),
];
/// The mainland: a coast to the north, this far from the island's centre.
const MAINLAND_M: f32 = 30_000.0;

fn hash(i: i32, seed: u32) -> f32 {
    let mut x = (i as u32).wrapping_mul(0x9E37_79B9) ^ seed.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// Smooth value noise along a line, 0..1.
fn noise(x: f32, seed: u32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let f = f * f * (3.0 - 2.0 * f);
    hash(i as i32, seed) * (1.0 - f) + hash(i as i32 + 1, seed) * f
}

/// Three octaves of [`noise`], 0..1.
fn fbm(x: f32, seed: u32) -> f32 {
    (noise(x, seed) * 0.57 + noise(x * 2.1, seed + 1) * 0.29 + noise(x * 4.3, seed + 2) * 0.14).clamp(0.0, 1.0)
}

/// Builds a mesh from rows of points: each row a line of vertices, the rows
/// stitched into quads. `shade` per vertex: 1 at the waterline, darker up.
fn grid(rows: &[Vec<Vec3>], heights: f32) -> Mesh {
    let cols = rows[0].len();
    let mut pos = Vec::new();
    let mut col = Vec::new();
    for r in rows {
        for p in r {
            pos.push(p.to_array());
            let s = 1.0 - 0.25 * (p.y / heights).clamp(0.0, 1.0);
            col.push([s, s, s, 1.0]);
        }
    }
    let mut idx = Vec::new();
    for r in 0..rows.len() - 1 {
        for c in 0..cols - 1 {
            let a = (r * cols + c) as u32;
            let b = a + cols as u32;
            idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    let n = pos.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; n])
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_inserted_indices(Indices::U32(idx))
}

/// An island: rings from the shore (sunk under the sea) to the top, the shore
/// and the slopes wobbling with the bearing.
fn island(radius: f32, height: f32, seed: u32) -> Mesh {
    const SECTORS: usize = 64;
    const RINGS: usize = 10;
    let mut rows = Vec::new();
    for k in 0..=RINGS {
        // k = 0: a skirt under the water; then from the shore up to the top
        let f = if k == 0 { 1.08 } else { 1.0 - (k - 1) as f32 / (RINGS - 1) as f32 };
        let mut row = Vec::new();
        for s in 0..=SECTORS {
            let a = s as f32 / SECTORS as f32 * std::f32::consts::TAU;
            let w = (s % SECTORS) as f32 / SECTORS as f32 * 8.0;
            let shore = radius * (0.75 + 0.5 * fbm(w, seed));
            let h = if k == 0 { -40.0 } else { height * (1.0 - f * f).powf(1.3) * (0.7 + 0.6 * fbm(w * 1.7 + 3.0, seed + 7)) };
            row.push(Vec3::new(a.sin() * shore * f, h, -a.cos() * shore * f));
        }
        rows.push(row);
    }
    grid(&rows, height)
}

/// The mainland: a coast `length` long seen face on, mountains behind it.
fn mainland(length: f32, depth: f32, seed: u32) -> Mesh {
    const ALONG: usize = 220;
    const ACROSS: usize = 7;
    let mut rows = Vec::new();
    for j in 0..=ACROSS {
        let v = j as f32 / ACROSS as f32;
        let mut row = Vec::new();
        for i in 0..=ALONG {
            let u = i as f32 / ALONG as f32;
            let x = (u - 0.5) * length;
            let ridge = 500.0 + 1100.0 * fbm(u * 14.0, seed);
            let coast = 900.0 * (fbm(u * 30.0, seed + 3) - 0.5);
            let h = if j == 0 { -40.0 } else { ridge * v.powf(0.8) * (0.75 + 0.5 * fbm(u * 40.0 + v * 3.0, seed + 5)) };
            row.push(Vec3::new(x, h, -(v * depth + coast)));
        }
        rows.push(row);
    }
    grid(&rows, 1600.0)
}

fn spawn(mut commands: Commands, sim: Res<Sim>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let w = &sim.scn.world;
    let centre = crate::frame::to_bevy(scenario::Pos { x: w.width_m * 0.5, y: w.height_m * 0.5 }, 0.0);
    let mut put = |commands: &mut Commands, mesh: Mesh, at: Vec3, turn: f32, haze: f32| {
        let material = materials.add(StandardMaterial { unlit: true, fog_enabled: false, cull_mode: None, ..default() });
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(at).with_rotation(Quat::from_rotation_y(turn)),
            NotShadowCaster,
            NotShadowReceiver,
            Distant { haze, material },
        ));
    };
    for (k, &(bearing, dist, radius, height)) in ISLANDS.iter().enumerate() {
        let a = bearing.to_radians();
        let at = centre + Vec3::new(a.sin() * dist, 0.0, -a.cos() * dist);
        // farther is paler: 19 km about half the horizon's colour, 28 km more
        let haze = (0.3 + dist / 45_000.0).min(0.9);
        put(&mut commands, island(radius, height, 11 + k as u32 * 17), at, hash(k as i32, 3) * 6.0, haze);
    }
    put(&mut commands, mainland(110_000.0, 7_000.0, 5), centre + Vec3::new(0.0, 0.0, -MAINLAND_M), 0.0, 0.72);
}

/// The land's colour mixed with the horizon's, as the sky sets it.
fn tint(clear: Res<ClearColor>, distant: Query<&Distant>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let h = clear.0.to_srgba();
    for d in &distant {
        if let Some(mut m) = materials.get_mut(&d.material) {
            let mix = |land: f32, sky: f32| land + (sky - land) * d.haze;
            m.base_color = Color::srgb(mix(LAND[0], h.red), mix(LAND[1], h.green), mix(LAND[2], h.blue));
        }
    }
}
