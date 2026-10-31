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
//! global index (`materials::ATTRIBUTE_PLANT`, a float), one RGBA8 image for
//! the whole island holds each plant's ignition instant and two flags
//! ([`Texel`]), and the vertex stage (`shaders/veg_state.wgsl`) works out from
//! the simulated clock whether the plant stands, burns, is charcoal or gone,
//! and collapses, recolours and flags it. A chunk is ~80 k triangles and Bevy
//! clones and uploads a modified mesh whole: rewriting two or three of them
//! every 0.15 s was a frame hitch of tens of milliseconds. [`shown`] is the
//! CPU's copy of the shader's decision (2026-10-31: the per-chunk float
//! images, the integer plant attribute and the CPU redraw path are gone).
//!
//! Every chunk is built twice: with the authored models, and with a proxy of
//! a couple of dozen faces per plant (a faceted crown on a three-sided
//! trunk) that stands in for it beyond [`crate::quality::Graphics::lod_m`].
//! A plant there covers a few pixels, and the GPU — which is what limits the
//! frame rate — draws every vegetation triangle once per pass, shadow
//! cascades included. Both meshes burn together. Far off the proxies have a
//! cheaper tier ([`FAR_TIER`]), a third mesh. The level of detail only shows
//! and hides entities: no mesh is rewritten after it is built (2026-10-31).

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
    /// Its texel in the state image as last written ([`Texel`]).
    texel: Texel,
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
    entity: Entity,
}

struct Chunk {
    /// Global index of its first plant in the state image.
    first: u32,
    /// The proxies' farthest tier (see [`FAR_TIER`]): a mesh of its own,
    /// built once, shown instead of `low` beyond the tier's distance. Never a
    /// rewrite of `low_mesh`: swapping its indices as the camera moved
    /// re-uploaded the whole mesh (suspected of killing the iPhone page,
    /// 2026-10-31). `None` when no proxy has a cheaper tier.
    far_low: Option<Entity>,
    /// `far_low` is on show rather than `low`.
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
    /// The burnt remains, once a plant here has caught.
    char: Option<CharLayer>,
    /// How many plants have caught, and how many are burnt (a burnt plant is
    /// final, until a restart).
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
    /// The one material of every plant mesh, and the state image it reads:
    /// one texel per plant of the island (2026-10-31: was one material and
    /// one float image per chunk).
    material: Handle<FoliageMaterial>,
    state: Handle<Image>,
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

/// Shown state of a plant gone from ground a crew has cleared, or thinned
/// away ([`GONE`]): bare earth (the ground itself is tinted by
/// `fire_view`). Above the `CellFire` values.
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


    // One material for every plant mesh: the atlas (`materials::foliage_atlas`)
    // and the island's state image, filled in once every plant is known.
    let atlas = images.add(materials::foliage_atlas());
    let state = images.add(state_image(&[]));
    let material = materials.add(FoliageMaterial::new(state.clone(), atlas));
    // two texels per plant: its fire state ([`Texel`]) and, for an instance,
    // its tint ([`tint_texel`])
    let mut texels: Vec<Texel> = Vec::new();
    let instanced = instanced();
    let mut variety_meshes: std::collections::HashMap<&'static str, Handle<Mesh>> = Default::default();
    let mut inst_tris_per: std::collections::HashMap<&'static str, usize> = Default::default();
    let (mut inst_count, mut inst_tris) = (0usize, 0usize);

    let (rows, cols) = (scn.world.fire_rows, scn.world.fire_cols);
    let chunks_y = rows.div_ceil(CHUNK_CELLS);
    let chunks_x = cols.div_ceil(CHUNK_CELLS);
    let mut chunks = Vec::with_capacity(chunks_x * chunks_y);
    let (mut plant_count, mut tri_count, mut low_tri_count) = (0usize, 0usize, 0usize);
    let (mut low_verts_count, mut far_verts) = (0usize, 0usize);

    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            let mut builder = Lods { instanced, ..Default::default() };
            builder.low.want_far = true;
            let mut plants = Vec::new();
            let mut chunk_insts: Vec<Option<Inst>> = Vec::new();

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
                        let inst_before = builder.inst.len();
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
                            texel: 0,
                        });
                        chunk_insts.push(builder.inst.get(inst_before).cloned());
                    }
                }
            }

            if plants.is_empty() {
                continue;
            }
            plant_count += plants.len();
            tri_count += builder.full.indices.len() / 3;
            low_tri_count += builder.low.indices.len() / 3;

            let bounds = builder.full.positions.iter().copied()
                .chain(chunk_insts.iter().flatten().map(|i| i.base.to_array()))
                .fold(
                    (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                    |(lo, hi), p| (lo.min(Vec3::from(p)), hi.max(Vec3::from(p))),
                );
            let first = (texels.len() / 2) as u32;
            for (p, inst) in plants.iter_mut().zip(&chunk_insts) {
                p.texel = texel(None, p.remains.is_some(), false);
                texels.push(p.texel);
                texels.push(inst.as_ref().map_or(NO_TINT, tint_texel));
            }
            // the plants as instances of their variety's mesh
            for (i, inst) in chunk_insts.iter().enumerate() {
                let Some(inst) = inst else { continue };
                let mesh = variety_meshes
                    .entry(inst.name)
                    .or_insert_with(|| {
                        let mesh = variety_proxy(inst.name);
                        inst_tris_per.insert(inst.name, mesh.indices().map_or(0, |i| i.len() / 3));
                        meshes.add(mesh)
                    })
                    .clone();
                inst_tris += inst_tris_per[inst.name];
                commands.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(inst.base)
                        .with_rotation(Quat::from_rotation_y(inst.yaw))
                        .with_scale(inst.scale),
                    bevy::mesh::MeshTag::new(first + i as u32),
                    TransmittedShadowReceiver,
                ));
                inst_count += 1;
            }
            if builder.low.positions.is_empty() {
                // every plant an instance: no chunk meshes
                chunks.push(Chunk {
                    first,
                    far_low: None,
                    far: false,
                    full: Entity::PLACEHOLDER,
                    low: Entity::PLACEHOLDER,
                    near: false,
                    full_casts: false,
                    shadow: None,
                    bounds,
                    cells: (
                        cy * CHUNK_CELLS,
                        ((cy + 1) * CHUNK_CELLS).min(rows),
                        cx * CHUNK_CELLS,
                        ((cx + 1) * CHUNK_CELLS).min(cols),
                    ),
                    plants,
                    char: None,
                    lit: 0,
                    burnt: 0,
                });
                continue;
            }
            let (verts, low_verts) = (builder.full.positions.len(), builder.low.positions.len());
            low_verts_count += low_verts;
            let (mut low, far_idx) = builder.low.finish_with_far();
            tag_plants(&mut low, low_verts, first, plants.iter().map(|p| (p.low_start, p.low_end)), false);
            // Static on the GPU from here: the burning goes through the state
            // image, the LOD through which entity is on show.
            let far_mesh = (!far_idx.is_empty()).then(|| {
                let mut far = compact(&low, &far_idx);
                far.asset_usage = RenderAssetUsages::RENDER_WORLD;
                far.compress_indices();
                pack(&mut far);
                far_verts += far.count_vertices();
                meshes.add(far)
            });
            low.asset_usage = RenderAssetUsages::RENDER_WORLD;
            low.compress_indices();
            pack(&mut low);
            let low_mesh = meshes.add(low);
            // The browser only draws proxies (lod_m = 0). Share their handle
            // with the hidden near entity instead of uploading a second copy.
            let mesh = if cfg!(target_arch = "wasm32") {
                low_mesh.clone()
            } else {
                let mut full_mesh = builder.full.finish();
                tag_plants(&mut full_mesh, verts, first, plants.iter().map(|p| (p.vert_start, p.vert_end)), false);
                full_mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
                full_mesh.compress_indices();
                pack(&mut full_mesh);
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
            let far_low = far_mesh.map(|m| commands.spawn(((
                Mesh3d(m),
                MeshMaterial3d(material.clone()),
                Visibility::Hidden,
            ), TransmittedShadowReceiver)).id());
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
                first,
                far_low,
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
                lit: 0,
                burnt: 0,
            });
        }
    }

    info!(
        "vegetation: {plant_count} plants in {} chunks, {:.2} M triangles ({:.2} M as distant proxies; proxy vertices {:.2} M, far tier {:.2} M)",
        chunks.len(),
        tri_count as f32 / 1e6,
        low_tri_count as f32 / 1e6,
        low_verts_count as f32 / 1e6,
        far_verts as f32 / 1e6
    );
    if instanced {
        info!(
            "vegetation: {inst_count} plants as instances of {} variety meshes, {:.2} M triangles",
            variety_meshes.len(),
            inst_tris as f32 / 1e6
        );
    }
    // the state image, now that every plant has its index
    let _ = images.insert(&state, state_image(&texels));
    commands.insert_resource(Vegetation { material, state, thin_dirty: gfx.veg_keep < 1.0, chunks, lod_m: gfx.lod_m, drawn_at: None, drawn_cleared: 0 });
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
        // Meadow tufts are drawn by the terrain shader (`shaders/terrain.wgsl`)
        // unless `ROCCA_GRASS_MESH` asks for the old geometry.
        Species::Grass if crate::ground_mask::shader_grass() => return None,
        Species::Grass => grass(out, base, scale, yaw, foliage, rng),
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
    /// Plants are entities of their own ([`instanced`]): they are not built
    /// into the chunk meshes, `inst` records them instead, in order.
    instanced: bool,
    inst: Vec<Inst>,
}

/// A plant drawn as an instance of its variety's mesh ([`instanced`]).
#[derive(Clone)]
struct Inst {
    name: &'static str,
    base: Vec3,
    scale: Vec3,
    yaw: f32,
    /// Linear multipliers of the atlas colours, as `Builder::model` bakes them.
    leaf: [f32; 3],
    wood: f32,
}

/// The browser draws every plant as an entity of its own, an instance of one
/// mesh per variety (Bevy batches them), instead of merged chunk meshes
/// (2026-10-31, the iPhone page dying with the plants drawn). `?istanze=0`
/// brings the merged meshes back. The desktop keeps them: some 100 k plants
/// as entities would cost the CPU every frame (visibility, extraction,
/// uniforms) what the merged chunks cost once, and it was never measured.
fn instanced() -> bool {
    cfg!(target_arch = "wasm32") && !crate::quality::web_off("istanze")
}

impl Lods {
    /// `species` is one of `models::SPECIES`; which of its varieties this
    /// plant is follows from its yaw, so it is the same on every run. The
    /// tints are linear multipliers of the atlas colours (1 = as painted).
    fn model(&mut self, species: &'static str, base: Vec3, scale: Vec3, yaw: f32,
        foliage: [f32; 3], wood: [f32; 3]) {
        let name = crate::models::pick_variety(species, yaw / std::f32::consts::TAU);
        if self.instanced {
            self.inst.push(Inst { name, base, scale, yaw, leaf: foliage, wood: wood[1] });
            return;
        }
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

    fn vertex(&mut self, p: Vec3, c: [f32; 3]) -> u32 {
        self.vertex_a(p, c, 0.0)
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

/// Burn the plants the fire has reached.
///
/// Each plant is tested against the *interpolated* arrival time at its own
/// position plus its own hashed lag, so a stand ignites as a wave crossing it
/// rather than as a cell-shaped block, and a burning cell contains flaring,
/// charred and untouched plants at once — which is what a fire edge looks like.
///
/// What is written is one texel per plant of the island's state image
/// ([`Texel`]): the instant it caught, and whether it is cleared. The vertex
/// stage (`veg_state.wgsl`) works out the rest from the simulated clock: the
/// flames, the charcoal, the remains, the collapse. So a texel is written
/// when a plant ignites, is cleared or comes back, and never again while it
/// burns. A chunk the fire has not reached (no arrival time in its cells or
/// their neighbours, whose times its plants interpolate) is not even looked
/// at, and the plants are looked at at most every [`BURN_REDRAW_S`] of real
/// time.
#[allow(clippy::too_many_arguments)]
pub fn burn(
    mut commands: Commands,
    sim: Res<Sim>,
    time: Res<Time>,
    mut veg: ResMut<Vegetation>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut restarted: MessageReader<crate::sim::SimRestarted>,
    mut since: Local<f32>,
) {
    // `?brucia=0` in the browser: the plants never burn on screen and no
    // remains are built (a switch to find what kills an iPhone's page)
    if crate::quality::web_off("brucia") {
        return;
    }
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

    let (state, material) = (veg.state.clone(), veg.material.clone());
    let mut writes: Vec<(u32, u32)> = Vec::new();
    for chunk in &mut veg.chunks {
        // every plant burnt: nothing here can change until a restart
        if chunk.burnt == chunk.plants.len() && !restarted {
            continue;
        }
        if chunk.lit == 0 && !reached(chunk.cells) && !any_cleared(chunk.cells) && !thin && !restarted {
            continue;
        }
        let (r0, r1, c0, c1) = chunk.cells;
        let scan_cleared = line_cells > 0 && any_cleared((r0.saturating_sub(1), (r1 + 1).min(rows), c0.saturating_sub(1), (c1 + 1).min(cols)));
        let before = writes.len();
        plan(&mut chunk.plants, chunk.first, &mut chunk.lit, &mut chunk.burnt, &field, now, scan_cleared, restarted, &mut writes);
        // (instances have no remains: the browser never built them)
        if chunk.char.is_none() && chunk.lit > 0 && writes.len() > before && chunk.full != Entity::PLACEHOLDER {
            chunk.char = Some(build_char_layer(&mut commands, &mut meshes, &*chunk, &material));
        }
    }
    if writes.is_empty() {
        return;
    }
    // Only the changed texels; the image is small (four bytes a plant) and
    // goes to the GPU into the same texture.
    if let Some(mut image) = images.get_mut(&state) {
        let data = image.data.as_mut().expect("the state image keeps its CPU copy");
        for (i, texel) in writes {
            // two texels a plant: the state, then the tint
            let at = i as usize * 8;
            data[at..at + 4].copy_from_slice(&texel.to_le_bytes());
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

/// Seconds of simulated time at which a plant catches (its own lag included).
fn ignition(field: &FireField, plant: &Plant) -> f32 {
    field.arrival(plant.pos).map_or(0.0, |a| a + plant.lag_s)
}

/// A plant's texel in the state image (`veg_state.wgsl`), Rgba8Unorm read
/// with `textureLoad` and decoded to exact integers: red, green and blue are
/// the 24 bits of its ignition instant in ticks of [`TICK_S`] ([`NEVER`]
/// until it catches), alpha its flags ([`HAS_REMAINS`], [`GONE`]).
type Texel = u32;

/// Resolution of the ignition instant: 2^24 ticks of a quarter second are
/// 48 days of simulated time, far beyond any game (hours), and every tick is
/// an exact `f32` in the shader.
const TICK_S: f32 = 0.25;
/// Ignition instant of a plant that has not caught.
const NEVER: u32 = 0x00FF_FFFF;
/// Flag: the plant has burnt remains (a twig skeleton): burnt, it loses its
/// crown and keeps its wood charred; without, it is all charcoal.
const HAS_REMAINS: u32 = 1 << 24;
/// Flag: the plant is not drawn (cut by a crew, or thinned for the machine's
/// sake, [`Vegetation::set_keep`]); its remains neither.
const GONE: u32 = 1 << 25;

fn texel(t0: Option<f32>, remains: bool, gone: bool) -> Texel {
    let ticks = t0.map_or(NEVER, |t| ((t.max(0.0) / TICK_S).round() as u32).min(NEVER - 1));
    ticks | if remains { HAS_REMAINS } else { 0 } | if gone { GONE } else { 0 }
}

fn texel_t0(t: Texel) -> Option<f32> {
    let ticks = t & NEVER;
    (ticks != NEVER).then(|| ticks as f32 * TICK_S)
}

/// What the vertex stage shows for a texel at `now`, as `plant_apply` in
/// `veg_state.wgsl` decides it: the CPU's reference for the shader and its
/// bookkeeping (which plants are alight, which are burnt for good).
fn shown(t: Texel, now: f32) -> u8 {
    if t & GONE != 0 {
        return CLEARED;
    }
    match texel_t0(t) {
        None => CellFire::Unburnt as u8,
        Some(t0) if now < t0 => CellFire::Unburnt as u8,
        Some(t0) if now - t0 < PLANT_FLAMING_S => CellFire::Burning as u8,
        Some(_) => CellFire::Burnt as u8,
    }
}

/// The texels of a chunk's plants that change at `now` go to `writes`
/// (global plant index, texel); `lit` (plants caught) and `burnt` (burnt for
/// good) are updated. A plant's ignition instant is written once, when it
/// catches, and kept: the shader burns it on from there. Cleared (a cut
/// line) only while unburnt; thinned only while not burnt, and a burnt plant
/// is final until a restart.
#[allow(clippy::too_many_arguments)]
fn plan(
    plants: &mut [Plant],
    first: u32,
    lit: &mut usize,
    burnt: &mut usize,
    field: &FireField,
    now: f32,
    scan_cleared: bool,
    restarted: bool,
    writes: &mut Vec<(u32, Texel)>,
) {
    const BURNT: u8 = CellFire::Burnt as u8;
    const UNBURNT: u8 = CellFire::Unburnt as u8;
    let (mut l, mut b) = (0, 0);
    for (i, plant) in plants.iter_mut().enumerate() {
        let old = if restarted { texel(None, plant.remains.is_some(), false) } else { plant.texel };
        let t0 = texel_t0(old).or_else(|| (plant_state(field, plant, now) != UNBURNT).then(|| ignition(field, plant)));
        let fire = shown(texel(t0, false, false), now);
        let gone = fire != BURNT && rank(plant) >= keep()
            || fire == UNBURNT && scan_cleared && {
                // Thinned, not razed: the closer the plant is to the middle
                // of the cut, the likelier it is gone, decided by its own
                // fixed random number, so the same plants fall on every
                // redraw and a few survive inside the line. The edge is sharp
                // (a narrow ramp around the half-way point).
                let inside = ((field.cleared_fraction(plant.pos) - 0.45) / 0.15).clamp(0.0, 1.0);
                plant.lag_s / 45.0 < CLEARED_SHARE * inside
            };
        let new = texel(t0, plant.remains.is_some(), gone);
        if new != plant.texel || restarted {
            plant.texel = new;
            writes.push((first + i as u32, new));
        }
        l += usize::from(t0.is_some());
        b += usize::from(fire == BURNT);
    }
    (*lit, *burnt) = (l, b);
}

/// Build the burnt remains of every plant of `chunk` that has any: standing
/// for good, the vertex stage shows each plant's from its ignition on.
fn build_char_layer(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    chunk: &Chunk,
    material: &Handle<FoliageMaterial>,
) -> CharLayer {
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
        // No plant here leaves remains (common with the browser's few
        // plants): one triangle of no area, belonging to no plant, so the mesh
        // is never empty. An empty one, spawned as the first plant of the chunk
        // caught fire, killed the page on an iPhone (2026-10-31).
        if builder.positions.is_empty() {
            for _ in 0..3 {
                builder.vertex(chunk.bounds.0, [0.0; 3]);
            }
            builder.indices.extend_from_slice(&[0, 1, 2]);
        }
        let verts = builder.positions.len();
        let mut mesh = builder.finish();
        tag_plants(&mut mesh, verts, chunk.first, ranges.iter().copied(), true);
        mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
        mesh.compress_indices();
        pack(&mut mesh);
        let on = chunk.near != low;
        let entity = commands
            .spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material.clone()),
                if on { Visibility::Inherited } else { Visibility::Hidden },
            ))
            .id();
        if on && !chunk.full_casts { commands.entity(entity).insert(NotShadowCaster); }
        CharMesh { entity }
    };
    CharLayer { near: make(false), far: make(true) }
}

/// Real seconds between two looks at the plants (see [`burn`]).
const BURN_REDRAW_S: f32 = 0.15;

/// Hand the simulated clock to the vegetation's material (`fire.y`, with the
/// flaming time in `fire.z`): every plant's fire state is a function of it
/// and of its ignition instant, so the fire keeps eating the canopy between
/// state updates and stands still while the world is paused. One material
/// for the island: one bind group made again when the clock moves.
pub fn fire_clock(sim: Res<Sim>, veg: Res<Vegetation>, mut materials: ResMut<Assets<FoliageMaterial>>) {
    let now = sim.time_s() as f32;
    let Some(m) = materials.get(&veg.material) else { return };
    if m.fire.y == now && m.fire.z == PLANT_FLAMING_S {
        return;
    }
    if let Some(mut m) = materials.get_mut(&veg.material) {
        m.fire.y = now;
        m.fire.z = PLANT_FLAMING_S;
    }
}

/// The plants' vertex ranges into the mesh's plant attribute, as `f32`
/// (exact up to 2^24; no integer vertex attribute, 2026-10-31): the global
/// index of the plant ([`Chunk::first`] + its place in the chunk), or for
/// the remains layer `-(index + 2)`; -1 on vertices of no plant.
fn tag_plants(mesh: &mut Mesh, verts: usize, first: u32, ranges: impl Iterator<Item = (u32, u32)>, remains: bool) {
    let mut ids = vec![NO_PLANT; verts];
    for (i, (a, b)) in ranges.enumerate() {
        let g = (first + i as u32) as f32;
        ids[a as usize..b as usize].fill(if remains { -(g + 2.0) } else { g });
    }
    mesh.insert_attribute(materials::ATTRIBUTE_PLANT, ids);
}

/// The second texel of a plant: the tint of an instance ([`Inst`]), which
/// the merged meshes bake into their vertex colours. Red, green, blue: the
/// leaf tint / 2; alpha: the wood's. Read when the plant index comes from
/// the mesh tag (`PLANT_TAG` in `veg_state.wgsl`).
fn tint_texel(i: &Inst) -> Texel {
    let b = |t: f32| ((t * 0.5).clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    b(i.leaf[0]) | b(i.leaf[1]) << 8 | b(i.leaf[2]) << 16 | b(i.wood) << 24
}

/// The tint texel of a plant of the merged meshes: unused (1, as built).
const NO_TINT: Texel = 0x8080_8080;

/// A variety's distant proxy as a mesh of its own, standing at the origin
/// unrotated and untinted: the browser's plants are instances of it, placed
/// by their transform, tinted and burnt from the state image by their mesh
/// tag. No plant attribute: the material reads the tag instead.
fn variety_proxy(name: &str) -> Mesh {
    let mut b = Builder::default();
    b.proxy(name, Vec3::ZERO, Vec3::ONE, 0.0, [1.0; 3], [1.0; 3]);
    let mut mesh = b.finish();
    mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
    mesh.compress_indices();
    pack(&mut mesh);
    mesh
}

/// Plant attribute of a vertex of no plant (`NO_PLANT` in `veg_state.wgsl`).
const NO_PLANT: f32 = -1.0;

/// Width of the state image in texels (`ROW` in `veg_state.wgsl`): rows of
/// 1024, two texels a plant, so even the desktop's ~100 k plants are some
/// two hundred rows, far within WebGL2's guaranteed 2048 (the browser's
/// 4–17 k plants: 9–33 rows).
const STATE_ROW: usize = 1024;

/// The island's state image: two texels per plant (state [`Texel`], tint),
/// every plant not caught and shown.
fn state_image(texels: &[Texel]) -> Image {
    let rows = texels.len().div_ceil(STATE_ROW).max(1);
    let mut data = vec![0u8; STATE_ROW * rows * 4];
    for (i, t) in texels.iter().enumerate() {
        data[i * 4..i * 4 + 4].copy_from_slice(&t.to_le_bytes());
    }
    Image::new(
        Extent3d { width: STATE_ROW as u32, height: rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        // plain bytes, not sRGB: decoded to exact integers in the shader
        TextureFormat::Rgba8Unorm,
        // the CPU copy stays: it is written to again
        RenderAssetUsages::default(),
    )
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

/// Only which entity of a chunk is on show changes: every mesh stays as built
/// on the GPU (2026-10-31).
pub fn lod(
    mut commands: Commands,
    mut veg: ResMut<Vegetation>,
    camera: Query<&Transform, (With<Camera3d>, Without<crate::camera::AuxiliaryCamera>)>,
    mut visibility: Query<&mut Visibility>,
) {
    let Ok(eye) = camera.single().map(|t: &Transform| t.translation) else { return };
    let lod_m = veg.lod_m;
    let far_m = (lod_m * FAR_TIER).max(FAR_TIER_MIN_M);
    for chunk in &mut veg.chunks {
        // all instances: no chunk meshes to choose between
        if chunk.full == Entity::PLACEHOLDER {
            continue;
        }
        let (lo, hi) = chunk.bounds;
        let d = eye.distance(eye.clamp(lo, hi));
        let near = if chunk.near { d < lod_m * 1.1 } else { d < lod_m };
        // the proxies' own two tiers, two static meshes
        let far = chunk.far_low.is_some() && !near && if chunk.far { d > far_m / 1.1 } else { d > far_m };
        let full_casts = near && if chunk.full_casts { d < FULL_SHADOW_M * 1.1 } else { d < FULL_SHADOW_M };
        if near == chunk.near && far == chunk.far && full_casts == chunk.full_casts {
            continue;
        }
        chunk.far = far;
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
            (Some(chunk.low), !near && !far),
            (chunk.far_low, !near && far),
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
        let plant = |x: f32, y: f32| Plant { pos: Pos { x, y }, lag_s: 0.0, vert_start: 0, vert_end: 0, low_start: 0, low_end: 0, texel: 0, remains: None };
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

    /// A texel holds its ignition instant to the tick and its flags, and the
    /// sentinel stays apart from every real instant.
    #[test]
    fn texels_keep_the_instant_and_the_flags() {
        for (t0, remains, gone) in [(None, false, false), (Some(0.0), true, false), (Some(1234.25), false, true), (Some(4.0e6), true, true)] {
            let t = texel(t0, remains, gone);
            assert_eq!(texel_t0(t), t0, "{t0:?}");
            assert_eq!(t & HAS_REMAINS != 0, remains);
            assert_eq!(t & GONE != 0, gone);
        }
        assert_eq!(texel_t0(texel(Some(1e12), false, false)), Some((NEVER - 1) as f32 * TICK_S));
        assert_eq!(shown(texel(Some(100.0), false, false), 99.0), CellFire::Unburnt as u8);
        assert_eq!(shown(texel(Some(100.0), false, false), 100.0 + PLANT_FLAMING_S - 1.0), CellFire::Burning as u8);
        assert_eq!(shown(texel(Some(100.0), true, false), 100.0 + PLANT_FLAMING_S), CellFire::Burnt as u8);
        assert_eq!(shown(texel(Some(100.0), true, true), 0.0), CLEARED);
    }

    /// Through a real `medium` fire: what the texels show is what the front
    /// says (a plant catches when the interpolated front and its lag reach
    /// it), a burnt plant never flares up again, a texel is written only when
    /// a plant catches or is cleared, and a restart brings everything back.
    #[test]
    fn the_texels_follow_the_fire() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut sim = Sim::new(&data, "medium", 2).unwrap();
        let w = sim.scn.world;
        let mut rng = Rng::seeded(7);
        let mut plants = Vec::new();
        let mut y = 5.0;
        while y < w.height_m {
            let mut x = 5.0;
            while x < w.width_m {
                let remains = (rng.unit() < 0.6).then(|| Remains { name: "pine_1", base: Vec3::ZERO, scale: Vec3::ONE, yaw: 0.0 });
                let texel = texel(None, remains.is_some(), false);
                plants.push(Plant { pos: Pos { x, y }, lag_s: rng.unit() * 45.0, vert_start: 0, vert_end: 0, low_start: 0, low_end: 0, remains, texel });
                x += 37.0;
            }
            y += 37.0;
        }
        let n = plants.len();
        let (mut lit, mut burnt) = (0, 0);
        let mut seen_burnt = vec![false; n];
        let (mut total_writes, mut mismatches, mut checks) = (0usize, 0usize, 0usize);
        fn field_of(sim: &Sim) -> FireField<'_> { FireField {
            state: sim.fire.state(),
            arrival: sim.fire.arrival_times(),
            intensity: sim.fire.intensity(),
            cleared: sim.fire.cleared(),
            world: sim.scn.world,
        } }
        for _ in 0..40 {
            for _ in 0..(300.0 / rocca::STEP_S as f32).max(1.0) as usize {
                if sim.is_over() { break; }
                sim.game.step().unwrap();
            }
            let now = sim.time_s() as f32;
            let field = field_of(&sim);
            let mut writes = Vec::new();
            plan(&mut plants, 0, &mut lit, &mut burnt, &field, now, sim.fire.line_cells > 0, false, &mut writes);
            total_writes += writes.len();
            for (i, p) in plants.iter().enumerate() {
                let s = shown(p.texel, now);
                if seen_burnt[i] {
                    assert_eq!(s, CellFire::Burnt as u8, "plant {i} flared up again at {now}");
                }
                seen_burnt[i] = s == CellFire::Burnt as u8;
                if s != CLEARED {
                    checks += 1;
                    // the front may drift a little after a plant caught (its
                    // instant is kept); anything else is a mismatch
                    mismatches += usize::from(s != plant_state(&field, p, now) && texel_t0(p.texel).is_none());
                }
            }
            assert_eq!(lit, plants.iter().filter(|p| texel_t0(p.texel).is_some()).count());
        }
        assert!(burnt > 0, "the fire never burnt anything");
        assert_eq!(mismatches, 0, "{mismatches} of {checks} plants not shown as the front says");
        // one write when a plant catches, give or take the cut lines
        assert!(total_writes <= lit + lit / 10 + 50, "{total_writes} writes for {lit} plants caught");
        // a fresh game: every plant back, the ignition patch alight
        let fresh = Sim::new(&data, "medium", 2).unwrap();
        let mut writes = Vec::new();
        plan(&mut plants, 0, &mut lit, &mut burnt, &field_of(&fresh), fresh.time_s() as f32, false, true, &mut writes);
        assert_eq!(writes.len(), n);
        assert!(burnt < 100 && lit < 200, "{lit} {burnt}");
    }
}
