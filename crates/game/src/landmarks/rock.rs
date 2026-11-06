//! Baked rock made credible for the realistic look (`KIOSK_SCANS=1`,
//! 2026-11-06): the crag under the rocca is a heightfield of 3 m cells and
//! stacked prisms of six to eight sides, and with photographed rock on it the
//! facets and the straight prism edges read as low-poly. [`refine`] welds the
//! bake, splits every edge longer than a target, rounds the prisms' hard
//! corners a little (Taubin smoothing, which does not shrink), and pushes the
//! steep faces in and out with a 3-D noise in the model's own metres; the
//! normals are then the smooth normals of the new surface.
//!
//! The flat tops (the beds the castle stands on) keep their height: the
//! smoothing and the noise only move a vertex as much as its face is steep,
//! so walls, scrub and the courtyard stay where the bake put them.
//!
//! Only with the scans; the mesh keeps `MAIN_WORLD`, so Mario walks on what is
//! drawn.

use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use std::collections::HashMap;

/// How to refine one bake.
pub struct Refine {
    /// No edge longer than this, model metres.
    pub edge_m: f32,
    /// Taubin passes (each a shrink and an inflate).
    pub rounding: u32,
    /// Displacement of the steep faces, model metres (peak of the coarsest
    /// octave).
    pub relief_m: f32,
    /// Wavelength of the coarsest octave, model metres.
    pub wave_m: f32,
    /// Faces meeting at more than this keep a hard edge.
    pub crease_deg: f32,
    /// Flat tops stay where they are (the beds things stand on); otherwise
    /// every vertex moves alike.
    pub keep_beds: bool,
    /// Swelling and necking of the steep faces with height, model metres: a
    /// noise that varies three times faster up than across, so a stack of
    /// prisms reads as weathered beds, not as a column.
    pub bulge_m: f32,
}

fn hash3(i: IVec3) -> f32 {
    let mut h = (i.x as u32).wrapping_mul(0x8da6_b343) ^ (i.y as u32).wrapping_mul(0xd816_3841) ^ (i.z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Value noise in [-1, 1], smooth (quintic).
fn noise3(p: Vec3) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let i = i.as_ivec3();
    let c = |dx, dy, dz| hash3(i + IVec3::new(dx, dy, dz));
    let x00 = c(0, 0, 0) + (c(1, 0, 0) - c(0, 0, 0)) * u.x;
    let x10 = c(0, 1, 0) + (c(1, 1, 0) - c(0, 1, 0)) * u.x;
    let x01 = c(0, 0, 1) + (c(1, 0, 1) - c(0, 0, 1)) * u.x;
    let x11 = c(0, 1, 1) + (c(1, 1, 1) - c(0, 1, 1)) * u.x;
    let y0 = x00 + (x10 - x00) * u.y;
    let y1 = x01 + (x11 - x01) * u.y;
    (y0 + (y1 - y0) * u.z) * 2.0 - 1.0
}

/// Ridged-ish fBm: three octaves, the finer ones half as strong, stretched a
/// little along the vertical so the faces read as bedded and cracked rather
/// than as dough.
fn relief(p: Vec3, wave: f32) -> f32 {
    let q = p / wave * Vec3::new(1.0, 1.6, 1.0);
    let a = noise3(q);
    let b = noise3(q * 2.13 + 17.0);
    let c = noise3(q * 4.37 + 41.0);
    // a crease where the coarse octave crosses zero: a fissure
    let crack = 1.0 - (a.abs() * 3.0).min(1.0);
    a * 0.6 + b * 0.3 + c * 0.15 - crack * crack * 0.35
}

/// Refine `mesh` in place (see the module). Keeps the vertex colours
/// (averaged where the bake had one colour per facet), drops the UVs to zero
/// as the bake had them.
pub fn refine(mesh: &mut Mesh, r: &Refine) {
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return };
    let pos = pos.clone();
    let col: Vec<[f32; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(c)) => c.clone(),
        _ => vec![[1.0; 4]; pos.len()],
    };
    let tri_idx: Vec<u32> = match mesh.indices() {
        Some(Indices::U16(i)) => i.iter().map(|&k| k as u32).collect(),
        Some(Indices::U32(i)) => i.clone(),
        None => (0..pos.len() as u32).collect(),
    };

    // --- weld (1 cm): one vertex per position, the mean colour
    let mut weld: HashMap<[i64; 3], u32> = HashMap::new();
    let mut p: Vec<Vec3> = Vec::new();
    let mut c: Vec<Vec4> = Vec::new();
    let mut cn: Vec<f32> = Vec::new();
    let mut remap = vec![0u32; pos.len()];
    for (i, v) in pos.iter().enumerate() {
        let k = [(v[0] * 100.0).round() as i64, (v[1] * 100.0).round() as i64, (v[2] * 100.0).round() as i64];
        let id = *weld.entry(k).or_insert_with(|| {
            p.push(Vec3::from(*v));
            c.push(Vec4::ZERO);
            cn.push(0.0);
            (p.len() - 1) as u32
        });
        c[id as usize] += Vec4::from(col[i]);
        cn[id as usize] += 1.0;
        remap[i] = id;
    }
    for (c, n) in c.iter_mut().zip(&cn) {
        *c /= n.max(1.0);
    }
    let mut tris: Vec<[u32; 3]> = tri_idx
        .chunks_exact(3)
        .map(|t| [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]])
        .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
        .collect();

    // --- split long edges (red-green: a shared midpoint per edge, so no cracks)
    let max2 = r.edge_m * r.edge_m;
    for _ in 0..6 {
        let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
        let mut split = |a: u32, b: u32, p: &mut Vec<Vec3>, c: &mut Vec<Vec4>| -> Option<u32> {
            let key = (a.min(b), a.max(b));
            if let Some(&m) = mid.get(&key) {
                return Some(m);
            }
            if p[a as usize].distance_squared(p[b as usize]) <= max2 {
                return None;
            }
            p.push((p[a as usize] + p[b as usize]) * 0.5);
            c.push((c[a as usize] + c[b as usize]) * 0.5);
            let m = (p.len() - 1) as u32;
            mid.insert(key, m);
            Some(m)
        };
        // decide the splits first, all of them, then cut: a triangle must see
        // its neighbour's midpoint on a shared edge
        for t in &tris {
            for k in 0..3 {
                split(t[k], t[(k + 1) % 3], &mut p, &mut c);
            }
        }
        if mid.is_empty() {
            break;
        }
        let mut out = Vec::with_capacity(tris.len() * 3);
        for t in &tris {
            let m = [0, 1, 2].map(|k| mid.get(&(t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3]))).copied());
            match m.iter().filter(|x| x.is_some()).count() {
                0 => out.push(*t),
                3 => {
                    let (ab, bc, ca) = (m[0].unwrap(), m[1].unwrap(), m[2].unwrap());
                    out.extend([[t[0], ab, ca], [ab, t[1], bc], [ca, bc, t[2]], [ab, bc, ca]]);
                }
                1 => {
                    let k = m.iter().position(|x| x.is_some()).unwrap();
                    let (a, b, cc, mm) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3], m[k].unwrap());
                    out.extend([[a, mm, cc], [mm, b, cc]]);
                }
                _ => {
                    // two edges split: the one left whole is opposite vertex `k`
                    let k = m.iter().position(|x| x.is_none()).unwrap();
                    // whole edge t[k] -> t[k+1]; split: (k+1 -> k+2) at m1, (k+2 -> k) at m2
                    let (a, b, cc) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                    let (m1, m2) = (m[(k + 1) % 3].unwrap(), m[(k + 2) % 3].unwrap());
                    out.extend([[m1, cc, m2], [a, b, m1], [a, m1, m2]]);
                }
            }
        }
        tris = out;
    }

    // --- neighbours and normals
    let n = p.len();
    let normals = |p: &[Vec3]| {
        let mut nor = vec![Vec3::ZERO; n];
        for t in &tris {
            let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
            let f = (b - a).cross(c - a); // area-weighted
            for &i in t {
                nor[i as usize] += f;
            }
        }
        nor.iter().map(|v| v.try_normalize().unwrap_or(Vec3::Y)).collect::<Vec<_>>()
    };
    let mut nbr: Vec<Vec<u32>> = vec![Vec::new(); n];
    for t in &tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if !nbr[a as usize].contains(&b) {
                nbr[a as usize].push(b);
            }
            if !nbr[b as usize].contains(&a) {
                nbr[b as usize].push(a);
            }
        }
    }
    // how much a vertex may move: by the steepness of its surface (beds stay)
    let nor0 = normals(&p);
    let free: Vec<f32> = nor0.iter().map(|v| if r.keep_beds { 1.0 - smooth(0.6, 0.92, v.y) } else { 1.0 }).collect();

    // --- round the corners: Taubin (shrink, then inflate), steep parts only
    for _ in 0..r.rounding {
        for lambda in [0.5f32, -0.53] {
            let q = p.clone();
            for i in 0..n {
                if nbr[i].is_empty() || free[i] <= 0.0 {
                    continue;
                }
                let mean = nbr[i].iter().map(|&j| q[j as usize]).sum::<Vec3>() / nbr[i].len() as f32;
                p[i] = q[i] + (mean - q[i]) * lambda * free[i];
            }
        }
    }

    // --- relief along the normal
    let nor1 = normals(&p);
    for i in 0..n {
        let bulge = noise3(p[i] / r.wave_m * Vec3::new(0.35, 1.2, 0.35) + 71.0) * r.bulge_m;
        let d = (relief(p[i], r.wave_m) * r.relief_m + bulge) * free[i];
        p[i] += nor1[i] * d;
    }
    // --- normals: smooth across the faces, hard where two meet at more than
    // `crease_deg` (a ledge's rim), each corner its own where they differ
    let face_n: Vec<Vec3> = tris
        .iter()
        .map(|t| (p[t[1] as usize] - p[t[0] as usize]).cross(p[t[2] as usize] - p[t[0] as usize]))
        .collect();
    let mut around: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (f, t) in tris.iter().enumerate() {
        for &i in t {
            around[i as usize].push(f as u32);
        }
    }
    let cos = r.crease_deg.to_radians().cos();
    let mut out_p: Vec<[f32; 3]> = Vec::with_capacity(n);
    let mut out_n: Vec<[f32; 3]> = Vec::with_capacity(n);
    let mut out_c: Vec<[f32; 4]> = Vec::with_capacity(n);
    let mut corner: HashMap<(u32, [i32; 3]), u32> = HashMap::new();
    let mut idx: Vec<u32> = Vec::with_capacity(tris.len() * 3);
    for (f, t) in tris.iter().enumerate() {
        let nf = face_n[f].normalize_or_zero();
        for &i in t {
            let sum: Vec3 = around[i as usize]
                .iter()
                .map(|&g| face_n[g as usize])
                .filter(|g| g.normalize_or_zero().dot(nf) >= cos)
                .sum();
            let v = sum.try_normalize().unwrap_or(nf);
            let key = (i, (v * 64.0).round().as_ivec3().to_array());
            let id = *corner.entry(key).or_insert_with(|| {
                out_p.push(p[i as usize].to_array());
                out_n.push(v.to_array());
                out_c.push(c[i as usize].to_array());
                (out_p.len() - 1) as u32
            });
            idx.push(id);
        }
    }
    let m = out_p.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, out_p);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, out_n);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, out_c);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; m]);
    mesh.insert_indices(Indices::U32(idx));
    mesh.compress_indices();
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refined_crag_is_closed_where_the_bake_was_and_keeps_its_beds() {
        let mut mesh = crate::models::island_mesh("rocca_roccia");
        let before = mesh.indices().unwrap().len() / 3;
        refine(&mut mesh, &CRAG);
        let tris = mesh.indices().unwrap().len() / 3;
        let Some(VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
        eprintln!("crag: {before} -> {tris} triangles, {} vertices", p.len());
        assert!(tris > before && tris < 400_000, "{tris}");
        assert!(p.iter().flatten().all(|v| v.is_finite()));
        // the summit's height is the bake's, within the relief
        let top = p.iter().map(|v| v[1]).fold(f32::MIN, f32::max);
        assert!((top - 35.0).abs() < 1.5, "top {top}");
    }
}

/// The crag under the rocca (model metres; the rocca is drawn at 1.9).
pub const CRAG: Refine = Refine { edge_m: 1.6, rounding: 1, relief_m: 0.8, wave_m: 4.5, crease_deg: 62.0, keep_beds: true, bulge_m: 0.9 };

/// The boulders at the stone tower's foot (model metres; drawn at 2.2):
/// rounder, a finer grain.
pub const BOULDERS: Refine = Refine { edge_m: 0.45, rounding: 8, relief_m: 0.25, wave_m: 1.6, crease_deg: 80.0, keep_beds: false, bulge_m: 0.0 };

/// The coast's boulders and reefs (model metres; drawn at 1-4).
pub const COAST_BOULDER: Refine = Refine { edge_m: 0.6, rounding: 4, relief_m: 0.12, wave_m: 1.2, crease_deg: 80.0, keep_beds: false, bulge_m: 0.0 };

/// The sea stacks (model metres, 8 x 13 x 6; drawn at 1.6-3).
pub const SEA_STACK: Refine = Refine { edge_m: 0.9, rounding: 1, relief_m: 0.3, wave_m: 2.4, crease_deg: 62.0, keep_beds: false, bulge_m: 0.0 };
