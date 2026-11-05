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
// the plants alight, for the flames on them (`fire_view`)
pub(crate) mod torch;

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

/// The levels of a group's quadtree ([`Node`]): its plants sorted along a
/// Morton curve on a grid of `2^NODE_LEVELS` per side of the group's
/// square, so every node is a run of its instance buffer.
const NODE_LEVELS: u32 = 6;
/// A node is not split below this side (m) or this many plants.
const NODE_LEAF_M: f32 = 40.0;
const NODE_LEAF_PLANTS: usize = 24;
/// No node.
pub(crate) const NO_NODE: u32 = u32::MAX;

/// A square of a group's plants (2026-11-04: the level of detail is chosen
/// per node, no longer per 1280 m tile): its plants are the run
/// `start..end` of the group's instances, in its box; its children cover its
/// four quarters, in the order of the run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Node {
    pub(crate) lo: Vec3,
    pub(crate) hi: Vec3,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) kids: [u32; 4],
}

/// A plant's box, generous: the models stand within about a unit of their
/// foot, a little more in height and in the wind.
fn plant_box(base: Vec3, scale: Vec3) -> (Vec3, Vec3) {
    (base - Vec3::new(scale.x * 1.3, scale.y * 0.5, scale.x * 1.3), base + Vec3::new(scale.x * 1.3, scale.y * 1.35, scale.x * 1.3))
}

fn morton(x: u32, z: u32) -> u32 {
    let spread = |mut v: u32| {
        v &= 0xffff;
        v = (v | (v << 8)) & 0x00ff_00ff;
        v = (v | (v << 4)) & 0x0f0f_0f0f;
        v = (v | (v << 2)) & 0x3333_3333;
        (v | (v << 1)) & 0x5555_5555
    };
    spread(x) | (spread(z) << 1)
}

/// The plants of `list` sorted along the Morton curve of their square, with
/// their keys.
fn morton_sorted<'a>(list: &[&'a scatter::Inst]) -> (Vec<&'a scatter::Inst>, Vec<u32>, f32) {
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for i in list {
        lo = lo.min(Vec2::new(i.base.x, i.base.z));
        hi = hi.max(Vec2::new(i.base.x, i.base.z));
    }
    let side = (hi - lo).max_element().max(1.0);
    let n = 1u32 << NODE_LEVELS;
    let q = |v: f32, o: f32| (((v - o) / side * n as f32) as u32).min(n - 1);
    let mut keyed: Vec<(u32, &scatter::Inst)> = list.iter().map(|i| (morton(q(i.base.x, lo.x), q(i.base.z, lo.y)), *i)).collect();
    keyed.sort_by_key(|k| k.0);
    (keyed.iter().map(|k| k.1).collect(), keyed.iter().map(|k| k.0).collect(), side)
}

/// The quadtree over plants sorted by [`morton_sorted`]: the root first.
fn build_nodes(list: &[&scatter::Inst], keys: &[u32], side: f32) -> Vec<Node> {
    fn node(nodes: &mut Vec<Node>, list: &[&scatter::Inst], keys: &[u32], start: usize, end: usize, level: u32, prefix: u32, side: f32) -> u32 {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for i in &list[start..end] {
            let (a, b) = plant_box(i.base, i.scale);
            lo = lo.min(a);
            hi = hi.max(b);
        }
        let id = nodes.len() as u32;
        nodes.push(Node { lo, hi, start: start as u32, end: end as u32, kids: [NO_NODE; 4] });
        let node_side = side / (1u32 << level) as f32;
        if level < NODE_LEVELS && end - start > NODE_LEAF_PLANTS && node_side > NODE_LEAF_M {
            let shift = 2 * (NODE_LEVELS - level - 1);
            let mut from = start;
            for q in 0..4u32 {
                let child = (prefix << 2) | q;
                let limit = (child + 1) << shift;
                let to = start + keys[start..end].partition_point(|&k| k < limit);
                if to > from {
                    let k = node(nodes, list, keys, from, to, level + 1, child, side);
                    nodes[id as usize].kids[q as usize] = k;
                }
                from = to;
            }
        }
        id
    }
    let mut nodes = Vec::new();
    if !list.is_empty() {
        node(&mut nodes, list, keys, 0, list.len(), 0, 0, side);
    }
    nodes
}

/// A single node holding all of a group's plants (`lo..hi` their box).
fn root_only(lo: Vec3, hi: Vec3, count: usize) -> Vec<Node> {
    vec![Node { lo, hi, start: 0, end: count as u32, kids: [NO_NODE; 4] }]
}

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
    /// burnt plant turns all charcoal and its crown shrinks.
    remains: bool,
    /// Size of a canopy tile in the atlas (`meshes::canopy_tile`), for the
    /// shader that paints it over the lobed stand-ins.
    canopy_tile: [f32; 2],
    /// A thinned plant is not drawn at all, burning or not: each group is
    /// sorted by the thinning rank and only the share kept is drawn, so the
    /// thinning saves vertices too (the browser, where nothing else does).
    /// Elsewhere every plant is drawn and the shader hides the thinned ones
    /// the fire has not reached.
    thin_by_count: bool,
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
    /// Each plant's thinning rank as the GPU reads it (a byte), ascending
    /// when the scene thins by count (`PlantScene::thin_by_count`).
    ranks: Vec<u8>,
    /// Its quadtree, the root first ([`Node`]); a single node when the
    /// scene thins by count (the plants are in the order of their ranks).
    nodes: Vec<Node>,
}

impl Group {
    /// How many of its plants the thinning keeps, when they are sorted by
    /// their rank: those the shader would show (`seed.x < keep`).
    pub(crate) fn kept(&self, keep: f32) -> u32 {
        self.ranks.partition_point(|&r| (r as f32 / 255.0) < keep) as u32
    }
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
/// own numbers (thinning rank, sway phase, shade, how closed the stand
/// around it is: [`canopy::StandDensity`], 0 when the forest's occlusion is
/// off).
fn instance(i: &scatter::Inst, stand: f32) -> [u8; INSTANCE_BYTES as usize] {
    let (x, y) = (i.base.x, -i.base.z);
    let mut out = [0u8; INSTANCE_BYTES as usize];
    for (k, v) in [i.base.x, i.base.y, i.base.z, i.yaw, i.scale.x, i.scale.y].into_iter().enumerate() {
        out[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
    }
    out[24..28].copy_from_slice(&scatter::tint_bytes(i));
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    out[28..32].copy_from_slice(&[byte(plant_rank(x, y)), byte(hash01(x, y, 0x7A11)), byte(hash01(x, y, 0x5AD3)), byte(stand)]);
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
    let started = bevy::platform::time::Instant::now();
    let scn = &sim.scn;
    let w = scn.world;
    let (rows, cols) = (w.fire_rows, w.fire_cols);
    // the browser: the whole island one tile of proxies, never far from the camera
    let tile_cells = if crate::quality::WEBGL2 { rows.max(cols) } else { TILE_CELLS };
    let full = gfx.lod_m > 0.0;
    let far = tile_cells < rows.max(cols);
    let remains = !crate::quality::WEBGL2;
    // the browser's stand-ins: the crown parts as lumps (`meshes::lobed_mesh`);
    // they cast their own shadows (2026-11-02: the cheapest crowns stood out
    // of the lumps and shadowed them in hard notches)
    let lobes = gfx.lobes;
    let thin_by_count = !remains;
    let density = canopy::forest_shade().then(|| canopy::StandDensity::new(scn));

    // the trial of full-resolution models (`hi_trees`): the nearest
    // broadleaves around a point, taken out of the instanced plants
    #[cfg(not(target_arch = "wasm32"))]
    let hi = crate::hi_trees::spec().map(|spec| {
        let mut found: Vec<(f32, Vec3, f32, f32)> = Vec::new();
        for r0 in (0..rows).step_by(tile_cells) {
            for c0 in (0..cols).step_by(tile_cells) {
                let cells = (r0, (r0 + tile_cells).min(rows), c0, (c0 + tile_cells).min(cols));
                for i in scatter::scatter_tile(scn, gfx.vegetation, cells).inst.iter().filter(|i| crate::hi_trees::replaces(i.name)) {
                    let d = Vec2::new(i.base.x, -i.base.z).distance(spec.at);
                    if d < spec.radius {
                        found.push((d, i.base, i.yaw, i.scale.y));
                    }
                }
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.truncate(spec.max);
        let keys: std::collections::HashSet<(u32, u32)> = found.iter().map(|f| (f.1.x.to_bits(), f.1.z.to_bits())).collect();
        let spots = found.iter().map(|f| crate::hi_trees::HiSpot { path: spec.path.clone(), foot: f.1, yaw: f.2, height: Some(f.3) }).collect();
        commands.insert_resource(crate::hi_trees::HiTrees { spots });
        keys
    });
    #[cfg(target_arch = "wasm32")]
    let hi: Option<std::collections::HashSet<(u32, u32)>> = None;

    let mut meshes: Vec<PlantMesh> = Vec::new();
    let mut mesh_of: HashMap<(&'static str, Tier), Option<u32>> = HashMap::new();
    let (mut groups, mut tiles) = (Vec::new(), Vec::new());
    let mut plants = 0usize;
    for r0 in (0..rows).step_by(tile_cells) {
        for c0 in (0..cols).step_by(tile_cells) {
            let cells = (r0, (r0 + tile_cells).min(rows), c0, (c0 + tile_cells).min(cols));
            let mut stand = scatter::scatter_tile(scn, gfx.vegetation, cells);
            if let Some(keys) = &hi {
                stand.inst.retain(|i| !keys.contains(&(i.base.x.to_bits(), i.base.z.to_bits())));
            }
            if stand.inst.is_empty() {
                continue;
            }
            plants += stand.inst.len();
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let first = groups.len();
            for (name, mut list) in by_variety(&stand) {
                // the plants kept by the thinning first (`Group::kept`), or
                // along the Morton curve of the group's quadtree
                let mut tree = None;
                if thin_by_count {
                    list.sort_by(|a, b| plant_rank(a.base.x, -a.base.z).total_cmp(&plant_rank(b.base.x, -b.base.z)));
                } else {
                    let (sorted, keys, side) = morton_sorted(&list);
                    tree = Some(build_nodes(&sorted, &keys, side));
                    list = sorted;
                }
                let mut tiers = [None; TIERS];
                // a scanned plant: its `lod3` for the proxy, lumps of it far away
                let scanned = crate::models::scan_of(name).is_some();
                for (tier, on) in [
                    (Tier::Full, full),
                    (Tier::Proxy, true),
                    (Tier::Far, far || scanned),
                    (Tier::RemainsNear, remains && full),
                    (Tier::RemainsFar, remains),
                    (Tier::Near, full && scanned),
                    (Tier::Mid, full && scanned),
                ] {
                    if on {
                        // `None`: nothing to draw (a scanned plant's remains without debris)
                        let id = *mesh_of.entry((name, tier)).or_insert_with(|| {
                            let mesh = if tier == Tier::Proxy && lobes { meshes::lobed_mesh(name) } else { meshes::variety_mesh(name, tier) };
                            (mesh.triangles() > 0).then(|| {
                                meshes.push(mesh);
                                meshes.len() as u32 - 1
                            })
                        });
                        tiers[tier as usize] = id;
                    }
                }
                let mut instances = Vec::with_capacity(list.len() * INSTANCE_BYTES as usize);
                let mut spots = Vec::with_capacity(list.len());
                let mut ranks = Vec::with_capacity(list.len());
                let (mut glo, mut ghi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for i in &list {
                    let (a, b) = plant_box(i.base, i.scale);
                    (glo, ghi) = (glo.min(a), ghi.max(b));
                    let record = instance(i, density.as_ref().map_or(0.0, |d| d.at(Pos { x: i.base.x, y: -i.base.z })));
                    ranks.push(record[28]);
                    instances.extend_from_slice(&record);
                    let (x, y) = (i.base.x, -i.base.z);
                    spots.push([x, y, plant_lag(x, y)]);
                }
                (lo, hi) = (lo.min(glo), hi.max(ghi));
                let nodes = tree.unwrap_or_else(|| root_only(glo, ghi, list.len()));
                groups.push(Group { tiers, instances, count: list.len() as u32, spots, ranks, nodes });
            }
            tiles.push(Tile { lo, hi, cells, groups: first..groups.len() });
        }
    }
    let triangles: usize = groups
        .iter()
        .map(|g| g.count as usize * g.tiers[if full { 0 } else { 1 }].map_or(0, |m| meshes[m as usize].triangles()))
        .sum();
    info!(
        "vegetation: {plants} plants in {} tiles, {} groups, {} meshes; {:.2} M triangles at the nearest level; built in {:.0} ms",
        tiles.len(),
        groups.len(),
        meshes.len(),
        triangles as f32 / 1e6,
        started.elapsed().as_secs_f32() * 1000.0,
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
        atlas: atlas::plant_atlas(),
        remains,
        canopy_tile: meshes::canopy_tile(),
        thin_by_count,
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
    let [tile_u, tile_v] = veg.scene.canopy_tile;
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
        tile_u,
        tile_v,
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
            let nodes = root_only(lo, hi, row.feet.len());
            groups.push(Group { tiers, instances, count: row.feet.len() as u32, spots: Vec::new(), ranks: vec![0; row.feet.len()], nodes });
        }
        let fire = groups.iter().map(|g| GroupFire::new(g.count as usize, 0)).collect();
        let touched = vec![false; tiles.len()];
        let scene = PlantScene {
            id: SCENES.fetch_add(1, Ordering::Relaxed),
            meshes,
            groups,
            tiles,
            atlas: atlas::plant_atlas(),
            remains: true,
            canopy_tile: meshes::canopy_tile(),
            thin_by_count: false,
        };
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
        let [tile_u, tile_v] = self.scene.canopy_tile;
        self.uniform = [now, PLANT_FLAMING_S, 1.0, 0.0, 0.0, 1.0, 0.0, real_s, 1.0, tile_u, tile_v, 0.0];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A group's quadtree: every node's children split its run exactly, in
    /// order; every plant of a node stands in its box; leaves are small.
    #[test]
    fn the_plant_quadtree_covers_its_runs() {
        let insts: Vec<scatter::Inst> = (0..5000)
            .map(|i| {
                let (x, z) = (hash01(i as f32, 1.0, 7) * 1280.0, hash01(i as f32, 2.0, 9) * -1280.0);
                scatter::Inst { name: "holm_1", base: Vec3::new(x, 10.0, z), scale: Vec3::new(3.0, 6.0, 3.0), yaw: 0.0, leaf: [1.0; 3], wood: 1.0 }
            })
            .collect();
        let refs: Vec<&scatter::Inst> = insts.iter().collect();
        let (sorted, keys, side) = morton_sorted(&refs);
        assert!(keys.windows(2).all(|k| k[0] <= k[1]));
        let nodes = build_nodes(&sorted, &keys, side);
        assert_eq!((nodes[0].start, nodes[0].end), (0, 5000));
        let mut leaves = 0;
        for n in &nodes {
            for i in &sorted[n.start as usize..n.end as usize] {
                let (a, b) = plant_box(i.base, i.scale);
                assert!(a.cmpge(n.lo).all() && b.cmple(n.hi).all());
            }
            let kids: Vec<&Node> = n.kids.iter().filter(|&&k| k != NO_NODE).map(|&k| &nodes[k as usize]).collect();
            if kids.is_empty() {
                leaves += 1;
                assert!(n.end - n.start <= NODE_LEAF_PLANTS as u32 || (n.hi.x - n.lo.x) < NODE_LEAF_M + 2.0 * 3.0 * 1.3 + 1.0 + side / 64.0);
                continue;
            }
            assert_eq!(kids[0].start, n.start);
            assert_eq!(kids.last().unwrap().end, n.end);
            assert!(kids.windows(2).all(|w| w[0].end == w[1].start));
        }
        assert!(leaves > 100, "{leaves} leaves");
    }

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

    /// Thinning by count draws exactly the plants the shader would show: a
    /// group sorted by rank keeps those whose rank byte is below the share.
    #[test]
    fn thinning_by_count_keeps_what_the_shader_shows() {
        let at = |i: usize| ((i % 50) as f32 * 9.7 + 0.13, (i / 50) as f32 * 13.3 + 0.71);
        let mut ranks: Vec<u8> = (0..2000).map(|i| (plant_rank(at(i).0, at(i).1) * 255.0).round() as u8).collect();
        ranks.sort();
        let group = Group { tiers: [None; TIERS], instances: Vec::new(), count: ranks.len() as u32, spots: Vec::new(), ranks: ranks.clone(), nodes: Vec::new() };
        for keep in [0.0f32, 0.2, 0.35, 0.6, 0.999, 1.0] {
            let shown = ranks.iter().filter(|&&r| r as f32 / 255.0 < keep).count() as u32;
            assert_eq!(group.kept(keep), shown, "keep {keep}");
        }
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
