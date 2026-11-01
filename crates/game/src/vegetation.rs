//! Fuel-driven vegetation using Blender-authored tree and shrub archetypes.
//!
//! The fuel raster is the only vegetation source this scenario has — there is
//! no tree inventory, and there never will be one at 10 km scale — so the
//! plants are generated from it: one archetype per `eu_fuel12` group, scattered
//! deterministically from a hash of the cell they stand in. Deterministic
//! matters more than it sounds: the same seed must produce the same forest on
//! every run, or a debrief replay would show a different landscape than the
//! incident did.
//!
//! The terrain under the plants is bare ground (see [`crate::terrain_mesh`]),
//! so *all* of the land cover a player sees is geometry: grass tussocks and
//! macchia as much as trees. That raises the plant count into the hundreds of
//! thousands, which sets the budget for everything below — each archetype is
//! built to at most 128 vertices, and the visual density comes from crown
//! width and overlap rather than from stem count, because a hundred wide
//! crowns close a canopy that a thousand narrow ones would not.
//!
//! The plants are not entities and have no state (2026-10-31). Each variety
//! has one mesh per level of detail (authored model, distant proxy and its
//! cheaper tier, burnt remains near and far), built once, and every plant is
//! an instance of it: a static per-instance vertex buffer (where it stands,
//! its yaw, scale and tint) written when the scene is built
//! (`veg_instancing.rs`). An entity is a tile × variety × tier, with the
//! tile's bounds for frustum culling: 1280 m tiles on the desktop, whose level
//! of detail [`lod`] picks by showing and hiding entities, and the whole
//! island as one tile in the browser (a few dozen draws, no culling: its
//! 0.1 M plant vertices are nothing for a GPU).
//!
//! The fire reaches the plants like the terrain: through a texture on the
//! fire grid ([`FireGrid`]: one RGBA8 texel per cell, the arrival time and
//! the cleared flag), written when a cell changes, never per plant. In the
//! vertex stage (`shaders/veg_state.wgsl`) each plant interpolates the
//! arrival at its foot (as `field::FireField::arrival` does, so a stand
//! catches as a wave crossing it rather than as a 20 m block), adds its own
//! lag (a hash of its position, [`plant_lag`]), and from the simulated clock
//! works out whether it stands, burns, is charcoal or gone (thinned by the
//! share kept, cut by a crew). [`plant_look`] is the CPU's copy of that rule,
//! for the tests.

use bevy::light::{NotShadowCaster, TransmittedShadowReceiver};
use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use fire::CellFire;
use scenario::{Cell, CoverClass, Pos, Scenario};

use crate::field::{noise, FireField};
use crate::materials::{self, FoliageMaterial};
use crate::sim::Sim;
use crate::veg_instancing::{PlantInstance, VegInstances};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::render::batching::NoAutomaticBatching;
use std::sync::Arc;

/// Chunk edge in fire cells. 32 cells = 640 m, matching the terrain chunking,
/// so a burning front dirties a similar number of both.
const CHUNK_CELLS: usize = 32;

/// Expected plants per 20 m fire cell (400 m²), per archetype, before the
/// patchiness modulation below. Scaled by the graphics quality
/// ([`crate::quality::Graphics::vegetation`], or `KIOSK_VEG_DENSITY`).
///
/// These are stand densities, not decoration: 8 tussocks per cell is 200/ha of
/// grassland, 6 macchia clumps is 150/ha of shrubland, and 3.6 stems is 90/ha
/// of open Mediterranean pine. Still below a real forest inventory, but with
/// the crown widths below it closes the cover, which is the point — bare
/// ground showing between plants is what made this look like decoration
/// scattered on a heightfield rather than like a landscape.
///
/// Set by measurement, not by taste: 10.5 M triangles rendered at 119 fps in
/// release on an M4 Pro, so there was room for roughly another half.
///
/// Broadleaf is 2.5 (was 3.4 for class 5 and 1.4 olives for class 4): the
/// chestnut crowns are wide, and the total plant count stays the same.
const DENSITY: [f32; 4] = [11.0, 2.5, 6.5, 3.8];

/// Within a group, how the fuel class scales [`DENSITY`]: moist broadleaf a
/// little denser than dry, macchia from sparse (7) to closed (9). Weighted by
/// the territory's cells, the mean stays at 1: no more plants overall.
fn class_density(fuel: i32) -> f32 {
    match fuel {
        4 => 1.04,
        5 => 0.97,
        7 => 0.75,
        9 => 1.15,
        _ => 1.0,
    }
}

/// Vegetation does not honour cell boundaries, and it is not uniform inside
/// one either: real stands are patchy at tens of metres. Two octaves of value
/// noise at 60 m and 25 m modulate the local count, which is what breaks up
/// the 20 m grid the fuel raster imposes.
fn patchiness(p: Pos) -> f32 {
    let coarse = noise(p.x / 60.0, p.y / 60.0, 0x5A17);
    let fine = noise(p.x / 25.0, p.y / 25.0, 0x91C3);
    (0.25 + 1.75 * (0.65 * coarse + 0.35 * fine)).clamp(0.0, 2.0)
}

/// How long a plant flames after the fire reaches it, in simulated seconds.
/// Well short of the cell's 20-minute burn-out: the crown goes fast, the
/// ground under it keeps smouldering.
pub(crate) const PLANT_FLAMING_S: f32 = 240.0;

/// Vegetation archetypes, one per `eu_fuel12` group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Species {
    /// classes 1-3: grassland, drawn as crossed blade tufts
    Grass = 0,
    /// classes 4-6: broadleaves, drawn as chestnuts: a big round lobed crown
    /// on a short trunk
    Broadleaf = 1,
    /// classes 7-9: shrubland and macchia, low dense rounded clumps
    Shrub = 2,
    /// classes 10-12: conifers, drawn as maritime pines: a tall bare reddish
    /// trunk under a flat umbrella crown
    Conifer = 3,
}

impl Species {
    fn of_fuel(fuel: i32) -> Option<Species> {
        Some(match fuel {
            1..=3 => Species::Grass,
            4..=6 => Species::Broadleaf,
            7..=9 => Species::Shrub,
            10..=12 => Species::Conifer,
            _ => return None,
        })
    }

    /// Foliage colour for a fuel class, as a pair the per-plant tint
    /// interpolates between.
    ///
    /// A single colour per class is what makes procedural vegetation look like
    /// plastic: a real stand varies within metres. The two ends are the dry
    /// and the vigorous extreme; each plant lands somewhere between them. The
    /// four groups are kept apart in hue as well as in shape (iteration 5b,
    /// the player must tell them apart): pine dark blue-green, chestnut a
    /// bright mid green, macchia dark olive, grass straw.
    fn foliage_range(fuel: i32) -> ([f32; 3], [f32; 3]) {
        match fuel {
            // gardens and lawns round the houses
            1 => ([0.46, 0.52, 0.24], [0.34, 0.48, 0.20]),
            // Cured Mediterranean grassland: straw; the fuel that carries fire
            // fastest. Class 3 (dry meadow) paler and more golden.
            2 => ([0.66, 0.56, 0.28], [0.54, 0.50, 0.24]),
            3 => ([0.80, 0.68, 0.36], [0.70, 0.60, 0.30]),
            // Chestnut; class 4 (moist) greener than 5.
            4 => ([0.24, 0.40, 0.13], [0.17, 0.34, 0.11]),
            5..=6 => ([0.32, 0.42, 0.15], [0.24, 0.36, 0.12]),
            // Macchia: dark olive, greyer than the chestnut, warmer than the pine.
            7..=9 => ([0.27, 0.32, 0.15], [0.18, 0.25, 0.12]),
            // Maritime pine: dark, toward blue.
            _ => ([0.13, 0.22, 0.18], [0.08, 0.17, 0.15]),
        }
    }
}

/// Enough of a plant to build its burnt model: the archetype, where and how
/// big it stood (its yaw also shades it).
#[derive(Clone)]
struct Remains {
    name: &'static str,
    base: Vec3,
    scale: Vec3,
    yaw: f32,
}

/// The levels of detail of a variety, each one mesh drawn as instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tier {
    /// The authored model, within [`crate::quality::Graphics::lod_m`].
    Full,
    /// The distant proxy beyond it, and its cheaper tier beyond [`FAR_TIER`].
    Proxy,
    Far,
    /// What fire adds (twig skeleton, debris), near and far.
    RemainsNear,
    RemainsFar,
}

/// A tile of the island (640 m on the desktop, the whole island in the
/// browser): per variety present and per tier, one entity drawing that
/// variety's mesh once per plant of the tile (`veg_instancing.rs`), with the
/// tile's bounds for culling. The level of detail shows and hides these.
struct Tile {
    full: Vec<Entity>,
    low: Vec<Entity>,
    far_low: Vec<Entity>,
    /// The proxies again, on the sun's layer only: the shadow of a
    /// full-model tile too far for its crowns to show in the shadow map.
    shadow: Vec<Entity>,
    /// The burnt remains, spawned when the fire first reaches the tile.
    char_near: Vec<Entity>,
    char_far: Vec<Entity>,
    /// The tile's plants, by variety, to spawn the remains from.
    plants: Vec<(&'static str, Arc<Vec<PlantInstance>>, Aabb)>,
    /// `far_low` is on show rather than `low`.
    far: bool,
    /// The full models are on show rather than the proxies.
    near: bool,
    /// The full models cast the sun's shadow (within [`FULL_SHADOW_M`]).
    full_casts: bool,
    /// World-space bounds (Bevy frame) of the plants, for the LOD distance.
    bounds: (Vec3, Vec3),
    /// Fire cells the tile covers, `(row0, row1, col0, col1)`, end exclusive.
    cells: (usize, usize, usize, usize),
}

/// Share of the built plants that are shown (bits of an `f32`; see
/// [`Vegetation::set_keep`]). 1 = all: the plants are as spawned.
static KEEP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

fn keep() -> f32 {
    f32::from_bits(KEEP.load(std::sync::atomic::Ordering::Relaxed))
}

/// A number in `[0, 1)` from the bits of a position, the same in the shader
/// (`hash01` in `veg_state.wgsl`): a plant's own fixed random numbers.
#[cfg_attr(not(test), allow(dead_code))] // the shader's rule (`veg_state.wgsl`), for the tests
fn hash01(x: f32, y: f32, salt: u32) -> f32 {
    let mut h = x.to_bits().wrapping_mul(0x9E37_79B1) ^ y.to_bits().wrapping_mul(0x85EB_CA6B) ^ salt;
    h = (h ^ (h >> 15)).wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 13;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// A plant's fixed place in the order in which the thinning takes plants
/// away (0..1): it is shown while this is below the share kept.
#[cfg_attr(not(test), allow(dead_code))] // the shader's rule (`veg_state.wgsl`), for the tests
fn plant_rank(p: Pos) -> f32 {
    hash01(p.x, p.y, 0)
}

/// Seconds a plant lags the interpolated front: even two shrubs a metre apart
/// do not catch at the same instant.
#[cfg_attr(not(test), allow(dead_code))] // the shader's rule (`veg_state.wgsl`), for the tests
pub(crate) fn plant_lag(p: Pos) -> f32 {
    hash01(p.x, p.y, 0x51ED) * LAG_S
}

/// The longest [`plant_lag`].
#[cfg_attr(not(test), allow(dead_code))] // the shader's rule (`veg_state.wgsl`), for the tests
const LAG_S: f32 = 45.0;

#[derive(Resource)]
pub struct Vegetation {
    /// The plants' material, and the remains' (`FoliageMaterial::layer`).
    material: Handle<FoliageMaterial>,
    remains_material: Option<Handle<FoliageMaterial>>,
    /// The fire grid the plants read (see [`FireGrid`]).
    grid: FireGrid,
    /// The variety meshes, by variety and tier.
    meshes: std::collections::HashMap<(&'static str, Tier), Handle<Mesh>>,
    tiles: Vec<Tile>,
    /// Beyond this distance from the camera a tile shows its proxies.
    lod_m: f32,
    /// Simulated time the grid was last written at.
    drawn_at: Option<f32>,
    /// Cells cleared by crews when the grid was last written.
    drawn_cleared: usize,
}

/// The fire as the plants see it: one RGBA8 texel per fire cell (row 0 to
/// the north), red, green and blue the 24 bits of the arrival time in whole
/// seconds ([`NEVER`] until the fire arrives), alpha the [`CLEARED_BIT`].
/// Written whole when a cell changes, not per plant.
struct FireGrid {
    image: Handle<Image>,
    /// What the image holds, to write it only when something changed.
    data: Vec<u8>,
}

/// Arrival time of a cell the fire has not reached.
const NEVER: u32 = 0x00FF_FFFF;
/// Alpha bit of a cell a crew has cleared.
const CLEARED_BIT: u8 = 1;

/// The grid's bytes for the fire's cells.
fn grid_bytes(arrival: &[i32], cleared: &[bool]) -> Vec<u8> {
    let mut data = Vec::with_capacity(arrival.len() * 4);
    for (&t, &c) in arrival.iter().zip(cleared) {
        let t = if t == i32::MIN { NEVER } else { (t.max(0) as u32).min(NEVER - 1) };
        data.extend_from_slice(&[t as u8, (t >> 8) as u8, (t >> 16) as u8, if c { CLEARED_BIT } else { 0 }]);
    }
    data
}

/// Shown state of a plant gone from ground a crew has cleared, or thinned
/// away: bare earth (the ground itself is tinted by `fire_view`). Above the
/// `CellFire` values.
#[cfg_attr(not(test), allow(dead_code))] // the shader's rule (`veg_state.wgsl`), for the tests
const CLEARED: u8 = 10;
/// Share of the plants inside a cleared line that are taken away.
#[cfg_attr(not(test), allow(dead_code))] // the shader's rule (`veg_state.wgsl`), for the tests
const CLEARED_SHARE: f32 = 0.85;

/// The browser draws the whole island as one tile: a few dozen draws, every
/// plant's vertices processed (0.1 M of them, nothing for a GPU); the desktop
/// culls and picks the level of detail per tile of 2 × 2 chunks (1280 m):
/// each tile is one draw per variety and tier, so smaller tiles would be
/// hundreds more draws.
fn tile_chunks() -> usize {
    if cfg!(target_arch = "wasm32") { usize::MAX } else { 2 }
}

/// The plants of the fire cells `(row0, row1, col0, col1)`, end exclusive.
fn scatter_tile(scn: &Scenario, density: f32, (r0, r1, c0, c1): (usize, usize, usize, usize)) -> Lods {
    let mut builder = Lods::default();
    for r in r0..r1 {
        for c in c0..c1 {
            let cell = Cell { row: r, col: c };
            let Some(species) = Species::of_fuel(scn.fuel_at(cell)) else {
                continue;
            };
            let mut rng = Rng::seeded(r as u64 * 65_536 + c as u64);
            let centre = scn.world.centre_of(cell);
            let stand_density = DENSITY[species as usize] * class_density(scn.fuel_at(cell));
            let expected = stand_density * density * patchiness(centre);
            let n = expected.floor() as u32 + u32::from(rng.unit() < expected.fract());
            for _ in 0..n {
                if scatter_plant(scn, cell, species, &mut rng, &mut builder).is_some() {
                    // (the old per-plant lag: drawn still, so the forest
                    // stays where it was)
                    let _ = rng.unit();
                }
            }
        }
    }
    builder
}

pub fn spawn(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FoliageMaterial>>,
    mut images: ResMut<Assets<Image>>,
    gfx: Res<crate::quality::Graphics>,
) {
    let scn = &sim.scn;
    // Half the stand densities below at the top quality (user decision
    // 2026-10-08: fewer trees for now), less on weaker machines.
    let density = gfx.vegetation;
    // a saved adaptive level may start thinned
    KEEP.store(gfx.veg_keep.to_bits(), std::sync::atomic::Ordering::Relaxed);

    let w = scn.world;
    let (rows, cols) = (w.fire_rows, w.fire_cols);
    let data = grid_bytes(sim.fire.arrival_times(), sim.fire.cleared());
    let image = images.add(Image::new(
        Extent3d { width: cols as u32, height: rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data.clone(),
        // plain bytes, not sRGB: decoded to exact integers in the shader
        TextureFormat::Rgba8Unorm,
        // the CPU copy stays: it is written to again
        RenderAssetUsages::default(),
    ));
    let atlas = images.add(materials::foliage_atlas());
    let grid = Vec4::new(w.cellsize, w.height_m, cols as f32, rows as f32);
    // The browser never had the remains (memory): there a burnt plant turns
    // all charcoal.
    let remains = !cfg!(target_arch = "wasm32");
    let mut plant_material = FoliageMaterial::new(image.clone(), atlas.clone(), grid, false);
    plant_material.layer.y = if remains { 1.0 } else { 0.0 };
    let material = materials.add(plant_material);
    let remains_material = remains.then(|| {
        let mut m = FoliageMaterial::new(image.clone(), atlas, grid, true);
        m.layer.y = 1.0;
        materials.add(m)
    });

    let tc = tile_chunks();
    let chunks_y = rows.div_ceil(CHUNK_CELLS);
    let chunks_x = cols.div_ceil(CHUNK_CELLS);
    let tiles_y = chunks_y.div_ceil(tc.min(chunks_y));
    let tiles_x = chunks_x.div_ceil(tc.min(chunks_x));
    let tile_cells = CHUNK_CELLS * tc.min(chunks_y.max(chunks_x));
    let mut variety_meshes: std::collections::HashMap<(&'static str, Tier), Handle<Mesh>> = Default::default();
    let mut tiles = Vec::new();
    let (mut plant_count, mut entity_count, mut instanced_tris) = (0usize, 0usize, 0usize);
    let mut tris_of: std::collections::HashMap<(&'static str, Tier), usize> = Default::default();
    let near_lod = gfx.lod_m > 0.0;

    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let (r0, r1) = (ty * tile_cells, ((ty + 1) * tile_cells).min(rows));
            let (c0, c1) = (tx * tile_cells, ((tx + 1) * tile_cells).min(cols));
            let builder = scatter_tile(scn, density, (r0, r1, c0, c1));
            if builder.inst.is_empty() {
                continue;
            }
            plant_count += builder.inst.len();

            // by variety, in a fixed order
            let mut by_variety: std::collections::BTreeMap<&'static str, Vec<PlantInstance>> = Default::default();
            let mut bounds = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let mut boxes: std::collections::HashMap<&'static str, (Vec3, Vec3)> = Default::default();
            for i in &builder.inst {
                // a generous box: the models stand within about a unit of
                // their foot, a little more in height
                let lo = i.base - Vec3::new(i.scale.x, 0.0, i.scale.z);
                let hi = i.base + Vec3::new(i.scale.x, i.scale.y * 1.3, i.scale.z);
                bounds = (bounds.0.min(lo), bounds.1.max(hi));
                let b = boxes.entry(i.name).or_insert((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)));
                *b = (b.0.min(lo), b.1.max(hi));
                by_variety.entry(i.name).or_default().push(PlantInstance {
                    pos: i.base.to_array(),
                    yaw: i.yaw,
                    scale: i.scale.to_array(),
                    tint: tint_bytes(i),
                });
            }
            let mut tile = Tile {
                full: Vec::new(),
                low: Vec::new(),
                far_low: Vec::new(),
                shadow: Vec::new(),
                char_near: Vec::new(),
                char_far: Vec::new(),
                plants: Vec::new(),
                far: false,
                near: near_lod,
                full_casts: near_lod,
                bounds,
                cells: (r0, r1, c0, c1),
            };
            for (name, list) in by_variety {
                let n = list.len();
                let list = Arc::new(list);
                let (lo, hi) = boxes[name];
                let aabb = Aabb::from_min_max(lo, hi);
                let mut spawn_tier = |tier: Tier, on: bool, shadow_layer: bool, mat: &Handle<FoliageMaterial>| {
                    let mesh = variety_meshes
                        .entry((name, tier))
                        .or_insert_with(|| {
                            let m = variety_mesh(name, tier);
                            tris_of.insert((name, tier), m.indices().map_or(0, |i| i.len() / 3));
                            meshes.add(m)
                        })
                        .clone();
                    let mut e = commands.spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(mat.clone()),
                        Transform::IDENTITY,
                        VegInstances(list.clone()),
                        aabb,
                        NoAutoAabb,
                        NoAutomaticBatching,
                        if on { Visibility::Inherited } else { Visibility::Hidden },
                    ));
                    if shadow_layer {
                        e.insert(bevy::camera::visibility::RenderLayers::layer(crate::terrain_mesh::SHADOW_LAYER));
                    } else {
                        // the light through the leaves is shadowed too
                        e.insert(TransmittedShadowReceiver);
                    }
                    if on && !shadow_layer {
                        instanced_tris += tris_of[&(name, tier)] * n;
                    }
                    e.id()
                };
                if near_lod {
                    tile.full.push(spawn_tier(Tier::Full, true, false, &material));
                }
                tile.low.push(spawn_tier(Tier::Proxy, !near_lod, false, &material));
                // the browser's one tile is never far from the camera
                if tc < usize::MAX {
                    tile.far_low.push(spawn_tier(Tier::Far, false, false, &material));
                }
                if gfx.shadows && near_lod {
                    tile.shadow.push(spawn_tier(Tier::Proxy, false, true, &material));
                }
                tile.plants.push((name, list, aabb));
            }
            entity_count += tile.full.len() + tile.low.len() + tile.far_low.len() + tile.shadow.len();
            tiles.push(tile);
        }
    }

    info!(
        "vegetation: {plant_count} plants in {} tiles as instances of {} variety meshes, {entity_count} entities (no per-plant entity), {:.2} M triangles on show at the start",
        tiles.len(),
        variety_meshes.len(),
        instanced_tris as f32 / 1e6,
    );
    commands.insert_resource(Vegetation {
        material,
        remains_material,
        grid: FireGrid { image, data },
        meshes: variety_meshes,
        tiles,
        lod_m: gfx.lod_m,
        drawn_at: None,
        drawn_cleared: 0,
    });
}

/// Whether `p` is within `margin` metres of a pitch, car park, assembly area,
/// cemetery or square: they are dressed by `town_kit` and the cover raster
/// does not know them, so scrub and trees would grow through the paving.
fn in_open_space(scn: &Scenario, p: Pos, margin: f32) -> bool {
    // Centre, long axis, half sizes, and a bounding radius to reject cheaply.
    static SPACES: std::sync::OnceLock<Vec<([f32; 2], [f32; 2], [f32; 2], f32)>> = std::sync::OnceLock::new();
    let spaces = SPACES.get_or_init(|| {
        scn.vectors.buildings
            .iter()
            .filter(|b| matches!(b.kind.as_deref(), Some("pitch" | "parking" | "assembly" | "cemetery" | "plaza")) && b.ring.len() >= 3)
            .map(|b| {
                let n = b.ring.len() as f32;
                let c = [b.ring.iter().map(|v| v[0]).sum::<f32>() / n, b.ring.iter().map(|v| v[1]).sum::<f32>() / n];
                let e = [b.ring[1][0] - b.ring[0][0], b.ring[1][1] - b.ring[0][1]];
                let l = (e[0] * e[0] + e[1] * e[1]).sqrt().max(1e-3);
                let u = [e[0] / l, e[1] / l];
                let v = [-u[1], u[0]];
                let (mut hu, mut hv) = (0.0f32, 0.0f32);
                for q in &b.ring {
                    hu = hu.max(((q[0] - c[0]) * u[0] + (q[1] - c[1]) * u[1]).abs());
                    hv = hv.max(((q[0] - c[0]) * v[0] + (q[1] - c[1]) * v[1]).abs());
                }
                // A car park is drawn `BUILDING_FOOTPRINT` times its footprint.
                let k = if b.kind.as_deref() == Some("parking") { crate::visual_scale::BUILDING_FOOTPRINT } else { 1.0 };
                (c, u, [hu * k, hv * k], (hu * hu + hv * hv).sqrt() * k + margin)
            })
            .collect()
    });
    spaces.iter().any(|&(c, u, h, r)| {
        let (dx, dy) = (p.x - c[0], p.y - c[1]);
        if dx * dx + dy * dy > r * r {
            return false;
        }
        let a = (dx * u[0] + dy * u[1]).abs();
        let b = (-dx * u[1] + dy * u[0]).abs();
        a < h[0] + margin && b < h[1] + margin
    })
}

/// Distance from `p` to the nearest drawn road or track, less its half width
/// and casing: negative or zero means `p` is under the ribbon. Uses the real
/// polylines (`roads.rs` draws those), not the 5 m cover raster, which misses
/// narrow ways and cuts corners.
fn clearance_to_roads(scn: &Scenario, p: Pos) -> f32 {
    const BIN_M: f32 = 40.0;
    type Seg = ([f32; 2], [f32; 2], f32);
    static INDEX: std::sync::OnceLock<std::collections::HashMap<(i32, i32), Vec<Seg>>> = std::sync::OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let mut m: std::collections::HashMap<(i32, i32), Vec<Seg>> = std::collections::HashMap::new();
        for road in &scn.vectors.roads {
            let half = if road.drivable {
                crate::roads::DRIVABLE_HALF_W
            } else if road.track {
                crate::roads::TRACK_HALF_W
            } else {
                continue;
            } + crate::roads::CASING_M;
            for w in road.line.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (x0, x1) = (a[0].min(b[0]) - 12.0, a[0].max(b[0]) + 12.0);
                let (y0, y1) = (a[1].min(b[1]) - 12.0, a[1].max(b[1]) + 12.0);
                for bx in (x0 / BIN_M).floor() as i32..=(x1 / BIN_M).floor() as i32 {
                    for by in (y0 / BIN_M).floor() as i32..=(y1 / BIN_M).floor() as i32 {
                        m.entry((bx, by)).or_default().push((a, b, half));
                    }
                }
            }
        }
        m
    });
    let key = ((p.x / BIN_M).floor() as i32, (p.y / BIN_M).floor() as i32);
    let mut best = f32::MAX;
    for &(a, b, half) in index.get(&key).into_iter().flatten() {
        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
        let l2 = (ex * ex + ey * ey).max(1e-6);
        let t = (((p.x - a[0]) * ex + (p.y - a[1]) * ey) / l2).clamp(0.0, 1.0);
        let (dx, dy) = (p.x - (a[0] + t * ex), p.y - (a[1] + t * ey));
        best = best.min((dx * dx + dy * dy).sqrt() - half);
    }
    best
}

/// Whether the 5 m land cover leaves room for this plant at `p`: nothing grows
/// on roads, tracks, buildings, yards or irrigated plots, and a tree keeps its
/// crown clear of them too. Without cover the fuel cell alone decides.
fn on_natural_cover(scn: &Scenario, species: Species, p: Pos) -> bool {
    if in_open_space(scn, p, 4.0) {
        return false;
    }
    // Keep the whole plant clear of the drawn ribbon; grass only the ribbon.
    let crown = match species {
        Species::Shrub => 5.0,
        Species::Conifer | Species::Broadleaf => 6.0,
        Species::Grass => 0.5,
    };
    if clearance_to_roads(scn, p) < crown {
        return false;
    }
    let Some(cover) = &scn.cover else {
        return true;
    };
    let reach = match species {
        Species::Conifer | Species::Broadleaf => 3.0,
        Species::Shrub => 1.5,
        Species::Grass => 0.0,
    };
    if !([(0.0, 0.0), (reach, 0.0), (-reach, 0.0), (0.0, reach), (0.0, -reach)]
        .into_iter()
        .all(|(dx, dy)| cover.is_natural(Pos { x: p.x + dx, y: p.y + dy })))
    {
        return false;
    }
    // The cover raster paints a road a lane wide; `roads.rs` draws the ribbon
    // and its casing far wider, and a shrub crown reaches several metres. Keep
    // the whole plant clear of the drawn ribbon.
    let crown = match species {
        Species::Shrub => 5.0,
        Species::Conifer | Species::Broadleaf => 2.0,
        Species::Grass => return true,
    };
    let drawn = |q: Pos| match cover.at(q) {
        CoverClass::Road => crate::roads::DRIVABLE_HALF_W + crate::roads::CASING_M,
        CoverClass::Track => crate::roads::TRACK_HALF_W + crate::roads::CASING_M,
        _ => 0.0,
    };
    let max_r = crate::roads::DRIVABLE_HALF_W + crate::roads::CASING_M + crown;
    let mut r = 2.0;
    while r <= max_r {
        for i in 0..12 {
            let a = i as f32 * std::f32::consts::TAU / 12.0;
            let half = drawn(Pos { x: p.x + r * a.cos(), y: p.y + r * a.sin() });
            // a road cell at distance r means the ribbon centre is within
            // about r + a cell; the plant must sit beyond half + crown of it
            if half > 0.0 && r < half + crown {
                return false;
            }
        }
        r += 2.0;
    }
    true
}

/// Share of the plants kept near a road or track: the wood thins out
/// toward the verge, so the roads read against it. Looks only; the fuel
/// cells, and so the fire, are untouched.
fn road_side_keep(scn: &Scenario, p: Pos) -> f32 {
    const RADII_M: [f32; 4] = [6.0, 12.0, 18.0, 26.0];
    const EDGE_KEEP: f32 = 0.2;
    let Some(cover) = &scn.cover else {
        return 1.0;
    };
    let is_road = |q: Pos| matches!(cover.at(q), CoverClass::Road | CoverClass::Track);
    for r in RADII_M {
        let hit = (0..8).any(|i| {
            let a = i as f32 * std::f32::consts::FRAC_PI_4;
            is_road(Pos { x: p.x + r * a.cos(), y: p.y + r * a.sin() })
        });
        if hit {
            return EDGE_KEEP + (1.0 - EDGE_KEEP) * (r - RADII_M[0]) / (RADII_M[3] - RADII_M[0] + 6.0);
        }
    }
    1.0
}

/// Place one plant somewhere inside `cell` and emit its geometry, returning
/// where it ended up.
fn scatter_plant(
    scn: &Scenario,
    cell: Cell,
    species: Species,
    rng: &mut Rng,
    out: &mut Lods,
) -> Option<Pos> {
    let centre = scn.world.centre_of(cell);
    let half = scn.world.cellsize * 0.5;
    let p = Pos {
        x: centre.x + (rng.unit() * 2.0 - 1.0) * half,
        y: centre.y + (rng.unit() * 2.0 - 1.0) * half,
    };
    if !scn.world.contains(p) || !on_natural_cover(scn, species, p) {
        return None;
    }
    if species != Species::Grass && rng.unit() > road_side_keep(scn, p) {
        return None;
    }
    let ground = scn.terrain.height_at(p);
    let base = Vec3::new(p.x, ground, -p.y);

    // A stand of one species is not a stand of clones: size, tint and yaw all
    // jitter, which is most of what stops merged geometry looking stamped.
    let scale = (0.75 + rng.unit() * 0.5) * match species {
        Species::Conifer | Species::Broadleaf => crate::visual_scale::TREE,
        Species::Shrub => crate::visual_scale::SHRUB,
        Species::Grass => 1.0,
    };
    let yaw = rng.unit() * std::f32::consts::TAU;

    // Where this plant sits between the dry and vigorous ends of its species,
    // correlated over ~40 m so drying runs in patches — slopes and aspects
    // cure together — with a per-plant scatter on top.
    let (dry, green) = Species::foliage_range(scn.fuel_at(cell));
    let local = noise(p.x / 40.0, p.y / 40.0, 0xC0FF);
    let t = (0.72 * local + 0.28 * rng.unit()).clamp(0.0, 1.0);
    let foliage = [
        dry[0] + (green[0] - dry[0]) * t,
        dry[1] + (green[1] - dry[1]) * t,
        dry[2] + (green[2] - dry[2]) * t,
    ];
    let shade = 0.88 + rng.unit() * 0.22;
    let foliage = mul(foliage, shade);
    // The textured models take their colour from the atlas: the vertex
    // carries the plant's tint against its fuel's mean colour (linear), so a
    // dry stand still cures in patches while a holm oak stays a holm oak.
    let mean = [(dry[0] + green[0]) * 0.5, (dry[1] + green[1]) * 0.5, (dry[2] + green[2]) * 0.5];
    let leaf_tint = ratio(foliage, mean);
    let wood_tint = [lin1(shade); 3];

    // The species of a group share its outline (umbrella and column
    // conifers, round broadleaves, low macchia clumps): the player reads the
    // fuel from the silhouette, the species from the colour and the detail.
    let name = species_for(scn.fuel_at(cell), p);
    match species {
        Species::Conifer => conifer(out, name, base, scale, yaw, leaf_tint, wood_tint, rng),
        Species::Broadleaf => broadleaf(out, name, base, scale, yaw, leaf_tint, wood_tint, rng),
        Species::Shrub => shrub(out, name, base, scale, yaw, leaf_tint, wood_tint, rng),
        Species::Grass if true && rng.unit() < 0.025 => {
            // Lone field trees break the meadow silhouette without changing
            // its simulation fuel; they share the same interpolated burn field.
            let height = (7.0 + rng.unit() * 5.0) * scale * crate::visual_scale::TREE;
            out.model("olive", base, Vec3::splat(height), yaw, wood_tint, wood_tint);
        }
        // Meadow tufts are drawn by the terrain shader (`shaders/terrain.wgsl`).
        Species::Grass => return None,
    }
    Some(p)
}

/// Which species a plant of fuel class `fuel` at `p` is (`models::SPECIES`).
/// Species come in patches of a few tens of metres (a holm oak grove, a
/// cistus clearing) with single plants mixed in, and every class keeps to
/// its group's outline: what is drawn is what burns.
fn species_for(fuel: i32, p: Pos) -> &'static str {
    let table: &[(&'static str, f32)] = match fuel {
        10 => &[("pine", 0.55), ("aleppo", 0.35), ("cypress", 0.10)],
        11 => &[("aleppo", 0.60), ("pine", 0.30), ("cypress", 0.10)],
        12 => &[("pine", 0.50), ("aleppo", 0.40), ("cypress", 0.10)],
        4 => &[("holm", 0.50), ("cork", 0.30), ("carob", 0.20)],
        5 => &[("holm", 0.45), ("carob", 0.30), ("cork", 0.25)],
        6 => &[("holm", 0.40), ("olive", 0.35), ("carob", 0.25)],
        7 => &[("cistus", 0.35), ("rosemary", 0.25), ("heather", 0.20), ("broom", 0.20)],
        8 => &[("lentisk", 0.35), ("myrtle", 0.20), ("cistus", 0.15), ("broom", 0.15), ("heather", 0.15)],
        9 => &[("lentisk", 0.35), ("arbutus", 0.25), ("heather", 0.20), ("myrtle", 0.20)],
        _ => &[("olive", 1.0)],
    };
    let h = p.x.to_bits().wrapping_mul(0x2C1B_3C6D) ^ p.y.to_bits().wrapping_mul(0x297A_2D39);
    let jitter = ((h ^ (h >> 13)).wrapping_mul(0x85EB_CA6B) >> 8) as f32 / (1u32 << 24) as f32;
    let u = (noise(p.x / 30.0, p.y / 30.0, 0x5EED) * 2.7 + jitter * 0.35).fract();
    let mut acc = 0.0;
    for &(name, w) in table {
        acc += w;
        if u < acc {
            return name;
        }
    }
    table[table.len() - 1].0
}

/// sRGB channel to linear.
fn lin1(c: f32) -> f32 {
    Color::srgb(c, c, c).to_linear().red
}

/// `c / reference`, channel by channel, in linear light.
fn ratio(c: [f32; 3], reference: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| lin1(c[i]) / lin1(reference[i]).max(1e-4))
}

fn leaf(is_leaf: bool) -> f32 {
    if is_leaf { 1.0 } else { 0.0 }
}

fn mul(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

// --- archetypes ------------------------------------------------------------
//
// Blender archetypes retain chunk batching, species palettes and burn ranges.
// Normalized trees scale to the existing ecological height distributions.
// `name` is the species (`species_for`); its models carry its proportions
// (a cypress is narrow, a carob low and wide), the sizes below the group's.
#[allow(clippy::too_many_arguments)]
fn conifer(out: &mut Lods, name: &'static str, base: Vec3, scale: f32, yaw: f32,
    leaf: [f32; 3], wood: [f32; 3], rng: &mut Rng) {
    // tall, the crown a little wider than the model
    let height = (13.0 + rng.unit() * 8.0) * scale;
    let width = height * (1.0 + rng.unit() * 0.3);
    out.model(name, base, Vec3::new(width, height, width), yaw, leaf, wood);
}

#[allow(clippy::too_many_arguments)]
fn broadleaf(out: &mut Lods, name: &'static str, base: Vec3, scale: f32, yaw: f32,
    leaf: [f32; 3], wood: [f32; 3], rng: &mut Rng) {
    // lower than the pines, the crown about as wide as tall
    let height = (8.0 + rng.unit() * 5.0) * scale;
    out.model(name, base, Vec3::new(height * 1.1, height, height * 1.1), yaw, leaf, wood);
}

#[allow(clippy::too_many_arguments)]
fn shrub(out: &mut Lods, name: &'static str, base: Vec3, scale: f32, yaw: f32,
    leaf: [f32; 3], wood: [f32; 3], rng: &mut Rng) {
    // low and wide: macchia must not read as young trees
    let r = (2.6 + rng.unit() * 2.2) * scale;
    out.model(name, base, Vec3::new(r, r * 0.8, r), yaw, leaf, wood);
}


// --- mesh building ---------------------------------------------------------

/// The plants of a tile as the scatter places them.
#[derive(Default)]
struct Lods {
    inst: Vec<Inst>,
}

/// A plant: its variety, where and how it stands, its tint.
#[derive(Clone)]
struct Inst {
    name: &'static str,
    base: Vec3,
    scale: Vec3,
    yaw: f32,
    /// Linear multipliers of the atlas colours (1 = as painted).
    leaf: [f32; 3],
    wood: f32,
}

impl Lods {
    /// `species` is one of `models::SPECIES`; which of its varieties this
    /// plant is follows from its yaw, so it is the same on every run.
    fn model(&mut self, species: &'static str, base: Vec3, scale: Vec3, yaw: f32,
        foliage: [f32; 3], wood: [f32; 3]) {
        let name = crate::models::pick_variety(species, yaw / std::f32::consts::TAU);
        self.inst.push(Inst { name, base, scale, yaw, leaf: foliage, wood: wood[1] });
    }
}

/// A plant's tint as the instance carries it: leaf red, green, blue and the
/// wood, each multiplier / 2 as a byte (the shader doubles them).
fn tint_bytes(i: &Inst) -> [u8; 4] {
    let b = |t: f32| ((t * 0.5).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [b(i.leaf[0]), b(i.leaf[1]), b(i.leaf[2]), b(i.wood)]
}

/// One variety's mesh for a tier, at the origin, unrotated, untinted (the
/// instances place and tint it), only on the GPU.
pub(crate) fn variety_mesh(name: &'static str, tier: Tier) -> Mesh {
    let mut b = Builder::default();
    let remains = |low| Remains { name, base: Vec3::ZERO, scale: Vec3::ONE, yaw: 0.0 }.build(low);
    let mut mesh = match tier {
        Tier::Full => {
            b.model(crate::models::live(name), Vec3::ZERO, Vec3::ONE, 0.0, [1.0; 3], [1.0; 3]);
            b.finish()
        }
        Tier::Proxy => {
            b.proxy(name, Vec3::ZERO, Vec3::ONE, 0.0, [1.0; 3], [1.0; 3]);
            b.finish()
        }
        Tier::Far => {
            b.want_far = true;
            b.proxy(name, Vec3::ZERO, Vec3::ONE, 0.0, [1.0; 3], [1.0; 3]);
            let (mesh, far) = b.finish_with_far();
            compact(&mesh, &far)
        }
        Tier::RemainsNear => remains(false),
        Tier::RemainsFar => remains(true),
    };
    mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
    mesh.compress_indices();
    pack(&mut mesh);
    mesh
}

impl Remains {
    fn build(&self, low: bool) -> Mesh {
        let mut b = Builder::default();
        b.remains(self, low);
        b.finish()
    }
}

/// What a distant plant keeps of its model: the crown's bounding box, its
/// tone low and high, and the trunk below it.
struct Proxy {
    crown: (Vec3, Vec3),
    shade_low: f32,
    shade_high: f32,
    /// Trunk radius, when the model has bare wood under the crown.
    trunk: Option<f32>,
    /// Atlas rects (u0, v0, u1, v1) of the crown and the trunk.
    canopy: [f32; 4],
    bark: [f32; 4],
}

fn proxy(name: &str) -> &'static Proxy {
    static PROXIES: std::sync::OnceLock<std::collections::HashMap<&'static str, Proxy>> =
        std::sync::OnceLock::new();
    &PROXIES.get_or_init(|| {
        crate::models::SPECIES
            .into_iter()
            .flat_map(|species| crate::models::variety_names(species).iter().copied())
            .map(|name| {
                let m = crate::models::live(name);
                let leaves = || (0..m.positions.len()).filter(|&i| !m.wood[i]);
                let (lo, hi) = leaves().fold(
                    (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                    |(lo, hi), i| (lo.min(Vec3::from(m.positions[i])), hi.max(Vec3::from(m.positions[i]))),
                );
                let mid = (lo.y + hi.y) * 0.5;
                // the same tone `Builder::model` gives each leaf vertex
                let mean = |upper: bool| {
                    let (sum, n) = leaves()
                        .filter(|&i| (m.positions[i][1] > mid) == upper)
                        .fold((0.0, 0), |(s, n), i| (s + m.colors[i][1], n + 1));
                    if n == 0 { 1.0 } else { sum / n as f32 }
                };
                let bare: Vec<usize> = (0..m.positions.len())
                    .filter(|&i| m.wood[i] && m.positions[i][1] < lo.y)
                    .collect();
                let trunk = (!bare.is_empty()).then(|| {
                    let r = bare.iter()
                        .map(|&i| Vec2::new(m.positions[i][0], m.positions[i][2]).length())
                        .fold(0.0, f32::max);
                    r.max(0.01)
                });
                let v = crate::models::variety(name);
                (name, Proxy { crown: (lo, hi), shade_low: mean(false), shade_high: mean(true), trunk, canopy: v.canopy, bark: v.bark })
            })
            .collect()
    })[name]
}

/// Uvs into the foliage atlas (`models::atlas`); what has no texture points
/// at its flat grey. On the GPU path colours and uvs go up as 16-bit
/// integers ([`pack`]), so a vertex costs less than it did before the uvs.
#[derive(Default)]
struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    /// The vertex keeps the normal it was given (the authored models);
    /// otherwise `finish` averages the faces around it.
    fixed: Vec<bool>,
    indices: Vec<u32>,
    /// The same triangles for the farthest tier (see [`FAR_TIER`]): only a
    /// `want_far` builder keeps them, and only up to `far_synced` of
    /// `indices` is mirrored in it.
    want_far: bool,
    far: Vec<u32>,
    far_synced: usize,
}

impl Builder {
    /// Copy what was added to `indices` since the last call into `far`
    /// unchanged: the shapes that have no cheaper version.
    fn sync_far(&mut self) {
        if self.want_far {
            self.far.extend_from_slice(&self.indices[self.far_synced..]);
        }
        self.far_synced = self.indices.len();
    }


    fn model(&mut self, model: &crate::models::Model, base: Vec3, scale: Vec3, yaw: f32,
        foliage: [f32; 3], wood: [f32; 3]) {
        let start = self.positions.len() as u32;
        let rotation = Quat::from_rotation_y(yaw);
        if model.uvs.is_empty() {
            // a single model of `meshes.json`: absolute colours, no texture
            for (i, p) in model.positions.iter().enumerate() {
                let color = if model.wood[i] { wood } else { foliage };
                let shade = if model.wood[i] { 1.0 } else { model.colors[i][1] / 0.38 };
                self.vertex_a(base + rotation * (Vec3::from(*p) * scale), mul(color, shade), leaf(!model.wood[i]));
            }
        } else {
            // the atlas paints it: the vertex is the tint (linear, halved, as
            // the shader doubles it) and the crown's authored shading
            let inv = Vec3::ONE / scale.max(Vec3::splat(1e-6));
            for (i, p) in model.positions.iter().enumerate() {
                let tint = if model.wood[i] { wood } else { foliage };
                let shade = if model.wood[i] { 1.0 } else { model.colors[i][1].powf(2.2) };
                let n = (rotation * (Vec3::from(model.normals[i]) * inv)).normalize_or_zero();
                self.vertex_lin(
                    base + rotation * (Vec3::from(*p) * scale),
                    [0, 1, 2].map(|c| (tint[c] * shade * 0.5).min(1.0)),
                    leaf(!model.wood[i]),
                    model.uvs[i],
                    Some(n),
                );
            }
        }
        self.indices.extend(model.indices.iter().map(|i| start + i));
    }

    /// What fire adds to a plant (see [`CharLayer`]), in absolute colours: its
    /// twig skeleton and the debris at its foot. Debris vertices carry alpha 1
    /// (the shader shows them last, see `draw_char`).
    fn remains(&mut self, r: &Remains, low: bool) {
        // how dark: fixed by the plant's own random yaw, so the same plants
        // have the same remains on every run
        let u = r.yaw / std::f32::consts::TAU;
        let shade = 0.85 + 0.3 * (u * 97.0).fract();
        let v = crate::models::variety(r.name);
        let kit = if low { &v.low } else { &v.remains };
        let start = self.positions.len() as u32;
        let rotation = Quat::from_rotation_y(r.yaw);
        for ((p, c), &debris) in kit.positions.iter().zip(&kit.colors).zip(&kit.debris) {
            self.vertex_a(r.base + rotation * (Vec3::from(*p) * r.scale), mul(*c, shade), leaf(debris));
        }
        self.indices.extend(kit.indices.iter().map(|i| start + i));
    }

    /// The distant stand-in for [`Builder::model`]: a six-sided crown (a
    /// bottom point, two rings, a top point) over the model's leaves, and a
    /// three-sided trunk where the model shows bare wood. 18–30 triangles
    /// instead of 56–102. The crown wears the species' canopy tile, mirrored
    /// on alternate faces (six is even: no seam), the trunk its bark; the
    /// tints are as in [`Builder::model`].
    fn proxy(&mut self, name: &str, base: Vec3, scale: Vec3, yaw: f32,
        foliage: [f32; 3], wood: [f32; 3]) {
        const SIDES: u32 = 6;
        self.sync_far();
        let p = proxy(name);
        // linear, halved (the shader doubles it), and a few percent darker:
        // the authored crown shades itself in its lobes
        let foliage = foliage.map(|c| c * 0.5 * 0.9);
        let wood = wood.map(|c| (c * 0.5).min(1.0));
        let shade = |k: f32| foliage.map(|c| (c * k.powf(2.2)).min(1.0));
        let [cu0, cv0, cu1, cv1] = p.canopy;
        let [bu0, bv0, bu1, bv1] = p.bark;
        let rotation = Quat::from_rotation_y(yaw);
        let at = |v: Vec3| base + rotation * (v * scale);
        let (lo, hi) = p.crown;
        let (centre, radius) = ((lo + hi) * 0.5, (hi - lo) * 0.5);

        if let Some(r) = p.trunk {
            let start = self.positions.len() as u32;
            let top_v = bv0 + (bv1 - bv0) * (lo.y + radius.y * 0.4).min(1.0);
            for i in 0..3 {
                let a = i as f32 * std::f32::consts::TAU / 3.0;
                let d = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                let u = if i == 1 { bu1 } else { bu0 };
                self.vertex_lin(at(Vec3::new(centre.x, 0.0, centre.z) + d), wood, 0.0, [u, bv0], None);
                self.vertex_lin(at(Vec3::new(centre.x, lo.y + radius.y * 0.4, centre.z) + d), wood, 0.0, [u, top_v], None);
            }
            for i in 0..3 {
                let (b0, b1) = (start + 2 * i, start + 2 * ((i + 1) % 3));
                self.indices.extend_from_slice(&[b0, b0 + 1, b1, b1, b0 + 1, b1 + 1]);
            }
            if self.want_far {
                // the trunk stays: it is what darkens the ground under a crown
                let n = self.indices.len();
                let trunk: Vec<u32> = self.indices[n - 18..].to_vec();
                self.far.extend(trunk);
            }
        }

        let y = |t: f32| lo.y + (hi.y - lo.y) * t;
        let cv = |t: f32| cv1 + (cv0 - cv1) * t;
        let mid_u = (cu0 + cu1) * 0.5;
        let side_u = |i: u32| if i % 2 == 0 { cu0 } else { cu1 };
        let bottom = self.vertex_lin(at(Vec3::new(centre.x, y(0.04), centre.z)), shade(p.shade_low * 0.8), 1.0, [mid_u, cv(0.0)], None);
        let ring = self.positions.len() as u32;
        for (t, k, sh) in [(0.3, 0.85, p.shade_low), (0.62, 1.0, p.shade_high)] {
            for i in 0..SIDES {
                let a = (i as f32 + 0.5 * t) * std::f32::consts::TAU / SIDES as f32;
                let v = Vec3::new(centre.x + a.cos() * radius.x * k, y(t), centre.z + a.sin() * radius.z * k);
                self.vertex_lin(at(v), shade(sh), 1.0, [side_u(i), cv(t)], None);
            }
        }
        let top = self.vertex_lin(at(Vec3::new(centre.x, hi.y, centre.z)), shade(p.shade_high * 1.05), 1.0, [mid_u, cv(1.0)], None);
        // The farthest tier is a bipyramid on a ring of its own (vertices the
        // nearer tier never indexes), as wide as the crown is at its widest:
        // the near rings are too far apart in height to drop either of them.
        let far_ring = self.positions.len() as u32;
        if self.want_far {
            // a little lighter than the near rings: the far faces are flatter
            // to the sun, and the crown would otherwise read darker
            let sh = shade(p.shade_high * 1.08);
            for i in 0..SIDES {
                let a = i as f32 * std::f32::consts::TAU / SIDES as f32;
                let v = Vec3::new(centre.x + a.cos() * radius.x * 0.97, y(0.45), centre.z + a.sin() * radius.z * 0.97);
                self.vertex_lin(at(v), sh, 1.0, [side_u(i), cv(0.45)], None);
            }
            for i in 0..SIDES {
                let (a, b) = (far_ring + i, far_ring + (i + 1) % SIDES);
                self.far.extend_from_slice(&[bottom, a, b, a, top, b]);
            }
        }
        for i in 0..SIDES {
            let (a, b) = (ring + i, ring + (i + 1) % SIDES);
            let (c, d) = (a + SIDES, b + SIDES);
            // counter-clockwise from outside, so the face normals `finish`
            // averages point out of the crown
            self.indices.extend_from_slice(&[bottom, a, b, a, c, b, c, d, b, c, top, d]);
        }
        self.far_synced = self.indices.len();
    }


    /// `leaf` goes in the colour's alpha (the material is opaque, so nothing
    /// else reads it): 1 for living foliage, which the shader textures; 0 for
    /// wood, rock, grass and everything fire has touched (see `paint`).
    fn vertex_a(&mut self, p: Vec3, c: [f32; 3], leaf: f32) -> u32 {
        // Vertex colours are linear in Bevy. Ecological palettes above are
        // sRGB; without conversion dark foliage washes out to pastel mint.
        let linear = Color::srgb(c[0], c[1], c[2]).to_linear();
        self.vertex_lin(p, [linear.red, linear.green, linear.blue], leaf, crate::models::atlas().grey, None)
    }

    /// A vertex with a linear colour, a uv and, if given, its own normal.
    fn vertex_lin(&mut self, p: Vec3, c: [f32; 3], leaf: f32, uv: [f32; 2], normal: Option<Vec3>) -> u32 {
        let i = self.positions.len() as u32;
        self.positions.push([p.x, p.y, p.z]);
        self.normals.push(normal.map_or([0.0, 1.0, 0.0], |n| n.to_array()));
        self.fixed.push(normal.is_some());
        self.colors.push([c[0], c[1], c[2], leaf]);
        self.uvs.push(uv);
        i
    }



    /// Area-weighted vertex normals from the faces, then the mesh.
    fn finish(self) -> Mesh {
        self.finish_with_far().0
    }

    /// The mesh and, for a `want_far` builder, the farthest tier's index list
    /// over the same vertices.
    fn finish_with_far(mut self) -> (Mesh, Vec<u32>) {
        self.sync_far();
        let given = std::mem::take(&mut self.normals);
        self.normals = vec![[0.0; 3]; given.len()];
        for tri in self.indices.chunks_exact(3) {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let (pa, pb, pc) = (
                Vec3::from(self.positions[a]),
                Vec3::from(self.positions[b]),
                Vec3::from(self.positions[c]),
            );
            let face = (pb - pa).cross(pc - pa);
            for i in [a, b, c] {
                self.normals[i] = (Vec3::from(self.normals[i]) + face).into();
            }
        }
        // vertices only the far tier uses take their normals from its faces
        let near_only = self.normals.clone();
        for tri in self.far.chunks_exact(3) {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let (pa, pb, pc) = (
                Vec3::from(self.positions[a]),
                Vec3::from(self.positions[b]),
                Vec3::from(self.positions[c]),
            );
            let face = (pb - pa).cross(pc - pa);
            for i in [a, b, c] {
                if near_only[i] == [0.0, 0.0, 0.0] {
                    self.normals[i] = (Vec3::from(self.normals[i]) + face).into();
                }
            }
        }
        for (i, n) in self.normals.iter_mut().enumerate() {
            *n = if self.fixed[i] { given[i] } else { Vec3::from(*n).normalize_or_zero().into() };
        }

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        mesh.insert_indices(Indices::U32(self.indices));
        (mesh, self.far)
    }
}

// --- burning ---------------------------------------------------------------

/// Hand the fire to the plants: the fire grid (arrival time and cleared flag
/// per cell, [`FireGrid`]) is written when a cell changes, at most every
/// [`BURN_REDRAW_S`] of real time; each plant works out the rest in the vertex
/// stage. A tile the fire first reaches gets its burnt remains (desktop).
pub fn burn(
    mut commands: Commands,
    sim: Res<Sim>,
    time: Res<Time>,
    mut veg: ResMut<Vegetation>,
    mut images: ResMut<Assets<Image>>,
    mut restarted: MessageReader<crate::sim::SimRestarted>,
    mut since: Local<f32>,
) {
    // `?brucia=0` in the browser: the plants never burn on screen (a switch
    // to find what kills an iPhone's page)
    if crate::quality::web_off("brucia") {
        return;
    }
    let restarted = restarted.read().count() > 0;
    *since += time.delta_secs();
    if !restarted && *since < BURN_REDRAW_S {
        return;
    }
    let now = sim.time_s() as f32;
    let line_cells = sim.fire.line_cells;
    if veg.drawn_at == Some(now) && veg.drawn_cleared == line_cells && !restarted {
        return;
    }
    *since = 0.0;
    veg.drawn_at = Some(now);
    veg.drawn_cleared = line_cells;
    let arrival = sim.fire.arrival_times();
    let data = grid_bytes(arrival, sim.fire.cleared());
    if data != veg.grid.data {
        if let Some(mut image) = images.get_mut(&veg.grid.image) {
            image.data = Some(data.clone());
        }
        veg.grid.data = data;
    }
    // the remains, where the fire has reached: they stand hidden until their
    // plant catches, and stay for good (a restart hides them again)
    let Some(remains_material) = veg.remains_material.clone() else { return };
    let cols = sim.scn.world.fire_cols;
    let veg = &mut *veg;
    for tile in &mut veg.tiles {
        if !tile.char_near.is_empty() || tile.plants.is_empty() {
            continue;
        }
        let (r0, r1, c0, c1) = tile.cells;
        if !(r0..r1).any(|r| arrival[r * cols + c0..r * cols + c1].iter().any(|&t| t != i32::MIN)) {
            continue;
        }
        for (name, list, aabb) in &tile.plants {
            for (tier, on) in [(Tier::RemainsNear, tile.near), (Tier::RemainsFar, !tile.near)] {
                let Some(mesh) = veg.meshes.get(&(*name, tier)) else { continue };
                let e = commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(remains_material.clone()),
                        Transform::IDENTITY,
                        VegInstances(list.clone()),
                        *aabb,
                        NoAutoAabb,
                        NoAutomaticBatching,
                        if on { Visibility::Inherited } else { Visibility::Hidden },
                    ))
                    .id();
                if on && !tile.full_casts {
                    commands.entity(e).insert(NotShadowCaster);
                }
                if tier == Tier::RemainsNear { tile.char_near.push(e) } else { tile.char_far.push(e) }
            }
        }
    }
}

/// The vertices of `mesh` that `indices` uses, every attribute with them, in
/// their order (a plant's vertex range stays a range), under `indices`
/// renumbered: the proxies' farthest tier as a mesh of its own.
fn compact(mesh: &Mesh, indices: &[u32]) -> Mesh {
    use bevy::mesh::VertexAttributeValues as V;
    let mut map = vec![u32::MAX; mesh.count_vertices()];
    for &i in indices {
        map[i as usize] = 0;
    }
    let mut kept = Vec::new();
    for (i, m) in map.iter_mut().enumerate() {
        if *m == 0 {
            *m = kept.len() as u32;
            kept.push(i);
        }
    }
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    for (attribute, values) in mesh.attributes() {
        let values = match values {
            V::Float32(v) => V::Float32(kept.iter().map(|&i| v[i]).collect()),
            V::Float32x2(v) => V::Float32x2(kept.iter().map(|&i| v[i]).collect()),
            V::Float32x3(v) => V::Float32x3(kept.iter().map(|&i| v[i]).collect()),
            V::Float32x4(v) => V::Float32x4(kept.iter().map(|&i| v[i]).collect()),
            V::Uint32(v) => V::Uint32(kept.iter().map(|&i| v[i]).collect()),
            _ => panic!("vegetation: proxy attribute {} of an unexpected format", attribute.name),
        };
        out.insert_attribute(attribute.clone(), values);
    }
    out.insert_indices(Indices::U32(indices.iter().map(|&i| map[i as usize]).collect()));
    out
}

/// Colours and uvs as 16-bit unsigned normalised integers, 8 + 4 bytes a
/// vertex instead of 16 + 8: the uvs cost nothing over the meshes before them
/// (2026-10-31). GPU path only: the CPU path rewrites the colours as floats.
/// Every colour the plants carry is in 0..1 (tints are halved).
fn pack(mesh: &mut Mesh) {
    use bevy::mesh::AttributeQuantization::Unorm16;
    if let Err(e) = mesh.quantize_colors(Unorm16) {
        warn!("vegetation: colours not packed: {e:?}");
    }
    if let Err(e) = mesh.quantize_float32_attribute(Mesh::ATTRIBUTE_UV_0, Unorm16) {
        warn!("vegetation: uvs not packed: {e:?}");
    }
}

/// Real seconds between two looks at the fire (see [`burn`]).
const BURN_REDRAW_S: f32 = 0.15;

/// Hand the simulated clock and the share of plants kept to the vegetation's
/// materials (`fire.y`, `fire.z` the flaming time, `fire.w` the share): every
/// plant's look is a function of them and of the fire grid, so the fire keeps
/// eating the canopy between grid updates and stands still while the world
/// is paused.
pub fn fire_clock(sim: Res<Sim>, veg: Res<Vegetation>, mut materials: ResMut<Assets<FoliageMaterial>>) {
    let now = sim.time_s() as f32;
    let want = Vec3::new(now, PLANT_FLAMING_S, keep());
    for handle in std::iter::once(&veg.material).chain(veg.remains_material.as_ref()) {
        let Some(m) = materials.get(handle) else { continue };
        if m.fire.yzw() == want {
            continue;
        }
        if let Some(mut m) = materials.get_mut(handle) {
            (m.fire.y, m.fire.z, m.fire.w) = (want.x, want.y, want.z);
        }
    }
}

/// What a plant at `p` shows at `now`, as the vertex stage decides it
/// (`look` in `veg_state.wgsl`) from the fire grid: the reference for the
/// shader, for the tests. Returns the look and the plant's ignition instant.
#[cfg_attr(not(test), allow(dead_code))] // the shader's rule (`veg_state.wgsl`), for the tests
fn plant_look(field: &FireField, p: Pos, now: f32, remains: bool, keep: f32) -> (u8, f32) {
    let lag = plant_lag(p);
    let mut look = CellFire::Unburnt as u8;
    let mut t0 = 0.0;
    if let Some(arrival) = field.arrival(p) {
        t0 = arrival + lag;
        let age = now - t0;
        if age >= PLANT_FLAMING_S {
            look = if remains { BURNT_REMAINS } else { CellFire::Burnt as u8 };
        } else if age >= 0.0 {
            look = CellFire::Burning as u8;
        }
    }
    let burnt = look == CellFire::Burnt as u8 || look == BURNT_REMAINS;
    if !burnt && plant_rank(p) >= keep {
        return (CLEARED, t0);
    }
    let inside = ((field.cleared_fraction(p) - 0.45) / 0.15).clamp(0.0, 1.0);
    if look == CellFire::Unburnt as u8 && lag / LAG_S < CLEARED_SHARE * inside {
        return (CLEARED, t0);
    }
    (look, t0)
}

/// Look of a burnt plant that keeps its wood under its remains.
#[cfg_attr(not(test), allow(dead_code))] // the shader's rule (`veg_state.wgsl`), for the tests
const BURNT_REMAINS: u8 = 11;

impl Vegetation {
    /// Show only this share of the plants built, whatever the fire does to
    /// them (a hash per plant against the share, in the shader): a weak
    /// machine sheds fill rate without a rebuild.
    pub fn set_keep(&mut self, k: f32) {
        KEEP.store(k.clamp(0.0, 1.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_lod_m(&mut self, m: f32) {
        if self.lod_m != m {
            self.lod_m = m;
        }
    }
}

/// Show each tile's full models near the camera and its proxies beyond
/// [`Vegetation::lod_m`], with a 10 % band so a tile on the edge does not
/// flicker while the camera drifts. Only which entities are on show changes.
pub fn lod(
    mut commands: Commands,
    mut veg: ResMut<Vegetation>,
    camera: Query<&Transform, (With<Camera3d>, Without<crate::camera::AuxiliaryCamera>)>,
    mut visibility: Query<&mut Visibility>,
) {
    let Ok(eye) = camera.single().map(|t: &Transform| t.translation) else { return };
    let lod_m = veg.lod_m;
    let far_m = (lod_m * FAR_TIER).max(FAR_TIER_MIN_M);
    for tile in &mut veg.tiles {
        let (lo, hi) = tile.bounds;
        let d = eye.distance(eye.clamp(lo, hi));
        let near = !tile.full.is_empty() && if tile.near { d < lod_m * 1.1 } else { d < lod_m };
        // the proxies' own two tiers
        let far = !tile.far_low.is_empty() && !near && if tile.far { d > far_m / 1.1 } else { d > far_m };
        let full_casts = near && if tile.full_casts { d < FULL_SHADOW_M * 1.1 } else { d < FULL_SHADOW_M };
        if near == tile.near && far == tile.far && full_casts == tile.full_casts {
            continue;
        }
        (tile.near, tile.far, tile.full_casts) = (near, far, full_casts);
        // the full models draw the shadow only close up; beyond that the
        // proxies do, from the sun's layer
        for &e in tile.full.iter().chain(&tile.char_near) {
            if full_casts { commands.entity(e).remove::<NotShadowCaster>(); }
            else { commands.entity(e).insert(NotShadowCaster); }
        }
        for (list, on) in [
            (&tile.full, near),
            (&tile.low, !near && !far),
            (&tile.far_low, !near && far),
            (&tile.char_near, near),
            (&tile.char_far, !near),
            (&tile.shadow, near && !full_casts),
        ] {
            for &e in list {
                if let Ok(mut v) = visibility.get_mut(e) {
                    *v = if on { Visibility::Inherited } else { Visibility::Hidden };
                }
            }
        }
    }
}

/// Within this distance the full models cast their own shadow; beyond it the
/// proxies do. The sun's first cascade ends at 80 m and the second at about
/// 300 m, where a texel is a hand's width: a crown's lobes are gone from the
/// shadow map, and its outline is the proxy's.
const FULL_SHADOW_M: f32 = 250.0;

/// Beyond this multiple of the LOD distance (and at least
/// [`FAR_TIER_MIN_M`]) the proxies draw their cheaper tier.
const FAR_TIER: f32 = 2.0;
const FAR_TIER_MIN_M: f32 = 2000.0;

/// Small deterministic PRNG (SplitMix64), seeded per cell so the same forest
/// grows on every run.
struct Rng(u64);

impl Rng {
    fn seeded(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xDA3E_39CB_94B9_5BDB)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_thinning_rank_is_spread_evenly_and_fixed() {
        let plant = |x: f32, y: f32| Pos { x, y };
        let ranks: Vec<f32> = (0..4000).map(|i| plant_rank(plant((i % 80) as f32 * 7.3 + 0.37, (i / 80) as f32 * 11.1 + 0.91))).collect();
        assert!(ranks.iter().all(|r| (0.0..=1.0).contains(r)));
        for keep in [0.2f32, 0.35, 0.6] {
            let shown = ranks.iter().filter(|&&r| r < keep).count() as f32 / ranks.len() as f32;
            assert!((shown - keep).abs() < 0.05, "keep {keep}: {shown}");
        }
        assert_eq!(plant_rank(plant(10.0, 20.0)), plant_rank(plant(10.0, 20.0)));
        // the lags spread over their range too, and differ from the ranks
        let lags: Vec<f32> = (0..4000).map(|i| plant_lag(plant((i % 80) as f32 * 7.3 + 0.37, (i / 80) as f32 * 11.1 + 0.91))).collect();
        let mean = lags.iter().sum::<f32>() / lags.len() as f32;
        assert!((mean - LAG_S / 2.0).abs() < 2.0, "{mean}");
    }

    use super::*;

    /// What fire adds to every variety must build in range of the index
    /// buffer, mark its debris, and stay inside the live plant's box.
    #[test]
    fn burnt_remains_build_for_every_variety() {
        for species in crate::models::SPECIES {
            for &name in crate::models::variety_names(species) {
                for (yaw, low) in [0.3_f32, 3.0, 5.5].into_iter().flat_map(|y| [(y, false), (y, true)]) {
                    let mut b = Builder::default();
                    let scale = Vec3::new(10.0, 12.0, 10.0);
                    b.remains(&Remains { name, base: Vec3::new(5.0, 2.0, 5.0), scale, yaw }, low);
                    assert!(!b.indices.is_empty(), "{name} {yaw}");
                    assert!(b.indices.iter().all(|&i| (i as usize) < b.positions.len()), "{name} {yaw}");
                    assert!(b.positions.iter().all(|p| p[1] >= 1.9 && p[1] <= 2.0 + 12.0 * 1.1), "{name} {yaw}");
                    assert!(b.colors.iter().any(|c| c[3] < 0.5), "{name}: skeleton");
                }
            }
        }
    }

    /// The distant proxies must stay under half of the models they stand in
    /// for (or 30 triangles), and face outward: `Builder::finish` averages face normals, and an
    /// inward crown would be lit from the wrong side.
    #[test]
    fn proxies_are_cheap_and_face_out() {
        for name in crate::models::SPECIES.into_iter().flat_map(|s| crate::models::variety_names(s).iter().copied()) {
            let mut b = Builder::default();
            b.proxy(name, Vec3::ZERO, Vec3::splat(10.0), 0.7, [0.3, 0.4, 0.2], [0.4, 0.2, 0.1]);
            let model = crate::models::live(name).indices.len() / 3;
            let tris = b.indices.len() / 3;
            // a cypress is cheap already: its proxy may be the full 30
            assert!(tris * 2 <= model.max(60), "{name}: {tris} proxy triangles for {model}");

            let mesh = b.finish();
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(nor)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!() };
            let (lo, hi) = pos.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))));
            let (crown_lo, crown_hi) = proxy(name).crown;
            let centre = Vec3::new((lo.x + hi.x) * 0.5, (crown_lo.y + crown_hi.y) * 5.0, (lo.z + hi.z) * 0.5);
            let crown_start = if proxy(name).trunk.is_some() { 6 } else { 0 };
            for (p, n) in pos.iter().zip(nor).skip(crown_start) {
                let out = Vec3::from(*p) - centre;
                assert!(out.dot(Vec3::from(*n)) > 0.0, "{name}: normal {n:?} at {p:?} points in");
            }
        }
    }

    /// The fire grid holds each cell's arrival to the second and its cleared
    /// flag, and decoded the way the shader does (`cell` and `ground` in
    /// `veg_state.wgsl`) gives back `FireField::arrival` and
    /// `cleared_fraction`; through a real `medium` fire the plants' looks
    /// (`plant_look`, the shader's rule) burn every plant the front passes
    /// and never bring a burnt one back while the front stays put.
    #[test]
    fn the_fire_grid_carries_the_fire_to_the_plants() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut sim = Sim::new(&data, "medium", 2).unwrap();
        let w = sim.scn.world;
        for _ in 0..40 {
            for _ in 0..(300.0 / rocca::STEP_S as f32).max(1.0) as usize {
                if sim.is_over() { break; }
                sim.game.step().unwrap();
            }
        }
        let field = FireField {
            state: sim.fire.state(),
            arrival: sim.fire.arrival_times(),
            intensity: sim.fire.intensity(),
            cleared: sim.fire.cleared(),
            world: w,
        };
        let bytes = grid_bytes(field.arrival, field.cleared);
        let (cols, rows) = (w.fire_cols, w.fire_rows);
        // the shader's decode, in Rust
        let cell = |c: usize, r: usize| {
            let i = (r * cols + c) * 4;
            let t = bytes[i] as u32 | (bytes[i + 1] as u32) << 8 | (bytes[i + 2] as u32) << 16;
            ((t != NEVER).then_some(t as f32), (bytes[i + 3] & CLEARED_BIT) as f32)
        };
        let ground = |p: Pos| {
            let fx = (p.x / w.cellsize - 0.5).clamp(0.0, (cols - 1) as f32);
            let fy = ((w.height_m - p.y) / w.cellsize - 0.5).clamp(0.0, (rows - 1) as f32);
            let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(cols - 1), (y0 + 1).min(rows - 1));
            let (tx, ty) = (fx.fract(), fy.fract());
            let cs = [cell(x0, y0), cell(x1, y0), cell(x0, y1), cell(x1, y1)];
            let ws = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
            let (mut sum, mut total, mut cleared) = (0.0, 0.0, 0.0);
            for (c, wt) in cs.iter().zip(ws) {
                cleared += c.1 * wt;
                if let Some(t) = c.0.filter(|_| wt > 0.0) {
                    sum += t * wt;
                    total += wt;
                }
            }
            ((total > 0.0).then(|| sum / total + (1.0 - total) * 90.0), cleared)
        };
        let now = sim.time_s() as f32;
        let (mut burnt, mut checked) = (0, 0);
        let mut rng = Rng::seeded(3);
        for _ in 0..20_000 {
            let p = Pos { x: rng.unit() * w.width_m, y: rng.unit() * w.height_m };
            let (a, c) = ground(p);
            let (fa, fc) = (field.arrival(p), field.cleared_fraction(p));
            assert_eq!(a.is_some(), fa.is_some(), "{p:?}");
            if let (Some(a), Some(fa)) = (a, fa) {
                assert!((a - fa).abs() < 1e-2 * fa.abs().max(1.0), "{p:?}: {a} {fa}");
            }
            assert!((c - fc).abs() < 1e-5, "{p:?}");
            let (look, t0) = plant_look(&field, p, now, true, 1.0);
            if fa.is_some_and(|a| now >= a + LAG_S + PLANT_FLAMING_S) {
                checked += 1;
                // every plant the front passed long enough ago is burnt
                // (or was cut before the fire came)
                assert!(look == BURNT_REMAINS || look == CLEARED, "{p:?}: {look} at {now}, t0 {t0}");
                burnt += usize::from(look == BURNT_REMAINS);
            }
            // keeping nothing hides every plant that is not burnt
            let (thin, _) = plant_look(&field, p, now, true, 0.0);
            assert!(thin == CLEARED || thin == BURNT_REMAINS);
        }
        assert!(burnt > 100 && checked > 100, "{burnt} of {checked}");
    }

    /// How many entities and draws the instancing makes on the desktop
    /// (`alta`: vegetation 0.5, 1280 m tiles) and in the browser (0.025,
    /// one tile): printed, run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn instancing_counts() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let sim = Sim::new(&data, "medium", 2).unwrap();
        let w = sim.scn.world;
        for (label, density, tile) in [("desktop alta", 0.5, 64usize), ("browser", 0.025, usize::MAX / 4)] {
            let tile = tile.min(w.fire_rows.max(w.fire_cols));
            let (mut plants, mut groups, mut tiles) = (0, 0, 0);
            for r0 in (0..w.fire_rows).step_by(tile) {
                for c0 in (0..w.fire_cols).step_by(tile) {
                    let lods = scatter_tile(&sim.scn, density, (r0, (r0 + tile).min(w.fire_rows), c0, (c0 + tile).min(w.fire_cols)));
                    if lods.inst.is_empty() { continue; }
                    tiles += 1;
                    plants += lods.inst.len();
                    let names: std::collections::BTreeSet<_> = lods.inst.iter().map(|i| i.name).collect();
                    groups += names.len();
                }
            }
            eprintln!("{label}: {plants} plants, {tiles} tiles, {groups} tile x variety groups");
        }
    }
}
