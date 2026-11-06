//! Farmland: the irrigated plots of the cover raster as a single draped mesh of
//! parcels, one vertex-coloured material and no texture.
//!
//! The raster knows a plot as a 500 m blob. Here it is cut into parcels (long,
//! in blocks of different size, turned off the compass, edges bent a little);
//! a parcel is farmed if its centre is inside a plot, so the farmland ends on
//! hedges, not on the blob. Each parcel has its own crop or tillage, a dark
//! hedge round it, and stripes for the furrows. About 5 k quads in all: cheap
//! enough for the browser, which has no spare texture units for a shader that
//! did this per pixel.

use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use scenario::{CoverClass, Pos, Scenario};

use crate::field::noise;

const ROT: f32 = 0.41;
const HEDGE: [f32; 3] = [0.16, 0.30, 0.12];
const HEDGE_M: f32 = 1.5;
/// Lift over the ground, metres, under the draped service areas (`town_kit`). The ground is the highest of the point and four
/// neighbours 3 m round it: the terrain mesh joins every second sample of the
/// fine grid with straight lines, which cut through its bumps.
const LIFT: f32 = 0.5;
const CROPS: [[f32; 3]; 6] = [
    [0.24, 0.50, 0.16],
    [0.40, 0.62, 0.20],
    [0.50, 0.36, 0.22],
    [0.66, 0.56, 0.26],
    [0.30, 0.44, 0.17],
    [0.56, 0.48, 0.24],
];

/// The realistic ground's crops (2026-11-06, with the scanned plants), in
/// the order of [`CROPS`]: vegetables, young wheat, tilled soil, stubble,
/// an olive-green crop, dry ploughed earth. Linear, as the photos' means.
const CROPS_REAL: [[f32; 3]; 6] = [
    [0.075, 0.13, 0.035],
    [0.13, 0.19, 0.055],
    [0.16, 0.10, 0.058],
    [0.36, 0.27, 0.13],
    [0.095, 0.13, 0.05],
    [0.23, 0.165, 0.095],
];
const HEDGE_REAL: [f32; 3] = [0.035, 0.06, 0.025];

fn top(scn: &Scenario, p: Pos) -> f32 {
    [(0.0, 0.0), (3.0, 0.0), (-3.0, 0.0), (0.0, 3.0), (0.0, -3.0)]
        .into_iter()
        .map(|(dx, dy)| scn.terrain.height_at(Pos { x: p.x + dx, y: p.y + dy }))
        .fold(0.0, f32::max)
}

#[derive(Default)]
struct Mesher {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    /// world metres / 4 (the soil photo's tile), only for the realistic ground
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    real: bool,
}

/// The soil photo over the realistic ground's parcels (`textures::
/// ground_layer_image`), put in place by `terrain_mesh::build`, which has
/// the images at hand.
pub const SOIL: Handle<Image> = bevy::asset::uuid_handle!("6d0c2f7e-3b8a-4c51-9e27-5a1f04b8d3c6");

/// Whether the parcels have the realistic ground's soil photo: set by
/// [`insert_soil`].
static REAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn insert_soil(images: &mut Assets<Image>) {
    let soil = crate::textures::ground_layer_image("track");
    REAL.store(soil.is_some(), std::sync::atomic::Ordering::Relaxed);
    if let Some(soil) = soil {
        let _ = images.insert(&SOIL, soil);
    }
}

pub fn build(scn: &Scenario, commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let Some(cover) = &scn.cover else { return };
    let (sn, cs) = ROT.sin_cos();
    // Extent of the plots in the turned frame.
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    let mut any = false;
    for r in (0..cover.rows).step_by(2) {
        for c in (0..cover.cols).step_by(2) {
            let p = Pos { x: (c as f32 + 0.5) * cover.cell_m, y: cover.height_m - (r as f32 + 0.5) * cover.cell_m };
            if cover.at(p) == CoverClass::Irrigated {
                let (u, v) = (p.x * cs + p.y * sn, -p.x * sn + p.y * cs);
                lo = [lo[0].min(u), lo[1].min(v)];
                hi = [hi[0].max(u), hi[1].max(v)];
                any = true;
            }
        }
    }
    if !any {
        return;
    }
    let to_world = |u: f32, v: f32| Pos { x: u * cs - v * sn, y: u * sn + v * cs };
    // The edges bend with the position, so neighbouring parcels share them.
    let bend = |p: Pos| Pos {
        x: p.x + 14.0 * (noise(p.x / 70.0 + 3.1, p.y / 70.0 + 3.1, 0x91) - 0.5),
        y: p.y + 14.0 * (noise(p.x / 70.0 + 9.7, p.y / 70.0 + 9.7, 0x92) - 0.5),
    };
    // the realistic ground (with the scanned plants): earthier crops and a
    // photo of the soil over them
    let real = REAL.load(std::sync::atomic::Ordering::Relaxed);
    let mut m = Mesher { real, ..default() };
    let mut parcels = 0;
    for block in (lo[1] / 160.0).floor() as i32..=(hi[1] / 160.0).floor() as i32 {
        let b = block as f32;
        let len = 38.0 + 30.0 * noise(b, 0.3, 0x93);
        let wid = 26.0 + 16.0 * noise(b, 1.3, 0x94);
        let off = noise(b, 2.3, 0x95) * 7.0;
        for iv in ((b * 160.0) / wid).floor() as i32..=(((b + 1.0) * 160.0) / wid).floor() as i32 {
            let vc = (iv as f32 + 0.5) * wid;
            if (vc / 160.0).floor() != b {
                continue;
            }
            for iu in ((lo[0] / len + off).floor() as i32 - 1)..=((hi[0] / len + off).ceil() as i32 + 1) {
                let uc = (iu as f32 + 0.5 - off) * len;
                let centre = to_world(uc, vc);
                if cover.at(centre) != CoverClass::Irrigated {
                    continue;
                }
                let (fi, fj) = (iu as f32, iv as f32);
                if noise(fi, fj, 0x98) < 0.12 || noise(centre.x / 110.0 + 5.0, centre.y / 110.0 + 5.0, 0x99) < 0.38 {
                    continue;
                }
                let pick = ((noise(fi, fj, 0x96) * 6.0) as usize).min(5);
                let tilled = CROPS[pick][0] > 0.45;
                let crop = if real { CROPS_REAL[pick] } else { CROPS[pick] };
                let vigour = 0.94 + 0.12 * noise(fi, fj, 0x97);
                let (u0, v0) = ((iu as f32 - off) * len, iv as f32 * wid);
                parcel(scn, &mut m, &to_world, &bend, [u0, v0], [len, wid], crop, vigour, tilled);
                parcels += 1;
            }
        }
    }
    info!("fields: {parcels} parcels, {} k triangles", m.indices.len() / 3000);
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; m.positions.len()]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, m.colors);
    if real {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs);
    }
    mesh.insert_indices(Indices::U32(m.indices));
    mesh.compress_indices();
    let material = if real {
        // the photo's ratio is stored * GROUND_ENCODE
        let k = 1.0 / crate::textures::GROUND_ENCODE;
        materials.add(StandardMaterial { base_color: Color::linear_rgb(k, k, k), base_color_texture: Some(SOIL), perceptual_roughness: 1.0, metallic: 0.0, reflectance: 0.02, ..default() })
    } else {
        materials.add(StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 1.0, metallic: 0.0, reflectance: 0.02, ..default() })
    };
    commands.spawn(((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material)), crate::terrain_mesh::PauseSolid));
}

/// One parcel: the hedge as the full rectangle, the crop on top of it inside,
/// in furrow stripes across the short side.
#[allow(clippy::too_many_arguments)]
fn parcel(
    scn: &Scenario,
    m: &mut Mesher,
    to_world: &dyn Fn(f32, f32) -> Pos,
    bend: &dyn Fn(Pos) -> Pos,
    at: [f32; 2],
    size: [f32; 2],
    crop: [f32; 3],
    vigour: f32,
    tilled: bool,
) {
    let amp = if tilled { 0.14 } else { 0.07 };
    let quad = |m: &mut Mesher, a: [f32; 2], b: [f32; 2], nu: usize, nv: usize, lift: f32, color: &dyn Fn(usize, usize) -> [f32; 3]| {
        let base = m.positions.len() as u32;
        for j in 0..=nv {
            for i in 0..=nu {
                let (u, v) = (a[0] + (b[0] - a[0]) * i as f32 / nu as f32, a[1] + (b[1] - a[1]) * j as f32 / nv as f32);
                let p = bend(to_world(u, v));
                m.positions.push([p.x, top(scn, p) + lift, -p.y]);
                if m.real {
                    m.uvs.push([p.x / 4.0, p.y / 4.0]);
                }
                let c = color(i.min(nu - 1), j.min(nv - 1));
                m.colors.push([c[0], c[1], c[2], 1.0]);
            }
        }
        for j in 0..nv {
            for i in 0..nu {
                let k = base + (j * (nu + 1) + i) as u32;
                let w = (nu + 1) as u32;
                // Turned frame: v runs north in the world, u east, so (u, v) is
                // counter-clockwise from above and Bevy's -Z is north.
                m.indices.extend_from_slice(&[k, k + 1, k + w, k + 1, k + w + 1, k + w]);
            }
        }
    };
    // The hedge is a frame round the crop, not a layer under it: two surfaces
    // a hand apart fight for the depth buffer.
    let h = HEDGE_M;
    let (u1, v1) = (at[0] + size[0], at[1] + size[1]);
    let n = |len: f32, step: f32| ((len / step).ceil() as usize).max(1);
    let hedge = if m.real { HEDGE_REAL } else { HEDGE };
    quad(m, at, [u1, at[1] + h], n(size[0], 6.0), 1, LIFT, &|_, _| hedge);
    quad(m, [at[0], v1 - h], [u1, v1], n(size[0], 6.0), 1, LIFT, &|_, _| hedge);
    quad(m, [at[0], at[1] + h], [at[0] + h, v1 - h], 1, n(size[1], 5.0), LIFT, &|_, _| hedge);
    quad(m, [u1 - h, at[1] + h], [u1, v1 - h], 1, n(size[1], 5.0), LIFT, &|_, _| hedge);
    let inner_a = [at[0] + HEDGE_M, at[1] + HEDGE_M];
    let inner_b = [at[0] + size[0] - HEDGE_M, at[1] + size[1] - HEDGE_M];
    let (nu, nv) = (((size[0] - 2.0 * HEDGE_M) / 6.0).ceil() as usize, (((size[1] - 2.0 * HEDGE_M) / 3.0).ceil() as usize).max(2));
    quad(m, inner_a, inner_b, nu, nv, LIFT, &|i, j| {
        let stripe = if j % 2 == 0 { 1.0 + amp } else { 1.0 - amp };
        let patch = 0.97 + 0.06 * ((i * 7 + j * 3) % 5) as f32 / 4.0;
        [crop[0] * vigour * stripe * patch, crop[1] * vigour * stripe * patch, crop[2] * vigour * stripe * patch]
    });
}
