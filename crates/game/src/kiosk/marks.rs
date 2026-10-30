//! Signs on the map for the orders (docs/01-GIOCO.md §6): the hexes a team
//! can be sent to (in pause, or with a team selected), the hex chosen and the
//! road to it, the roads the teams are driving, the evacuation's route
//! (proposed, dashed; in course, solid).
//!
//! Everything sits above the canopy (`rings::RING_LIFT_M`), in flat bright
//! colours drawn over the trees (`flat::FlatMaterial`), and is rebuilt from the game's state behind a key, so a restart needs nothing
//! but the key cleared.

use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::asset::RenderAssetUsages;
use rocca::tutorial::Expect;
use scenario::Pos;

use super::{Kiosk, Phase, Selection};
use crate::flat::FlatMaterial;
use crate::reach_zone::{band_mesh, route_mesh, sparse_mesh, ReachMaterial, RouteMaterial};
use crate::rings::{path_mesh, path_mesh_at, RING_LIFT_M};
use crate::sim::Sim;
use crate::tape::TapeMaterial;

/// Widths on the map, metres.
/// Width of the band along the roads where a team can be placed (a buffer of
/// half of it on each side; the internal rule stays the hex grid).
const BUFFER_M: f32 = 110.0;
/// Half the width of the district's zone: the buffer around each home, so
/// homes closer than twice this are one patch (user, 2026-10-21).
const DISTRICT_BUFFER_M: f32 = 80.0;
const HEX_PICK_M: f32 = 16.0;
/// Radius of the circle that marks a place, and the larger one for the tutorial's.
const DOT_R_M: f32 = 28.0;
const WANT_R_M: f32 = 45.0;
/// The covered zone's boundary line.
const ZONE_LINE_M: f32 = 6.0;
/// The zone line bobs up and down by this much, metres, once per period.
const ZONE_BOB_M: f32 = 6.0;
const ZONE_BOB_S: f32 = 1.6;
/// The drivable roads shown while a move is chosen.
const ROAD_M: f32 = 9.0;
/// Extra pull toward the camera for the hex lines: enough that the hills'
/// own bumps do not cut them into dashes.
const HEX_PULL_M: f32 = 60.0;
const ROUTE_M: f32 = 12.0;
/// As wide as the road drawn under it (`roads::DRIVABLE_HALF_W`), a little above it.
const CLOSED_M: f32 = 9.6;
const CLOSED_LIFT_M: f32 = 0.8;
/// Closed roads are recomputed this often, in game steps.
const CLOSED_EVERY: u64 = 10;
/// How much wider the dark under-stroke of a route is than the route.
const UNDER: f32 = 1.8;

#[derive(Component)]
pub struct HexMark;
/// The ground a district covers: a buffer around its homes (pixel shader).
#[derive(Component)]
pub struct DistrictZoneMark;
/// The drivable roads, shown while a move is chosen.
#[derive(Component)]
pub struct RoadNetMark;
#[derive(Component)]
pub struct OrderMark;
/// A road stretch the fire has closed, junction to junction: hazard tape laid on it (`tape.rs`).
#[derive(Component)]
pub struct ClosedMark;
/// The assigned team hexes, highlighted in steady yellow.
#[derive(Component)]
pub struct TeamMark;

/// The zone line and its under-stroke, which bob.
#[derive(Component)]
pub struct ZoneBob;

#[derive(Resource)]
pub struct MarkAssets {
    /// The band where a team can be placed (pixel shader).
    reach: Handle<ReachMaterial>,
    /// The ground of the chosen district.
    district_zone: Handle<ReachMaterial>,
    district_key: Option<usize>,
    /// The route a team would drive (pixel shader), per team colour.
    team_line: Vec<Handle<RouteMaterial>>,
    /// The drivable roads, while a move is chosen.
    road: Handle<RouteMaterial>,
    /// Dark, under the hexes and the routes.
    under: Handle<FlatMaterial>,
    pick: Handle<FlatMaterial>,
    want: Handle<FlatMaterial>,
    /// The hexes a team placed on a hex covers.
    area: Handle<FlatMaterial>,
    /// The evacuation's route, in course (flowing light) and proposed (plain).
    evac_line: Handle<RouteMaterial>,
    evac_plan: Handle<RouteMaterial>,
    closed: Handle<FlatMaterial>,
    tape: Handle<TapeMaterial>,
    tape_open: Handle<TapeMaterial>,
    closed_key: u64,
    /// Per team: zone line, the road it drives and its hex outline.
    team_zone: Vec<Handle<FlatMaterial>>,
    team_hex: Vec<Handle<FlatMaterial>>,
    team_key: u64,
    /// What the hex grid and the orders were built for.
    hex_key: Option<String>,
    /// The team, time and tutorial hex the usable hexes were built for.
    targets_key: Option<(usize, i64, Option<rocca::Axial>)>,
    order_key: u64,
}

/// One colour per team (sRGB), the same on its badge, cone, zone and road.
/// Red is kept for danger, green for the evacuation, amber for the tutorial.
pub const TEAM_COLOURS: [[f32; 3]; 5] = [
    [0.25, 0.66, 1.0],  // A blue
    [1.0, 0.54, 0.16],  // B orange
    [0.86, 0.38, 1.0],  // C magenta
    [1.0, 0.88, 0.25],  // D yellow
    [0.90, 0.92, 0.95], // E white
];

/// The colour of team `t`.
pub fn team_rgb(t: usize) -> [f32; 3] {
    TEAM_COLOURS[t % TEAM_COLOURS.len()]
}

/// The colours of the signs (sRGB). Each means one thing: the evacuation's
/// is green (the escape-route sign), the refused hex red, the hex the
/// tutorial wants amber; a team's own signs wear its colour.
pub const ROUTE: [f32; 3] = [0.0, 0.85, 1.0];
pub const EVAC: [f32; 3] = [0.25, 1.0, 0.2];
const CLOSED: [f32; 3] = [1.0, 0.12, 0.08];
const WANT: [f32; 3] = [1.0, 0.72, 0.0];

pub fn setup(mut commands: Commands, mut lines: ResMut<Assets<RouteMaterial>>, mut reaches: ResMut<Assets<ReachMaterial>>, mut materials: ResMut<Assets<FlatMaterial>>, mut tapes: ResMut<Assets<TapeMaterial>>) {
    commands.insert_resource(MarkAssets {
        team_line: TEAM_COLOURS.iter().map(|&c| lines.add(RouteMaterial::dashed(c, ROUTE_M * 1.0, ROUTE_M * 1.8, 0.95, 60.0))).collect(),
        reach: reaches.add(ReachMaterial::new([1.0, 1.0, 1.0], 0.22, BUFFER_M / 2.0)),
        district_zone: reaches.add(ReachMaterial::new([0.43, 0.72, 1.0], 0.30, DISTRICT_BUFFER_M)),
        district_key: None,
        road: lines.add(RouteMaterial::plain([1.0, 0.93, 0.62], ROAD_M * 0.5, ROAD_M * 0.9, 0.75)),
        under: materials.add(FlatMaterial::new([0.02, 0.03, 0.05]).pulled(HEX_PULL_M - 3.0)),
        pick: materials.add(FlatMaterial::new([1.0, 1.0, 1.0]).pulled(HEX_PULL_M + 4.0)),
        want: materials.add(FlatMaterial::new(WANT).pulled(HEX_PULL_M + 4.0)),
        area: materials.add(FlatMaterial::new(ROUTE).pulled(HEX_PULL_M + 2.0)),
        evac_line: lines.add(RouteMaterial::new(EVAC, ROUTE_M * 1.0, ROUTE_M * 1.8)),
        evac_plan: lines.add(RouteMaterial::dashed(EVAC, ROUTE_M * 1.0, ROUTE_M * 1.8, 0.95, 60.0)),
        closed: materials.add(FlatMaterial::new(CLOSED).pulled(1.0)),
        team_zone: TEAM_COLOURS.iter().map(|&c| materials.add(FlatMaterial::new(c).pulled(HEX_PULL_M + 3.0))).collect(),
        team_hex: TEAM_COLOURS.iter().map(|&c| materials.add(FlatMaterial::new(c).pulled(HEX_PULL_M + 5.0))).collect(),
        tape: tapes.add(TapeMaterial::closed()),
        tape_open: tapes.add(TapeMaterial::open_part()),
        closed_key: u64::MAX,
        team_key: u64::MAX,
        hex_key: None,
        targets_key: None,
        order_key: u64::MAX,
    });
}

/// Several meshes as one draw.
fn merge(parts: Vec<Mesh>) -> Mesh {
    let (mut pos, mut idx): (Vec<[f32; 3]>, Vec<u32>) = (vec![], vec![]);
    let mut uv: Vec<[f32; 2]> = vec![];
    for m in parts {
        let base = pos.len() as u32;
        if let Some(VertexAttributeValues::Float32x2(u)) = m.attribute(Mesh::ATTRIBUTE_UV_0) {
            uv.extend_from_slice(u);
        }
        if let Some(VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) {
            pos.extend_from_slice(p);
        }
        if let Some(Indices::U32(i)) = m.indices() {
            idx.extend(i.iter().map(|v| v + base));
        }
    }
    let n = pos.len();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// The hex a selected team is placed on, or being placed on.
fn area_hex(kiosk: &Kiosk, sim: &Sim) -> Option<rocca::Axial> {
    let Selection::Team(t) = kiosk.selection else { return None };
    kiosk.move_hex.or(sim.teams.get(t)?.hex)
}

/// A small circle: where a team is, goes or is asked to go.
fn dot(c: Pos, r: f32) -> Vec<Pos> {
    (0..=32).map(|i| {
        let a = i as f32 / 32.0 * std::f32::consts::TAU;
        Pos { x: c.x + r * a.cos(), y: c.y + r * a.sin() }
    }).collect()
}

/// Where the team would stand for hex `at`: its road point, else the centre.
fn spot(sim: &Sim, at: rocca::Axial) -> Pos {
    sim.hexes.get(at).map_or(at.centre(), |h| h.target)
}

/// A team's area of intervention, drawn as a circle around its hex rather
/// than as the union of hexes (user, 2026-10-17).
fn area_circle(at: rocca::Axial) -> Vec<Pos> {
    let c = at.centre();
    let r = rocca::TEAM_AREA_M;
    (0..=96).map(|i| {
        let a = i as f32 / 96.0 * std::f32::consts::TAU;
        Pos { x: c.x + r * a.cos(), y: c.y + r * a.sin() }
    }).collect()
}

fn spawn<M: Component>(commands: &mut Commands, meshes: &mut Assets<Mesh>, mesh: Mesh, mat: &Handle<FlatMaterial>, tag: M) -> Entity {
    commands.spawn((
        (Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat.clone())),
        tag,
        bevy::light::NotShadowCaster,
        bevy::light::NotShadowReceiver,
        bevy::camera::visibility::NoFrustumCulling,
    )).id()
}

/// While a team's move is being chosen (in pause, the team taken or dragged)
/// the drivable roads show over the canopy, and the hexes the team can
/// a faint band along the roads the team can drive to (a buffer; user,
/// 2026-10-17: no hex outlines, the grid stays the internal rule of the
/// order). The roads are built once, then shown or hidden; the band is rebuilt
/// for each team and pause.
pub fn update_hexes(
    mut commands: Commands,
    sim: Res<Sim>,
    kiosk: Res<Kiosk>,
    mut assets: ResMut<MarkAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut roads: Query<(Entity, &mut Visibility), (With<RoadNetMark>, Without<HexMark>)>,
    hexes: Query<Entity, With<HexMark>>,
) {
    if assets.hex_key.is_none() {
        for (e, _) in &roads {
            commands.entity(e).despawn();
        }
        assets.hex_key = Some(String::new());
        assets.targets_key = None;
        let net = &sim.net;
        let main = sim.hexes.hexes.first().map(|h| net.component(h.node, true));
        let mut segments = vec![];
        for n in 0..net.len() {
            let id = n as abm::network::NodeId;
            if Some(net.component(id, true)) != main {
                continue;
            }
            for e in net.neighbours(id) {
                if e.drivable && (e.to as usize) > n {
                    segments.push(([net.pos(id), net.pos(e.to)], 0.0));
                }
            }
        }
        if let Some(mesh) = sparse_mesh(&sim.scn, &segments, ROAD_M * 0.9 + 6.0, 5.0) {
            commands.spawn((
                (Mesh3d(meshes.add(mesh)), MeshMaterial3d(assets.road.clone()), Visibility::Hidden),
                RoadNetMark,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
                bevy::camera::visibility::NoFrustumCulling,
            ));
        }
        return;
    }
    let playing = matches!(kiosk.phase, Phase::Tutorial | Phase::Gioco);
    let team = kiosk.dragging.map(|(t, _)| t).or(match kiosk.selection {
        Selection::Team(t) => Some(t),
        _ => None,
    });
    let choosing = playing && sim.paused() && team.is_some();
    let vis = if choosing { Visibility::Inherited } else { Visibility::Hidden };
    for (_, mut v) in &mut roads {
        if *v != vis {
            *v = vis;
        }
    }
    let want = match kiosk.expect() {
        Some(Expect::Move(_, at)) => Some(at),
        _ => None,
    };
    let key = choosing.then(|| (team.unwrap_or(0), sim.time_s(), want));
    if key == assets.targets_key {
        return;
    }
    assets.targets_key = key;
    for e in &hexes {
        commands.entity(e).despawn();
    }
    let Some((t, _, want)) = key else { return };
    // the tutorial's step asks for one place, shown by the order marks
    if want.is_some() {
        return;
    }
    // a band along the roads the team can drive to now
    let Some(team) = sim.teams.get(t) else { return };
    let Some(from) = sim.net.nearest(sim.crews.units[team.unit].pos, true) else { return };
    let seen = abm::network::reachable_from(&sim.net, from, sim.fire.threat(), true);
    let mut segments = vec![];
    for n in 0..sim.net.len() {
        let id = n as abm::network::NodeId;
        if !seen[n] {
            continue;
        }
        for e in sim.net.neighbours(id) {
            if e.drivable && (e.to as usize) > n && seen[e.to as usize] {
                segments.push([sim.net.pos(id), sim.net.pos(e.to)]);
            }
        }
    }
    if let Some(mesh) = band_mesh(&sim.scn, &segments, BUFFER_M) {
        commands.spawn((
            (Mesh3d(meshes.add(mesh)), MeshMaterial3d(assets.reach.clone())),
            HexMark,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
}

/// The ground of the district chosen or under the pointer in the sidebar and
/// on the map: the union of a buffer around each of its homes, so a district
/// of scattered hamlets shows that the click takes all of them.
pub fn update_district_zone(
    mut commands: Commands,
    sim: Res<Sim>,
    kiosk: Res<Kiosk>,
    mut assets: ResMut<MarkAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    old: Query<Entity, With<DistrictZoneMark>>,
) {
    let playing = matches!(kiosk.phase, Phase::Tutorial | Phase::Gioco);
    let shown = match kiosk.selection {
        Selection::District(d) => Some(d),
        _ => kiosk.evac_hover.as_ref().map(|h| h.0),
    };
    let key = shown.filter(|&d| playing && d < sim.districts.len());
    if key == assets.district_key {
        return;
    }
    assets.district_key = key;
    for e in &old {
        commands.entity(e).despawn();
    }
    let Some(d) = key else { return };
    let homes: Vec<[Pos; 2]> = sim.districts[d].households.iter().map(|&i| {
        let p = Pos::from(sim.scn.population.households[i].pos);
        [p, p]
    }).collect();
    if let Some(mesh) = band_mesh(&sim.scn, &homes, DISTRICT_BUFFER_M + 40.0) {
        commands.spawn((
            (Mesh3d(meshes.add(mesh)), MeshMaterial3d(assets.district_zone.clone())),
            DistrictZoneMark,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
}

/// The chosen hex and its road, the tutorial's hex, the teams' roads, the
/// evacuation's route.
pub fn update_orders(
    mut commands: Commands,
    sim: Res<Sim>,
    kiosk: Res<Kiosk>,
    mut assets: ResMut<MarkAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    old: Query<Entity, With<OrderMark>>,
) {
    let playing = matches!(kiosk.phase, Phase::Tutorial | Phase::Gioco);
    let mut key = (sim.time_s() / (2 * rocca::STEP_S)) as u64 ^ (playing as u64) << 63;
    let mut mix = |v: u64| key = key.wrapping_mul(0x100_0000_01b3) ^ v;
    if let Some(at) = area_hex(&kiosk, &sim) {
        mix((at.q as i64 as u64) << 8 ^ at.r as i64 as u64 ^ 0xa4ea);
    }
    if let Some(at) = kiosk.move_hex {
        mix((at.q as i64 as u64) << 20 ^ at.r as i64 as u64);
        if let Selection::Team(t) = kiosk.selection {
            mix(t as u64 + 77);
        }
    }
    match &kiosk.move_plan {
        Some(Ok(p)) => mix(p.path.len() as u64),
        Some(Err(_)) => mix(0xbad),
        None => {}
    }
    if let Some(Ok(p)) = &kiosk.evac_plan {
        mix(p.district as u64 + 1000 + p.path.points.len() as u64);
    }
    if let Some((_, _, Ok(p))) = &kiosk.evac_hover {
        mix(p.district as u64 + 5000 + p.path.points.len() as u64);
    }
    if let Some(Expect::Move(t, at)) = kiosk.expect() {
        mix(t as u64 * 7 + at.q as u64 * 13 + at.r as u64);
    }
    if key == assets.order_key {
        return;
    }
    assets.order_key = key;
    for e in &old {
        commands.entity(e).despawn();
    }
    if !playing {
        return;
    }
    // the hex the tutorial asks for
    if let Some(Expect::Move(_, at)) = kiosk.expect() {
        spawn(&mut commands, &mut meshes, path_mesh(&sim.scn, &dot(spot(&sim, at), WANT_R_M), HEX_PICK_M * 1.4, None), &assets.want, OrderMark);
    }
    let chosen = match kiosk.selection {
        Selection::Team(t) => Some(t),
        _ => None,
    };
    // the area the team works around its hex, lit up in its colour
    if let Some(at) = area_hex(&kiosk, &sim).filter(|_| !matches!(kiosk.move_plan, Some(Err(_)))) {
        let rings = vec![path_mesh(&sim.scn, &area_circle(at), HEX_PICK_M * 0.6, None)];
        let mat = chosen.map_or(&assets.area, |t| &assets.team_hex[t % TEAM_COLOURS.len()]);
        spawn(&mut commands, &mut meshes, merge(rings), mat, OrderMark);
    }
    // the move being chosen: start hex and destination hex
    if let (Some(_), Selection::Team(t)) = (kiosk.move_hex, kiosk.selection) {
        let u = &sim.crews.units[sim.teams[t].unit];
        let from = sim.teams[t].hex.unwrap_or_else(|| rocca::Axial::of(u.pos));
        spawn(&mut commands, &mut meshes, path_mesh(&sim.scn, &dot(spot(&sim, from), DOT_R_M), HEX_PICK_M, None), &assets.pick, OrderMark);
    }
    if let Some(at) = kiosk.move_hex {
        // a hex the team cannot go to is red, not white
        let refused = matches!(kiosk.move_plan, Some(Err(_)));
        let mat = if refused { &assets.closed } else { chosen.map_or(&assets.pick, |t| &assets.team_hex[t % TEAM_COLOURS.len()]) };
        spawn(&mut commands, &mut meshes, path_mesh(&sim.scn, &dot(spot(&sim, at), DOT_R_M), HEX_PICK_M * 1.4, None), mat, OrderMark);
    }
    // the road the chosen team would drive: a shader line in its colour
    if let (Some(Ok(p)), Some(t)) = (&kiosk.move_plan, chosen) {
        if let Some(mesh) = route_mesh(&sim.scn, &p.path, ROUTE_M * 1.8 + 6.0) {
            commands.spawn((
                (Mesh3d(meshes.add(mesh)), MeshMaterial3d(assets.team_line[t % TEAM_COLOURS.len()].clone())),
                OrderMark,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
                bevy::camera::visibility::NoFrustumCulling,
            ));
        }
    }
    // the evacuation: under the pointer or chosen (plain), and in course (flowing light)
    let hover = kiosk.evac_hover.as_ref().and_then(|(_, _, p)| p.as_ref().ok());
    let proposed = kiosk.evac_plan.as_ref().and_then(|p| p.as_ref().ok()).into_iter().chain(hover).map(|p| (&p.path.points, true));
    for (pts, plan) in proposed.chain(sim.evac.active().map(|e| (&e.path.points, false))) {
        if let Some(mesh) = route_mesh(&sim.scn, pts, ROUTE_M * 1.8 + 6.0) {
            let material = if plan { assets.evac_plan.clone() } else { assets.evac_line.clone() };
            commands.spawn((
                (Mesh3d(meshes.add(mesh)), MeshMaterial3d(material)),
                OrderMark,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
                bevy::camera::visibility::NoFrustumCulling,
            ));
        }
    }
}

/// Each team's zone, the hexes it works in, outlined in its colour.
/// Rebuilt when the set changes.
pub fn update_teams(
    mut commands: Commands,
    sim: Res<Sim>,
    kiosk: Res<Kiosk>,
    mut assets: ResMut<MarkAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    old: Query<Entity, With<TeamMark>>,
) {
    let playing = matches!(kiosk.phase, Phase::Tutorial | Phase::Gioco);
    let zones: Vec<(usize, Vec<rocca::Axial>)> = if playing {
        sim.teams.iter().enumerate().filter_map(|(t, team)| {
            Some((t, vec![team.hex?]))
        }).collect()
    } else {
        vec![]
    };
    let mut key = playing as u64;
    for (t, z) in &zones {
        key = key.wrapping_mul(0x100_0000_01b3) ^ (*t as u64 + 1) << 56;
        for h in z {
            key = key.wrapping_mul(0x100_0000_01b3) ^ ((1u64 << 50) ^ (h.q as i64 as u64) << 24 ^ h.r as i64 as u64);
        }
    }
    if key == assets.team_key {
        return;
    }
    assets.team_key = key;
    for e in &old {
        commands.entity(e).despawn();
    }
    for (t, z) in &zones {
        let loops = vec![area_circle(z[0])];
        if loops.is_empty() {
            continue;
        }
        // a dark under-stroke lifts the colour off the ground
        let under = loops.iter().map(|l| path_mesh_at(&sim.scn, l, ZONE_LINE_M * UNDER, None, RING_LIFT_M - 2.0)).collect();
        let e = spawn(&mut commands, &mut meshes, merge(under), &assets.under, TeamMark);
        commands.entity(e).insert(ZoneBob);
        let parts = loops.iter().map(|l| path_mesh(&sim.scn, l, ZONE_LINE_M, None)).collect();
        let e = spawn(&mut commands, &mut meshes, merge(parts), &assets.team_zone[t % TEAM_COLOURS.len()], TeamMark);
        commands.entity(e).insert(ZoneBob);
    }
}

/// The zone line floats up and down a little, on real time: the world is
/// paused when it matters most.
pub fn bob_zone(time: Res<Time<Real>>, mut marks: Query<&mut Transform, With<ZoneBob>>) {
    let y = ZONE_BOB_M * 0.5 * (1.0 - (time.elapsed_secs() * std::f32::consts::TAU / ZONE_BOB_S).cos());
    for mut t in &mut marks {
        t.translation.y = y;
    }
}

/// The roads the fire has closed: hazard tape, animated by its shader.
pub fn update_closed(
    mut commands: Commands,
    sim: Res<Sim>,
    kiosk: Res<Kiosk>,
    mut assets: ResMut<MarkAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    old: Query<Entity, With<ClosedMark>>,
) {
    let playing = matches!(kiosk.phase, Phase::Tutorial | Phase::Gioco);
    let key = if playing { sim.generation / CLOSED_EVERY } else { u64::MAX - 1 };
    if key == assets.closed_key {
        return;
    }
    assets.closed_key = key;
    for e in &old {
        commands.entity(e).despawn();
    }
    if !playing {
        return;
    }
    // each stretch in runs: the closed pieces in full tape, the rest dashed
    let (mut shut, mut open): (Vec<Mesh>, Vec<Mesh>) = (vec![], vec![]);
    for st in sim.closed_stretches() {
        let mut from = 0;
        for i in 1..=st.blocked.len() {
            if i == st.blocked.len() || st.blocked[i] != st.blocked[from] {
                let mesh = path_mesh_at(&sim.scn, &st.line[from..=i], CLOSED_M, None, CLOSED_LIFT_M);
                if st.blocked[from] { shut.push(mesh) } else { open.push(mesh) }
                from = i;
            }
        }
    }
    for (parts, mat) in [(shut, &assets.tape), (open, &assets.tape_open)] {
        if !parts.is_empty() {
            commands.spawn((
                (Mesh3d(meshes.add(merge(parts))), MeshMaterial3d(mat.clone())),
                ClosedMark,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
                bevy::camera::visibility::NoFrustumCulling,
            ));
        }
    }
}

pub fn reset(mut restarted: MessageReader<crate::sim::SimRestarted>, mut assets: ResMut<MarkAssets>) {
    if !restarted.is_empty() {
        restarted.clear();
        assets.hex_key = None;
        assets.order_key = u64::MAX;
        assets.closed_key = u64::MAX;
        assets.team_key = u64::MAX;
    }
}
