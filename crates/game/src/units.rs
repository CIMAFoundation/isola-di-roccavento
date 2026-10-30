//! Drawing the suppression units, their orders, and what their work left behind.
//!
//! Three layers, and they answer three different questions the commander asks
//! constantly:
//!
//! - **the units** — where is my engine *now*, and is it working or driving?
//! - **the orders** — what did I tell it to do, and where? A ring at the task
//!   point and, for a hand line, the alignment itself with the cut part drawn
//!   solid and the rest ghosted, so "how far have they got" is a glance.
//! - **the work** — cut line and wetted ground. Without this the suppression
//!   model is invisible: water and fuel removal only ever show up as fire that
//!   *didn't* spread, and a player cannot see a counterfactual.
//!
//! Everything is drawn as a map symbol rather than to scale, for the reason
//! [`crate::people`] spells out: at command altitude a fire engine is a pixel.
//! Same family, same exaggeration, and the [`SYMBOL_SCALE`] knob is deliberately
//! larger than the civilian one — a unit the player has to click on and re-task
//! has to be findable, and there are eight of them against 1,577 people.
//!
//! The one thing worth knowing before editing: the work overlay sits at
//! [`crate::rings::RING_LIFT_M`] above the ground like every other
//! symbol here, *not* on it. Vegetation on this map is 5–15 m of real plants, so
//! a cut line painted on the terrain is rendered perfectly and seen never.

use abm::suppression::{Task, UnitKind, UnitState};
use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use scenario::{Cell, Pos, Scenario};

use crate::materials::{self, Finish, StructureMaterial};
use crate::frame;
use crate::rings::{ring_mesh, tube_transform, wall_mesh, TubeSpin, RING_LIFT_M};
use crate::sim::Sim;

/// How far above life size units are drawn.
pub(crate) const SYMBOL_SCALE: f32 = crate::visual_scale::UNIT;

pub(crate) fn symbol_scale(vr: bool) -> f32 {
    SYMBOL_SCALE * if vr { 1.45 } else { 1.0 }
}

/// Altitude an air tanker is drawn at, metres above the ground beneath it. A
/// Canadair on a drop run is at 30-45 m; drawn there it is lost in the canopy
/// and the smoke, so it flies as a symbol well above both.
pub(crate) const AIR_ALTITUDE_M: f32 = 220.0;

/// Radius of the ring marking where a unit was ordered to work.
const ORDER_RING_M: f32 = 45.0;

#[derive(Component)]
pub struct UnitView {
    pub id: usize,
}

/// A ring or ribbon belonging to a unit's current order. Rebuilt wholesale
/// whenever the orders change, which is rare and cheap.
#[derive(Component)]
pub struct OrderMarker;

/// The arc of water one fire engine throws: a single ribbon mesh, rewritten
/// every frame by `animate_spray`, that sweeps across the cells being wetted.
#[derive(Component)]
pub struct Jet {
    unit: usize,
}

/// Cells an engine's jet sweeps across, and the segments an arc is cut into.
const JETS: usize = 4;
/// Seconds the jet rests on one cell, and takes to turn to the next.
const DWELL_S: f32 = 1.6;
const TURN_S: f32 = 0.9;
const ARC_SEGMENTS: usize = 24;
/// Height of the nozzle above the ground, and of the arc's crown over the chord, metres.
const NOZZLE_M: f32 = 9.0;
const ARC_BASE_M: f32 = 8.0;
const ARC_PER_M: f32 = 0.18;
/// Width of the stream at the nozzle and where it lands, metres.
const STREAM_NOZZLE_M: f32 = 1.4;
const STREAM_END_M: f32 = 3.6;

/// The cut-line overlay (nothing is drawn on the wetted ground any more).
#[derive(Component)]
pub struct WorkOverlay;

#[derive(Resource)]
pub struct UnitAssets {
    order_ring: Handle<StandardMaterial>,
    line_cut: Handle<StandardMaterial>,
    line_todo: Handle<StandardMaterial>,
    /// Suppression generation the markers were built for.
    orders_gen: u64,
    /// Fire generation the work overlay was built for, plus the counts it was
    /// built from: the overlay only changes when a cell is newly cut or newly
    /// wet, which is far rarer than the fire changing.
    work_gen: u64,
    /// Wet cells seen at the last scan (so a cell is announced once, when it
    /// first gets wet) and the ones on show now, with the simulated time they appeared.
    wet_seen: Vec<usize>,
    /// Scratch for the next scan (swapped with `wet_seen`, so no allocation).
    wet_next: Vec<usize>,
    wet_shown: Vec<(usize, f32)>,
    /// What the overlay was last built from, to rebuild only on a change.
    wet_built: usize,
}


/// The models carry their real colours (red engine, orange jackets, white
/// aircraft), so the material tint is white. Only the two states the player must
/// not miss colour them: a unit pulling out and one that is lost.
pub fn tint(state: UnitState) -> Color {
    match state {
        UnitState::Lost => Color::srgb(0.10, 0.10, 0.10),
        UnitState::Withdrawing => Color::srgb(1.00, 0.55, 0.50),
        _ => Color::WHITE,
    }
}


/// An animated part of a unit (a wheel, a propeller, a limb, a beacon): a child
/// of the unit's symbol, turning about the pivot of its model.
#[derive(Component)]
pub struct Rigged {
    unit: usize,
    rig: crate::models::Rig,
    /// Accumulated rotation, or the gait clock for limbs, radians.
    clock: f32,
    /// How much of the gait is on, 0..1, so limbs ease in and out.
    amp: f32,
}

/// Where a unit was last frame and when it last moved, in real seconds.
#[derive(Clone, Copy)]
pub struct Motion {
    pub pos: Pos,
    pub moved_at: f32,
    /// Metres per real second at the last move.
    pub speed: f32,
}

/// Where each unit is *drawn*. The game moves its people in steps of
/// [`rocca::STEP_S`] simulated seconds, which at a slow speed are seconds of
/// real time apart, so a walker would stand still and then jump. The drawing
/// glides from the last drawn spot to the new one over the real time the step
/// lasts. Only the picture: the game state is not touched.
#[derive(Resource, Default)]
pub struct Glide(Vec<Option<Glided>>);

#[derive(Clone, Copy)]
struct Glided {
    from: Pos,
    to: Pos,
    shown: Pos,
    /// Real time the glide began and lasts.
    t0: f32,
    span: f32,
}

/// A step longer than this is a cut (a restart, a move of the whole scene), not a walk.
const GLIDE_MAX_M: f32 = 250.0;

pub fn setup(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bodies: ResMut<Assets<StructureMaterial>>,
) {
    // Blender models (scripts/build_unit_models.py): the engine, a firefighter
    // on foot (each person of a team is a unit of its own) and the air tanker.
    let engine = meshes.add(crate::models::unit_mesh("engine"));
    let crew = meshes.add(crate::models::unit_mesh("firefighter"));
    let tanker = meshes.add(crate::models::unit_mesh("tanker"));

    let vr = false;
    let symbol_scale = symbol_scale(vr);
    // Paint, glass, metal, cloth... per vertex (`materials::surface_alpha`):
    // the base only carries the status tint. A clear coat here turns the
    // shader's clear-coat path on; the paint is the only part that keeps it.
    // Findable in smoke shadow by their hi-vis tape and lamps, not by glowing
    // all over.
    let mut mat = |c: Color| {
        bodies.add(materials::structure(
            StandardMaterial { base_color: c, perceptual_roughness: 0.5, clearcoat: 1.0, unlit: vr, ..default() },
            Finish::None,
        ))
    };

    for u in &sim.crews.units {
        let (mesh, scale) = match u.kind {
            UnitKind::Engine => (engine.clone(), symbol_scale),
            UnitKind::HandCrew => (crew.clone(), symbol_scale),
            UnitKind::AirTanker => (tanker.clone(), symbol_scale * 1.6),
        };
        let ground = sim.scn.terrain.height_at(u.pos);
        let material = mat(tint(u.state));
        let model = match u.kind {
            UnitKind::Engine => "engine",
            UnitKind::HandCrew => "firefighter",
            UnitKind::AirTanker => "tanker",
        };
        commands
            .spawn((
                (
                    Mesh3d(mesh),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(frame::to_bevy(u.pos, ground))
                        .with_scale(Vec3::splat(scale)),
                    // Air support is not on the map until it is actually here.
                    if u.on_scene() {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    },
                ),
                UnitView { id: u.id },
            ))
            .with_children(|parent| {
                for (_, part, rig) in crate::models::unit_parts(model) {
                    parent.spawn((
                        (
                            Mesh3d(meshes.add(part)),
                            MeshMaterial3d(material.clone()),
                            Transform::from_translation(rig.pivot),
                        ),
                        Rigged { unit: u.id, rig, clock: 0.0, amp: 0.0 },
                    ));
                }
            });
    }
    info!("suppression layer: {} unit symbols", sim.crews.units.len());

    // The jets: one arc per engine, moved every frame by `animate_spray`.
    let arc_mat = materials.add(StandardMaterial {
        base_color: Color::LinearRgba(LinearRgba::new(0.55, 1.3, 2.6, 0.6)),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    for u in sim.crews.units.iter().filter(|u| u.kind == UnitKind::Engine) {
        // Never an empty mesh: the prepass panics on one. A flat arc is valid, and replaced on the first frame.
        let flat: Vec<(Vec3, f32)> = (0..=ARC_SEGMENTS).map(|_| (Vec3::ZERO, 0.0)).collect();
        commands.spawn((
            (
                Mesh3d(meshes.add(arc_mesh(&flat))),
                MeshMaterial3d(arc_mat.clone()),
                Visibility::Hidden,
            ),
            bevy::camera::visibility::NoFrustumCulling,
            Jet { unit: u.id },
        ));
    }

    let order_ring = hdr_blue(&mut materials, 0.95);
    let mut unlit = |r: f32, g: f32, b: f32, a: f32| {
        materials.add(StandardMaterial {
            base_color: Color::srgba(r, g, b, a),
            emissive: LinearRgba::rgb(r * 1.4, g * 1.4, b * 1.4),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        })
    };
    commands.insert_resource(UnitAssets {
        order_ring,
        // Lines to cut: dashed yellow ahead of the crew, solid yellow once cut (user, 2026-10-15).
        line_cut: unlit(1.0, 0.85, 0.10, 0.95),
        line_todo: unlit(1.0, 0.85, 0.10, 0.95),
        // Cut line reads as bare earth; wetted ground as a cool wash. The wash
        // is deliberately faint: a busy pair of aircraft saturates a hundred
        // cells inside an hour, and at any higher alpha that is a lake painted
        // over the hillside rather than an indication of where the water went.
        orders_gen: u64::MAX,
        work_gen: u64::MAX,
        wet_seen: Vec::new(),
        wet_next: Vec::new(),
        wet_shown: Vec::new(),
        wet_built: 0,
    });
}

/// A restart rebuilds the roster, so every order marker and every metre of
/// overlay belongs to a run that no longer exists.
///
/// The unit *symbols* are not touched: unit ids are stable across a rebuild
/// (`Suppression::new` is deterministic), so they are in the same family as the
/// person entities — keyed by id, and the same roster either way. What has to go
/// is everything keyed by *state*: the markers, and the caches that would
/// otherwise decide the overlay is still current.
pub fn reset(
    mut commands: Commands,
    mut restarted: MessageReader<crate::sim::SimRestarted>,
    mut assets: ResMut<UnitAssets>,
    markers: Query<Entity, With<OrderMarker>>,
    overlay: Query<Entity, With<WorkOverlay>>,
) {
    if restarted.is_empty() {
        return;
    }
    restarted.clear();
    for e in markers.iter().chain(overlay.iter()) {
        commands.entity(e).despawn();
    }
    assets.orders_gen = u64::MAX;
    assets.work_gen = u64::MAX;
    assets.wet_seen.clear();
    assets.wet_next.clear();
    assets.wet_shown.clear();
    assets.wet_built = 0;
}

pub fn update_units(
    sim: Res<Sim>,
    time: Res<Time>,
    mut glide: ResMut<Glide>,
    mut materials: ResMut<Assets<StructureMaterial>>,
    mut query: Query<(
        &UnitView,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StructureMaterial>,
    )>,
) {
    let now = time.elapsed_secs();
    if glide.0.len() != sim.crews.units.len() {
        glide.0.clear();
        glide.0.resize(sim.crews.units.len(), None);
    }
    // real seconds one game step lasts at this speed (0 = no glide)
    let span = if sim.speed > 0.0 { rocca::STEP_S as f32 / sim.speed } else { 0.0 };
    for (u, g) in sim.crews.units.iter().zip(glide.0.iter_mut()) {
        let mut cur = g.unwrap_or(Glided { from: u.pos, to: u.pos, shown: u.pos, t0: now, span: 0.0 });
        if (u.pos.x, u.pos.y) != (cur.to.x, cur.to.y) {
            let far = (u.pos.x - cur.shown.x).hypot(u.pos.y - cur.shown.y) > GLIDE_MAX_M;
            cur = Glided { from: if far { u.pos } else { cur.shown }, to: u.pos, shown: cur.shown, t0: now, span };
        }
        let f = if cur.span > 0.0 { ((now - cur.t0) / cur.span).clamp(0.0, 1.0) } else { 1.0 };
        cur.shown = Pos { x: cur.from.x + (cur.to.x - cur.from.x) * f, y: cur.from.y + (cur.to.y - cur.from.y) * f };
        *g = Some(cur);
    }
    for (view, mut tf, mut vis, mat) in &mut query {
        let Some(u) = sim.crews.units.get(view.id) else {
            continue;
        };
        let shown = glide.0.get(view.id).copied().flatten().map_or(u.pos, |g| g.shown);
        // people sitting in the engine are the engine's symbol
        let want = if (u.on_scene() && !u.aboard) || u.state == UnitState::Lost {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        if want == Visibility::Hidden {
            continue;
        }

        let ground = sim.scn.terrain.height_at(shown);
        let lift = if u.kind.is_air() {
            AIR_ALTITUDE_M
        } else {
            0.05
        };
        tf.translation = frame::to_bevy(shown, ground + lift);
        // World bearing to Bevy yaw: north is -Z and the meshes' long axis is
        // +Z, so a heading of zero (due east) is a quarter turn. Same conversion
        // as the civilian vehicles, for the same reason.
        let forward_offset = if u.kind.is_air() {
            -std::f32::consts::FRAC_PI_2
        } else {
            std::f32::consts::FRAC_PI_2
        };
        tf.rotation = Quat::from_rotation_y(u.heading + forward_offset);

        // One material per entity here rather than a shared palette: there are
        // eight of these, and mutating the material in place is what makes a
        // unit visibly change state without a swap table.
        if let Some(mut m) = materials.get_mut(mat) {
            let c = tint(u.state);
            if m.base.base_color != c {
                m.base.base_color = c;
            }
        }
    }
}

/// Turn the wheels and propellers, swing the limbs, blink the beacons. On real
/// time and on what the unit is doing: the sim runs at x300, and wheels or legs
/// tied to it would only strobe. A unit that moved in the last quarter second
/// is travelling; the world paused, everything stands still (but the aircraft's
/// propellers and the beacons).
pub fn animate_units(
    sim: Res<Sim>,
    time: Res<Time>,
    glide: Res<Glide>,
    mut motion: Local<Vec<Option<Motion>>>,
    mut parts: Query<(&mut Rigged, &mut Transform, &mut Visibility)>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs().max(1e-4);
    if motion.len() != sim.crews.units.len() {
        motion.clear();
        motion.resize(sim.crews.units.len(), None);
    }
    for ((u, g), m) in sim.crews.units.iter().zip(glide.0.iter()).zip(motion.iter_mut()) {
        // the drawn spot, not the game's: a walker at 1x moves 2 cm a frame
        let pos = g.map_or(u.pos, |g| g.shown);
        let moved = m.map_or(0.0, |m| (pos.x - m.pos.x).hypot(pos.y - m.pos.y));
        let prev = m.unwrap_or(Motion { pos, moved_at: f32::NEG_INFINITY, speed: 0.0 });
        *m = Some(if moved / dt > 0.2 {
            Motion { pos, moved_at: now, speed: moved / dt }
        } else {
            Motion { pos, ..prev }
        });
    }

    for (mut p, mut tf, mut vis) in &mut parts {
        let (Some(u), Some(Some(m))) = (sim.crews.units.get(p.unit), motion.get(p.unit)) else {
            continue;
        };
        let travelling = now - m.moved_at < 0.25;
        let working = u.state == UnitState::Working;
        let axis = p.rig.axis;
        let phase = p.rig.phase;
        let kind = p.rig.kind.clone();
        match kind.as_str() {
            "wheel" => {
                // rolling without slipping on a 0.52 m wheel, capped before it strobes
                let omega = if travelling { (m.speed / 0.52).min(12.0) } else { 0.0 };
                p.clock = (p.clock + omega * dt) % std::f32::consts::TAU;
                tf.rotation = Quat::from_axis_angle(axis, p.clock);
            }
            "prop" => {
                p.clock = (p.clock + 30.0 * dt) % std::f32::consts::TAU;
                tf.rotation = Quat::from_axis_angle(axis, p.clock);
            }
            kind @ ("swing" | "tool") => {
                let on = if kind == "tool" { travelling || working } else { travelling };
                let target = if on { 1.0 } else { 0.0 };
                p.amp += (target - p.amp) * (8.0 * dt).min(1.0);
                // legs and arms keep step; the tool arm carries gently while walking
                // and chops when the crew works
                let (rate, reach) = match (kind, working && !travelling) {
                    ("tool", true) => (5.0, 0.9),
                    ("tool", false) => (9.0, 0.3),
                    _ => (9.0, 0.7),
                };
                p.clock += rate * dt;
                let a = p.amp * reach * (p.clock + phase).sin();
                tf.rotation = Quat::from_axis_angle(axis, a);
            }
            "blink" => {
                let active = matches!(
                    u.state,
                    UnitState::Moving | UnitState::Working | UnitState::Refilling | UnitState::Withdrawing
                );
                let lit = active && ((now * 4.0).floor() as i32 + phase as i32) % 2 == 0;
                let want = if lit { Visibility::Inherited } else { Visibility::Hidden };
                if *vis != want {
                    *vis = want;
                }
            }
            _ => {}
        }
    }
}

/// Move the messenger patrols: on the road, at the door they knock on, hidden
/// while parked at their station; dark once caught by the fire.
/// UI-in-3D blue, overbright: unlit output goes through the tone mapper, which
/// greys anything at or under 1.0 against the terrain, so the glow is >1.
fn hdr_blue(materials: &mut Assets<StandardMaterial>, a: f32) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: Color::LinearRgba(LinearRgba::new(0.15, 1.2, 3.0, a)),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    })
}

/// Rebuild the order markers when the orders change.
pub fn sync_orders(
    mut commands: Commands,
    sim: Res<Sim>,
    mut assets: ResMut<UnitAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    existing: Query<Entity, With<OrderMarker>>,
    kiosk: Res<crate::kiosk::Kiosk>,
) {
    // The opening demo does not draw the lines the crews on foot are cutting
    // (user, 2026-10-17).
    let demo = kiosk.phase == crate::kiosk::Phase::Intro;
    // `Suppression::generation` bumps every step, so gate on the orders
    // themselves: a marker set that is rebuilt 30 times a second would be a
    // mesh allocation per frame for no visible change.
    let fingerprint = orders_fingerprint(&sim) ^ demo as u64;
    if assets.orders_gen == fingerprint {
        return;
    }
    assets.orders_gen = fingerprint;
    for e in &existing {
        commands.entity(e).despawn();
    }

    for u in &sim.crews.units {
        if !u.assignable() {
            continue;
        }
        match u.task {
            // where a team is sent is shown by its flag (`kiosk::pins`)
            Task::Hold | Task::Return | Task::Attack { .. } => {}
            Task::Drop { at } => {
                commands.spawn((
                    (
                        Mesh3d(meshes.add(ring_mesh(&sim.scn, at, ORDER_RING_M))),
                        MeshMaterial3d(assets.order_ring.clone()),
                    ),
                    OrderMarker,
                ));
                commands.spawn((
                    (
                        Mesh3d(meshes.add(wall_mesh(&sim.scn, at, ORDER_RING_M))),
                        MeshMaterial3d(assets.order_ring.clone()),
                        tube_transform(at),
                    ),
                    OrderMarker,
                    TubeSpin,
                ));
            }
            Task::Line { .. } if demo => {}
            Task::Line { from, to } => {
                let total = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();
                let done = (u.line_done_m / total.max(1e-3)).clamp(0.0, 1.0);
                let head = lerp(from, to, done);
                // Two ribbons: what has been cut, and what has not. The join
                // moves as the crew works, which is the only progress readout
                // that does not need a number.
                if done > 0.001 {
                    commands.spawn((
                        (
                            Mesh3d(meshes.add(ribbon(&sim.scn, from, head, 8.0))),
                            MeshMaterial3d(assets.line_cut.clone()),
                        ),
                        OrderMarker,
                    ));
                }
                if done < 0.999 {
                    commands.spawn((
                        (
                            Mesh3d(meshes.add(dashed_ribbon(&sim.scn, head, to, 4.0))),
                            MeshMaterial3d(assets.line_todo.clone()),
                        ),
                        OrderMarker,
                    ));
                }
            }
        }
    }
}

/// A cheap hash of every order, so the markers rebuild when an order changes
/// and not when a unit merely moved. Line progress is quantised to 10 m: the
/// ribbon does not need to be redrawn for every centimetre of a 120 m/h crew.
fn orders_fingerprint(sim: &Sim) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x100_0000_01b3);
    };
    for u in &sim.crews.units {
        mix(u.id as u64);
        mix(u.state as u64);
        match u.task {
            Task::Hold => mix(1),
            Task::Return => mix(2),
            Task::Attack { at } => {
                mix(3);
                mix(at.x as u64);
                mix(at.y as u64);
            }
            Task::Drop { at } => {
                mix(4);
                mix(at.x as u64);
                mix(at.y as u64);
            }
            Task::Line { from, to } => {
                mix(5);
                mix(from.x as u64);
                mix(from.y as u64);
                mix(to.x as u64);
                mix(to.y as u64);
                mix((u.line_done_m / 10.0) as u64);
            }
        }
    }
    h
}

/// Keep the list of cells the engines have just wetted. They are no longer
/// painted blue (user, 2026-10-16): [`animate_spray`] aims a jet of water at them.
pub fn update_work_overlay(sim: Res<Sim>, mut assets: ResMut<UnitAssets>) {
    // Simulated time, so a jet stays up while the world is paused.
    let now = sim.time_s() as f32;
    // Scan for newly wet cells on the sim generation (the moisture changes
    // slowly); each one is shown for WET_SHOW_S of real time, then dropped.
    let gen = sim.generation;
    let scan = assets.work_gen != gen;
    if scan {
        let moist = sim.fire.added_moisture();
        // Both lists are ascending in cell index, so "seen last time" is a
        // merge pointer instead of a hash lookup.
        let UnitAssets { wet_seen, wet_next, wet_shown, .. } = &mut *assets;
        wet_next.clear();
        let mut k = 0;
        for (i, (flag, m)) in sim.fire.cleared().iter().zip(moist).enumerate() {
            if !*flag && *m >= WET_SHOW_PTS {
                wet_next.push(i);
                while k < wet_seen.len() && wet_seen[k] < i {
                    k += 1;
                }
                if wet_seen.get(k) != Some(&i) {
                    wet_shown.push((i, now));
                }
            }
        }
        std::mem::swap(wet_seen, wet_next);
    }
    assets.work_gen = gen;
    assets.wet_shown.retain(|(_, t0)| now - *t0 < WET_SHOW_S);
}

/// How long the water an engine laid stays drawn, in simulated seconds.
const WET_SHOW_S: f32 = 600.0;
/// Added moisture at which a cell is worth showing as wet, percentage points.
/// A tenth of the core's moisture of extinction: enough to see where a drop
/// landed, high enough that the 1%/minute decay makes it fade rather than
/// linger as a stain for the rest of the incident.
const WET_SHOW_PTS: f32 = 1.5;

/// A flat ribbon of `width_m` from `a` to `b`, draped and resampled.
///
/// Resampled at the render posting for the reason the roads are: a straight run
/// over a ridge drawn as one long quad drapes as a chord *through* the hill and
/// disappears into it. Vertices only exist where you put them.
pub(crate) fn ribbon(scn: &Scenario, a: Pos, b: Pos, width_m: f32) -> Mesh {
    ribbon_pieces(scn, &[(a, b)], width_m)
}

/// A ribbon of dashes: DASH_M drawn, GAP_M left out.
pub(crate) fn dashed_ribbon(scn: &Scenario, a: Pos, b: Pos, width_m: f32) -> Mesh {
    const DASH_M: f32 = 12.0;
    const GAP_M: f32 = 8.0;
    let len = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
    let mut pieces = Vec::new();
    let mut s = 0.0;
    while s < len {
        let e = (s + DASH_M).min(len);
        pieces.push((lerp(a, b, s / len), lerp(a, b, e / len)));
        s = e + GAP_M;
    }
    if pieces.is_empty() {
        pieces.push((a, b));
    }
    ribbon_pieces(scn, &pieces, width_m)
}

fn ribbon_pieces(scn: &Scenario, pieces: &[(Pos, Pos)], width_m: f32) -> Mesh {
    const RESAMPLE_M: f32 = 5.0;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for &(a, b) in pieces {
        let base = positions.len() as u32;
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len = (dx * dx + dy * dy).sqrt();
        let steps = (len / RESAMPLE_M).ceil().max(1.0) as usize;
        let (nx, ny) = if len > 1e-3 {
            (-dy / len * width_m * 0.5, dx / len * width_m * 0.5)
        } else {
            (width_m * 0.5, 0.0)
        };
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let c = lerp(a, b, t);
            for s in [-1.0f32, 1.0] {
                let p = Pos {
                    x: c.x + nx * s,
                    y: c.y + ny * s,
                };
                positions.push([p.x, scn.terrain.height_at(p) + RING_LIFT_M, -p.y]);
                normals.push([0.0, 1.0, 0.0]);
                uvs.push([(s > 0.0) as u32 as f32, t]);
            }
        }
        for i in 0..steps as u32 {
            let k = base + i * 2;
            indices.extend_from_slice(&[k, k + 2, k + 1, k + 1, k + 2, k + 3]);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

fn lerp(a: Pos, b: Pos, t: f32) -> Pos {
    let t = t.clamp(0.0, 1.0);
    Pos {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
    }
}

/// A ribbon along `points` (position, width): two quads crossed, one lying
/// flat and one upright, so the stream reads as a body of water from any side.
fn arc_mesh(points: &[(Vec3, f32)]) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let n = points.len();
    for (i, &(p, w)) in points.iter().enumerate() {
        let a = points[i.saturating_sub(1)].0;
        let b = points[(i + 1).min(n - 1)].0;
        let dir = (b - a).normalize_or_zero();
        let side = dir.cross(Vec3::Y).normalize_or_zero();
        let up = side.cross(dir).normalize_or_zero();
        let v = i as f32 / (n - 1) as f32;
        for off in [side, -side, up, -up] {
            positions.push((p + off * w * 0.5).to_array());
            normals.push(Vec3::Y.to_array());
            uvs.push([0.0, v]);
        }
    }
    for i in 0..n as u32 - 1 {
        let (a, b) = (i * 4, (i + 1) * 4);
        // flat quad: side/-side; upright quad: up/-up
        indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        indices.extend([a + 2, b + 2, a + 3, a + 3, b + 2, b + 3]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// The engines' water: an arc from each working engine to the cells it has
/// just wetted, hottest first (the same order the model wets them in), with a
/// slow sweep from one cell to the next.
pub fn animate_spray(
    sim: Res<Sim>,
    time: Res<Time>,
    assets: Res<UnitAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut arcs: Query<(&Jet, &Mesh3d, &mut Visibility)>,
) {
    let now = time.elapsed_secs();
    let w = sim.scn.world;
    let reach = rocca::game::ENGINE_RANGE_M * 1.5;
    // The targets of each engine, computed once per frame and not per jet.
    let mut jets: std::collections::HashMap<usize, Vec<Pos>> = Default::default();
    for u in sim.crews.units.iter().filter(|u| u.kind == UnitKind::Engine) {
        if u.state != UnitState::Working || u.aboard || !matches!(u.task, Task::Attack { .. }) {
            continue;
        }
        let mut near: Vec<(f32, Pos)> = assets
            .wet_shown
            .iter()
            .map(|(i, _)| w.centre_of(Cell { row: i / w.fire_cols, col: i % w.fire_cols }))
            .filter(|p| (p.x - u.pos.x).hypot(p.y - u.pos.y) <= reach)
            .map(|p| (sim.fire.threat().at(p), p))
            .collect();
        near.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        jets.insert(u.id, near.into_iter().take(JETS).map(|(_, p)| p).collect());
    }
    // Where the jet of an engine points now: rests on a cell, then turns to the next.
    let aim = |unit: usize| {
        let list = jets.get(&unit)?;
        if list.is_empty() {
            return None;
        }
        let period = DWELL_S + TURN_S;
        let k = now / period + unit as f32 * 0.31;
        let (i, r) = (k.floor() as usize, (k.fract() * period - DWELL_S).max(0.0) / TURN_S);
        let (a, b) = (list[i % list.len()], list[(i + 1) % list.len()]);
        let e = r * r * (3.0 - 2.0 * r);
        let to = Pos { x: a.x + (b.x - a.x) * e, y: a.y + (b.y - a.y) * e };
        Some((to, sim.crews.units.get(unit)?))
    };
    for (j, handle, mut vis) in &mut arcs {
        let Some((to, u)) = aim(j.unit) else {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        };
        let from_y = sim.scn.terrain.height_at(u.pos) + NOZZLE_M;
        let to_y = sim.scn.terrain.height_at(to) + RING_LIFT_M;
        let len = (to.x - u.pos.x).hypot(to.y - u.pos.y);
        let crown = ARC_BASE_M + ARC_PER_M * len;
        let points: Vec<(Vec3, f32)> = (0..=ARC_SEGMENTS)
            .map(|i| {
                let t = i as f32 / ARC_SEGMENTS as f32;
                let p = Pos {
                    x: u.pos.x + (to.x - u.pos.x) * t,
                    y: u.pos.y + (to.y - u.pos.y) * t,
                };
                let y = from_y + (to_y - from_y) * t + crown * 4.0 * t * (1.0 - t);
                // Ripples run down the stream so it reads as flowing water.
                let ripple = 1.0 + 0.18 * (t * 22.0 - now * 9.0).sin();
                let width = (STREAM_NOZZLE_M + (STREAM_END_M - STREAM_NOZZLE_M) * t) * ripple;
                (frame::to_bevy(p, y), width)
            })
            .collect();
        if let Some(mut m) = meshes.get_mut(handle) {
            *m = arc_mesh(&points);
        }
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
    }
}
