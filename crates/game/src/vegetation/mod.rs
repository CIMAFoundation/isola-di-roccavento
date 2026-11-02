//! The vegetation: where it grows (`scatter`), how it is drawn (`render`),
//! and how the fire reaches it (here).
//!
//! Rewritten from scratch on 2026-11-01 to run the same on the desktop and on
//! an iPhone (WebGL2 in iOS WebKit), with the practices of vegetation
//! rendering:
//!
//! - **Instancing.** Each variety has one mesh per level of detail
//!   (`meshes`), every plant is an instance of it: a static record of 32
//!   bytes (where, yaw, size, tint, its own random numbers), uploaded once.
//! - **Nothing is rebuilt.** Meshes, instance buffers, the atlas and the bind
//!   group are made once per scene; per frame the GPU receives 48 bytes of
//!   uniforms, and 8 bytes per plant of a group whose fire state changed.
//!   No Bevy `Mesh`, `Material` or `Image` asset is created or modified.
//! - **Tiles, culling, LOD per view.** On the desktop the island is cut in
//!   1280 m tiles: each view (camera and shadow cascades) draws only the
//!   tiles in its frustum, the authored models near the camera, proxies
//!   beyond, a cheaper crown far away (`render`). The browser draws the
//!   island as one tile of proxies: a few dozen draws.
//! - **Shading made for foliage.** Wrapped diffuse, light through the leaves
//!   against the sun, sky and ground ambient, occlusion toward the heart of
//!   the crown, the sun's shadows; no full PBR (`shaders/plants.wesl`).
//!   Leaf cards are cut out of the atlas (alpha to coverage with MSAA) only
//!   where they are drawn, on the desktop; the proxies are opaque, and a
//!   `discard` never runs in their pass (tile-based GPUs keep their
//!   hidden-surface removal).
//!
//! **Fire.** For each plant the CPU decides one number, the instant it
//! catches ([`plant_fire`]): the interpolated arrival of the fire at its
//! foot (as `field::FireField::arrival`, so a stand catches as a wave rather
//! than as a 20 m block) plus its own lag, and whether a crew cut it. Once a
//! plant has caught it stays caught. The shader works out the rest from the
//! simulated clock: scorch, the glowing front crawling through the crown,
//! embers, charcoal, the crown gone and the remains uncovered.

mod atlas;
pub mod canopy;
mod meshes;
mod render;
mod scatter;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::sync_world::SyncToRenderWorld;
use scenario::Pos;

use crate::field::FireField;
use crate::sim::Sim;
use meshes::{PlantMesh, Tier, TIERS};
pub use render::PlantRenderPlugin;

/// How long a plant flames after the fire reaches it, in simulated seconds.
/// Well short of the cell's 20-minute burn-out: the crown goes fast, the
/// ground under it keeps smouldering.
pub(crate) const PLANT_FLAMING_S: f32 = 240.0;

/// The longest lag of a plant behind the interpolated front: even two shrubs
/// a metre apart do not catch at the same instant.
const LAG_S: f32 = 45.0;

/// Share of the plants inside a cleared line that are taken away.
const CLEARED_SHARE: f32 = 0.85;

/// When a plant the fire has not come near catches (`NEVER` in the shaders).
const NEVER: f32 = 1.0e9;

/// Real seconds between two looks at the fire (see [`burn`]).
const BURN_REDRAW_S: f32 = 0.15;

/// Fire cells per tile edge on the desktop: 64 cells = 1280 m. Smaller tiles
/// cull and pick the LOD more finely but cost a draw per variety each.
const TILE_CELLS: usize = 64;

/// Bytes of a plant's static record (see [`instance`]).
const INSTANCE_BYTES: u64 = 32;

/// Everything the GPU needs that never changes, built once per scene and
/// handed to the render world as it is.
pub(crate) struct PlantScene {
    /// Unique per build: the render world rebuilds its buffers when it changes.
    id: u64,
    meshes: Vec<PlantMesh>,
    groups: Vec<Group>,
    tiles: Vec<Tile>,
    atlas: atlas::Atlas,
    /// A burnt crown is gone and the remains show (desktop); otherwise the
    /// burnt plant turns all charcoal.
    remains: bool,
}

pub(crate) struct Tile {
    /// World bounds (Bevy frame) of its plants, sway included.
    lo: Vec3,
    hi: Vec3,
    /// Fire cells it covers, `(row0, row1, col0, col1)`, end exclusive.
    cells: (usize, usize, usize, usize),
    groups: std::ops::Range<usize>,
}

/// The plants of one variety in one tile.
pub(crate) struct Group {
    /// The mesh of each tier ([`Tier`] as index) in `PlantScene::meshes`.
    tiers: [Option<u32>; TIERS],
    /// [`INSTANCE_BYTES`] per plant.
    instances: Vec<u8>,
    count: u32,
    /// Each plant in the scenario frame (x east, y north) and its lag.
    spots: Vec<[f32; 3]>,
}

/// The fire as one group's plants see it: per plant the instant it catches
/// and whether it is cut (as the GPU reads them, `[f32; 2]`).
#[derive(Clone)]
pub(crate) struct GroupFire {
    state: Arc<Vec<[f32; 2]>>,
    version: u32,
    /// The plants the fire has come to (an instant of ignition known), in
    /// the order they came: only these draw their remains, each from its
    /// instant (`render`). A list that only grows: burnt is burnt.
    caught: Arc<Vec<u32>>,
}

impl GroupFire {
    fn new(plants: usize, version: u32) -> GroupFire {
        GroupFire { state: Arc::new(vec![[NEVER, 0.0]; plants]), version, caught: Arc::new(Vec::new()) }
    }
}

#[derive(Resource)]
pub struct Vegetation {
    scene: Arc<PlantScene>,
    fire: Vec<GroupFire>,
    /// Tiles the fire or a crew has touched (the others need no look).
    touched: Vec<bool>,
    /// What the shaders read each frame (`plant_common.wesl`).
    uniform: [f32; 12],
    lod_m: f32,
    keep: f32,
    tactical: f32,
    /// Simulated time and cleared cells when the fire was last looked at.
    drawn_at: Option<f32>,
    drawn_cleared: usize,
    since: f32,
}

/// The one entity the plants' draws hang on in the render phases.
#[derive(Component)]
pub(crate) struct PlantsDrawable;

/// A number in `[0, 1)` from the bits of a position: a plant's own fixed
/// random numbers.
fn hash01(x: f32, y: f32, salt: u32) -> f32 {
    let mut h = x.to_bits().wrapping_mul(0x9E37_79B1) ^ y.to_bits().wrapping_mul(0x85EB_CA6B) ^ salt;
    h = (h ^ (h >> 15)).wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 13;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// A plant's fixed place in the order in which the thinning takes plants
/// away: it is shown while this is below the share kept.
fn plant_rank(x: f32, y: f32) -> f32 {
    hash01(x, y, 0)
}

fn plant_lag(x: f32, y: f32) -> f32 {
    hash01(x, y, 0x51ED) * LAG_S
}

/// A plant's static record: foot (Bevy frame) and yaw, crown width and
/// height (m), tint (leaf red, green, blue, wood: multipliers / 2), and its
/// own numbers (thinning rank, sway phase, shade).
fn instance(i: &scatter::Inst) -> [u8; INSTANCE_BYTES as usize] {
    let (x, y) = (i.base.x, -i.base.z);
    let mut out = [0u8; INSTANCE_BYTES as usize];
    for (k, v) in [i.base.x, i.base.y, i.base.z, i.yaw, i.scale.x, i.scale.y].into_iter().enumerate() {
        out[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
    }
    out[24..28].copy_from_slice(&scatter::tint_bytes(i));
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    out[28..32].copy_from_slice(&[byte(plant_rank(x, y)), byte(hash01(x, y, 0x7A11)), byte(hash01(x, y, 0x5AD3)), 0]);
    out
}

/// What the GPU is told about the plant at `spot` (x, y, lag) when the fire
/// is `field` at `now`, given what it was told before: the instant it
/// catches (the interpolated arrival at its foot plus its lag; [`NEVER`]
/// while the fire is not near) and 1 if a crew cut it before the fire came.
/// A plant that has caught stays as it is: burnt is burnt.
fn plant_fire(field: &FireField, spot: [f32; 3], now: f32, was: [f32; 2]) -> [f32; 2] {
    if was[0] <= now {
        return was;
    }
    let [x, y, lag] = spot;
    let p = Pos { x, y };
    let ignite = field.arrival(p).map_or(NEVER, |a| a + lag);
    // cut by a crew: thinned, not razed, the closer to the middle of the line
    // the likelier, by the plant's own number
    let inside = ((field.cleared_fraction(p) - 0.45) / 0.15).clamp(0.0, 1.0);
    let cut = lag / LAG_S < CLEARED_SHARE * inside;
    [ignite, if cut { 1.0 } else { 0.0 }]
}

/// Plants of one tile, by variety, in a fixed order.
fn by_variety(stand: &scatter::Stand) -> BTreeMap<&'static str, Vec<&scatter::Inst>> {
    let mut out: BTreeMap<&'static str, Vec<&scatter::Inst>> = BTreeMap::new();
    for i in &stand.inst {
        out.entry(i.name).or_default().push(i);
    }
    out
}

static SCENES: AtomicU64 = AtomicU64::new(1);

pub fn spawn(mut commands: Commands, sim: Res<Sim>, gfx: Res<crate::quality::Graphics>) {
    let scn = &sim.scn;
    let w = scn.world;
    let (rows, cols) = (w.fire_rows, w.fire_cols);
    // the browser: the whole island one tile of proxies, never far from the camera
    let tile_cells = if cfg!(target_arch = "wasm32") { rows.max(cols) } else { TILE_CELLS };
    let full = gfx.lod_m > 0.0;
    let far = tile_cells < rows.max(cols);
    let remains = !cfg!(target_arch = "wasm32");

    let mut meshes: Vec<PlantMesh> = Vec::new();
    let mut mesh_of: HashMap<(&'static str, Tier), u32> = HashMap::new();
    let (mut groups, mut tiles) = (Vec::new(), Vec::new());
    let mut plants = 0usize;
    for r0 in (0..rows).step_by(tile_cells) {
        for c0 in (0..cols).step_by(tile_cells) {
            let cells = (r0, (r0 + tile_cells).min(rows), c0, (c0 + tile_cells).min(cols));
            let stand = scatter::scatter_tile(scn, gfx.vegetation, cells);
            if stand.inst.is_empty() {
                continue;
            }
            plants += stand.inst.len();
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let first = groups.len();
            for (name, list) in by_variety(&stand) {
                let mut tiers = [None; TIERS];
                for (tier, on) in [
                    (Tier::Full, full),
                    (Tier::Proxy, true),
                    (Tier::Far, far),
                    (Tier::RemainsNear, remains && full),
                    (Tier::RemainsFar, remains),
                ] {
                    if on {
                        let id = *mesh_of.entry((name, tier)).or_insert_with(|| {
                            meshes.push(meshes::variety_mesh(name, tier));
                            meshes.len() as u32 - 1
                        });
                        tiers[tier as usize] = Some(id);
                    }
                }
                let mut instances = Vec::with_capacity(list.len() * INSTANCE_BYTES as usize);
                let mut spots = Vec::with_capacity(list.len());
                for i in &list {
                    // a generous box: the models stand within about a unit of
                    // their foot, a little more in height and in the wind
                    lo = lo.min(i.base - Vec3::new(i.scale.x * 1.3, i.scale.y * 0.5, i.scale.x * 1.3));
                    hi = hi.max(i.base + Vec3::new(i.scale.x * 1.3, i.scale.y * 1.35, i.scale.x * 1.3));
                    instances.extend_from_slice(&instance(i));
                    let (x, y) = (i.base.x, -i.base.z);
                    spots.push([x, y, plant_lag(x, y)]);
                }
                groups.push(Group { tiers, instances, count: list.len() as u32, spots });
            }
            tiles.push(Tile { lo, hi, cells, groups: first..groups.len() });
        }
    }
    let triangles: usize = groups
        .iter()
        .map(|g| g.count as usize * g.tiers[if full { 0 } else { 1 }].map_or(0, |m| meshes[m as usize].triangles()))
        .sum();
    info!(
        "vegetation: {plants} plants in {} tiles, {} groups, {} meshes; {:.2} M triangles at the nearest level",
        tiles.len(),
        groups.len(),
        meshes.len(),
        triangles as f32 / 1e6,
    );
    let fire = groups
        .iter()
        .map(|g| GroupFire::new(g.count as usize, 0))
        .collect();
    let touched = vec![false; tiles.len()];
    let scene = PlantScene {
        id: SCENES.fetch_add(1, Ordering::Relaxed),
        meshes,
        groups,
        tiles,
        atlas: atlas::atlas(),
        remains,
    };
    commands.insert_resource(Vegetation {
        scene: Arc::new(scene),
        fire,
        touched,
        uniform: [0.0; 12],
        lod_m: gfx.lod_m,
        keep: gfx.veg_keep.clamp(0.0, 1.0),
        tactical: 0.0,
        drawn_at: None,
        drawn_cleared: 0,
        since: BURN_REDRAW_S,
    });
    // a root with a transform: the scene's teardown takes it away
    commands.spawn((PlantsDrawable, Transform::default(), SyncToRenderWorld::default()));
}

/// Hand the fire to the plants: at most every [`BURN_REDRAW_S`] of real
/// time, when the fire or the cleared lines changed, each plant of a tile
/// the fire has touched gets its instant of ignition ([`plant_fire`]).
pub fn burn(
    sim: Res<Sim>,
    time: Res<Time>,
    mut veg: ResMut<Vegetation>,
    mut restarted: MessageReader<crate::sim::SimRestarted>,
) {
    let veg = &mut *veg;
    let restarted = restarted.read().count() > 0;
    if restarted {
        for f in &mut veg.fire {
            *f = GroupFire::new(f.state.len(), f.version + 1);
        }
        veg.touched.fill(false);
        veg.drawn_at = None;
    }
    veg.since += time.delta_secs();
    if !restarted && veg.since < BURN_REDRAW_S {
        return;
    }
    let now = sim.time_s() as f32;
    let line_cells = sim.fire.line_cells;
    if veg.drawn_at == Some(now) && veg.drawn_cleared == line_cells {
        return;
    }
    (veg.since, veg.drawn_at, veg.drawn_cleared) = (0.0, Some(now), line_cells);
    let w = sim.scn.world;
    let (arrival, cleared) = (sim.fire.arrival_times(), sim.fire.cleared());
    let field = FireField { arrival, intensity: sim.fire.intensity(), cleared, world: w };
    let scene = veg.scene.clone();
    for (t, tile) in scene.tiles.iter().enumerate() {
        if !veg.touched[t] {
            // one cell around: the interpolation reaches across the edge
            let (r0, r1, c0, c1) = tile.cells;
            let (r0, r1) = (r0.saturating_sub(1), (r1 + 1).min(w.fire_rows));
            let (c0, c1) = (c0.saturating_sub(1), (c1 + 1).min(w.fire_cols));
            let hit = |i: usize| arrival[i] != i32::MIN || cleared[i];
            veg.touched[t] = (r0..r1).any(|r| (c0..c1).any(|c| hit(r * w.fire_cols + c)));
            if !veg.touched[t] {
                continue;
            }
        }
        for g in tile.groups.clone() {
            let fire = &mut veg.fire[g];
            let mut next: Option<Vec<[f32; 2]>> = None;
            for (i, &spot) in scene.groups[g].spots.iter().enumerate() {
                let was = fire.state[i];
                let now_is = plant_fire(&field, spot, now, was);
                if now_is != was {
                    next.get_or_insert_with(|| fire.state.as_ref().clone())[i] = now_is;
                }
            }
            if let Some(next) = next {
                let new: Vec<u32> = (0..next.len() as u32)
                    .filter(|&i| next[i as usize][0] < NEVER && fire.state[i as usize][0] >= NEVER)
                    .collect();
                if !new.is_empty() {
                    Arc::make_mut(&mut fire.caught).extend(new);
                }
                fire.state = Arc::new(next);
                fire.version += 1;
            }
        }
    }
}

/// The uniforms of the frame: the simulated clock, the share of plants
/// shown, the tactical greying, the wind (sway), real time (flicker).
pub fn clock(sim: Res<Sim>, time: Res<Time>, mut veg: ResMut<Vegetation>) {
    let weather = sim.fire.weather();
    // where the wind blows to, east and north, in the Bevy frame (x, z)
    let to = ((weather.wind_dir_deg + 180.0) as f32).to_radians();
    let strength = (weather.wind_speed_kmh as f32 / 45.0).clamp(0.0, 1.0);
    let remains = if veg.scene.remains { 1.0 } else { 0.0 };
    veg.uniform = [
        sim.time_s() as f32,
        PLANT_FLAMING_S,
        veg.keep,
        veg.tactical,
        to.sin(),
        -to.cos(),
        strength,
        time.elapsed_secs_wrapped(),
        remains,
        0.0,
        0.0,
        0.0,
    ];
}

impl Vegetation {
    /// Show only this share of the plants built, whatever the fire does to
    /// them (a rank per plant against the share, in the shader): a weak
    /// machine sheds fill rate without a rebuild.
    pub fn set_keep(&mut self, k: f32) {
        self.keep = k.clamp(0.0, 1.0);
    }

    /// Beyond this distance from the camera a tile shows its proxies.
    pub fn set_lod_m(&mut self, m: f32) {
        self.lod_m = m;
    }

    /// The tactical view's greying (`tactical.rs`).
    pub fn set_tactical(&mut self, amount: f32) {
        self.tactical = amount;
    }
}

/// A row of the plant lab (`veg_lab`): one variety, its plants' feet (Bevy
/// frame), their crown width and height (m) and yaw.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct LabRow {
    pub name: &'static str,
    pub feet: Vec<Vec3>,
    pub size: Vec3,
    pub yaw: f32,
}

#[cfg(not(target_arch = "wasm32"))]
impl Vegetation {
    /// The plant lab's scene (`veg_lab`): each row a tile of its own, the
    /// authored models at any distance, and their remains. No plant is
    /// thinned, none catches until [`Vegetation::set_lab_ignitions`] says when.
    pub(crate) fn lab(rows: &[LabRow]) -> Vegetation {
        let mut meshes = Vec::new();
        let (mut groups, mut tiles) = (Vec::new(), Vec::new());
        for row in rows {
            let mut tiers = [None; TIERS];
            for tier in [Tier::Full, Tier::Proxy, Tier::RemainsNear, Tier::RemainsFar] {
                meshes.push(meshes::variety_mesh(row.name, tier));
                tiers[tier as usize] = Some(meshes.len() as u32 - 1);
            }
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let mut instances = Vec::with_capacity(row.feet.len() * INSTANCE_BYTES as usize);
            for foot in &row.feet {
                lo = lo.min(*foot - Vec3::new(row.size.x * 1.3, row.size.y * 0.5, row.size.x * 1.3));
                hi = hi.max(*foot + Vec3::new(row.size.x * 1.3, row.size.y * 1.35, row.size.x * 1.3));
                let mut record = [0u8; INSTANCE_BYTES as usize];
                for (k, v) in [foot.x, foot.y, foot.z, row.yaw, row.size.x, row.size.y].into_iter().enumerate() {
                    record[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
                }
                // the atlas' own colours (multipliers 1, stored halved), always
                // shown (rank 0), the middle shade
                record[24..32].copy_from_slice(&[128, 128, 128, 128, 0, 128, 128, 0]);
                instances.extend_from_slice(&record);
            }
            tiles.push(Tile { lo, hi, cells: (0, 0, 0, 0), groups: groups.len()..groups.len() + 1 });
            groups.push(Group { tiers, instances, count: row.feet.len() as u32, spots: Vec::new() });
        }
        let fire = groups.iter().map(|g| GroupFire::new(g.count as usize, 0)).collect();
        let touched = vec![false; tiles.len()];
        let scene = PlantScene { id: SCENES.fetch_add(1, Ordering::Relaxed), meshes, groups, tiles, atlas: atlas::atlas(), remains: true };
        Vegetation {
            scene: Arc::new(scene),
            fire,
            touched,
            uniform: [0.0; 12],
            // the full models however far the camera goes
            lod_m: f32::MAX,
            keep: 1.0,
            tactical: 0.0,
            drawn_at: None,
            drawn_cleared: 0,
            since: 0.0,
        }
    }

    /// When each plant of row `r` catches, in simulated seconds: every one
    /// of them does.
    pub(crate) fn set_lab_ignitions(&mut self, r: usize, at: &[f32]) {
        let Some(fire) = self.fire.get_mut(r) else { return };
        if at.len() != fire.state.len() {
            return;
        }
        fire.state = Arc::new(at.iter().map(|&t| [t, 0.0]).collect());
        let mut caught: Vec<u32> = (0..at.len() as u32).collect();
        caught.sort_by(|&a, &b| at[a as usize].total_cmp(&at[b as usize]));
        fire.caught = Arc::new(caught);
        fire.version += 1;
    }

    /// The lab's uniforms, as [`clock`] writes the game's: its own clock,
    /// still air, every plant shown, the remains under burnt crowns.
    pub(crate) fn set_lab_clock(&mut self, now: f32, real_s: f32) {
        self.uniform = [now, PLANT_FLAMING_S, 1.0, 0.0, 0.0, 1.0, 0.0, real_s, 1.0, 0.0, 0.0, 0.0];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thinning_rank_is_spread_evenly_and_fixed() {
        let at = |i: usize| ((i % 80) as f32 * 7.3 + 0.37, (i / 80) as f32 * 11.1 + 0.91);
        let ranks: Vec<f32> = (0..4000).map(|i| plant_rank(at(i).0, at(i).1)).collect();
        assert!(ranks.iter().all(|r| (0.0..=1.0).contains(r)));
        for keep in [0.2f32, 0.35, 0.6] {
            let shown = ranks.iter().filter(|&&r| r < keep).count() as f32 / ranks.len() as f32;
            assert!((shown - keep).abs() < 0.05, "keep {keep}: {shown}");
        }
        assert_eq!(plant_rank(10.0, 20.0), plant_rank(10.0, 20.0));
        let lags: Vec<f32> = (0..4000).map(|i| plant_lag(at(i).0, at(i).1)).collect();
        let mean = lags.iter().sum::<f32>() / lags.len() as f32;
        assert!((mean - LAG_S / 2.0).abs() < 2.0, "{mean}");
    }

    /// Through a real `medium` fire, every plant the front passed long
    /// enough ago has caught (or was cut before the fire came), a plant
    /// once caught keeps its instant while the fire goes on, and a plant
    /// far from the fire is never told it will catch.
    #[test]
    fn the_plants_catch_with_the_front_and_stay_burnt() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut sim = Sim::new(&data, "medium", 2).unwrap();
        let w = sim.scn.world;
        let mut rng = scatter::Rng::seeded(3);
        let spots: Vec<[f32; 3]> = (0..20_000)
            .map(|_| {
                let (x, y) = (rng.unit() * w.width_m, rng.unit() * w.height_m);
                [x, y, plant_lag(x, y)]
            })
            .collect();
        let mut states = vec![[NEVER, 0.0f32]; spots.len()];
        fn field_of(sim: &Sim) -> FireField<'_> {
            FireField {
                        arrival: sim.fire.arrival_times(),
                intensity: sim.fire.intensity(),
                cleared: sim.fire.cleared(),
                world: sim.scn.world,
            }
        }
        let mut caught_at: HashMap<usize, f32> = HashMap::new();
        for _ in 0..40 {
            for _ in 0..(300.0 / rocca::STEP_S as f32).max(1.0) as usize {
                if sim.is_over() {
                    break;
                }
                sim.game.step().unwrap();
            }
            let now = sim.time_s() as f32;
            let field = field_of(&sim);
            for (i, s) in states.iter_mut().enumerate() {
                *s = plant_fire(&field, spots[i], now, *s);
                if s[0] <= now {
                    let t = *caught_at.entry(i).or_insert(s[0]);
                    assert_eq!(t, s[0], "a caught plant keeps its instant");
                }
            }
        }
        let now = sim.time_s() as f32;
        let field = field_of(&sim);
        let (mut checked, mut burnt) = (0, 0);
        for (i, s) in states.iter().enumerate() {
            let p = Pos { x: spots[i][0], y: spots[i][1] };
            match field.arrival(p) {
                Some(a) if now >= a + LAG_S + PLANT_FLAMING_S => {
                    checked += 1;
                    assert!(s[0] <= now || s[1] > 0.5, "{p:?}: not caught, arrival {a}, now {now}");
                    burnt += usize::from(s[0] <= now);
                }
                None => assert_eq!(s[0], NEVER, "{p:?}: no fire near"),
                _ => {}
            }
        }
        assert!(burnt > 100 && checked > 100, "{burnt} of {checked}");
    }

    /// How many plants, groups and draws the desktop (`alta`, 0.5) and the
    /// browser (0.025, one tile) build: printed, run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn instancing_counts() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let sim = Sim::new(&data, "medium", 2).unwrap();
        let w = sim.scn.world;
        for (label, density, tile) in [("desktop alta", 0.5, TILE_CELLS), ("browser", 0.025, w.fire_rows.max(w.fire_cols))] {
            let (mut plants, mut groups, mut tiles) = (0, 0, 0);
            for r0 in (0..w.fire_rows).step_by(tile) {
                for c0 in (0..w.fire_cols).step_by(tile) {
                    let stand = scatter::scatter_tile(&sim.scn, density, (r0, (r0 + tile).min(w.fire_rows), c0, (c0 + tile).min(w.fire_cols)));
                    if stand.inst.is_empty() {
                        continue;
                    }
                    tiles += 1;
                    plants += stand.inst.len();
                    groups += by_variety(&stand).len();
                }
            }
            eprintln!("{label}: {plants} plants, {tiles} tiles, {groups} tile x variety groups");
        }
    }
}
