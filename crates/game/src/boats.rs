//! Sailing boats: scenery only (the models are Blender's, see `models::island`). A few lie along the pier in the port, bows
//! into the wind; a few sail the open sea, close-hauled or on a reach, and
//! tack. They are not in the game: no sim state, no clock but the frame's.
//!
//! They read the wind the sea reads ([`crate::sea::wind_drift`]) and ride the
//! swell the water shader raises ([`crate::sea::wave_height`]), so a wind shift
//! turns the moored bows and changes the sailors' course the way it chops the
//! water. Sizes are diorama sizes ([`crate::visual_scale::BOAT`]): a 9 m boat
//! is invisible from the camera's distance otherwise.

use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use scenario::Pos;

use crate::models;
use crate::materials::{self, Finish, StructureMaterial};
use crate::sea;
use crate::sim::Sim;
use crate::visual_scale;

/// Sailing boats, and the longest a sailor keeps one tack, seconds.
const SAILORS: usize = 7;
/// How far from the window's centre a sailor may roam, metres.
const ROAM_M: f32 = 8_500.0;
/// Sail speed exaggeration: at the camera's distance a real 3 m/s is a crawl.
const SPEED_SCALE: f32 = 3.5;

#[derive(Component)]
struct Boat {
    /// Direction of travel in the XZ plane, radians from +X toward +Z.
    heading: f32,
    kind: Kind,
    /// Which side of the wind it sails on (+1 / -1), flipped at each tack.
    side: f32,
    /// How far off the wind it sails, radians: close-hauled to a broad reach.
    off_wind: f32,
    tack_in: f32,
    avoid_s: f32,
    phase: f32,
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Moored,
    Sailing,
}

pub struct BoatsPlugin;

impl Plugin for BoatsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(crate::AppState::Playing), spawn_boats)
            .add_systems(Update, move_boats.run_if(in_state(crate::AppState::Playing)));
    }
}

/// A tiny deterministic generator: the same boats in the same places on every
/// run, with no clock or entropy to make a screenshot differ.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u32 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

// ---- spawning --------------------------------------------------------------------

fn spawn_boats(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StructureMaterial>>,
) {
    let scn = &sim.scn;
    let material = materials.add(materials::structure(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.55,
        reflectance: 0.2,
        // turns the clear-coat path on: the painted hulls keep it (`boat_pbr`)
        clearcoat: 1.0,
        cull_mode: None,
        double_sided: true,
        ..default()
    }, Finish::Boat));
    // The Blender models (`scripts/build_island_models.py`): sloops under sail
    // for the open sea, the same hulls with the sails furled for the berths.
    let sailing_meshes: Vec<_> = ["veliero_bianco", "veliero_crema", "veliero_rosso", "veliero_blu", "veliero_giallo"]
        .iter()
        .map(|n| meshes.add(models::island_mesh(n)))
        .collect();
    let moored_meshes: Vec<_> = ["ormeggiato_bianco", "ormeggiato_blu", "ormeggiato_crema"].iter().map(|n| meshes.add(models::island_mesh(n))).collect();
    let (drift, _) = sea::wind_drift(&sim);
    let upwind = (-drift.y).atan2(-drift.x);
    let mut rng = Lcg(0x0B0A_7500_1D);
    let scale = Vec3::splat(visual_scale::BOAT);
    let bundle = |mesh: Handle<Mesh>, material: Handle<StructureMaterial>, at: Vec3, heading: f32| {
        (
            (
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform { translation: at, rotation: Quat::from_rotation_y(-heading), scale },
            ),
            NotShadowCaster,
            NotShadowReceiver,
        )
    };

    // the moorings the factory laid along the pier, bows into the wind
    let island = datafs::read_to_string(sim.data.join("scenarios").join(sim.scn.id.as_str()).join("island.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let moorings: Vec<Pos> = island
        .as_ref()
        .and_then(|v| v["port"]["moorings"].as_array().cloned())
        .map(|a| a.iter().filter_map(|m| Some(Pos { x: m[0].as_f64()? as f32, y: m[1].as_f64()? as f32 })).collect())
        .unwrap_or_default();
    for (i, m) in moorings.iter().enumerate() {
        if rng.next() < 0.15 {
            continue; // an empty berth
        }
        let at = Vec3::new(m.x, 0.8, -m.y);
        commands.spawn((
            bundle(moored_meshes[i % moored_meshes.len()].clone(), material.clone(), at, upwind),
            Boat { heading: upwind, kind: Kind::Moored, side: 1.0, off_wind: 0.0, tack_in: 0.0, avoid_s: 0.0, phase: rng.range(0.0, 6.28) },
        ));
    }

    // the sailors: open water, away from the shore
    let (cx, cy) = (scn.terrain.width_m * 0.5, scn.terrain.height_m * 0.5);
    let mut placed = 0;
    let mut tries = 0;
    while placed < SAILORS && tries < 4000 {
        tries += 1;
        let ang = rng.range(0.0, std::f32::consts::TAU);
        let rad = rng.range(2_600.0, 5_200.0);
        let p = Pos { x: cx + rad * ang.cos(), y: cy + rad * ang.sin() };
        if !sea::is_water(scn, p) || sea::shore_distance(scn, p) < 160.0 {
            continue;
        }
        let side = if rng.next() < 0.5 { -1.0 } else { 1.0 };
        let off_wind = rng.range(0.8, 2.2);
        let heading = upwind + side * off_wind;
        commands.spawn((
            bundle(sailing_meshes[placed % sailing_meshes.len()].clone(), material.clone(), Vec3::new(p.x, 0.8, -p.y), heading),
            Boat { heading, kind: Kind::Sailing, side, off_wind, tack_in: rng.range(30.0, 90.0), avoid_s: 0.0, phase: rng.range(0.0, 6.28) },
        ));
        placed += 1;
    }
}

// ---- motion ----------------------------------------------------------------------

fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

fn move_boats(time: Res<Time>, sim: Res<Sim>, mut boats: Query<(&mut Transform, &mut Boat)>) {
    let dt = time.delta_secs().min(0.1);
    let t = time.elapsed_secs_wrapped();
    let scn = &sim.scn;
    let (drift, wind) = sea::wind_drift(&sim);
    let upwind = (-drift.y).atan2(-drift.x);
    let (cx, cy) = (scn.terrain.width_m * 0.5, scn.terrain.height_m * 0.5);
    let heel_k = (0.045 * wind).clamp(0.04, 0.30);

    for (mut tf, mut boat) in &mut boats {
        let pos = tf.translation;
        let (roll, pitch);
        match boat.kind {
            Kind::Moored => {
                // bows into the wind, swinging a little on the lines
                let swing = 0.12 * (t * 0.31 + boat.phase).sin();
                let target = upwind + swing;
                boat.heading += angle_diff(target, boat.heading).clamp(-0.5 * dt, 0.5 * dt);
                roll = 0.05 * (t * 0.9 + boat.phase).sin();
                pitch = 0.0;
            }
            Kind::Sailing => {
                boat.tack_in -= dt;
                if boat.tack_in <= 0.0 {
                    boat.side = -boat.side;
                    boat.tack_in = 45.0 + 30.0 * (0.5 + 0.5 * (boat.phase * 7.0 + t * 0.05).sin());
                }
                boat.avoid_s = (boat.avoid_s - dt).max(0.0);
                let f = Vec2::new(boat.heading.cos(), boat.heading.sin());
                let at = |d: f32| Pos { x: pos.x + f.x * d, y: -(pos.z + f.y * d) };
                let blocked = !sea::is_water(scn, at(260.0)) || !sea::is_water(scn, at(520.0));
                let out = Vec2::new(pos.x - cx, -pos.z - cy).length() > ROAM_M;
                if blocked || out {
                    boat.avoid_s = 14.0;
                }
                let mut target = upwind + boat.side * boat.off_wind;
                if boat.avoid_s > 0.0 {
                    // open water: away from the coast or back toward the middle
                    let home = Vec2::new(cx - pos.x, -cy - pos.z);
                    let away = if out { home.y.atan2(home.x) } else { boat.heading + boat.side * 1.9 };
                    target = away;
                }
                boat.heading += angle_diff(target, boat.heading).clamp(-0.35 * dt, 0.35 * dt);
                // sailing speed: slowest hard on the wind, fastest on a reach
                let off = angle_diff(boat.heading, upwind).abs();
                let polar = 0.45 + 0.55 * off.min(std::f32::consts::PI * 0.8).sin();
                let v = (2.2 + 0.32 * wind) * polar * SPEED_SCALE;
                let f = Vec2::new(boat.heading.cos(), boat.heading.sin());
                tf.translation.x += f.x * v * dt;
                tf.translation.z += f.y * v * dt;
                // heel away from the wind, more when close-hauled
                let lee = -(angle_diff(boat.heading, upwind)).signum();
                roll = lee * heel_k * (0.6 + 0.4 * (1.0 - off / std::f32::consts::PI));
                pitch = 0.03 * (t * 1.3 + boat.phase).sin();
            }
        }
        let p = tf.translation;
        let swell = sea::wave_height(p.x, p.z, t, drift, wind);
        tf.translation.y = 0.8 + swell;
        tf.rotation = Quat::from_rotation_y(-boat.heading) * Quat::from_rotation_x(roll) * Quat::from_rotation_z(pitch);
    }
}
