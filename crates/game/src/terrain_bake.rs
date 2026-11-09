//! The terrain's adaptive meshes as a file (`assets/terrain/tin_s<stride>.bin`,
//! 2026-11-09): on the browser's single thread `terrain_tin` held the page for
//! seconds at every start, for a result that depends only on the island. The
//! file is baked once and kept in the repository; it changes only when the
//! ground, the roads or the meshing do, and a test says when:
//! `cargo test --release -p game bake_web_terrain -- --ignored` writes it,
//! `baked_terrain_is_current` fails when it is out of date.
//!
//! Vertices are quantised: x and z to 1/256 of the grid's posting, so grid
//! samples stay exact and neighbouring chunks still share their edges, the
//! height to 16 bits of the island's range. The normals take 8 bits a side: the
//! shader draws with the normal texture. Triangles follow a Morton curve, the
//! vertices are numbered by first use, and each stream is delta coded apart,
//! which is what gzip (`scripts/build_web.sh`) then finds to squeeze.

use crate::ground_mask::MaskTexels;
use crate::terrain_tin::ChunkGeometry;
use std::collections::HashMap;

pub(crate) type Chunks = HashMap<(usize, usize, usize), ChunkGeometry>;

const MAGIC: &[u8; 4] = b"TIN1";
/// Parts of a grid posting in a unit of x and z. A chunk is 128 postings wide
/// and its origin is its centre, so these fit an `i16`.
const PER_POSTING: f32 = 256.0;

/// The baked meshes of this stride, where they are read: the browser. `None`
/// if the page did not get them, and they are computed as on the desktop.
#[cfg(target_arch = "wasm32")]
pub(crate) fn read(stride: usize, posting: f32) -> Option<Chunks> {
    let bytes = datafs::take(format!("assets/terrain/tin_s{stride}.bin")).ok()?;
    let chunks = decode(&bytes, posting);
    if chunks.is_none() {
        bevy::log::warn!("terrain: assets/terrain/tin_s{stride}.bin is not for this grid");
    }
    chunks
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read(_stride: usize, _posting: f32) -> Option<Chunks> {
    None
}

fn oct_encode(n: [f32; 3]) -> [u8; 2] {
    let l1 = n[0].abs() + n[1].abs() + n[2].abs();
    let (mut u, mut v) = (n[0] / l1, n[2] / l1);
    if n[1] < 0.0 {
        (u, v) = ((1.0 - v.abs()) * u.signum(), (1.0 - u.abs()) * v.signum());
    }
    [u, v].map(|x| ((x * 127.0).round() as i8) as u8)
}

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
fn oct_decode(e: [u8; 2]) -> [f32; 3] {
    let (u, v) = (e[0] as i8 as f32 / 127.0, e[1] as i8 as f32 / 127.0);
    let y = 1.0 - u.abs() - v.abs();
    let (x, z) = if y < 0.0 { ((1.0 - v.abs()) * u.signum(), (1.0 - u.abs()) * v.signum()) } else { (u, v) };
    let len = (x * x + y * y + z * z).sqrt();
    [x / len, y / len, z / len]
}

/// Bits of a triangle's place along a Morton curve.
#[cfg(test)]
fn morton(x: i32, z: i32) -> u64 {
    let spread = |v: i32| {
        let mut v = (v + 40_000) as u64 >> 5;
        let mut out = 0;
        for b in 0..16 {
            out |= (v & 1) << (2 * b);
            v >>= 1;
        }
        out
    };
    spread(x) | (spread(z) << 1)
}

#[cfg(test)]
pub(crate) fn quantised(p: [f32; 3], unit: f32, y0: f32, dy: f32) -> [i32; 3] {
    [(p[0] / unit).round() as i32, ((p[1] - y0) / dy).round() as i32, (p[2] / unit).round() as i32]
}

/// Lowest height and height of a step over every chunk.
#[cfg(test)]
pub(crate) fn height_range(chunks: &Chunks) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for p in chunks.values().flat_map(|g| &g.positions) {
        (lo, hi) = (lo.min(p[1]), hi.max(p[1]));
    }
    (lo, (hi - lo) / 65535.0)
}

#[cfg(test)]
pub(crate) fn encode(chunks: &Chunks, posting: f32) -> Vec<u8> {
    let unit = posting / PER_POSTING;
    let (y0, dy) = height_range(chunks);
    let mut keys: Vec<_> = chunks.keys().copied().collect();
    keys.sort();
    let mut head = Vec::new();
    // low and high bytes of x, y, z; the two of the normal; the indices
    let mut planes: [Vec<u8>; 9] = Default::default();
    for key in &keys {
        let g = &chunks[key];
        let q: Vec<[i32; 3]> = g.positions.iter().map(|p| quantised(*p, unit, y0, dy)).collect();
        let mut tris: Vec<[u32; 3]> = g.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
        tris.sort_by_key(|t| {
            let c = t.map(|v| q[v as usize]);
            morton((c[0][0] + c[1][0] + c[2][0]) / 3, (c[0][2] + c[1][2] + c[2][2]) / 3)
        });
        let mut number = vec![u32::MAX; g.positions.len()];
        let mut order: Vec<u32> = Vec::new();
        for v in tris.iter().flatten() {
            let n = number[*v as usize];
            let code = if n == u32::MAX {
                number[*v as usize] = order.len() as u32;
                order.push(*v);
                0
            } else {
                order.len() as u32 - n
            };
            let mut c = code;
            while c >= 0x80 {
                planes[8].push(c as u8 | 0x80);
                c >>= 7;
            }
            planes[8].push(c as u8);
        }
        for part in [key.0, key.1, key.2] {
            head.extend_from_slice(&(part as u16).to_le_bytes());
        }
        head.extend_from_slice(&(order.len() as u32).to_le_bytes());
        head.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        let mut last = [0u16; 3];
        let mut last_n = [0u8; 2];
        for v in &order {
            let p = q[*v as usize].map(|x| x as u16);
            for a in 0..3 {
                let d = p[a].wrapping_sub(last[a]).to_le_bytes();
                planes[a * 2].push(d[0]);
                planes[a * 2 + 1].push(d[1]);
            }
            last = p;
            let n = oct_encode(g.normals[*v as usize]);
            for a in 0..2 {
                planes[6 + a].push(n[a].wrapping_sub(last_n[a]));
            }
            last_n = n;
        }
    }
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&posting.to_le_bytes());
    out.extend_from_slice(&y0.to_le_bytes());
    out.extend_from_slice(&dy.to_le_bytes());
    out.extend_from_slice(&(keys.len() as u32).to_le_bytes());
    out.extend_from_slice(&head);
    for p in &planes {
        out.extend_from_slice(&(p.len() as u32).to_le_bytes());
        out.extend_from_slice(p);
    }
    out
}

/// `None` if the bytes are not a bake of a grid of this `posting`.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub(crate) fn decode(bytes: &[u8], posting: f32) -> Option<Chunks> {
    let mut at = 0;
    let mut take = |n: usize| {
        let s = bytes.get(at..at + n)?;
        at += n;
        Some(s)
    };
    if take(4)? != MAGIC {
        return None;
    }
    let f32_at = |s: &[u8]| f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
    let u32_at = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize;
    if f32_at(take(4)?) != posting {
        return None;
    }
    let (y0, dy) = (f32_at(take(4)?), f32_at(take(4)?));
    let count = u32_at(take(4)?);
    let head = take(count * 14)?;
    let mut planes: [&[u8]; 9] = [&[]; 9];
    for p in &mut planes {
        let n = u32_at(take(4)?);
        *p = take(n)?;
    }
    let unit = posting / PER_POSTING;
    let (mut v_at, mut i_at) = (0, 0);
    let mut out = Chunks::with_capacity(count);
    for h in head.chunks_exact(14) {
        let word = |i: usize| u16::from_le_bytes([h[i], h[i + 1]]) as usize;
        let key = (word(0), word(2), word(4));
        let (verts, tris) = (u32_at(&h[6..10]), u32_at(&h[10..14]));
        let mut positions = Vec::with_capacity(verts);
        let mut normals = Vec::with_capacity(verts);
        let mut last = [0u16; 3];
        let mut last_n = [0u8; 2];
        for k in v_at..v_at + verts {
            for a in 0..3 {
                let d = u16::from_le_bytes([*planes[a * 2].get(k)?, *planes[a * 2 + 1].get(k)?]);
                last[a] = last[a].wrapping_add(d);
            }
            positions.push([last[0] as i16 as f32 * unit, y0 + last[1] as f32 * dy, last[2] as i16 as f32 * unit]);
            for a in 0..2 {
                last_n[a] = last_n[a].wrapping_add(*planes[6 + a].get(k)?);
            }
            normals.push(oct_decode(last_n));
        }
        v_at += verts;
        let mut indices = Vec::with_capacity(tris * 3);
        let mut seen = 0u32;
        for _ in 0..tris * 3 {
            let (mut code, mut shift) = (0u32, 0);
            loop {
                let b = *planes[8].get(i_at)?;
                i_at += 1;
                code |= ((b & 0x7f) as u32) << shift;
                shift += 7;
                if b < 0x80 {
                    break;
                }
            }
            if code == 0 {
                indices.push(seen);
                seen += 1;
            } else {
                indices.push(seen.checked_sub(code)?);
            }
        }
        out.insert(key, ChunkGeometry { positions, normals, indices });
    }
    Some(out)
}

/// The ground's textures, baked apart from the meshes because they differ with
/// the realistic palette (only the WebGPU build's, `ground_s1_real.bin`).
pub(crate) struct GroundTexels {
    pub cols: usize,
    pub mask: MaskTexels,
    pub colour: Vec<u8>,
    pub normal: Vec<u8>,
    /// Mean crown colour of each kind (`vegetation::canopy::Canopy`).
    pub colours: [[f32; 3]; 3],
    /// By chunk, row by row: all under the sea.
    pub under: Vec<bool>,
}

const GROUND_MAGIC: &[u8; 4] = b"GRD1";

pub(crate) fn ground_file(stride: usize, real: bool) -> String {
    format!("assets/terrain/ground_s{stride}{}.bin", if real { "_real" } else { "" })
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_ground(stride: usize, real: bool, size: (usize, usize)) -> Option<GroundTexels> {
    let bytes = datafs::take(ground_file(stride, real)).ok()?;
    let ground = decode_ground(&bytes, size);
    if ground.is_none() {
        bevy::log::warn!("terrain: {} is not for this grid", ground_file(stride, real));
    }
    ground
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_ground(_stride: usize, _real: bool, _size: (usize, usize)) -> Option<GroundTexels> {
    None
}

/// Texels' bytes as planes, each byte the difference from the one on its left
/// (from the one above at the start of a row): what gzip then squeezes.
#[cfg(test)]
fn pack(out: &mut Vec<u8>, data: &[u8], w: usize, per_texel: usize) {
    for c in 0..per_texel {
        for (i, px) in data.chunks_exact(per_texel).enumerate() {
            let before = if i % w > 0 { data[(i - 1) * per_texel + c] } else if i >= w { data[(i - w) * per_texel + c] } else { 0 };
            out.push(px[c].wrapping_sub(before));
        }
    }
}

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
fn unpack(from: &[u8], w: usize, per_texel: usize, spare: usize) -> Vec<u8> {
    let n = from.len() / per_texel;
    let mut data = vec![0u8; from.len()];
    data.reserve_exact(spare);
    for c in 0..per_texel {
        let plane = &from[c * n..(c + 1) * n];
        for (i, d) in plane.iter().enumerate() {
            let before = if i % w > 0 { data[(i - 1) * per_texel + c] } else if i >= w { data[(i - w) * per_texel + c] } else { 0 };
            data[i * per_texel + c] = d.wrapping_add(before);
        }
    }
    data
}

#[cfg(test)]
pub(crate) fn encode_ground(g: &GroundTexels) -> Vec<u8> {
    let mut out = GROUND_MAGIC.to_vec();
    for n in [g.mask.nx, g.mask.ny, g.cols, g.under.len()] {
        out.extend_from_slice(&(n as u32).to_le_bytes());
    }
    for x in g.colours.iter().flatten() {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.extend(g.under.iter().map(|u| *u as u8));
    pack(&mut out, &g.mask.data, g.mask.nx, 4);
    pack(&mut out, &g.colour, g.cols, 4);
    pack(&mut out, &g.normal, g.cols, 8);
    out
}

/// `None` if the bytes are not a bake of a grid of `(cols, rows)`.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub(crate) fn decode_ground(bytes: &[u8], (cols, rows): (usize, usize)) -> Option<GroundTexels> {
    if bytes.get(..4)? != GROUND_MAGIC {
        return None;
    }
    let word = |i: usize| Some(u32::from_le_bytes(bytes.get(i..i + 4)?.try_into().ok()?) as usize);
    let (nx, ny, file_cols, chunks) = (word(4)?, word(8)?, word(12)?, word(16)?);
    let mut at = 20;
    if file_cols != cols || bytes.len() != at + 36 + chunks + nx * ny * 4 + cols * rows * 12 {
        return None;
    }
    let mut colours = [[0.0f32; 3]; 3];
    for x in colours.iter_mut().flatten() {
        *x = f32::from_le_bytes(bytes[at..at + 4].try_into().ok()?);
        at += 4;
    }
    let under = bytes[at..at + chunks].iter().map(|b| *b != 0).collect();
    at += chunks;
    // the mask's mips are appended later: room for them now (`ground_mask::image`)
    let mask = unpack(&bytes[at..at + nx * ny * 4], nx, 4, nx * ny * 2);
    at += nx * ny * 4;
    let colour = unpack(&bytes[at..at + cols * rows * 4], cols, 4, 0);
    at += cols * rows * 4;
    let normal = unpack(&bytes[at..at + cols * rows * 8], cols, 8, 0);
    Some(GroundTexels { cols, mask: MaskTexels { nx, ny, data: mask }, colour, normal, colours, under })
}
