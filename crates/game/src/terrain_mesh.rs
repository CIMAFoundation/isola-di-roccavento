//! Building the terrain mesh from the 5 m render heightfield.
//!
//! The full field is 2048² = 4.19 M vertices, which is far too much for one
//! mesh: it defeats frustum culling and blows past the 16-bit index limit. It
//! is split into chunks instead, each its own entity, so the renderer can cull
//! and the driver gets sensibly sized buffers.
//!
//! The ground colour and the normal are not vertex attributes but two
//! textures, one texel per base sample (`shaders/terrain.wgsl` reads them), so the picture does not depend on how dense the mesh is. That
//! lets every chunk be meshed at three levels (base stride x1, x2, x4: 32 k,
//! 8 k and 2 k triangles) and drawn by distance with a [`VisibilityRange`]
//! (abrupt: the terrain shader does not dither). Each level has a skirt
//! to hide the cracks between chunks at different levels. A ranged entity is
//! never a shadow caster, so the shadows come from one extra entity per chunk
//! on [`SHADOW_LAYER`] (the sun's layer, not the camera's), sharing the x2 mesh.
//!
//! The terrain is **ground**, not vegetation. It used to be painted with the
//! fuel raster's classes, which read as a camouflage patchwork however much
//! the boundaries were warped — the underlying data is 20 m and no amount of
//! colour dithering hides that. Vegetation is now drawn as actual plants (see
//! [`crate::vegetation`]), and this layer paints only what is under them:
//! soil, rock on the steep faces, sand at the shore, sea below the waterline.
//! Along the coast the ground has two materials of its own ([`coast_rock`],
//! [`coast_sand`]): banded limestone on the cliffs, rippled sand on the beaches.
//! Ground colour therefore varies with slope, elevation and noise — geology,
//! not land cover — which is both honest and far better looking.

use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::camera::visibility::{RenderLayers, VisibilityRange};
use scenario::{Cell, Cover, CoverClass, Pos, Scenario};

use crate::field::noise;
use crate::materials::TerrainMaterial;

/// Samples per chunk edge. 128 keeps each chunk at ~16 k vertices.
const CHUNK: usize = 128;

#[derive(Component)]
pub struct TerrainChunk;

/// Static rendered geometry used by the optional native pause playground.
#[derive(Component)]
pub struct PauseSolid;

/// Elevation at or below which a non-burnable fire cell is water: the rule
/// `abm::haven::is_water` uses, so the picture and the model agree.
const SEA_FIRE_DEM_M: f64 = 2.0;

/// A chunk whose ground is all below this is under the (opaque) sea even in
/// the deepest wave trough (`sea::WATER_BASE_Y` 0.8 m less a swell of at most
/// 0.75 m on the 15 m lattice, `shaders/water.wgsl`): it is never seen, so it
/// is not drawn. The island's sea floor is flat at 0 m, and over half the
/// window.
const SUBMERGED_M: f32 = 0.03;

/// Distance, metres, from each fire cell to the nearest water cell: what the
/// coast's materials key off, since a vertex does not know where the sea is.
pub(crate) struct ShoreField {
    cols: usize,
    rows: usize,
    cell: f32,
    height_m: f32,
    d: Vec<f32>,
}

impl ShoreField {
    pub(crate) fn new(scn: &Scenario) -> Self {
        let w = &scn.world;
        let (cols, rows) = (w.fire_cols, w.fire_rows);
        let mut d = vec![f32::INFINITY; cols * rows];
        for r in 0..rows {
            for c in 0..cols {
                let i = r * cols + c;
                if !scn.is_burnable(Cell { row: r, col: c }) && scn.dem.get(i).is_some_and(|e| *e <= SEA_FIRE_DEM_M) {
                    d[i] = 0.0;
                }
            }
        }
        // two-pass chamfer distance (1, sqrt 2 in cells)
        const D2: f32 = std::f32::consts::SQRT_2;
        for r in 0..rows {
            for c in 0..cols {
                let i = r * cols + c;
                let mut v = d[i];
                if c > 0 { v = v.min(d[i - 1] + 1.0); }
                if r > 0 {
                    v = v.min(d[i - cols] + 1.0);
                    if c > 0 { v = v.min(d[i - cols - 1] + D2); }
                    if c + 1 < cols { v = v.min(d[i - cols + 1] + D2); }
                }
                d[i] = v;
            }
        }
        for r in (0..rows).rev() {
            for c in (0..cols).rev() {
                let i = r * cols + c;
                let mut v = d[i];
                if c + 1 < cols { v = v.min(d[i + 1] + 1.0); }
                if r + 1 < rows {
                    v = v.min(d[i + cols] + 1.0);
                    if c + 1 < cols { v = v.min(d[i + cols + 1] + D2); }
                    if c > 0 { v = v.min(d[i + cols - 1] + D2); }
                }
                d[i] = v;
            }
        }
        for v in &mut d {
            *v = if v.is_finite() { *v * w.cellsize } else { 1.0e6 };
        }
        ShoreField { cols, rows, cell: w.cellsize, height_m: w.height_m, d }
    }

    /// Bilinear between cell centres.
    pub(crate) fn at(&self, p: Pos) -> f32 {
        let fx = (p.x / self.cell - 0.5).clamp(0.0, (self.cols - 1) as f32);
        let fy = ((self.height_m - p.y) / self.cell - 0.5).clamp(0.0, (self.rows - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.cols - 1), (y0 + 1).min(self.rows - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let g = |r: usize, c: usize| self.d[r * self.cols + c];
        let top = g(y0, x0) * (1.0 - tx) + g(y0, x1) * tx;
        let bot = g(y1, x0) * (1.0 - tx) + g(y1, x1) * tx;
        top * (1.0 - ty) + bot * ty
    }
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Sea cliffs, which a 5 m heightfield only knows as a smooth ramp: on steep
/// coastal faces the ground is displaced into strata ledges (the same bands
/// [`coast_rock`] paints, so the colour steps and the geometry steps agree)
/// and ridged crags, up to a few metres. Zero on the flat, on the beach, at
/// the foot (below 1.5 m the waterline is untouched) and beyond ~300 m from
/// the sea, so roads, houses and the water mesh meet the same ground as
/// before. A function of position only: chunk edges stay seamless.
pub(crate) fn cliff_elev(scn: &Scenario, shore: &ShoreField, p: Pos, elev: f32) -> f32 {
    if elev < 1.5 || shore.at(p) > COAST_REACH_M {
        return elev;
    }
    let slope_cos = scn.terrain.normal_at(p)[1];
    let steep = smooth((0.93 - slope_cos) / 0.12);
    let near = 1.0 - smooth((shore.at(p) - 60.0) / (COAST_REACH_M - 60.0));
    let k = steep * near * smooth((elev - 1.5) / 3.0);
    if k <= 0.0 {
        return elev;
    }
    // ledges: a staircase in the same strata as `coast_rock`
    let dip = noise(p.x / 160.0, p.y / 160.0, 0x5A) * 26.0 + noise(p.x / 38.0, p.y / 38.0, 0x5B) * 5.0;
    let u = (elev + dip) / 14.0;
    let stair = u.floor() + smooth((u.fract() - 0.25) / 0.5);
    let ledge = (stair - u) * 14.0 * 0.07;
    // crags: ridged noise, so the face has arêtes and gullies, not bumps
    // broad, rounded: the 5 m grid turns anything sharper into sawteeth
    let ridge = noise(p.x / 30.0, p.y / 30.0, 0x81);
    let crag = (ridge - 0.5) * 0.6 + (noise(p.x / 70.0, p.y / 70.0, 0x82) - 0.5) * 2.4;
    let d = k * (ledge + crag);
    // never carve below the foot of the face
    elev + d.max(-(elev - 1.0) * 0.8)
}

/// How far from the sea the cliffs are shaped, metres.
const COAST_REACH_M: f32 = 320.0;

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Limestone cliff: pale and ochre bands with a dark iron seam, whose
/// elevation wanders with noise so the strata dip and fault; vertical streaks
/// where water runs; dark and wet in the spray zone at the foot. The bands are
/// ~14 m thick: thinner than that and a 5 m vertex grid aliases them.
fn coast_rock(elev: f32, p: Pos) -> [f32; 3] {
    const PALE: [f32; 3] = [0.80, 0.76, 0.66];
    const OCHRE: [f32; 3] = [0.70, 0.57, 0.40];
    const GREY: [f32; 3] = [0.55, 0.54, 0.51];
    const IRON: [f32; 3] = [0.36, 0.30, 0.26];
    const WET: [f32; 3] = [0.27, 0.26, 0.25];
    let dip = noise(p.x / 160.0, p.y / 160.0, 0x5A) * 26.0 + noise(p.x / 38.0, p.y / 38.0, 0x5B) * 5.0;
    let t = ((elev + dip) / 14.0).rem_euclid(1.0);
    let band = if t < 0.34 {
        PALE
    } else if t < 0.40 {
        mix3(PALE, OCHRE, smooth((t - 0.34) / 0.06))
    } else if t < 0.62 {
        OCHRE
    } else if t < 0.70 {
        mix3(OCHRE, PALE, smooth((t - 0.62) / 0.08))
    } else if t < 0.86 {
        PALE
    } else if t < 0.92 {
        mix3(PALE, GREY, smooth((t - 0.86) / 0.06))
    } else if t < 0.96 {
        mix3(GREY, IRON, smooth((t - 0.92) / 0.04))
    } else {
        mix3(IRON, PALE, smooth((t - 0.96) / 0.04))
    };
    let streak = 0.94 + 0.12 * noise(p.x / 4.0, p.y / 30.0, 0x5C);
    let grain = 0.94 + 0.12 * noise(p.x / 7.0, p.y / 7.0, 0x5D);
    let lit = [band[0] * streak * grain, band[1] * streak * grain, band[2] * streak * grain];
    mix3(lit, WET, (1.0 - elev / 3.5).clamp(0.0, 1.0) * 0.65)
}

/// Beach sand: dark and wet at the waterline, pale and dry above it, with
/// ripples, a little shell, and tufts of dune grass on the high back beach.
fn coast_sand(elev: f32, p: Pos) -> [f32; 3] {
    let (wet, dry_c, grass) = match real_palette() {
        Some(r) => ([r.sand[0] * 0.72, r.sand[1] * 0.70, r.sand[2] * 0.68], [r.sand[0] * 1.08, r.sand[1] * 1.08, r.sand[2] * 1.06], r.grass),
        None => ([0.58, 0.52, 0.40], [0.91, 0.84, 0.66], [0.60, 0.63, 0.38]),
    };
    let dry = smooth((elev - 0.7) / 1.6);
    let mut c = mix3(wet, dry_c, dry);
    let ripple = (noise(p.x / 3.0, p.y / 8.0, 0x6C) - 0.5) * 0.10 + (noise(p.x / 20.0, p.y / 20.0, 0x6D) - 0.5) * 0.08;
    c = [c[0] + ripple, c[1] + ripple, c[2] + ripple * 0.9];
    // shells: the realistic ground's photo has its own
    if real_palette().is_none() && noise(p.x / 1.7, p.y / 1.7, 0x6E) > 0.92 {
        c = mix3(c, [0.96, 0.95, 0.90], 0.5);
    }
    let tuft = smooth((noise(p.x / 28.0, p.y / 28.0, 0x6F) - 0.52) / 0.2) * smooth((elev - 1.8) / 1.6);
    mix3(c, grass, tuft * 0.55)
}

const PAVING: [f32; 3] = [0.84, 0.81, 0.74];
const GARDEN: [f32; 3] = [0.50, 0.66, 0.36];
/// Cleared ground away from any house (the service areas, verges): mown dry
/// grass, not the lawn green of a garden.
const MEADOW: [f32; 3] = [0.73, 0.69, 0.42];

/// 0..1: how close the nearest house or yard is to fire cell `c`
/// (1 within 20 m, fading out by 60 m). Gardens stay green round the houses;
/// the rest of a cleared patch reads as meadow. Computed once per scenario.
pub(crate) fn built_nearby(scn: &Scenario, c: Cell) -> f32 {
    static NEAR: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let w = &scn.world;
    let near = NEAR.get_or_init(|| {
        let Some(cv) = &scn.cover else { return Vec::new() };
        let (rows, cols) = (w.fire_rows, w.fire_cols);
        // Built cells of the 5 m cover, then the distance to the nearest in
        // fire cells (chessboard, 3 cells reach), by a short sweep.
        let built: Vec<bool> = (0..rows * cols)
            .map(|i| {
                let (r, col) = (i / cols, i % cols);
                let (x0, y0) = (col as f32 * w.cellsize, w.height_m - (r as f32 + 1.0) * w.cellsize);
                (0..4).any(|a| (0..4).any(|b| matches!(cv.at(Pos { x: x0 + (a as f32 + 0.5) * 5.0, y: y0 + (b as f32 + 0.5) * 5.0 }), CoverClass::Building | CoverClass::Yard)))
            })
            .collect();
        (0..rows * cols)
            .map(|i| {
                let (r, col) = ((i / cols) as isize, (i % cols) as isize);
                let mut best = 4.0f32;
                for dr in -3..=3isize {
                    for dc in -3..=3isize {
                        let (rr, cc) = (r + dr, col + dc);
                        if rr >= 0 && cc >= 0 && (rr as usize) < rows && (cc as usize) < cols && built[rr as usize * cols + cc as usize] {
                            best = best.min(dr.abs().max(dc.abs()) as f32);
                        }
                    }
                }
                (1.0 - (best - 1.0) / 2.0).clamp(0.0, 1.0)
            })
            .collect()
    });
    near.get(c.row * w.fire_cols + c.col).copied().unwrap_or(1.0)
}

/// What the 5 m cover says is built at `p`, as a colour and a weight in 0..1:
/// the share of the four cover cells round `p` that are not natural ground.
/// Vertices sit on cell corners, so the weight gives edges one posting soft.
fn built_tint(cover: &Cover, p: Pos) -> ([f32; 3], f32) {
    let h = cover.cell_m * 0.5;
    let (mut sum, mut n) = ([0.0; 3], 0.0);
    for (dx, dy) in [(-h, -h), (h, -h), (-h, h), (h, h)] {
        let c = match cover.at(Pos { x: p.x + dx, y: p.y + dy }) {
            CoverClass::Natural => continue,
            // Tracks are drawn as ribbons too; beneath and beside them, bare
            // compacted earth.
            CoverClass::Track => real_palette().map_or([0.66, 0.58, 0.44], |r| r.track),
            // Farmland is a mesh of parcels over the ground (`fields.rs`).
            CoverClass::Irrigated => continue,
            CoverClass::Water => [0.20, 0.36, 0.44],
            // Paving is drawn per pixel by the terrain shader (`ground_mask`).
            CoverClass::Road | CoverClass::Building | CoverClass::Yard => continue,
        };
        for i in 0..3 {
            sum[i] += c[i];
        }
        n += 1.0;
    }
    if n == 0.0 {
        return (PAVING, 0.0);
    }
    ([sum[0] / n, sum[1] / n, sum[2] / n], n / 4.0)
}

/// The realistic ground's macro colours (2026-11-06, with the scanned
/// plants: `textures::ground_detail`), sRGB like the rest of this file: the
/// mean colours of the photos the terrain shader lays over them, a little
/// graded, so the ground seen from afar is the ground seen from near.
struct RealPalette {
    grass: [f32; 3],
    grass_pale: [f32; 3],
    lawn: [f32; 3],
    leaves: [f32; 3],
    soil: [f32; 3],
    needles: [f32; 3],
    track: [f32; 3],
    sand: [f32; 3],
    rock: [f32; 3],
}

fn real_palette() -> Option<&'static RealPalette> {
    static PALETTE: std::sync::OnceLock<Option<RealPalette>> = std::sync::OnceLock::new();
    PALETTE
        .get_or_init(|| {
            let d = crate::textures::ground_detail()?;
            let layer = |name: &str, grade: [f32; 3]| {
                let k = crate::textures::GROUND_LAYERS.iter().position(|l| *l == name).unwrap_or(0);
                let m = d.means[k];
                [0, 1, 2].map(|c| linear_to_srgb(m[c] * grade[c]))
            };
            Some(RealPalette {
                grass: layer("grass", GRADE_GRASS),
                grass_pale: layer("grass", [GRADE_GRASS[0] * 1.12, GRADE_GRASS[1] * 1.12, GRADE_GRASS[2] * 1.08]),
                lawn: layer("grass", GRADE_LAWN),
                leaves: layer("leaves", GRADE_LEAVES),
                soil: layer("soil", GRADE_SOIL),
                needles: layer("needles", GRADE_NEEDLES),
                track: layer("track", [1.0; 3]),
                sand: layer("sand", GRADE_SAND),
                rock: layer("rock", [0.8; 3]),
            })
        })
        .as_ref()
}

/// The photos' means, graded (linear factors): the photos were taken under
/// other skies; these bring them to a dry Ligurian summer and apart enough to
/// tell grass, macchia, woods and rock at a glance.
const GRADE_GRASS: [f32; 3] = [0.8, 0.76, 0.45];
/// Class 1, short grass (round the houses, but also on open hillsides):
/// the dry grass, a little greener, not a watered lawn.
const GRADE_LAWN: [f32; 3] = [0.74, 0.78, 0.46];
/// The photos are of warmer, redder ground than the grey-brown karst soil.
const GRADE_LEAVES: [f32; 3] = [0.77, 1.09, 1.3];
const GRADE_SOIL: [f32; 3] = [0.75, 0.98, 1.43];
const GRADE_NEEDLES: [f32; 3] = [0.8, 0.93, 1.15];
/// The photo is of a wet grey beach; ours is dry and pale.
const GRADE_SAND: [f32; 3] = [1.9, 2.1, 2.35];

fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// What lies on the open ground at fire cell `c`, in the canopy's code
/// (0 conifer needles, 0.5 broadleaf litter, 1 macchia soil), for the
/// realistic ground's shader where no crown is painted.
fn ground_kind(scn: &Scenario, p: Pos) -> f32 {
    match scn.fuel_at(scn.world.cell_of(p)) {
        4..=6 => 0.5,
        10..=12 => 0.0,
        _ => 1.0,
    }
}

/// Land-cover tint at `p`, bilinear between fire-cell centres so the 20 m raster
/// reads as soft pastel meadows rather than pixels.
fn cover_tint(scn: &Scenario, p: Pos) -> [f32; 3] {
    let w = &scn.world;
    let fx = p.x / w.cellsize - 0.5;
    let fy = (w.height_m - p.y) / w.cellsize - 0.5;
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let tint = |dx: f32, dy: f32| -> [f32; 3] {
        let c = Cell {
            row: (y0 + dy).clamp(0.0, (w.fire_rows - 1) as f32) as usize,
            col: (x0 + dx).clamp(0.0, (w.fire_cols - 1) as f32) as usize,
        };
        if let Some(real) = real_palette() {
            return match scn.fuel_at(c) {
                1 => real.lawn,
                2 => real.grass,
                3 => real.grass_pale,
                4..=6 => real.leaves,
                7..=9 => real.soil,
                10..=12 => real.needles,
                _ if scn.cover.is_some() => mix3(real.grass, real.lawn, built_nearby(scn, c)),
                _ => PAVING,
            };
        }
        match scn.fuel_at(c) {
            // Gardens and lawns round the houses: the green that makes a
            // town read as a town (class 1 is only ever painted there).
            1 => [0.56, 0.74, 0.40],
            // The hue of each group's plants, lighter, so the cover reads
            // where the plants are sparse (12 % in the browser): straw gold
            // (class 3 paler), chestnut green, olive macchia, pine
            // blue-green. Bilinear and warped below, never hard-edged.
            2 => [0.78, 0.68, 0.38],
            3 => [0.86, 0.76, 0.46],
            4 => [0.40, 0.60, 0.26],
            5..=6 => [0.46, 0.60, 0.28],
            7..=9 => [0.44, 0.48, 0.28],
            10..=12 => [0.30, 0.42, 0.33],
            // Non-burnable fire cell. With 5 m cover, what is actually built
            // is painted by `built_tint`; the rest of the cell is the yards
            // and verges round the houses.
            _ if scn.cover.is_some() => mix3(MEADOW, GARDEN, built_nearby(scn, c)),
            // Built-up ground: pale paving rather than bare earth.
            _ => PAVING,
        }
    };
    let mut out = [0.0; 3];
    for (dx, dy, k) in [(0.0, 0.0, (1.0 - tx) * (1.0 - ty)), (1.0, 0.0, tx * (1.0 - ty)), (0.0, 1.0, (1.0 - tx) * ty), (1.0, 1.0, tx * ty)] {
        let c = tint(dx, dy);
        for i in 0..3 {
            out[i] += c[i] * k;
        }
    }
    out
}

/// Ground colour from elevation, steepness and noise.
///
/// `slope_cos` is the vertical component of the surface normal: 1 on the flat,
/// falling toward 0 on a cliff. Ligurian ground is dry pale limestone soil,
/// with bare rock showing wherever it is too steep to hold any.
fn ground_color(scn: &Scenario, sea: &crate::ground_mask::SeaGrid, elev: f32, slope_cos: f32, p: Pos, shore: f32) -> [f32; 3] {
    if elev <= 0.5 && sea.is_sea(p) {
        return [0.07, 0.13, 0.26]; // sea
    }

    // Two scales of variation: broad soil banding, plus a fine grain that
    // keeps the 5 m posting from reading as flat facets.
    let broad = noise(p.x / 140.0, p.y / 140.0, 0x1A);
    let grain = noise(p.x / 11.0, p.y / 11.0, 0x2B);
    let mottle = 0.90 + 0.18 * (0.7 * broad + 0.3 * grain);

    // Grey-olive, not red-brown: this is limestone karst with a thin dry duff
    // over it. Saturated soil colour reads as Mars from altitude, especially
    // under a warm sun.
    // Toy-diorama ground: the land cover is painted here as a pastel tint
    // (a few hundred chunky props stand on top, see `vegetation`), warped by
    // noise so the 20 m fuel cells do not show.
    let warp = Pos {
        x: p.x + 38.0 * (noise(p.x / 55.0, p.y / 55.0, 0x71) - 0.5),
        y: p.y + 38.0 * (noise(p.x / 55.0, p.y / 55.0, 0x72) - 0.5),
    };
    let mut cover = cover_tint(scn, warp);
    // Built cover is sharp at 5 m, so it is sampled where it is, not warped.
    if let Some(cv) = &scn.cover {
        let (built, k) = built_tint(cv, p);
        for i in 0..3 {
            cover[i] = cover[i] * (1.0 - k) + built[i] * k;
        }
    }
    let soil = cover;
    let duff = [cover[0] * 0.92, cover[1] * 0.95, cover[2] * 0.92];
    let (rock, sand) = match real_palette() {
        Some(r) => (r.rock, r.sand),
        None => ([0.46, 0.45, 0.42], [0.60, 0.56, 0.46]),
    };

    // Flatter ground holds more litter and is darker; ridges wash out.
    let soil = {
        let litter = ((slope_cos - 0.90) / 0.10).clamp(0.0, 1.0) * broad;
        [
            soil[0] * (1.0 - litter) + duff[0] * litter,
            soil[1] * (1.0 - litter) + duff[1] * litter,
            soil[2] * (1.0 - litter) + duff[2] * litter,
        ]
    };

    // Rock takes over as the face steepens; sand only within a few metres of
    // sea level, where the beach actually is.
    let rockiness = ((0.86 - slope_cos) / 0.26).clamp(0.0, 1.0);
    let beach = ((6.0 - elev) / 6.0).clamp(0.0, 1.0) * (slope_cos - 0.8).max(0.0) * 5.0;
    let beach = beach.clamp(0.0, 1.0);

    let mut c = [0.0; 3];
    for i in 0..3 {
        let base = soil[i] * (1.0 - rockiness) + rock[i] * rockiness;
        c[i] = (base * (1.0 - beach) + sand[i] * beach) * mottle;
    }

    // The coast has its own materials. The non-burnable shore strip would
    // otherwise take the garden green of the cover tint.
    // Rocky scrub along the cliff edge, then bare banded limestone where it
    // is steep; sand where it is low and flat.
    let strip = 1.0 - smooth((shore - 40.0) / 70.0);
    let scrub = match real_palette() {
        Some(r) => mix3(r.soil, r.rock, 0.5),
        None => [0.62, 0.58, 0.46],
    };
    c = mix3(c, [scrub[0] * mottle, scrub[1] * mottle, scrub[2] * mottle], strip * smooth((elev - 5.0) / 4.0));
    let cliff = smooth((0.95 - slope_cos) / 0.14) * (1.0 - smooth((shore - 90.0) / 280.0));
    let mut cliff_rgb = coast_rock(elev, p);
    if real_palette().is_some() {
        // the realistic ground's limestone: the photo's grey, not plaster
        cliff_rgb = cliff_rgb.map(|v| v * 0.72);
    }
    c = mix3(c, cliff_rgb, cliff);
    let flat = smooth((slope_cos - 0.88) / 0.08);
    let low = 1.0 - smooth((elev - 5.0) / 5.0);
    let sand_w = flat * low * (1.0 - smooth((shore - 90.0) / 80.0));
    mix3(c, coast_sand(elev, p), sand_w)
}


/// From where to where (metres from the eye) the canopy on the ground turns
/// from the shade under the real plants to the crowns' colour. The browser
/// draws few plants, so its painted canopy comes closer.
fn canopy_fade(gfx: &crate::quality::Graphics) -> [f32; 2] {
    if gfx.vegetation <= 0.15 { [80.0, 320.0] } else { [300.0, 900.0] }
}

/// Layer of the terrain's shadow casters: the sun sees it (it is on the sun's
/// layers, see `main.rs`), the camera does not. A [`VisibilityRange`] entity is
/// never queued as a shadow caster (the light's views are not in the range
/// table), so the shadows come from one extra entity per chunk that shares the
/// middle level's mesh.
pub const SHADOW_LAYER: usize = 1;

/// Distance from the camera, metres, to a chunk's centre beyond which the
/// terrain is drawn with twice, and then four times, the base stride (the base
/// is `Graphics::terrain_stride`). The chunk is 640 m wide, so its near edge is
/// up to ~450 m closer than its centre. The switch is abrupt: the colour and
/// the normal come from textures, so only the silhouette changes.
pub const LOD1_M: f32 = 1100.0;
pub const LOD2_M: f32 = 2600.0;

/// Mesh strides per level, relative to the base stride.
const LEVELS: [usize; 3] = [1, 2, 4];

/// Skirt depth as a fraction of the level's posting: hides the cracks where
/// neighbouring chunks are drawn at different strides.
const SKIRT: f32 = 0.6;

pub(crate) fn f16_bits(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let mant = b & 0x7f_ffff;
    if exp >= 31 {
        sign | 0x7c00
    } else if exp <= 0 {
        if exp < -10 {
            return sign;
        }
        let m = mant | 0x80_0000;
        let shift = (14 - exp) as u32;
        sign | ((m >> shift) + ((m >> (shift - 1)) & 1)) as u16
    } else {
        sign.wrapping_add(((exp as u32) << 10) as u16).wrapping_add(((mant >> 13) + ((mant >> 12) & 1)) as u16)
    }
}

fn ground_texture(w: usize, h: usize, data: Vec<u8>, format: TextureFormat) -> Image {
    let mut image = Image::new(
        Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// An RGBA8 ground texture with its mip chain, box-filtered (only the alpha,
/// the canopy's cover, is read from the mips: no sRGB care needed).
fn with_mips(mut image: Image, w: usize, h: usize) -> Image {
    let mut data = image.data.take().unwrap_or_default();
    let (mut lw, mut lh, mut levels, mut offset) = (w, h, 1, 0);
    while lw > 1 || lh > 1 {
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        data.reserve(nw * nh * 4);
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let at = |xx: usize, yy: usize| data[offset + (yy.min(lh - 1) * lw + xx.min(lw - 1)) * 4 + c] as u32;
                    let s = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                    data.push(((s + 2) / 4) as u8);
                }
            }
        }
        offset += lw * lh * 4;
        (lw, lh) = (nw, nh);
        levels += 1;
    }
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    if let ImageSampler::Descriptor(d) = &mut image.sampler {
        d.mipmap_filter = ImageFilterMode::Linear;
    }
    image
}

/// One chunk at one level: samples `c0..=c0+ncx` by `r0..=r0+ncy` of the base
/// grid, every `f`-th, the last one always the chunk's edge; plus a skirt.
#[allow(clippy::too_many_arguments)]
fn level_mesh(
    grid: &Grid,
    (c0, r0): (usize, usize),
    (ncx, ncy): (usize, usize),
    f: usize,
    origin: [f32; 2],
    height_m: f32,
) -> Mesh {
    let xs: Vec<usize> = (0..=ncx.div_ceil(f)).map(|c| c0 + (c * f).min(ncx)).collect();
    let ys: Vec<usize> = (0..=ncy.div_ceil(f)).map(|r| r0 + (r * f).min(ncy)).collect();
    let (nvx, nvy) = (xs.len(), ys.len());
    let mut positions = Vec::with_capacity(nvx * nvy + 2 * (nvx + nvy));
    let mut normals = Vec::with_capacity(positions.capacity());
    for &r in &ys {
        for &c in &xs {
            let i = r * grid.cols + c;
            positions.push([c as f32 * grid.posting - origin[0], grid.elev[i], r as f32 * grid.posting - height_m - origin[1]]);
            normals.push(grid.normal[i]);
        }
    }
    let mut indices = Vec::with_capacity((nvx - 1) * (nvy - 1) * 6 + 12 * (nvx + nvy));
    for r in 0..nvy - 1 {
        for c in 0..nvx - 1 {
            let i = (r * nvx + c) as u32;
            let (right, down) = (i + 1, i + nvx as u32);
            indices.extend_from_slice(&[i, down, right, right, down, down + 1]);
        }
    }
    // skirt: a copy of each edge vertex, dropped; both windings, so it shows
    // from whichever side the gap is seen
    let depth = SKIRT * f as f32 * grid.posting;
    let edges: [Vec<u32>; 4] = [
        (0..nvx).map(|c| c as u32).collect(),
        (0..nvx).map(|c| ((nvy - 1) * nvx + c) as u32).collect(),
        (0..nvy).map(|r| (r * nvx) as u32).collect(),
        (0..nvy).map(|r| (r * nvx + nvx - 1) as u32).collect(),
    ];
    for edge in edges {
        let base = positions.len() as u32;
        for &v in &edge {
            let mut p = positions[v as usize];
            p[1] -= depth;
            let n = normals[v as usize];
            positions.push(p);
            normals.push(n);
        }
        for k in 0..edge.len() as u32 - 1 {
            let (a, b, c, d) = (edge[k as usize], edge[k as usize + 1], base + k, base + k + 1);
            indices.extend_from_slice(&[a, b, c, c, b, d, a, c, b, c, d, b]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        // Static geometry: release the CPU copy after GPU upload on
        // native too. Height queries and Mario's floor use the terrain
        // data directly, not this rendered mesh.
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh.compress_indices();
    mesh
}

/// Whether the terrain is an adaptive mesh (`terrain_tin`); `ROCCA_TIN=0`
/// draws the regular grid, for comparisons.
pub fn tin_enabled() -> bool {
    std::env::var("ROCCA_TIN").map_or(true, |v| v != "0")
}

/// Tolerance of each level's adaptive mesh, metres: no DEM sample is farther
/// from the surface. The browser's single level (base stride 4) is coarser.
const TIN_TOL_M: [f32; 3] = [0.2, 0.8, 3.0];
const TIN_TOL_COARSE_M: f32 = 2.0;
/// Spacing of the kerb vertices by level, metres (none on the farthest).
const KERB_STEP_M: [f32; 3] = [4.0, 8.0, 0.0];
const KERB_STEP_COARSE_M: f32 = 10.0;
/// DEM samples per refinement sample in the browser (1: every 5 m sample).
const TIN_CANDIDATES_COARSE: usize = 2;

/// Both edges of every road's level bench (`scenario::grade`), every `step`
/// metres, as vertices on the shaped ground; in base-grid units.
fn kerbs(scn: &Scenario, shore: &ShoreField, posting: f32, step: f32) -> Vec<crate::terrain_tin::Fixed> {
    let t = &scn.terrain;
    let Some(grade) = &t.grade else { return Vec::new() };
    let mut out = Vec::new();
    for road in &scn.vectors.roads {
        if !(road.drivable || road.track) || road.line.len() < 2 {
            continue;
        }
        let flat = scenario::grade::flat_half_width(road.drivable);
        // resample the centreline
        let mut pts: Vec<[f32; 2]> = vec![road.line[0]];
        for w in road.line.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let n = (len / step).ceil().max(1.0) as usize;
            for k in 1..=n {
                let u = k as f32 / n as f32;
                pts.push([a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u]);
            }
        }
        for i in 0..pts.len() {
            let (a, b) = (pts[i.saturating_sub(1)], pts[(i + 1).min(pts.len() - 1)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-3 {
                continue;
            }
            for side in [-1.0f32, 1.0] {
                let p = [pts[i][0] - dy / len * flat * side, pts[i][1] + dx / len * flat * side];
                // not inside another stretch's bench (junctions, the inside of bends)
                if grade.near(p).iter().any(|&k| {
                    let s = &grade.segs[k as usize];
                    s.nearest(p).0 < s.flat - 0.4
                }) {
                    continue;
                }
                let q = Pos { x: p[0], y: p[1] };
                let z = cliff_elev(scn, shore, q, t.height_at(q));
                let n = t.normal_at(q);
                out.push(crate::terrain_tin::Fixed { c: p[0] / posting, r: (t.height_m - p[1]) / posting, z, n: [n[0], n[1], -n[2]] });
            }
        }
    }
    out
}

/// Every chunk's adaptive mesh at every level, keyed by (chunk x, chunk y,
/// level index); natively on all cores.
///
/// The mesh is refined against the full-resolution DEM (5 m) whatever the
/// base stride: with a coarser base (the browser's 20 m) the grid saw the
/// ground only every fourth sample, and plants, engines and houses, which
/// stand on `height_at`, floated or sank by metres (3 m at the 90th
/// percentile, test `coarse_terrain_error`). The chunk edges stay on the base
/// grid, so neighbouring chunks still share them.
fn tin_geometry(
    scn: &Scenario,
    shore: &ShoreField,
    grid: &Grid,
    rows: usize,
    (chunks_x, chunks_y): (usize, usize),
    levels: &[usize],
    stride: usize,
    (coarse_tol, coarse_kerb, coarse_cand): (f32, f32, usize),
) -> std::collections::HashMap<(usize, usize, usize), crate::terrain_tin::ChunkGeometry> {
    let t = &scn.terrain;
    let cols = grid.cols;
    let coarse = levels.len() == 1;
    // `s`: base-grid samples per refinement sample (the stride, or the stride
    // over the browser's candidate spacing)
    let cand = if stride == 1 { 1 } else { coarse_cand.clamp(1, stride) };
    let s = stride / cand;
    let fine_posting = t.posting * cand as f32;
    // kerbs per level, in full-resolution samples, binned by chunk
    let mut fixed: Vec<std::collections::HashMap<(usize, usize), Vec<crate::terrain_tin::Fixed>>> = Vec::new();
    for k in 0..levels.len() {
        let step = if coarse { coarse_kerb } else { KERB_STEP_M[k] };
        let mut bins: std::collections::HashMap<(usize, usize), Vec<_>> = Default::default();
        if step > 0.0 {
            for p in kerbs(scn, shore, fine_posting, step) {
                let span = (CHUNK * s) as f32;
                let (cx, cy) = ((p.c / span).floor() as usize, (p.r / span).floor() as usize);
                if cx >= chunks_x || cy >= chunks_y {
                    continue;
                }
                let (c0, r0) = (cx * CHUNK, cy * CHUNK);
                let (ncx, ncy) = (CHUNK.min(cols - 1 - c0), CHUNK.min(rows - 1 - r0));
                let (lo_c, hi_c, lo_r, hi_r) = ((c0 * s) as f32, ((c0 + ncx) * s) as f32, (r0 * s) as f32, ((r0 + ncy) * s) as f32);
                // strictly inside, clear of the shared edge
                let m = 0.3;
                if p.c > lo_c + m && p.c < hi_c - m && p.r > lo_r + m && p.r < hi_r - m {
                    bins.entry((cx, cy)).or_default().push(p);
                }
            }
        }
        fixed.push(bins);
    }
    let mut jobs = Vec::new();
    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            let (c0, r0) = (cx * CHUNK, cy * CHUNK);
            let (ncx, ncy) = (CHUNK.min(cols - 1 - c0), CHUNK.min(rows - 1 - r0));
            if ncx < 1 || ncy < 1 {
                continue;
            }
            let under = (r0..=r0 + ncy).all(|r| (c0..=c0 + ncx).all(|c| grid.elev[r * cols + c] < SUBMERGED_M));
            let used = if under { 1 } else { levels.len() };
            for k in 0..used {
                jobs.push((cx, cy, k));
            }
        }
    }
    let full = crate::terrain_tin::GridView { cols, rows, c_off: 0, r_off: 0, posting: grid.posting, elev: &grid.elev, normal: Some(&grid.normal) };
    let build = |&(cx, cy, k): &(usize, usize, usize)| {
        let (c0, r0) = (cx * CHUNK, cy * CHUNK);
        let (ncx, ncy) = (CHUNK.min(cols - 1 - c0), CHUNK.min(rows - 1 - r0));
        let origin = [(c0 as f32 + ncx as f32 * 0.5) * grid.posting, (r0 as f32 + ncy as f32 * 0.5) * grid.posting - t.height_m];
        let f = levels[k];
        let tol = if coarse { coarse_tol } else { TIN_TOL_M[k] };
        let kerb = fixed[k].get(&(cx, cy)).map_or(&[][..], |v| &v[..]);
        let skirt = SKIRT * f as f32 * grid.posting;
        let g = if s == 1 {
            crate::terrain_tin::chunk(&full, (c0, r0), (ncx, ncy), f, kerb, tol, origin, t.height_m, skirt)
        } else {
            // the chunk's window of the full-resolution DEM, shaped as the grid
            let (fc0, fr0, w, h) = (c0 * s, r0 * s, ncx * s + 1, ncy * s + 1);
            let mut elev = Vec::with_capacity(w * h);
            for r in fr0..fr0 + h {
                for c in fc0..fc0 + w {
                    let p = Pos { x: c as f32 * fine_posting, y: t.height_m - r as f32 * fine_posting };
                    let (dc, dr) = ((c * cand).min(t.cols - 1), (r * cand).min(t.rows - 1));
                    elev.push(cliff_elev(scn, shore, p, t.elev[dr * t.cols + dc]));
                }
            }
            let view = crate::terrain_tin::GridView { cols: w, rows: h, c_off: fc0, r_off: fr0, posting: fine_posting, elev: &elev, normal: None };
            crate::terrain_tin::chunk(&view, (fc0, fr0), (ncx * s, ncy * s), f * s, kerb, tol, origin, t.height_m, skirt)
        };
        ((cx, cy, k), g)
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let workers = match TIN_WORKERS.load(std::sync::atomic::Ordering::Relaxed) {
            0 => std::thread::available_parallelism().map_or(4, |n| n.get()),
            n => n,
        };
        let out = std::sync::Mutex::new(std::collections::HashMap::new());
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(job) = jobs.get(i) else { break };
                    let (key, g) = build(job);
                    out.lock().unwrap().insert(key, g);
                });
            }
        });
        out.into_inner().unwrap()
    }
    #[cfg(target_arch = "wasm32")]
    {
        jobs.iter().map(build).collect()
    }
}

/// Threads building the adaptive meshes (0: all cores). The tests set 1 to
/// time what the browser does on its one thread.
#[cfg(not(target_arch = "wasm32"))]
static TIN_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn geometry_mesh(g: crate::terrain_tin::ChunkGeometry) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, g.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, g.normals);
    mesh.insert_indices(Indices::U32(g.indices));
    mesh.compress_indices();
    mesh
}

/// The shaped heightfield and its normals at the base stride.
struct Grid {
    cols: usize,
    posting: f32,
    elev: Vec<f32>,
    /// In Bevy's frame (z flipped).
    normal: Vec<[f32; 3]>,
}

pub fn build(
    scn: &Scenario,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<TerrainMaterial>,
    images: &mut Assets<Image>,
    gfx: &crate::quality::Graphics,
) {
    // Terrain samples per base mesh vertex (`Graphics::terrain_stride`): every
    // sample natively; in the browser every fourth (20 m), which WebGL can
    // draw. The data (and so the agents and every height lookup) stay at full
    // resolution. The colour and normal textures have one texel per base
    // vertex; natively, chunks farther away are meshed at 2x and 4x that
    // stride (`LOD1_M`, `LOD2_M`) and read the same textures.
    let stride = gfx.terrain_stride.max(1);
    let t = &scn.terrain;
    let mut mask = crate::ground_mask::build(scn, gfx);
    // beyond this distance from the eye there are no real blades: the terrain
    // shader adds a grain there so the meadow keeps its contrast (0 = no blades)
    mask.rect.w = if gfx.cover_m > 0.0 { crate::near_cover::BANDS_M[crate::near_cover::BANDS_M.len() - 1] } else { 0.0 };
    let _ = images.insert(&crate::ground_mask::GRASS_MAP, crate::ground_mask::grass_map(scn));
    let _ = images.insert(&crate::fire_ahead::FRONT_MAP, crate::fire_ahead::front_image(scn.world.fire_cols, scn.world.fire_rows));

    // the base grid: every `stride`-th sample
    let (cols, rows, posting) = ((t.cols - 1) / stride + 1, (t.rows - 1) / stride + 1, t.posting * stride as f32);
    let shore = ShoreField::new(scn);
    let sea = mask.sea;

    // Per sample: shaped height, normal and ground colour. The colour and the
    // normal go to textures, so the look does not depend on the mesh density.
    let mut grid = Grid { cols, posting, elev: Vec::with_capacity(cols * rows), normal: Vec::with_capacity(cols * rows) };
    let mut colour_px: Vec<u8> = Vec::with_capacity(cols * rows * 4);
    let mut normal_px: Vec<u8> = Vec::with_capacity(cols * rows * 8);
    for r in 0..rows {
        for c in 0..cols {
            let gx = c as f32 * posting;
            // row 0 is the north edge; world +y is north
            let gy = t.height_m - r as f32 * posting;
            let p = Pos { x: gx, y: gy };
            let raw = t.elev[(r * stride).min(t.rows - 1) * t.cols + (c * stride).min(t.cols - 1)];
            let shore_d = shore.at(p);
            let elev = cliff_elev(scn, &shore, p, raw);
            // Deliberately the plain heightfield normal, not a
            // noise-perturbed one: a jittered normal feeds
            // straight into the directional light's shadow-map bias,
            // and at a jitter scale finer than the shadow map texel
            // it produces exactly the blotchy close-range
            // self-shadowing this comment is here to stop someone
            // reintroducing (it shipped once, briefly).
            let n = if shore_d > COAST_REACH_M {
                t.normal_at(p)
            } else {
                // the same formula as `normal_at`, over the shaped ground
                let d = posting;
                let h = |q: Pos| cliff_elev(scn, &shore, q, t.height_at(q));
                let (hl, hr) = (h(Pos { x: p.x - d, ..p }), h(Pos { x: p.x + d, ..p }));
                let (hd, hu) = (h(Pos { y: p.y - d, ..p }), h(Pos { y: p.y + d, ..p }));
                let (nx, ny, nz) = (hl - hr, 2.0 * d, hd - hu);
                let len = (nx * nx + ny * ny + nz * nz).sqrt();
                [nx / len, ny / len, nz / len]
            };
            let n = [n[0], n[1], -n[2]];
            grid.elev.push(elev);
            grid.normal.push(n);
            let col = ground_color(scn, &sea, elev, n[1], p, shore_d);
            colour_px.extend(col.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
            colour_px.push(255);
        }
    }
    // The canopy (`vegetation::canopy`): its cover in the colour's alpha, its
    // kind and brightness beside the normal's x and z (y is rebuilt, it is
    // always up), so the terrain needs no texture more (WebGL2 counts them).
    let started = bevy::platform::time::Instant::now();
    let mut canopy = crate::vegetation::canopy::canopy(scn, cols, rows, posting);
    // `ROCCA_CANOPY=0`: the bare ground, for comparisons
    if std::env::var("ROCCA_CANOPY").is_ok_and(|v| v == "0") {
        canopy.cover.fill(0.0);
    }
    info!("terrain: canopy in {:.2} s", started.elapsed().as_secs_f32());
    // The realistic ground (`textures::ground_detail`): where no crown is
    // painted, the canopy's kind says what lies on the ground instead
    // (needles, leaves or the macchia's soil), for the photo layers.
    if real_palette().is_some() {
        for (i, k) in canopy.kind.iter_mut().enumerate() {
            if canopy.cover[i] <= 0.0 {
                let p = Pos { x: (i % cols) as f32 * posting, y: t.height_m - (i / cols) as f32 * posting };
                *k = ground_kind(scn, p);
            }
        }
    }
    for (i, n) in grid.normal.iter().enumerate() {
        colour_px[i * 4 + 3] = (canopy.cover[i].clamp(0.0, 1.0) * 255.0).round() as u8;
        for v in [n[0], n[2], canopy.kind[i], canopy.light[i]] {
            normal_px.extend_from_slice(&f16_bits(v).to_le_bytes());
        }
    }
    let colour_tex = images.add(if crate::vegetation::canopy::forest_shade() {
        // with its mips: the canopy's cover averaged over 2^k samples, read by
        // the terrain shader as the forest's occlusion (`forest_shade`)
        with_mips(ground_texture(cols, rows, colour_px, TextureFormat::Rgba8UnormSrgb), cols, rows)
    } else {
        ground_texture(cols, rows, colour_px, TextureFormat::Rgba8UnormSrgb)
    });
    let normal_tex = images.add(ground_texture(cols, rows, normal_px, TextureFormat::Rgba16Float));
    // world metres to texel-centre uv (`terrain_uv` in the shader)
    let tex_rect = Vec4::new(
        1.0 / (posting * cols as f32),
        1.0 / (posting * rows as f32),
        0.5 / cols as f32,
        t.height_m / (posting * rows as f32) + 0.5 / rows as f32,
    );

    let _ = images.insert(&crate::ground_mask::ROAD_MASK, mask.image);
    crate::fields::insert_soil(images);
    let mut terrain_material = TerrainMaterial::new(
        mask.rect,
        tex_rect,
        crate::ground_mask::ROAD_MASK,
        crate::ground_mask::GRASS_MAP,
        colour_tex,
        normal_tex,
        images.add(crate::textures::ground_detail_image()),
    );
    terrain_material.set_canopy(canopy.colours, canopy_fade(gfx));
    terrain_material.set_forest_shade(if crate::vegetation::canopy::forest_shade() { 1.0 } else { 0.0 }, posting);
    let material = materials.add(terrain_material);

    let levels: &[usize] = if stride >= 4 { &LEVELS[..1] } else { &LEVELS };
    // distances at which each level hands over to the next
    let cuts = [LOD1_M, LOD2_M];
    let (chunks_x, chunks_y) = ((cols - 1).div_ceil(CHUNK), (rows - 1).div_ceil(CHUNK));
    let (mut count, mut submerged, mut tris) = (0, 0, [0usize; 3]);

    // the adaptive meshes, all chunks and levels at once (`terrain_tin`)
    let tin = tin_enabled();
    let started = bevy::platform::time::Instant::now();
    let mut tin_meshes = if tin {
        tin_geometry(scn, &shore, &grid, rows, (chunks_x, chunks_y), levels, stride, (TIN_TOL_COARSE_M, KERB_STEP_COARSE_M, TIN_CANDIDATES_COARSE))
    } else {
        Default::default()
    };
    if tin {
        info!("terrain: adaptive meshes in {:.2} s", started.elapsed().as_secs_f32());
    }

    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            let (c0, r0) = (cx * CHUNK, cy * CHUNK);
            // neighbouring chunks share an edge sample and leave no seam
            let (ncx, ncy) = (CHUNK.min(cols - 1 - c0), CHUNK.min(rows - 1 - r0));
            if ncx < 1 || ncy < 1 {
                continue;
            }
            // each level's entity sits at the chunk's centre: that is where
            // its visibility range measures from
            let origin = [(c0 as f32 + ncx as f32 * 0.5) * posting, (r0 as f32 + ncy as f32 * 0.5) * posting - t.height_m];
            let centre = Vec3::new(origin[0], 0.0, origin[1]);
            let under = (r0..=r0 + ncy).all(|r| (c0..=c0 + ncx).all(|c| grid.elev[r * cols + c] < SUBMERGED_M));
            submerged += usize::from(under);
            // a hidden chunk keeps only the finest level
            let used = if under { &levels[..1] } else { levels };
            for (k, &f) in used.iter().enumerate() {
                let mesh = match tin_meshes.remove(&(cx, cy, k)) {
                    Some(g) => geometry_mesh(g),
                    None => level_mesh(&grid, (c0, r0), (ncx, ncy), f, origin, t.height_m),
                };
                tris[k] += mesh.indices().map_or(0, |i| i.len() / 3);
                let mesh = meshes.add(mesh);
                let mut chunk = commands.spawn((
                    (
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(centre),
                        // still spawned: Mario swims over it, and needs a floor
                        if under { Visibility::Hidden } else { Visibility::Inherited },
                    ),
                    TerrainChunk,
                    PauseSolid,
                ));
                if used.len() > 1 {
                    let near = if k == 0 { 0.0 } else { cuts[k - 1] };
                    let far = cuts.get(k).copied().unwrap_or(1.0e9);
                    chunk.insert((VisibilityRange::abrupt(near, far), bevy::light::NotShadowCaster));
                } else if !gfx.terrain_shadows {
                    chunk.insert(bevy::light::NotShadowCaster);
                }
                // the shadows come from the middle level, on the sun's layer only
                if used.len() > 1 && k == 1 {
                    let mut shadow = commands.spawn((
                        (
                            Mesh3d(mesh),
                            MeshMaterial3d(material.clone()),
                            Transform::from_translation(centre),
                        ),
                        TerrainChunk,
                        RenderLayers::layer(SHADOW_LAYER),
                    ));
                    if !gfx.terrain_shadows {
                        shadow.insert(bevy::light::NotShadowCaster);
                    }
                }
            }
            count += 1;
        }
    }

    info!(
        "terrain: {count} chunks ({submerged} under the sea, not drawn), {:.2} M samples @ {} m posting, levels {:?} x base stride {stride}, triangles per level {:?}",
        (t.rows * t.cols) as f32 / 1e6,
        t.posting,
        &levels,
        tris
    );
}

/// Centre of a fire cell, lifted onto the terrain.
pub fn cell_ground(scn: &Scenario, c: Cell) -> (Pos, f32) {
    let p = scn.world.centre_of(c);
    (p, scn.terrain.height_at(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How far the browser's terrain (base stride 4) sits from `height_at`,
    /// where plants, engines and houses stand: the old 20 m grid against the
    /// adaptive mesh at a few tolerances. A table, not a check:
    /// `cargo test --release -p game coarse_terrain_error -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn coarse_terrain_error() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let Ok(scn) = Scenario::load(&dir) else { return };
        let shore = ShoreField::new(&scn);
        let t = &scn.terrain;
        let stride = 4;
        let shaped = |p: Pos| cliff_elev(&scn, &shore, p, t.height_at(p));
        let (cols, rows, posting) = ((t.cols - 1) / stride + 1, (t.rows - 1) / stride + 1, t.posting * stride as f32);
        let mut elev = vec![0.0; cols * rows];
        let mut normal = vec![[0.0, 1.0, 0.0]; cols * rows];
        for r in 0..rows {
            for c in 0..cols {
                let p = Pos { x: c as f32 * posting, y: t.height_m - r as f32 * posting };
                elev[r * cols + c] = shaped(p);
                let m = t.normal_at(p);
                normal[r * cols + c] = [m[0], m[1], -m[2]];
            }
        }
        // land points only: the sea floor is under opaque water
        let pts: Vec<Pos> = (0..40_000u32)
            .map(|k| Pos { x: 100.0 + (k as f32 * 0.618_034).fract() * 7800.0, y: 100.0 + (k as f32 * 0.754_878).fract() * 7800.0 })
            .filter(|p| t.height_at(*p) > 2.0)
            .collect();
        let report = |label: &str, errs: &mut Vec<f32>| {
            errs.sort_by(|a, b| a.total_cmp(b));
            let q = |f: f32| errs[((errs.len() - 1) as f32 * f) as usize];
            eprintln!("{label}: p50 {:.2} p90 {:.2} p99 {:.2} max {:.2} m", q(0.5), q(0.9), q(0.99), q(1.0));
        };
        // the old grid: two triangles per 20 m square, split as `level_mesh` does
        let mut errs: Vec<f32> = pts
            .iter()
            .map(|p| {
                let (fc, fr) = (p.x / posting, (t.height_m - p.y) / posting);
                let (c, r) = (fc.floor() as usize, fr.floor() as usize);
                let (u, v) = (fc - c as f32, fr - r as f32);
                let h = |dc: usize, dr: usize| elev[(r + dr) * cols + c + dc];
                // triangles (i, down, right) and (right, down, down+1)
                let z = if u + v <= 1.0 { h(0, 0) + (h(1, 0) - h(0, 0)) * u + (h(0, 1) - h(0, 0)) * v } else { h(1, 1) + (h(0, 1) - h(1, 1)) * (1.0 - u) + (h(1, 0) - h(1, 1)) * (1.0 - v) };
                (z - shaped(*p)).abs()
            })
            .collect();
        report("grid 20 m", &mut errs);
        let grid = Grid { cols, posting, elev, normal };
        let (chunks_x, chunks_y) = ((cols - 1).div_ceil(CHUNK), (rows - 1).div_ceil(CHUNK));
        for (tol, step, cand) in [(2.0, 10.0, 1), (3.0, 10.0, 1), (2.0, 10.0, 2), (3.0, 10.0, 2)] {
            TIN_WORKERS.store(1, std::sync::atomic::Ordering::Relaxed);
            let started = std::time::Instant::now();
            let meshes = tin_geometry(&scn, &shore, &grid, rows, (chunks_x, chunks_y), &[1], stride, (tol, step, cand));
            let took = started.elapsed().as_secs_f32();
            let mut errs = Vec::new();
            let mut tris_total = 0;
            for (&(cx, cy, _), g) in &meshes {
                let (c0, r0) = (cx * CHUNK, cy * CHUNK);
                let (ncx, ncy) = (CHUNK.min(cols - 1 - c0), CHUNK.min(rows - 1 - r0));
                let origin = [(c0 as f32 + ncx as f32 * 0.5) * posting, (r0 as f32 + ncy as f32 * 0.5) * posting - t.height_m];
                // back to world metres (x east, z = -north), skirts dropped
                let tris: Vec<[[f32; 3]; 3]> = g
                    .indices
                    .chunks(3)
                    .map(|i| [i[0], i[1], i[2]].map(|v| { let q = g.positions[v as usize]; [q[0] + origin[0], q[1], q[2] + origin[1]] }))
                    .filter(|[a, b, c]| ((b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])).abs() > 1e-6)
                    .collect();
                tris_total += tris.len();
                let (x0, x1) = (c0 as f32 * posting, (c0 + ncx) as f32 * posting);
                let (y1, y0) = (t.height_m - r0 as f32 * posting, t.height_m - (r0 + ncy) as f32 * posting);
                let cell = 40.0;
                let nb = (((x1 - x0) / cell).ceil() as usize, ((y1 - y0) / cell).ceil() as usize);
                let mut buckets = vec![Vec::new(); nb.0 * nb.1];
                for (k, [a, b, c]) in tris.iter().enumerate() {
                    let (lx, hx) = (a[0].min(b[0]).min(c[0]), a[0].max(b[0]).max(c[0]));
                    let (ly, hy) = ((-a[2]).min(-b[2]).min(-c[2]), (-a[2]).max(-b[2]).max(-c[2]));
                    for j in (((ly - y0) / cell).floor().max(0.0) as usize)..=(((hy - y0) / cell).floor().max(0.0) as usize).min(nb.1 - 1) {
                        for i in (((lx - x0) / cell).floor().max(0.0) as usize)..=(((hx - x0) / cell).floor().max(0.0) as usize).min(nb.0 - 1) {
                            buckets[j * nb.0 + i].push(k);
                        }
                    }
                }
                for p in pts.iter().filter(|p| p.x >= x0 && p.x < x1 && p.y >= y0 && p.y < y1) {
                    let (x, z) = (p.x, -p.y);
                    let (i, j) = ((((p.x - x0) / cell) as usize).min(nb.0 - 1), (((p.y - y0) / cell) as usize).min(nb.1 - 1));
                    for &k in &buckets[j * nb.0 + i] {
                        let [a, b, c] = tris[k];
                        let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
                        let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
                        let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
                        let l3 = 1.0 - l1 - l2;
                        if l1 >= -1e-5 && l2 >= -1e-5 && l3 >= -1e-5 {
                            let e = (l1 * a[1] + l2 * b[1] + l3 * c[1] - shaped(*p)).abs();
                            errs.push(e);
                            break;
                        }
                    }
                }
            }
            report(&format!("tin tol {tol} kerbs {step} m candidates every {} m ({tris_total} triangles, {took:.2} s on one thread)", 5 * cand), &mut errs);
        }
    }

    /// The adaptive mesh stays on the ground everything else stands on
    /// (`height_at`, shaped like the mesh): within the tolerance almost
    /// everywhere, and on the road kerbs and centreline within centimetres.
    #[test]
    fn the_adaptive_mesh_follows_the_ground_and_the_roads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let Ok(scn) = Scenario::load(&dir) else { return };
        let shore = ShoreField::new(&scn);
        let t = &scn.terrain;
        let shaped = |p: Pos| cliff_elev(&scn, &shore, p, t.height_at(p));
        // a chunk of hillside with a highway across it
        let (cx, cy) = (7usize, 5usize);
        let (c0, r0, n) = (cx * CHUNK, cy * CHUNK, CHUNK);
        let mut elev = vec![0.0; t.cols * t.rows];
        let mut normal = vec![[0.0, 1.0, 0.0]; t.cols * t.rows];
        for r in r0..=r0 + n {
            for c in c0..=c0 + n {
                let p = Pos { x: c as f32 * t.posting, y: t.height_m - r as f32 * t.posting };
                elev[r * t.cols + c] = cliff_elev(&scn, &shore, p, t.elev[r * t.cols + c]);
                let m = t.normal_at(p);
                normal[r * t.cols + c] = [m[0], m[1], -m[2]];
            }
        }
        let view = crate::terrain_tin::GridView { cols: t.cols, rows: t.rows, c_off: 0, r_off: 0, posting: t.posting, elev: &elev, normal: Some(&normal) };
        let inside = |k: &crate::terrain_tin::Fixed| k.c > c0 as f32 + 0.3 && k.c < (c0 + n) as f32 - 0.3 && k.r > r0 as f32 + 0.3 && k.r < (r0 + n) as f32 - 0.3;
        let kerb: Vec<_> = kerbs(&scn, &shore, t.posting, KERB_STEP_M[0]).into_iter().filter(inside).collect();
        assert!(kerb.len() > 100, "the chunk has roads ({})", kerb.len());
        let g = crate::terrain_tin::chunk(&view, (c0, r0), (n, n), 1, &kerb, TIN_TOL_M[0], [0.0, 0.0], t.height_m, 3.0);
        // the mesh's height at a world point (brute force over the triangles)
        let tris: Vec<[[f32; 3]; 3]> = g.indices.chunks(3).map(|i| [g.positions[i[0] as usize], g.positions[i[1] as usize], g.positions[i[2] as usize]]).collect();
        let mesh_at = |p: Pos| -> Option<f32> {
            let (x, z) = (p.x, -p.y);
            for [a, b, c] in &tris {
                let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
                if det.abs() < 1e-6 {
                    continue; // a skirt
                }
                let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
                let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
                let l3 = 1.0 - l1 - l2;
                if l1 >= -1e-5 && l2 >= -1e-5 && l3 >= -1e-5 {
                    return Some(l1 * a[1] + l2 * b[1] + l3 * c[1]);
                }
            }
            None
        };
        let mut errs = Vec::new();
        for k in 0..3000u32 {
            let (u, v) = ((k as f32 * 0.618_034).fract(), (k as f32 * 0.754_878).fract());
            let p = Pos { x: (c0 as f32 + 1.0 + u * (n as f32 - 2.0)) * t.posting, y: t.height_m - (r0 as f32 + 1.0 + v * (n as f32 - 2.0)) * t.posting };
            errs.push((mesh_at(p).expect("covered") - shaped(p)).abs());
        }
        errs.sort_by(|a, b| a.total_cmp(b));
        let (p50, p99) = (errs[errs.len() / 2], errs[errs.len() * 99 / 100]);
        let mut on_road = Vec::new();
        for k in &kerb {
            let p = Pos { x: k.c * t.posting, y: t.height_m - k.r * t.posting };
            on_road.push((mesh_at(p).unwrap() - shaped(p)).abs());
        }
        on_road.sort_by(|a, b| a.total_cmp(b));
        let road99 = on_road[on_road.len() * 99 / 100];
        eprintln!("tin: {} triangles; off the mesh p50 {p50:.3} p99 {p99:.3} max {:.3} m; kerbs p99 {road99:.3} m", tris.len(), errs[errs.len() - 1]);
        assert!(p99 < 0.35, "p99 {p99}");
        assert!(road99 < 0.02, "kerbs {road99}");
    }

    /// The cliffs are shaped only where they should be: a few metres at most,
    /// nothing at the waterline, a minority of the vertices.
    #[test]
    fn cliffs_are_shaped_gently_and_only_on_the_coast() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let Ok(scn) = Scenario::load(&dir) else { return };
        let shore = ShoreField::new(&scn);
        let t = &scn.terrain;
        let (mut shaped, mut total, mut worst) = (0usize, 0usize, 0.0f32);
        for r in (0..t.rows).step_by(3) {
            for c in (0..t.cols).step_by(3) {
                let p = Pos { x: c as f32 * t.posting, y: t.height_m - r as f32 * t.posting };
                let e = t.elev[r * t.cols + c];
                let d = cliff_elev(&scn, &shore, p, e) - e;
                total += 1;
                if d != 0.0 {
                    shaped += 1;
                    worst = worst.max(d.abs());
                    assert!(e >= 1.5, "{e} m shaped at {p:?}");
                    assert!(shore.at(p) <= COAST_REACH_M);
                }
            }
        }
        eprintln!("cliffs: {shaped}/{total} vertices shaped, worst {worst:.2} m");
        assert!(worst < 5.0, "{worst}");
        assert!(shaped * 6 < total, "{shaped}/{total}");
    }
}
