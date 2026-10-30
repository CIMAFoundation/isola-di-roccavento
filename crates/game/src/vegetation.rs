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
//! built to at most 120 vertices, and the visual density comes from crown
//! width and overlap rather than from stem count, because a hundred wide
//! crowns close a canopy that a thousand narrow ones would not.
//!
//! Geometry is merged into per-chunk meshes rather than spawned per plant: one
//! entity each would cost more in transform propagation and draw calls than
//! the triangles themselves, while merged chunks give the renderer a couple of
//! hundred draws for the entire landscape and still allow frustum culling.
//!
//! The plants respond to the fire: each one remembers which fire cell it
//! stands in, and when that cell's state changes the plant is redrawn —
//! foliage flares, then goes to charcoal. Only chunks whose cells actually
//! changed are touched, so the cost tracks the front.
//!
//! The meshes are never modified for that: every vertex carries its plant's
//! index (`materials::ATTRIBUTE_PLANT`), a small per-chunk image holds each
//! plant's state and ignition time, and the vertex stage
//! (`shaders/veg_state.wgsl`) collapses, recolours and flags the plant. A
//! chunk is ~80 k triangles and Bevy clones and uploads a modified mesh whole:
//! rewriting two or three of them every 0.15 s was a frame hitch of tens of
//! milliseconds on both the main and the render thread. `draw_standing`,
//! `draw_char` and `paint` do the same on the CPU: they are the reference for
//! the shader.
//!
//! Every chunk is built twice: with the authored models, and with a proxy of
//! a couple of dozen faces per plant (a faceted crown on a three-sided
//! trunk) that stands in for it beyond [`crate::quality::Graphics::lod_m`].
//! A plant there covers a few pixels, and the GPU — which is what limits the
//! frame rate — draws every vegetation triangle once per pass, shadow
//! cascades included. Both meshes burn together.

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

/// Plants burn through a per-chunk state image and the vertex stage (see the
/// module doc); without it the meshes are rewritten.
const GPU: bool = true;

/// Width of a chunk's state image in texels (`ROW` in `veg_state.wgsl`).
const STATE_ROW: usize = 256;

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
const PLANT_FLAMING_S: f32 = 240.0;

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

    fn wood(self) -> [f32; 3] {
        match self {
            Species::Grass => [0.32, 0.29, 0.17],
            // maritime pine bark is red-brown, and it is what shows: the
            // trunk is bare for most of its height
            Species::Conifer => [0.46, 0.23, 0.14],
            _ => [0.24, 0.18, 0.13],
        }
    }
}

/// One scattered plant, remembered so it can react to the fire.
struct Plant {
    /// Where it stands, in world metres. The plant burns on the *interpolated*
    /// arrival time at this point rather than on its cell's, so a stand
    /// catches progressively in the direction the fire is running instead of
    /// flipping as a 20 m block.
    pos: Pos,
    /// Seconds this plant lags the interpolated front, hashed per plant: even
    /// two shrubs a metre apart do not catch at the same instant.
    lag_s: f32,
    vert_start: u32,
    vert_end: u32,
    /// The same plant in the distant proxy mesh.
    low_start: u32,
    low_end: u32,
    /// What its burnt remains are made of, if it has any (grass leaves none).
    remains: Option<Remains>,
}

/// Enough of a plant to build its burnt model later: the archetype, where and
/// how big it stood.
#[derive(Clone)]
struct Remains {
    name: &'static str,
    base: Vec3,
    scale: Vec3,
    yaw: f32,
}

/// What fire adds to a chunk's plants (`scripts/build_vegetation_models.py`),
/// built when the fire first burns a plant of the chunk, so the forest the
/// fire never reaches does not pay for them. Every plant's remains are there
/// from then on, collapsed to a point until the plant is burnt, the way a
/// cleared plant is (see [`CLEARED`]).
struct CharLayer {
    /// Shown within the LOD distance, and beyond it (the `_low` models).
    near: CharMesh,
    far: CharMesh,
}

struct CharMesh {
    mesh: Handle<Mesh>,
    entity: Entity,
    /// Vertex positions with every plant standing, and with every plant
    /// collapsed.
    shown: Vec<[f32; 3]>,
    hidden: Vec<[f32; 3]>,
    /// Each plant's vertex range.
    ranges: Vec<(u32, u32)>,
    /// Vertex colours as built (alpha 0): what a burnt plant shows.
    colors: Vec<[f32; 4]>,
}

struct Chunk {
    /// The material of this chunk's meshes, its own: it holds the state image.
    material: Handle<FoliageMaterial>,
    /// One texel per plant: fire state, ignition time (GPU path).
    state: Option<Handle<Image>>,
    /// Authored models and distant proxies (see the module doc).
    mesh: Handle<Mesh>,
    low_mesh: Handle<Mesh>,
    /// Index list the proxies' mesh swaps with when the chunk crosses
    /// [`FAR_TIER`]: the farthest tier's, or the nearer one's once swapped.
    far_idx: Vec<u32>,
    /// The proxies' mesh is on its farthest tier.
    far: bool,
    full: Entity,
    low: Entity,
    /// Which of the two is on show.
    near: bool,
    /// The full models cast the sun's shadow (within [`FULL_SHADOW_M`]); beyond
    /// it `shadow` casts it for them.
    full_casts: bool,
    /// The proxies again, on the sun's layer only: the shadow of a full-model
    /// chunk too far for its crowns to show in the shadow map.
    shadow: Option<Entity>,
    /// World-space bounds (Bevy frame) of the plants, for the LOD distance.
    bounds: (Vec3, Vec3),
    /// Fire cells the chunk covers, `(row0, row1, col0, col1)`, end exclusive.
    cells: (usize, usize, usize, usize),
    plants: Vec<Plant>,
    /// The burnt remains, once a plant here has burnt.
    char: Option<CharLayer>,
    /// Colours as generated, before any fire damage.
    base: Vec<[f32; 4]>,
    low_base: Vec<[f32; 4]>,
    /// Vertex positions as generated, so a plant hidden on cleared ground
    /// (see [`CLEARED`]) can be brought back when the game restarts.
    pos_base: Vec<[f32; 3]>,
    low_pos_base: Vec<[f32; 3]>,
    /// Fire state each plant was last drawn with, so a chunk is rebuilt only
    /// when something in it actually changed.
    drawn: Vec<u8>,
    /// How many plants are drawn other than unburnt, and how many burnt (a
    /// burnt plant is final, until a restart).
    lit: usize,
    burnt: usize,
}

/// Share of the built plants that are shown (bits of an `f32`; see
/// [`Vegetation::set_keep`]). 1 = all: the plants are as spawned.
static KEEP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

fn keep() -> f32 {
    f32::from_bits(KEEP.load(std::sync::atomic::Ordering::Relaxed))
}

/// A plant's fixed place in the order in which the thinning takes plants
/// away (0..1, from its position: the same plants go on every redraw).
fn rank(plant: &Plant) -> f32 {
    let h = plant.pos.x.to_bits().wrapping_mul(0x9E37_79B1) ^ plant.pos.y.to_bits().wrapping_mul(0x85EB_CA6B);
    (h ^ (h >> 15)).wrapping_mul(0x2C1B_3C6D) as f32 / u32::MAX as f32
}

#[derive(Resource)]
pub struct Vegetation {
    /// The thinning changed: every chunk is looked at once.
    thin_dirty: bool,
    chunks: Vec<Chunk>,
    /// Beyond this distance from the camera a chunk shows its proxies.
    lod_m: f32,
    /// Simulated time the plants were last drawn at: the plants are a
    /// function of the fire and the clock, so an unchanged clock (a paused
    /// world, the planning phase) costs nothing.
    drawn_at: Option<f32>,
    /// Cells cleared by crews when the plants were last drawn.
    drawn_cleared: usize,
}

/// Drawn state of a plant standing on ground a crew has cleared: it is gone,
/// leaving bare earth (the ground itself is tinted by `fire_view`). Above
/// the `CellFire` values.
const CLEARED: u8 = 10;
/// Share of the plants inside a cleared line that are taken away.
const CLEARED_SHARE: f32 = 0.85;

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


    let (rows, cols) = (scn.world.fire_rows, scn.world.fire_cols);
    let chunks_y = rows.div_ceil(CHUNK_CELLS);
    let chunks_x = cols.div_ceil(CHUNK_CELLS);
    let mut chunks = Vec::with_capacity(chunks_x * chunks_y);
    let (mut plant_count, mut tri_count, mut low_tri_count) = (0usize, 0usize, 0usize);

    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            let mut builder = Lods::default();
            builder.low.want_far = true;
            let mut plants = Vec::new();

            for r in cy * CHUNK_CELLS..((cy + 1) * CHUNK_CELLS).min(rows) {
                for c in cx * CHUNK_CELLS..((cx + 1) * CHUNK_CELLS).min(cols) {
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
                        let start = builder.full.positions.len() as u32;
                        let low_start = builder.low.positions.len() as u32;
                        let remains_before = builder.remains.len();
                        let Some(pos) = scatter_plant(scn, cell, species, &mut rng, &mut builder)
                        else {
                            continue;
                        };
                        plants.push(Plant {
                            pos,
                            lag_s: rng.unit() * 45.0,
                            vert_start: start,
                            vert_end: builder.full.positions.len() as u32,
                            low_start,
                            low_end: builder.low.positions.len() as u32,
                            remains: builder.remains.get(remains_before).cloned(),
                        });
                    }
                }
            }

            if builder.full.positions.is_empty() {
                continue;
            }
            plant_count += plants.len();
            tri_count += builder.full.indices.len() / 3;
            low_tri_count += builder.low.indices.len() / 3;

            let bounds = builder.full.positions.iter().fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))),
            );
            // the generated shapes, to redraw from: the CPU path only
            let base = if GPU { Vec::new() } else { builder.full.colors.clone() };
            let low_base = if GPU { Vec::new() } else { builder.low.colors.clone() };
            let drawn = vec![CellFire::Unburnt as u8; plants.len()];
            let pos_base = if GPU { Vec::new() } else { builder.full.positions.clone() };
            let low_pos_base = if GPU { Vec::new() } else { builder.low.positions.clone() };
            let (verts, low_verts) = (builder.full.positions.len(), builder.low.positions.len());
            let (mut low, far_idx) = builder.low.finish_with_far();
            tag_plants(&mut low, low_verts, plants.iter().map(|p| (p.low_start, p.low_end)), 0);
            let image = images.add(state_image(plants.len()));
            let material = materials.add(FoliageMaterial::new(image.clone()));
            let state = Some(image);
            let low_mesh = meshes.add(low);
            // The browser only draws proxies (lod_m = 0). Share their handle
            // with the hidden near entity instead of uploading a second copy.
            let mesh = if cfg!(target_arch = "wasm32") {
                low_mesh.clone()
            } else {
                let mut full_mesh = builder.full.finish();
                tag_plants(&mut full_mesh, verts, plants.iter().map(|p| (p.vert_start, p.vert_end)), 0);
                if GPU {
                    full_mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
                    full_mesh.compress_indices();
                }
                meshes.add(full_mesh)
            };

            // Vegetation is visual only: Mario can run through trunks and
            // canopies, and the follow camera is not clipped by the forest.
            // The full models start on show, until the LOD system picks.
            // The light through the leaves is shadowed too: a crown in the
            // shade of another does not glow as if the sun were behind it.
            let full = commands.spawn(((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())), TransmittedShadowReceiver)).id();
            let low = commands.spawn(((
                Mesh3d(low_mesh.clone()),
                MeshMaterial3d(material.clone()),
                Visibility::Hidden,
            ), TransmittedShadowReceiver)).id();
            // Same mesh handle as `low`: the burn edits reach it for free.
            let shadow = gfx.shadows.then(|| {
                commands.spawn((
                    (
                        Mesh3d(low_mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Visibility::Hidden,
                    ),
                    bevy::camera::visibility::RenderLayers::layer(crate::terrain_mesh::SHADOW_LAYER),
                )).id()
            });
            chunks.push(Chunk {
                material: material.clone(),
                state,
                mesh,
                low_mesh,
                far_idx,
                far: false,
                full,
                low,
                near: true,
                full_casts: true,
                shadow,
                bounds,
                cells: (
                    cy * CHUNK_CELLS,
                    ((cy + 1) * CHUNK_CELLS).min(rows),
                    cx * CHUNK_CELLS,
                    ((cx + 1) * CHUNK_CELLS).min(cols),
                ),
                plants,
                char: None,
                base,
                low_base,
                pos_base,
                low_pos_base,
                drawn,
                lit: 0,
                burnt: 0,
            });
        }
    }

    info!(
        "vegetation: {plant_count} plants in {} chunks, {:.2} M triangles ({:.2} M as distant proxies)",
        chunks.len(),
        tri_count as f32 / 1e6,
        low_tri_count as f32 / 1e6
    );
    commands.insert_resource(Vegetation { thin_dirty: gfx.veg_keep < 1.0, chunks, lod_m: gfx.lod_m, drawn_at: None, drawn_cleared: 0 });
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
    let wood = mul(species.wood(), shade);

    // One model per group, no mixing: the player reads the fuel from the
    // silhouette (cypresses and olives used to stand among pines and oaks).
    match species {
        Species::Conifer => conifer(out, base, scale, yaw, foliage, wood, rng),
        Species::Broadleaf => broadleaf(out, base, scale, yaw, foliage, wood, rng),
        Species::Shrub => shrub(out, base, scale, yaw, foliage, rng),
        Species::Grass if true && rng.unit() < 0.025 => {
            // Lone field trees break the meadow silhouette without changing
            // its simulation fuel; they share the same interpolated burn field.
            let height = (7.0 + rng.unit() * 5.0) * scale * crate::visual_scale::TREE;
            out.model("olive", base, Vec3::splat(height), yaw, [0.33, 0.40, 0.26], wood);
        }
        // Meadow tufts are drawn by the terrain shader (`shaders/terrain.wgsl`)
        // unless `ROCCA_GRASS_MESH` asks for the old geometry.
        Species::Grass if crate::ground_mask::shader_grass() => return None,
        Species::Grass => grass(out, base, scale, yaw, foliage, rng),
    }
    Some(p)
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
fn conifer(out: &mut Lods, base: Vec3, scale: f32, yaw: f32,
    foliage: [f32; 3], wood: [f32; 3], rng: &mut Rng) {
    // maritime pine: tall, the umbrella a little wider than the model
    let height = (13.0 + rng.unit() * 8.0) * scale;
    let width = height * (1.0 + rng.unit() * 0.3);
    out.model("pine", base, Vec3::new(width, height, width), yaw, foliage, wood);
}

fn broadleaf(out: &mut Lods, base: Vec3, scale: f32, yaw: f32,
    foliage: [f32; 3], wood: [f32; 3], rng: &mut Rng) {
    // chestnut: lower than the pines, the crown about as wide as tall
    let height = (8.0 + rng.unit() * 5.0) * scale;
    out.model("chestnut", base, Vec3::new(height * 1.1, height, height * 1.1), yaw, foliage, wood);
}

fn shrub(out: &mut Lods, base: Vec3, scale: f32, yaw: f32,
    foliage: [f32; 3], rng: &mut Rng) {
    // low, wide and stemless: macchia must not read as young trees
    let r = (2.6 + rng.unit() * 2.2) * scale;
    out.model("bush", base, Vec3::new(r, r * 0.8, r), yaw, foliage, foliage);
}

fn grass(out: &mut Lods, base: Vec3, scale: f32, yaw: f32, foliage: [f32; 3], rng: &mut Rng) {
    // A tussock, not a blade: the unit of grassland at this scale is a clump
    // half a metre high and a couple of metres across. Drawn as a low fan plus
    // a few upright blades, so it holds a silhouette from a low camera angle
    // and still covers ground when seen from above.
    let spread = (1.5 + rng.unit() * 1.4) * scale;
    let h = (0.5 + rng.unit() * 0.6) * scale;

    // Ground fan: what actually closes the cover from altitude. It is also
    // the whole distant proxy: the blades are under a pixel there.
    out.full.dome(base, spread, h * 0.55, yaw, mul(foliage, 0.9));
    out.low.dome(base, spread, h * 0.55, yaw, mul(foliage, 0.9));

    for i in 0..2 {
        let a = yaw + i as f32 * (std::f32::consts::TAU / 3.0) + rng.unit() * 0.5;
        out.full.blade(
            base + Vec3::new(a.cos() * spread * 0.3, 0.0, a.sin() * spread * 0.3),
            spread * 0.45,
            h * (1.2 + rng.unit() * 1.0),
            a,
            mul(foliage, 1.0 + rng.unit() * 0.2),
        );
    }
}

// --- mesh building ---------------------------------------------------------

/// The two meshes of a chunk, built side by side.
#[derive(Default)]
struct Lods {
    full: Builder,
    low: Builder,
    /// One entry per plant drawn from an archetype, in order.
    remains: Vec<Remains>,
}

impl Lods {
    /// `species` is pine, chestnut, bush or olive; which of its varieties this
    /// plant is follows from its yaw, so it is the same on every run.
    fn model(&mut self, species: &'static str, base: Vec3, scale: Vec3, yaw: f32,
        foliage: [f32; 3], wood: [f32; 3]) {
        let name = crate::models::pick_variety(species, yaw / std::f32::consts::TAU);
        // Do not allocate the detailed tree on web.
        // Merely hiding that mesh with LOD still retained all its CPU/GPU data.
        if cfg!(target_arch = "wasm32") {
            self.full.proxy(name, base, scale, yaw, foliage, wood);
            self.low.proxy(name, base, scale, yaw, foliage, wood);
            return;
        }
        self.remains.push(Remains { name, base, scale, yaw });
        self.full.model(crate::models::live(name), base, scale, yaw, foliage, wood);
        self.low.proxy(name, base, scale, yaw, foliage, wood);
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
}

fn proxy(name: &str) -> &'static Proxy {
    static PROXIES: std::sync::OnceLock<std::collections::HashMap<&'static str, Proxy>> =
        std::sync::OnceLock::new();
    &PROXIES.get_or_init(|| {
        ["pine", "chestnut", "bush", "olive"]
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
                        .fold((0.0, 0), |(s, n), i| (s + m.colors[i][1] / 0.38, n + 1));
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
                (name, Proxy { crown: (lo, hi), shade_low: mean(false), shade_high: mean(true), trunk })
            })
            .collect()
    })[name]
}

/// No UV attribute: the vegetation material carries no texture, and at these
/// vertex counts eight bytes each is tens of megabytes of nothing.
#[derive(Default)]
struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
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
        for (i, p) in model.positions.iter().enumerate() {
            let color = if model.wood[i] { wood } else { foliage };
            // Preserve species colour and the authored crown's tonal variation.
            let shade = if model.wood[i] { 1.0 } else { model.colors[i][1] / 0.38 };
            self.vertex_a(base + rotation * (Vec3::from(*p) * scale), mul(color, shade), leaf(!model.wood[i]));
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
    /// instead of 100–156.
    fn proxy(&mut self, name: &str, base: Vec3, scale: Vec3, yaw: f32,
        foliage: [f32; 3], wood: [f32; 3]) {
        const SIDES: u32 = 6;
        self.sync_far();
        let p = proxy(name);
        // a few percent darker: the authored crown shades itself in its lobes
        let foliage = mul(foliage, 0.95);
        let rotation = Quat::from_rotation_y(yaw);
        let at = |v: Vec3| base + rotation * (v * scale);
        let (lo, hi) = p.crown;
        let (centre, radius) = ((lo + hi) * 0.5, (hi - lo) * 0.5);

        if let Some(r) = p.trunk {
            let start = self.positions.len() as u32;
            for i in 0..3 {
                let a = i as f32 * std::f32::consts::TAU / 3.0;
                let d = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                self.vertex(at(Vec3::new(centre.x, 0.0, centre.z) + d), wood);
                self.vertex(at(Vec3::new(centre.x, lo.y + radius.y * 0.4, centre.z) + d), wood);
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
        let bottom = self.vertex_a(at(Vec3::new(centre.x, y(0.04), centre.z)), mul(foliage, p.shade_low * 0.8), 1.0);
        let ring = self.positions.len() as u32;
        for (t, k, shade) in [(0.3, 0.85, p.shade_low), (0.62, 1.0, p.shade_high)] {
            for i in 0..SIDES {
                let a = (i as f32 + 0.5 * t) * std::f32::consts::TAU / SIDES as f32;
                let v = Vec3::new(centre.x + a.cos() * radius.x * k, y(t), centre.z + a.sin() * radius.z * k);
                self.vertex_a(at(v), mul(foliage, shade), 1.0);
            }
        }
        let top = self.vertex_a(at(Vec3::new(centre.x, hi.y, centre.z)), mul(foliage, p.shade_high * 1.05), 1.0);
        // The farthest tier is a bipyramid on a ring of its own (vertices the
        // nearer tier never indexes), as wide as the crown is at its widest:
        // the near rings are too far apart in height to drop either of them.
        let far_ring = self.positions.len() as u32;
        if self.want_far {
            // a little lighter than the near rings: the far faces are flatter
            // to the sun, and the crown would otherwise read darker
            let shade = p.shade_high * 1.08;
            for i in 0..SIDES {
                let a = i as f32 * std::f32::consts::TAU / SIDES as f32;
                let v = Vec3::new(centre.x + a.cos() * radius.x * 0.97, y(0.45), centre.z + a.sin() * radius.z * 0.97);
                self.vertex_a(at(v), mul(foliage, shade), 1.0);
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

    fn vertex(&mut self, p: Vec3, c: [f32; 3]) -> u32 {
        self.vertex_a(p, c, 0.0)
    }

    /// `leaf` goes in the colour's alpha (the material is opaque, so nothing
    /// else reads it): 1 for living foliage, which the shader textures; 0 for
    /// wood, rock, grass and everything fire has touched (see `paint`).
    fn vertex_a(&mut self, p: Vec3, c: [f32; 3], leaf: f32) -> u32 {
        let i = self.positions.len() as u32;
        self.positions.push([p.x, p.y, p.z]);
        self.normals.push([0.0, 1.0, 0.0]); // replaced in `finish`
        // Vertex colours are linear in Bevy. Ecological palettes above are
        // sRGB; without conversion dark foliage washes out to pastel mint.
        let linear = Color::srgb(c[0], c[1], c[2]).to_linear();
        self.colors.push([linear.red, linear.green, linear.blue, leaf]);
        i
    }

    /// Low, irregular dome for the remaining procedural grass tussocks.
    fn dome(&mut self, base: Vec3, radius: f32, height: f32, yaw: f32, color: [f32; 3]) {
        const SEGMENTS: usize = 5;
        // The tip leans, so a clump of domes does not read as a row of cones.
        let (ls, lc) = yaw.sin_cos();
        let apex = self.vertex(
            base + Vec3::new(lc * radius * 0.18, height, ls * radius * 0.18),
            color,
        );
        let ring = self.positions.len() as u32;
        for i in 0..SEGMENTS {
            let a = yaw + i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            // Deterministic rim wobble, and a darker skirt where the lobe
            // meets the ground.
            let r = radius * (0.78 + 0.44 * ((a * 2.7).sin() * 0.5 + 0.5));
            let skirt = [color[0] * 0.72, color[1] * 0.72, color[2] * 0.72];
            self.vertex(base + Vec3::new(c * r, height * 0.06, s * r), skirt);
        }
        for i in 0..SEGMENTS {
            let a = ring + i as u32;
            let b = ring + ((i + 1) % SEGMENTS) as u32;
            self.indices.extend_from_slice(&[apex, a, b]);
        }
    }

    /// Single upright quad, leaning slightly: one blade of a grass tuft.
    fn blade(&mut self, base: Vec3, width: f32, height: f32, yaw: f32, color: [f32; 3]) {
        let (s, c) = yaw.sin_cos();
        let across = Vec3::new(c, 0.0, s) * width * 0.5;
        let lean = Vec3::new(-s, 0.0, c) * height * 0.25;
        let start = self.positions.len() as u32;
        // Root colour is darker: a lit blade should not read as a flat card.
        let root = mul(color, 0.65);
        self.vertex(base - across, root);
        self.vertex(base + across, root);
        self.vertex(base + across * 0.35 + lean + Vec3::Y * height, color);
        self.vertex(base - across * 0.35 + lean + Vec3::Y * height, color);
        self.indices
            .extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    }

    /// Area-weighted vertex normals from the faces, then the mesh.
    fn finish(self) -> Mesh {
        self.finish_with_far().0
    }

    /// The mesh and, for a `want_far` builder, the farthest tier's index list
    /// over the same vertices.
    fn finish_with_far(mut self) -> (Mesh, Vec<u32>) {
        self.sync_far();
        for n in &mut self.normals {
            *n = [0.0, 0.0, 0.0];
        }
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
        for n in &mut self.normals {
            *n = Vec3::from(*n).normalize_or_zero().into();
        }

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_indices(Indices::U32(self.indices));
        (mesh, self.far)
    }
}

// --- burning ---------------------------------------------------------------

/// Burn the plants the fire has reached.
///
/// Each plant is tested against the *interpolated* arrival time at its own
/// position plus its own hashed lag, so a stand ignites as a wave crossing it
/// rather than as a cell-shaped block, and a burning cell contains flaring,
/// charred and untouched plants at once — which is what a fire edge looks like.
///
/// Only chunks with at least one changed plant get their colour attribute
/// rewritten, so a front crossing two chunks costs two buffer uploads rather
/// than 250; and a chunk the fire has not reached (no arrival time in its
/// cells or their neighbours, whose times its plants interpolate) is not even
/// looked at.
///
/// A rewritten chunk goes to the GPU whole (megabytes for a forest chunk), so
/// the plants are redrawn at most every [`BURN_REDRAW_S`] of real time: at
/// ×40 that is still a step of a few simulated seconds against a flaming
/// phase of four minutes.
pub fn burn(
    mut commands: Commands,
    sim: Res<Sim>,
    time: Res<Time>,
    mut veg: ResMut<Vegetation>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<FoliageMaterial>>,
    mut restarted: MessageReader<crate::sim::SimRestarted>,
    mut since: Local<f32>,
) {
    let restarted = restarted.read().count() > 0;
    let thin = std::mem::take(&mut veg.thin_dirty);
    *since += time.delta_secs();
    if !restarted && !thin && *since < BURN_REDRAW_S {
        return;
    }
    let now = sim.time_s() as f32;
    let line_cells = sim.fire.line_cells;
    if veg.drawn_at == Some(now) && veg.drawn_cleared == line_cells && !restarted && !thin {
        return;
    }
    *since = 0.0;
    veg.drawn_at = Some(now);
    veg.drawn_cleared = line_cells;
    let field = FireField {
        state: sim.fire.state(),
        arrival: sim.fire.arrival_times(),
        intensity: sim.fire.intensity(),
        cleared: sim.fire.cleared(),
        world: sim.scn.world,
    };
    let (rows, cols) = (sim.scn.world.fire_rows, sim.scn.world.fire_cols);
    let reached = |(r0, r1, c0, c1): (usize, usize, usize, usize)| {
        (r0.saturating_sub(1)..(r1 + 1).min(rows)).any(|r| {
            field.arrival[r * cols + c0.saturating_sub(1)..r * cols + (c1 + 1).min(cols)]
                .iter()
                .any(|&t| t != i32::MIN)
        })
    };

    let cleared = sim.fire.cleared();
    let any_cleared = |(r0, r1, c0, c1): (usize, usize, usize, usize)| {
        line_cells > 0 && (r0..r1).any(|r| cleared[r * cols + c0..r * cols + c1].iter().any(|&b| b))
    };

    for chunk in &mut veg.chunks {
        // every plant burnt: nothing here can change until a restart
        if chunk.burnt == chunk.plants.len() && !restarted {
            continue;
        }
        if chunk.lit == 0 && !reached(chunk.cells) && !any_cleared(chunk.cells) && !thin {
            continue;
        }
        let (r0, r1, c0, c1) = chunk.cells;
        let scan_cleared = line_cells > 0 && any_cleared((r0.saturating_sub(1), (r1 + 1).min(rows), c0.saturating_sub(1), (c1 + 1).min(cols)));
        let redraw = plan_redraw(&chunk.plants, &mut chunk.drawn, &mut chunk.lit, &mut chunk.burnt, &field, now, scan_cleared, restarted);
        if redraw.is_empty() {
            continue;
        }
        if chunk.char.is_none() && redraw.iter().any(|&(_, s)| s == CellFire::Burnt as u8 || s == CellFire::Burning as u8) {
            chunk.char = Some(build_char_layer(&mut commands, &mut meshes, &*chunk));
        }

        let t0s: Vec<f32> = redraw.iter().map(|&(i, _)| ignition(&field, &chunk.plants[i])).collect();
        if GPU {
            // Only the plants in `redraw` change, in the chunk's state image
            // (a few hundred kilobytes at most, against megabytes of mesh).
            if let Some(mut image) = chunk.state.as_ref().and_then(|h| images.get_mut(h)) {
                let data = image.data.as_mut().expect("the state image keeps its CPU copy");
                for (&(i, s), &t0) in redraw.iter().zip(&t0s) {
                    let at = i * 8;
                    data[at..at + 4].copy_from_slice(&gpu_mode(s, &chunk.plants[i]).to_le_bytes());
                    data[at + 4..at + 8].copy_from_slice(&t0.to_le_bytes());
                }
            }
            // A changed image is a new texture on the GPU: the material's bind
            // group has to be made again to see it. The clock goes with it, so
            // a plant that has just caught is not drawn at the old time.
            if let Some(mut m) = materials.get_mut(&chunk.material) {
                m.fire.y = now;
                m.fire.z = PLANT_FLAMING_S;
            }
            continue;
        }

        // The browser: only the plants in `redraw` are rewritten, in place, in
        // the attributes the mesh already holds (taken out and put back, so no
        // buffer is copied).
        for (handle, base, pos_base, low) in [
            (&chunk.mesh, &chunk.base, &chunk.pos_base, false),
            (&chunk.low_mesh, &chunk.low_base, &chunk.low_pos_base, true),
        ] {
            edit_mesh(&mut meshes, handle, |pos, col| {
                for (&(i, s), &t0) in redraw.iter().zip(&t0s) {
                    let p = &chunk.plants[i];
                    let range = if low { (p.low_start as usize, p.low_end as usize) } else { (p.vert_start as usize, p.vert_end as usize) };
                    draw_standing(pos, col, pos_base, base, range, s, t0, p);
                }
            });
        }
        if let Some(layer) = &chunk.char {
            for m in [&layer.near, &layer.far] {
                edit_mesh(&mut meshes, &m.mesh, |pos, col| {
                    for (&(i, s), &t0) in redraw.iter().zip(&t0s) {
                        if chunk.plants[i].remains.is_some() {
                            let (a, b) = m.ranges[i];
                            draw_char(pos, col, m, (a as usize, b as usize), s, t0);
                        }
                    }
                });
            }
        }
    }
}

/// Run `f` on a mesh's position and colour attributes, in place. The
/// attributes are moved out and back rather than cloned.
fn edit_mesh(meshes: &mut Assets<Mesh>, handle: &Handle<Mesh>, f: impl FnOnce(&mut [[f32; 3]], &mut [[f32; 4]])) {
    use bevy::mesh::VertexAttributeValues::{Float32x3, Float32x4};
    let Some(mut mesh) = meshes.get_mut(handle) else { return };
    let (Some(Float32x3(mut pos)), Some(Float32x4(mut col))) =
        (mesh.remove_attribute(Mesh::ATTRIBUTE_POSITION), mesh.remove_attribute(Mesh::ATTRIBUTE_COLOR))
    else {
        warn!("vegetation: a chunk mesh lost its position or colour attribute");
        return;
    };
    f(&mut pos, &mut col);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
}

/// Seconds of simulated time at which a plant catches (its own lag included).
fn ignition(field: &FireField, plant: &Plant) -> f32 {
    field.arrival(plant.pos).map_or(0.0, |a| a + plant.lag_s)
}

/// State a plant should be drawn in: the fire's, or [`CLEARED`] if a crew has
/// thinned the ground under it (`scan_cleared`: some cleared cell is near).
fn wanted_state(field: &FireField, plant: &Plant, now: f32, scan_cleared: bool) -> u8 {
    let mut s = plant_state(field, plant, now);
    // thinned for the machine's sake (a burnt plant is final: it stays)
    if s != CellFire::Burnt as u8 && rank(plant) >= keep() {
        return CLEARED;
    }
    if s == CellFire::Unburnt as u8 && scan_cleared {
        // Thinned, not razed: the closer the plant is to the middle of
        // the cut, the likelier it is gone, decided by its own fixed
        // random number, so the same plants fall on every redraw and a
        // few survive inside the line. The edge is sharp (a narrow
        // ramp around the half-way point).
        let inside = ((field.cleared_fraction(plant.pos) - 0.45) / 0.15).clamp(0.0, 1.0);
        if plant.lag_s / 45.0 < CLEARED_SHARE * inside {
            s = CLEARED;
        }
    }
    s
}

/// Which plants of a chunk to rewrite, with their new state; `drawn`, `lit`
/// (plants not unburnt) and `burnt` are updated. A burnt plant is final and
/// not even looked at (except on a restart, which brings everything back).
/// Nothing is returned if no plant changed state; otherwise the changed ones
/// come with every plant still burning, whose ignition time (the shader's
/// clock) is read again from the front as it has moved since.
#[allow(clippy::too_many_arguments)]
fn plan_redraw(
    plants: &[Plant],
    drawn: &mut [u8],
    lit: &mut usize,
    burnt: &mut usize,
    field: &FireField,
    now: f32,
    scan_cleared: bool,
    restarted: bool,
) -> Vec<(usize, u8)> {
    const BURNT: u8 = CellFire::Burnt as u8;
    const BURNING: u8 = CellFire::Burning as u8;
    const UNBURNT: u8 = CellFire::Unburnt as u8;
    let mut out = Vec::new();
    for (i, plant) in plants.iter().enumerate() {
        if drawn[i] == BURNT && !restarted {
            continue;
        }
        let s = wanted_state(field, plant, now, scan_cleared);
        if s != drawn[i] {
            let lit_state = |x: u8| x != UNBURNT && x != CLEARED;
            *lit = *lit + usize::from(lit_state(s)) - usize::from(lit_state(drawn[i]));
            *burnt = *burnt + usize::from(s == BURNT) - usize::from(drawn[i] == BURNT);
            drawn[i] = s;
            out.push((i, s));
        }
    }
    if out.is_empty() {
        return out;
    }
    let changed = out.len();
    let mut next = 0;
    for (i, &d) in drawn.iter().enumerate() {
        if d == BURNING {
            while next < changed && out[next].0 < i {
                next += 1;
            }
            if next < changed && out[next].0 == i {
                continue;
            }
            out.push((i, BURNING));
        }
    }
    out
}

/// Redraw one plant's vertices `range` for fire state `s`: back to the
/// generated shape and colour, then the damage. Reads only `*_base`, so it
/// gives the same result whatever the plant showed before.
#[allow(clippy::too_many_arguments)]
fn draw_standing(
    pos: &mut [[f32; 3]],
    col: &mut [[f32; 4]],
    pos_base: &[[f32; 3]],
    col_base: &[[f32; 4]],
    (a, b): (usize, usize),
    s: u8,
    t0: f32,
    plant: &Plant,
) {
    pos[a..b].copy_from_slice(&pos_base[a..b]);
    col[a..b].copy_from_slice(&col_base[a..b]);
    if s == CellFire::Unburnt as u8 {
        return;
    }
    if s == CLEARED {
        // collapse every vertex of the plant onto its first one:
        // degenerate triangles draw nothing
        if a < b {
            let first = pos_base[a];
            pos[a..b].fill(first);
        }
        return;
    }
    if s == CellFire::Burnt as u8 && plant.remains.is_some() {
        // The crown is gone (the shader has eaten it by now); the trunk and
        // boughs stay, charred, under the twig skeleton of the remains.
        // Leaves are the vertices with alpha 1.
        if a < b {
            let first = pos_base[a];
            for i in a..b {
                if col_base[i][3] > 0.5 {
                    pos[i] = first;
                } else {
                    col[i] = charcoal(col_base[i]);
                }
            }
        }
        return;
    }
    // the shader consumes a burning plant over time from its ignition
    paint(&mut col[a..b], s, t0);
}

/// The same for the plant's remains: hidden until it ignites, then standing
/// (flagged with the same clock, so the shader shows each twig where the
/// leaves around it have been eaten, and the debris last) and for good once
/// it is burnt. Debris is the vertices built with alpha 1.
fn draw_char(pos: &mut [[f32; 3]], col: &mut [[f32; 4]], m: &CharMesh, (a, b): (usize, usize), s: u8, t0: f32) {
    pos[a..b].copy_from_slice(&m.hidden[a..b]);
    col[a..b].copy_from_slice(&m.colors[a..b]);
    let burning = s == CellFire::Burning as u8;
    if s == CellFire::Burnt as u8 || burning {
        pos[a..b].copy_from_slice(&m.shown[a..b]);
        for c in &mut col[a..b] {
            let debris = c[3] > 0.5;
            // ash is soot: the model's pale grey reads as a platform under the trees
            if debris {
                *c = [c[0] * 0.2, c[1] * 0.17, c[2] * 0.15, c[3]];
            }
            c[3] = if burning {
                -(1.0 + REMAINS_OFFSET + if debris { DEBRIS_OFFSET } else { 0.0 } + t0.max(0.0))
            } else {
                0.0
            };
        }
    }
}

/// Build the burnt remains of every plant of `chunk` that has any, hidden.
fn build_char_layer(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    chunk: &Chunk,
) -> CharLayer {
    let material = &chunk.material;
    let mut make = |low: bool| {
        let mut builder = Builder::default();
        let mut ranges = Vec::with_capacity(chunk.plants.len());
        for plant in &chunk.plants {
            let start = builder.positions.len() as u32;
            if let Some(r) = &plant.remains {
                builder.remains(r, low);
            }
            ranges.push((start, builder.positions.len() as u32));
        }
        let verts = builder.positions.len();
        let (mut colors, mut shown, mut hidden) = (Vec::new(), Vec::new(), Vec::new());
        if !GPU {
            colors = builder.colors.clone();
            shown = builder.positions.clone();
            hidden = shown.clone();
            for &(a, b) in &ranges {
                if a < b {
                    let first = hidden[a as usize];
                    hidden[a as usize..b as usize].fill(first);
                }
            }
        }
        let mut mesh = builder.finish();
        if GPU {
            // standing where the plants stood, whole and for good: the state
            // image shows it from the plant's ignition (`veg_state.wgsl`)
            tag_plants(&mut mesh, verts, ranges.iter().copied(), CHAR_PLANT);
            mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
            mesh.compress_indices();
        } else {
            // the normals above are those of the standing remains
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, hidden.clone());
        }
        let mesh = meshes.add(mesh);
        let on = chunk.near != low;
        let entity = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                if on { Visibility::Inherited } else { Visibility::Hidden },
            ))
            .id();
        if on && !chunk.full_casts { commands.entity(entity).insert(NotShadowCaster); }
        CharMesh { mesh, entity, shown, hidden, ranges, colors }
    };
    CharLayer { near: make(false), far: make(true) }
}

/// Real seconds between two redraws of the burning plants (see [`burn`]).
const BURN_REDRAW_S: f32 = 0.15;

/// Vertex alpha of a burning vertex holds its ignition time: `-(1 + t0)` for
/// wood, rock and grass, `-(1 + LEAF_OFFSET + t0)` for leaves (a plain 1.0
/// would be lost in the sign). The foliage shader (`foliage.wgsl`, `burning`)
/// reads it back and consumes the plant against the simulated clock
/// (`tactical.y`, see [`fire_clock`]), so the plant need not be rewritten
/// while it burns.
const LEAF_OFFSET: f32 = 1.0e5;
/// The same for the vertices of a plant's burnt remains, which dissolve in
/// while it burns (`-(1 + REMAINS_OFFSET [+ DEBRIS_OFFSET] + t0)`).
const REMAINS_OFFSET: f32 = 2.0e5;
/// Added to a vertex of the remains that is debris on the ground (ash, fallen
/// wood), which shows last.
const DEBRIS_OFFSET: f32 = 4.0e5;

/// What a burnt vertex shows: charcoal, keeping a trace of the original tint
/// so a burnt conifer stand still reads differently from burnt grass. It
/// continues the colour the shader ends a burning plant on.
fn charcoal(c: [f32; 4]) -> [f32; 4] {
    [0.022 + c[0] * 0.03, 0.019 + c[1] * 0.025, 0.017 + c[2] * 0.025, 0.0]
}

/// Recolour one plant's vertices for its fire state.
fn paint(colors: &mut [[f32; 4]], state: u8, t0: f32) {
    if state == CellFire::Burning as u8 {
        // Alight: colours stay as generated (the shader chars and lights
        // them); only the alpha flags the burn and carries its start.
        for c in colors {
            let leaf = if c[3] > 0.5 { LEAF_OFFSET } else { 0.0 };
            c[3] = -(1.0 + leaf + t0.max(0.0));
        }
    } else {
        for c in colors {
            *c = charcoal(*c);
        }
    }
}

/// Hand the simulated clock to the foliage shader (`fire.y`, with the
/// flaming time in `fire.z`): a burning plant is consumed as a function of
/// it, so the fire keeps eating the canopy between state updates and stands
/// still while the world is paused.
///
/// Every chunk has a material of its own, and only the chunks with a plant
/// alight need the clock: a material written is a bind group made again,
/// which two hundred chunks every frame would be.
pub fn fire_clock(sim: Res<Sim>, veg: Res<Vegetation>, mut materials: ResMut<Assets<FoliageMaterial>>) {
    let now = sim.time_s() as f32;
    for chunk in veg.chunks.iter().filter(|c| c.lit > c.burnt) {
        let Some(m) = materials.get(&chunk.material) else { continue };
        if m.fire.y == now && m.fire.z == PLANT_FLAMING_S {
            continue;
        }
        if let Some(mut m) = materials.get_mut(&chunk.material) {
            m.fire.y = now;
            m.fire.z = PLANT_FLAMING_S;
        }
    }
}

/// `plants`' vertex ranges into the mesh's plant index attribute (the
/// vertices outside every range read no plant). `flag` is [`CHAR_PLANT`] for
/// the burnt-remains layer.
fn tag_plants(mesh: &mut Mesh, verts: usize, ranges: impl Iterator<Item = (u32, u32)>, flag: u32) {
    let mut ids = vec![NO_PLANT; verts];
    for (i, (a, b)) in ranges.enumerate() {
        ids[a as usize..b as usize].fill(i as u32 | flag);
    }
    mesh.insert_attribute(materials::ATTRIBUTE_PLANT, ids);
}

/// Vertex plant index flag of the burnt-remains layer, and of "no plant".
const CHAR_PLANT: u32 = 0x8000_0000;
const NO_PLANT: u32 = 0x7fff_ffff;

/// A chunk's state image: one texel per plant, rows of [`STATE_ROW`], two
/// floats each (fire state, ignition time), all zero = every plant unburnt.
fn state_image(plants: usize) -> Image {
    let rows = plants.div_ceil(STATE_ROW).max(1);
    Image::new(
        Extent3d { width: STATE_ROW as u32, height: rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; STATE_ROW * rows * 8],
        TextureFormat::Rg32Float,
        // the CPU copy stays: it is written to again and again
        RenderAssetUsages::default(),
    )
}

/// A plant's drawn state as the vertex stage reads it (`veg_state.wgsl`).
fn gpu_mode(s: u8, plant: &Plant) -> f32 {
    match s {
        s if s == CellFire::Burning as u8 => 1.0,
        s if s == CellFire::Burnt as u8 => if plant.remains.is_some() { 2.0 } else { 3.0 },
        CLEARED => 4.0,
        _ => 0.0,
    }
}

/// Show each chunk's full models near the camera and its proxies beyond
/// [`Vegetation::lod_m`], with a 10 % band so a chunk on the edge does not
/// flicker while the camera drifts.
impl Vegetation {
    /// Show only this share of the plants built, whatever the fire does to
    /// them: the rest are drawn as [`CLEARED`] (gone), so a weak machine
    /// sheds fill rate without a rebuild. Vertices and memory stay.
    pub fn set_keep(&mut self, k: f32) {
        let k = k.clamp(0.0, 1.0);
        if keep() != k {
            KEEP.store(k.to_bits(), std::sync::atomic::Ordering::Relaxed);
            self.thin_dirty = true;
        }
    }

    pub fn set_lod_m(&mut self, m: f32) {
        if self.lod_m != m {
            self.lod_m = m;
            // the next `lod` pass looks at every chunk again
            self.drawn_at = None;
        }
    }
}

pub fn lod(
    mut commands: Commands,
    mut veg: ResMut<Vegetation>,
    camera: Query<&Transform, (With<Camera3d>, Without<crate::camera::AuxiliaryCamera>)>,
    mut visibility: Query<&mut Visibility>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Ok(eye) = camera.single().map(|t: &Transform| t.translation) else { return };
    let lod_m = veg.lod_m;
    let far_m = (lod_m * FAR_TIER).max(FAR_TIER_MIN_M);
        for chunk in &mut veg.chunks {
        let (lo, hi) = chunk.bounds;
        let d = eye.distance(eye.clamp(lo, hi));
        let near = if chunk.near { d < lod_m * 1.1 } else { d < lod_m };
        // the proxies' own two tiers: the same vertices, fewer triangles
        let far = !near && if chunk.far { d > far_m / 1.1 } else { d > far_m };
        if far != chunk.far {
            if let Some(mut m) = meshes.get_mut(&chunk.low_mesh) {
                if let Some(Indices::U32(idx)) = m.indices_mut() {
                    std::mem::swap(idx, &mut chunk.far_idx);
                    chunk.far = far;
                }
            }
        }
        let full_casts = near && if chunk.full_casts { d < FULL_SHADOW_M * 1.1 } else { d < FULL_SHADOW_M };
        if near == chunk.near && full_casts == chunk.full_casts {
            continue;
        }
        let (near_char, far_char) = match &chunk.char {
            Some(l) => (Some(l.near.entity), Some(l.far.entity)),
            None => (None, None),
        };
        chunk.near = near;
        chunk.full_casts = full_casts;
        // the full models draw the shadow only close up; beyond that the
        // proxies do, from the sun's layer
        for e in [Some(chunk.full), near_char].into_iter().flatten() {
            if full_casts { commands.entity(e).remove::<NotShadowCaster>(); }
            else { commands.entity(e).insert(NotShadowCaster); }
        }
        for (e, on) in [
            (Some(chunk.full), near),
            (Some(chunk.low), !near),
            (near_char, near),
            (far_char, !near),
            (chunk.shadow, near && !full_casts),
        ] {
            if let Some(Ok(mut v)) = e.map(|e| visibility.get_mut(e)) {
                *v = if on { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
    }
}

/// Within this distance the full models cast their own shadow; beyond it the
/// proxies do. The sun's first cascade ends at 80 m and the second at about
/// 300 m, where a texel is a hand's width: a crown's lobes are gone from the
/// shadow map, and its outline is the proxy's.
const FULL_SHADOW_M: f32 = 250.0;

/// Beyond this many times [`Vegetation::lod_m`] (and at least
/// [`FAR_TIER_MIN_M`]) the proxies drop to 12 triangles a crown and a
/// three-sided trunk: a plant there is a few pixels.
const FAR_TIER: f32 = 2.0;
const FAR_TIER_MIN_M: f32 = 2000.0;

/// Fire state of a single plant, from the interpolated front.
fn plant_state(field: &FireField, plant: &Plant, now: f32) -> u8 {
    let Some(arrival) = field.arrival(plant.pos) else {
        return CellFire::Unburnt as u8;
    };
    let age = now - (arrival + plant.lag_s);
    if age < 0.0 {
        CellFire::Unburnt as u8
    } else if age < PLANT_FLAMING_S {
        CellFire::Burning as u8
    } else {
        CellFire::Burnt as u8
    }
}

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
        let plant = |x: f32, y: f32| Plant { pos: Pos { x, y }, lag_s: 0.0, vert_start: 0, vert_end: 0, low_start: 0, low_end: 0, remains: None };
        let ranks: Vec<f32> = (0..4000).map(|i| rank(&plant((i % 80) as f32 * 7.3 + 0.37, (i / 80) as f32 * 11.1 + 0.91))).collect();
        assert!(ranks.iter().all(|r| (0.0..=1.0).contains(r)));
        for keep in [0.2f32, 0.35, 0.6] {
            let shown = ranks.iter().filter(|&&r| r < keep).count() as f32 / ranks.len() as f32;
            assert!((shown - keep).abs() < 0.05, "keep {keep}: {shown}");
        }
        assert_eq!(rank(&plant(10.0, 20.0)), rank(&plant(10.0, 20.0)));
    }

    use super::*;

    /// What fire adds to every variety must build in range of the index
    /// buffer, mark its debris, and stay inside the live plant's box.
    #[test]
    fn burnt_remains_build_for_every_variety() {
        for species in ["pine", "chestnut", "bush", "olive"] {
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
    /// for, and face outward: `Builder::finish` averages face normals, and an
    /// inward crown would be lit from the wrong side.
    #[test]
    fn proxies_are_cheap_and_face_out() {
        for name in ["pine", "chestnut", "bush", "olive"].into_iter().flat_map(|s| crate::models::variety_names(s).iter().copied()) {
            let mut b = Builder::default();
            b.proxy(name, Vec3::ZERO, Vec3::splat(10.0), 0.7, [0.3, 0.4, 0.2], [0.4, 0.2, 0.1]);
            let model = crate::models::live(name).indices.len() / 3;
            let tris = b.indices.len() / 3;
            assert!(tris * 2 <= model, "{name}: {tris} proxy triangles for {model}");

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

    /// The incremental redraw (only changed plants and those still burning, in
    /// place, burnt plants skipped) draws exactly what the old whole-chunk
    /// rebuild drew, after every redraw of a real `medium` fire, and after a
    /// restart.
    #[test]
    fn the_incremental_redraw_equals_the_full_rebuild() {
        use std::time::{Duration, Instant};
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut sim = Sim::new(&data, "medium", 2).unwrap();
        let w = sim.scn.world;
        let mut rng = Rng::seeded(7);
        let (mut plants, mut base, mut low_base, mut pos_base, mut low_pos_base) = (vec![], vec![], vec![], vec![], vec![]);
        let (mut shown, mut hidden, mut cbase, mut ranges) = (vec![], vec![], vec![], vec![]);
        let mut y = 5.0;
        while y < w.height_m {
            let mut x = 5.0;
            while x < w.width_m {
                let (vs, ls, cs) = (base.len() as u32, low_base.len() as u32, shown.len() as u32);
                let remains = (rng.unit() < 0.6).then(|| Remains { name: "pine", base: Vec3::ZERO, scale: Vec3::ONE, yaw: 0.0 });
                for k in 0..4 {
                    let q = [x + k as f32, rng.unit(), y];
                    pos_base.push(q);
                    base.push([rng.unit(), rng.unit(), rng.unit(), if rng.unit() < 0.5 { 1.0 } else { 0.0 }]);
                }
                for k in 0..2 {
                    low_pos_base.push([x + k as f32, rng.unit(), y]);
                    low_base.push([rng.unit(), rng.unit(), rng.unit(), 0.0]);
                }
                if remains.is_some() {
                    for k in 0..3 {
                        shown.push([x + 0.5 * k as f32, 1.0 + rng.unit(), y]);
                        cbase.push([rng.unit(), rng.unit(), rng.unit(), 0.0]);
                    }
                    let first = shown[cs as usize];
                    hidden.extend(std::iter::repeat(first).take(3));
                }
                ranges.push((cs, shown.len() as u32));
                plants.push(Plant {
                    pos: Pos { x, y },
                    lag_s: rng.unit() * 45.0,
                    vert_start: vs,
                    vert_end: vs + 4,
                    low_start: ls,
                    low_end: ls + 2,
                    remains,
                });
                x += 37.0;
            }
            y += 37.0;
        }
        let m = |shown: &Vec<[f32; 3]>, hidden: &Vec<[f32; 3]>| CharMesh {
            mesh: Handle::default(),
            entity: Entity::PLACEHOLDER,
            shown: shown.clone(),
            hidden: hidden.clone(),
            ranges: ranges.clone(),
            colors: cbase.clone(),
        };
        let cm = m(&shown, &hidden);
        // incremental state
        let n = plants.len();
        let (mut drawn, mut lit, mut burnt) = (vec![0u8; n], 0usize, 0usize);
        let (pos, col, lpos, lcol) = (pos_base.clone(), base.clone(), low_pos_base.clone(), low_base.clone());
        let (cpos, ccol) = (hidden.clone(), cbase.clone());
        // reference state
        let mut st = (vec![0u8; n], Duration::ZERO, Duration::ZERO, 0usize);
        let check = |sim: &Sim, restarted: bool, st: &mut (Vec<u8>, Duration, Duration, usize), drawn: &mut Vec<u8>, lit: &mut usize, burnt: &mut usize,
                         bufs: &mut (Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<[f32; 3]>, Vec<[f32; 4]>)| {
            let now = sim.time_s() as f32;
            let field = FireField {
                state: sim.fire.state(),
                arrival: sim.fire.arrival_times(),
                intensity: sim.fire.intensity(),
                cleared: sim.fire.cleared(),
                world: sim.scn.world,
            };
            let scan_cleared = sim.fire.line_cells > 0;
            // old: every plant, then rebuild all if any changed
            let t = Instant::now();
            // (the one deliberate change: a burnt plant stays burnt. The old
            // code let the interpolated arrival drift later as neighbours
            // caught, and a handful of burnt plants flared up again.)
            let mut wanted: Vec<u8> = plants.iter().map(|p| wanted_state(&field, p, now, scan_cleared)).collect();
            if !restarted {
                for (w, &d) in wanted.iter_mut().zip(&st.0) {
                    if d == CellFire::Burnt as u8 {
                        *w = d;
                    }
                }
            }
            let dirty = wanted != st.0;
            let mut r = (pos_base.clone(), base.clone(), low_pos_base.clone(), low_base.clone(), hidden.clone(), cbase.clone());
            if dirty {
                for (i, p) in plants.iter().enumerate() {
                    let t0 = ignition(&field, p);
                    let s = wanted[i];
                    draw_standing(&mut r.0, &mut r.1, &pos_base, &base, (p.vert_start as usize, p.vert_end as usize), s, t0, p);
                    draw_standing(&mut r.2, &mut r.3, &low_pos_base, &low_base, (p.low_start as usize, p.low_end as usize), s, t0, p);
                    if p.remains.is_some() {
                        let (a, b) = ranges[i];
                        draw_char(&mut r.4, &mut r.5, &cm, (a as usize, b as usize), s, t0);
                    }
                }
                st.0 = wanted;
            }
            st.1 += t.elapsed();
            // new
            let t = Instant::now();
            let redraw = plan_redraw(&plants, drawn, lit, burnt, &field, now, scan_cleared, restarted);
            for &(i, s) in &redraw {
                let p = &plants[i];
                let t0 = ignition(&field, p);
                draw_standing(&mut bufs.0, &mut bufs.1, &pos_base, &base, (p.vert_start as usize, p.vert_end as usize), s, t0, p);
                draw_standing(&mut bufs.2, &mut bufs.3, &low_pos_base, &low_base, (p.low_start as usize, p.low_end as usize), s, t0, p);
                if p.remains.is_some() {
                    let (a, b) = ranges[i];
                    draw_char(&mut bufs.4, &mut bufs.5, &cm, (a as usize, b as usize), s, t0);
                }
            }
            st.2 += t.elapsed();
            if dirty {
                st.3 += 1;
            }
            let bad: Vec<_> = (0..plants.len()).filter(|&i| st.0[i] != drawn[i]).take(5).map(|i| (i, st.0[i], drawn[i])).collect();
            assert!(bad.is_empty(), "state mismatches (i, ref, new) at {now}: {bad:?}");
            assert_eq!(&r.0, &bufs.0, "positions at {now}");
            assert_eq!(&r.1, &bufs.1, "colours at {now}");
            assert_eq!(&r.2, &bufs.2, "low positions at {now}");
            assert_eq!(&r.3, &bufs.3, "low colours at {now}");
            assert_eq!(&r.4, &bufs.4, "char positions at {now}");
            assert_eq!(&r.5, &bufs.5, "char colours at {now}");
            assert_eq!(*drawn, st.0);
            assert_eq!(*lit, drawn.iter().filter(|&&d| d != 0).count());
            assert_eq!(*burnt, drawn.iter().filter(|&&d| d == CellFire::Burnt as u8).count());
        };
        let mut bufs = (pos, col, lpos, lcol, cpos, ccol);
        for _ in 0..40 {
            for _ in 0..(300.0 / rocca::STEP_S as f32).max(1.0) as usize {
                if sim.is_over() { break; }
                sim.game.step().unwrap();
            }
            check(&sim, false, &mut st, &mut drawn, &mut lit, &mut burnt, &mut bufs);
        }
        let (redraws, t_old, t_new) = (st.3, st.1, st.2);
        assert!(burnt > 0 && redraws > 3, "the fire never burnt anything: {burnt} {redraws}");
        eprintln!("redraws {redraws}, burnt {burnt}/{n}, old {t_old:?}, new {t_new:?}");
        // restart: a fresh game brings everything back to the generated shape
        let fresh = Sim::new(&data, "medium", 2).unwrap();
        st.0 = drawn.clone();
        check(&fresh, true, &mut st, &mut drawn, &mut lit, &mut burnt, &mut bufs);
        // (the fresh game has its ignition patch, so a few plants are alight)
        assert!(burnt < 100 && lit < 200, "{lit} {burnt}");
    }
}
