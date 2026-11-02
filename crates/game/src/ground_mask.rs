//! Road mask: a signed-distance texture of the road network, sampled by the
//! terrain's fragment shader (`shaders/terrain.wgsl`).
//!
//! The terrain mesh colours per vertex, one every 5 m, which is too coarse for
//! a 10 m road. Rasterising the network once into a texture lets the shader draw
//! surface, kerb and junctions per pixel: no geometry laid over the ground, so
//! nothing to z-fight, and a junction is the union of its roads by construction.
//!
//! Channels: drivable roads and tracks (distances), and the share of paved
//! ground (town squares and yards).
//! The first two hold Each stores the distance to the
//! nearest ribbon *edge* (negative inside), clamped to ±[`RANGE_M`] and mapped
//! to 0..255. A distance field survives box-filtered mip levels, so a trilinear
//! sample is a plain blur of the roads at a distance, never a hole in them.

use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use scenario::{Pos, Scenario};

/// Only the visual mask is coarser on phones and on native machines short of
/// memory (a coarser terrain, `quality::Graphics::RISPARMIO`): the simulation
/// and road network retain their full data. Each doubling takes a quarter of
/// the texels (native 2 m: 85 MB with the mipmaps).
fn resolution(gfx: &crate::quality::Graphics) -> f32 {
    match (cfg!(target_arch = "wasm32"), gfx.terrain_stride) {
        (false, 1) => 2.0,
        (_, s) if s >= 4 => 8.0,
        _ => 4.0,
    }
}
/// Half the encoded range, metres. Must match `RANGE_M` in the shader.
const RANGE_M: f32 = 12.0;
/// Half-widths of the ribbons, metres. Must match the shader.
pub use scenario::grade::{DRIVABLE_HALF_W, TRACK_HALF_W};

/// Where the mask lives in `Assets<Image>`: a fixed weak handle, so the fire's
/// ground overlay can bind it without waiting for the terrain to be built.
pub const ROAD_MASK: Handle<Image> = uuid_handle!("5a3c91e0-77b2-4d18-8e41-c0a61f2d7b09");

/// Which texels are open sea (low ground connected to the window's edge), so
/// an inland hollow below sea level is not painted as water.
pub struct SeaGrid {
    nx: usize,
    ny: usize,
    res: f32,
    sea: Vec<bool>,
}

impl SeaGrid {
    pub fn is_sea(&self, p: Pos) -> bool {
        let i = ((p.x / self.res) as usize).min(self.nx - 1);
        let j = ((p.y / self.res) as usize).min(self.ny - 1);
        self.sea[j * self.nx + i]
    }
}

pub struct RoadMask {
    pub image: Image,
    pub sea: SeaGrid,
    /// `(1 / width, 1 / height, 0, 0)`: world metres to texture coordinates.
    pub rect: Vec4,
}

pub fn build(scn: &Scenario, gfx: &crate::quality::Graphics) -> RoadMask {
    let res_m = resolution(gfx);
    let (w_m, h_m) = (scn.world.width_m, scn.world.height_m);
    let (nx, ny) = ((w_m / res_m).ceil() as usize, (h_m / res_m).ceil() as usize);
    let mut field = [vec![RANGE_M; nx * ny], vec![RANGE_M; nx * ny]];

    for road in &scn.vectors.roads {
        let (ch, half) = if road.drivable {
            (0, DRIVABLE_HALF_W)
        } else if road.track {
            (1, TRACK_HALF_W)
        } else {
            continue;
        };
        for seg in road.line.windows(2) {
            stamp(&mut field[ch], nx, ny, seg[0], seg[1], half, res_m);
        }
    }

    // Pack the roads before building the other raster channels: their float
    // fields need not coexist with the shoreline's two distance fields.
    // Reserve the entire mip chain once; growing a cloned base level doubled
    // its allocation while the old and new buffers were both still live.
    let (mut mw, mut mh, mut bytes) = (nx, ny, nx * ny * 4);
    while mw > 1 || mh > 1 {
        (mw, mh) = ((mw / 2).max(1), (mh / 2).max(1));
        bytes += mw * mh * 4;
    }
    let mut data = Vec::with_capacity(bytes);
    for i in 0..nx * ny {
        data.extend_from_slice(&[encode(field[0][i]), encode(field[1][i]), 0, 0]);
    }
    drop(field);
    for (pixel, paved) in data.chunks_exact_mut(4).zip(paved_field(scn, nx, ny, res_m)) {
        pixel[2] = paved;
    }
    let (shore, sea) = shore_field(scn, nx, ny, res_m);
    for (pixel, shore) in data.chunks_exact_mut(4).zip(shore) {
        pixel[3] = shore;
    }

    // Read the preceding level directly from the chain: no separate copy of
    // the full-resolution image, or intermediate mip buffers.
    let (mut lw, mut lh, mut levels, mut offset) = (nx, ny, 1, 0);
    while lw > 1 || lh > 1 {
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
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

    let mut image = Image::new_uninit(
        Extent3d { width: nx as u32, height: ny as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    info!("road mask: {nx}x{ny} at {res_m} m, {levels} levels");
    RoadMask { image, rect: Vec4::new(1.0 / w_m, 1.0 / h_m, if shader_grass() { 1.0 } else { 0.0 }, 0.0), sea: SeaGrid { nx, ny, res: res_m, sea } }
}

/// Share of paved ground (roads, buildings and yards of the cover raster) at
/// each texel, as 0..255. The cover raster is 5 m; sampling its four nearest
/// cells and blurring once turns the squares into a smooth field the shader
/// can threshold into a crisp, noise-ragged edge.
fn paved_field(scn: &Scenario, nx: usize, ny: usize, res_m: f32) -> Vec<u8> {
    use scenario::CoverClass;
    let mut out = vec![0u8; nx * ny];
    let Some(cover) = &scn.cover else { return out };
    let h = cover.cell_m * 0.5;
    let is = |x: f32, y: f32| {
        matches!(cover.at(scenario::Pos { x, y }), CoverClass::Road | CoverClass::Building | CoverClass::Yard) as u8
    };
    let mut raw = vec![0u8; nx * ny];
    for j in 0..ny {
        let y = (j as f32 + 0.5) * res_m;
        for i in 0..nx {
            let x = (i as f32 + 0.5) * res_m;
            raw[j * nx + i] = is(x - h, y - h) + is(x + h, y - h) + is(x - h, y + h) + is(x + h, y + h);
        }
    }
    // 5x5 box blur, separable: 0..4 -> 0..255.
    let r = 2usize;
    let mut tmp = vec![0u16; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            let lo = i.saturating_sub(r);
            let hi = (i + r).min(nx - 1);
            tmp[j * nx + i] = (lo..=hi).map(|k| u16::from(raw[j * nx + k])).sum::<u16>() * 8 / (hi - lo + 1) as u16;
        }
    }
    for j in 0..ny {
        let lo = j.saturating_sub(r);
        let hi = (j + r).min(ny - 1);
        for i in 0..nx {
            let sum: u32 = (lo..=hi).map(|k| u32::from(tmp[k * nx + i])).sum();
            out[j * nx + i] = ((sum / (hi - lo + 1) as u32) * 255 / 32).min(255) as u8;
        }
    }
    out
}

/// Ground at or below this is sea: the rule `sea::WATER_ELEV_MAX` uses.
const WATER_ELEV_MAX: f32 = 0.6;

/// Signed distance to the waterline, metres, as 0..255: 127.5 at the shore,
/// above it in the water, below it on land; 1 m a step, so ±127 m.
fn shore_field(scn: &Scenario, nx: usize, ny: usize, res_m: f32) -> (Vec<u8>, Vec<bool>) {
    let n = nx * ny;
    let mut water = vec![false; n];
    for j in 0..ny {
        let y = (j as f32 + 0.5) * res_m;
        for i in 0..nx {
            let x = (i as f32 + 0.5) * res_m;
            water[j * nx + i] = scn.terrain.height_at(Pos { x, y }) <= WATER_ELEV_MAX;
        }
    }
    let sea = connected_sea(&water, nx, ny);
    drop(water);
    // Distance from each water texel to land, and from each land texel to water.
    let to_land = chamfer(&sea.iter().map(|w| !w).collect::<Vec<_>>(), nx, ny);
    let to_water = chamfer(&sea, nx, ny);
    let field = (0..n)
        .map(|i| {
            let d = if sea[i] { to_land[i] } else { -to_water[i] } * res_m;
            (d.clamp(-127.0, 127.0) + 127.5) as u8
        })
        .collect();
    (field, sea)
}

/// Only low ground reachable from the window's edge is sea. Mark cells
/// when enqueuing them, so each is queued once. Breadth-first traversal keeps
/// the frontier small on open water instead of retaining a per-pixel DFS stack.
fn connected_sea(water: &[bool], nx: usize, ny: usize) -> Vec<bool> {
    let mut sea = vec![false; nx * ny];
    let mut queue = std::collections::VecDeque::new();
    let enqueue = |k: usize, sea: &mut [bool], queue: &mut std::collections::VecDeque<usize>| {
        if water[k] && !sea[k] {
            sea[k] = true;
            queue.push_back(k);
        }
    };
    for i in 0..nx {
        enqueue(i, &mut sea, &mut queue);
        enqueue((ny - 1) * nx + i, &mut sea, &mut queue);
    }
    for j in 0..ny {
        enqueue(j * nx, &mut sea, &mut queue);
        enqueue(j * nx + nx - 1, &mut sea, &mut queue);
    }
    while let Some(k) = queue.pop_front() {
        let (i, j) = (k % nx, k / nx);
        if i > 0 { enqueue(k - 1, &mut sea, &mut queue); }
        if i + 1 < nx { enqueue(k + 1, &mut sea, &mut queue); }
        if j > 0 { enqueue(k - nx, &mut sea, &mut queue); }
        if j + 1 < ny { enqueue(k + nx, &mut sea, &mut queue); }
    }
    sea
}

/// Distance in texels to the nearest `seed` texel (0 on it), by the two-pass
/// chamfer: within a few percent of Euclidean, and linear in the texel count.
fn chamfer(seed: &[bool], nx: usize, ny: usize) -> Vec<f32> {
    const D: f32 = std::f32::consts::SQRT_2;
    let mut d: Vec<f32> = seed.iter().map(|&s| if s { 0.0 } else { 1.0e6 }).collect();
    for j in 0..ny {
        for i in 0..nx {
            let k = j * nx + i;
            let mut v = d[k];
            if i > 0 { v = v.min(d[k - 1] + 1.0); }
            if j > 0 {
                v = v.min(d[k - nx] + 1.0);
                if i > 0 { v = v.min(d[k - nx - 1] + D); }
                if i + 1 < nx { v = v.min(d[k - nx + 1] + D); }
            }
            d[k] = v;
        }
    }
    for j in (0..ny).rev() {
        for i in (0..nx).rev() {
            let k = j * nx + i;
            let mut v = d[k];
            if i + 1 < nx { v = v.min(d[k + 1] + 1.0); }
            if j + 1 < ny {
                v = v.min(d[k + nx] + 1.0);
                if i + 1 < nx { v = v.min(d[k + nx + 1] + D); }
                if i > 0 { v = v.min(d[k + nx - 1] + D); }
            }
            d[k] = v;
        }
    }
    d
}

fn encode(d: f32) -> u8 {
    ((d / RANGE_M).clamp(-1.0, 1.0) * 0.5 + 0.5).mul_add(255.0, 0.0).round() as u8
}

/// Min the distance-to-edge of one segment of half-width `half` into `field`.
fn stamp(field: &mut [f32], nx: usize, ny: usize, a: [f32; 2], b: [f32; 2], half: f32, res_m: f32) {
    let reach = half + RANGE_M;
    let lo = |v: f32| ((v - reach) / res_m).floor().max(0.0) as usize;
    let (x0, x1) = (lo(a[0].min(b[0])), (((a[0].max(b[0]) + reach) / res_m).ceil() as usize).min(nx));
    let (y0, y1) = (lo(a[1].min(b[1])), (((a[1].max(b[1]) + reach) / res_m).ceil() as usize).min(ny));
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    for j in y0..y1 {
        let py = (j as f32 + 0.5) * res_m;
        for i in x0..x1 {
            let px = (i as f32 + 0.5) * res_m;
            let t = if len2 < 1e-6 { 0.0 } else { (((px - a[0]) * dx + (py - a[1]) * dy) / len2).clamp(0.0, 1.0) };
            let (cx, cy) = (a[0] + dx * t - px, a[1] + dy * t - py);
            let d = (cx * cx + cy * cy).sqrt() - half;
            let v = &mut field[j * nx + i];
            if d < *v {
                *v = d;
            }
        }
    }
}

/// Whether meadows are drawn by the terrain shader (default) or as meshes
/// (`ROCCA_GRASS_MESH=1`, for comparing the two).
pub fn shader_grass() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ROCCA_GRASS_MESH").is_none())
}

pub const GRASS_MAP: Handle<Image> = uuid_handle!("2f7bc4d1-0a93-4e66-b5c8-91d36e0a48f2");

/// Meadow cover on the fire grid, one texel per cell, row 0 at the south so
/// that `v = y / height` indexes it like the mask. 255 where the fuel is
/// grassland (classes 1-3). The shader thresholds the bilinear blend with
/// noise, which is what makes the meadow's edge ragged and not 20 m squares.
pub fn grass_map(scn: &Scenario) -> Image {
    let w = &scn.world;
    // r: meadow; g, b, a: the fire, rewritten as it moves
    // (`fire_view::update_fire_map`); half floats, filtered on WebGL2 too
    let one = crate::terrain_mesh::f16_bits(1.0).to_le_bytes();
    let mut data = vec![0u8; w.fire_cols * w.fire_rows * 8];
    for r in 0..w.fire_rows {
        for c in 0..w.fire_cols {
            if matches!(scn.fuel_at(scenario::Cell { row: r, col: c }), 1..=3) {
                let at = ((w.fire_rows - 1 - r) * w.fire_cols + c) * 8;
                data[at..at + 2].copy_from_slice(&one);
            }
        }
    }
    let mut image = Image::new(
        Extent3d { width: w.fire_cols as u32, height: w.fire_rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        // the main world rewrites the fire's channels
        RenderAssetUsages::default(),
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// The memory budget must not change roads, paving, shoreline or mipmaps.
    #[test]
    fn scenario_mask_pixels_are_unchanged() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let scn = Scenario::load_by_id(&dir, scenario::ID).unwrap();
        let mask = build(&scn, &crate::quality::Graphics::ALTA);
        let data = mask.image.data.as_ref().unwrap();
        let hash = data.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3));
        assert_eq!(data.len(), 85_332_856);
        // The waterline reads the graded ground (`scenario::grade`): with
        // `ROCCA_STRADE_PIANE=0` the hash is the earlier 0xd4de3d2b63e08559.
        let expected = if scenario::grade::enabled() { 0xda68_b9d3_14bf_7441 } else { 0xd4de3d2b63e08559 };
        assert_eq!(hash, expected, "roads, paving, coast and all mip levels must keep their pixels");
    }
}
