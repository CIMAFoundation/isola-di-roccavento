//! Downloaded house models in place of the procedural houses (realistic
//! style, `KIOSK_SCANS=1`; 2026-11-07).
//!
//! The library is `assets/models/houses_real.{json,bin}`, baked by
//! `tools/buildings/build_real_houses.py` from models downloaded into
//! `buildings_src/` (now the Poly Haven CC0 facade kit, put together into
//! sixteen houses of two to four storeys; see the script and
//! `assets/models/CREDITS.txt`). Each model has two levels of detail.
//!
//! A house of the scenario (its rectangle in the data, never touched) takes
//! the model whose footprint is nearest its own in shape and size, scaled
//! the same along both sides but for a small correction (at most 15 %),
//! raised a little more than wide (the diorama's houses are taller than
//! life, `visual_scale`), turned to the road on its long side, standing on
//! its highest corner with a stone footing down the slope. The choice is
//! the house's own, always the same. Shops, rustic houses and footprints no
//! model fits keep the procedural house (`houses`).
//!
//! One mesh per model and level, drawn for every house that uses it (Bevy
//! batches them): the house's colours, its ground, its number and the
//! fire's state are a column of a small texture (`materials::house::
//! data_image`), found by the instance's [`MeshTag`]. Three entities per
//! house: the detailed model near, the light one far (`VisibilityRange`),
//! and the light one again for the sun's shadow (a ranged entity casts
//! none), which is also what Mario walks on.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{RenderLayers, VisibilityRange};
use bevy::mesh::{Indices, MeshTag, PrimitiveTopology};
use bevy::prelude::*;
use scenario::{Building, Pos, Scenario};

use super::houses::{self, Context, Type};
use super::{hash01, linear, Builder, TOY_SCALE};
use crate::materials::house::{HouseMaterial, DATA_ROWS};
use crate::materials::masonry::Part;

/// A model's height over its footprint's scale: the diorama's houses are
/// drawn taller than wide (`visual_scale::BUILDING_HEIGHT` over
/// `BUILDING_FOOTPRINT`), the models a little.
const RAISE: f32 = 1.15;

/// How far a model may be stretched or squeezed along one side, beyond the
/// scale it shares with the other.
const STRETCH: f32 = 1.15;

/// Distance (m, camera to the house's middle) where the light model takes
/// over; `KIOSK_HOUSE_LOD_M` to move it.
const LOD_M: f32 = 420.0;
/// The same on a light profile (`Graphics::light`, 2026-11-09).
const LIGHT_LOD_M: f32 = 220.0;

#[derive(serde::Deserialize)]
struct Index {
    houses: Vec<Entry>,
}

#[derive(serde::Deserialize)]
struct Entry {
    name: String,
    storeys: u8,
    /// Width (x), height to the eave, depth (z), metres.
    size: [f32; 3],
    lods: Vec<LodEntry>,
}

#[derive(serde::Deserialize)]
struct LodEntry {
    vertices: usize,
    indices: usize,
    vertex_offset: usize,
    index_offset: usize,
}

struct Model {
    name: String,
    storeys: usize,
    w: f32,
    d: f32,
    /// Detailed and light.
    meshes: [Handle<Mesh>; 2],
    triangles: [usize; 2],
}

pub(super) struct Library {
    models: Vec<Model>,
}

/// A file of `assets/models/`: of the source tree on the desktop, of the
/// page's data in the browser (only with `?scans=1`).
fn file(name: &str) -> Option<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models").join(name));
    #[cfg(target_arch = "wasm32")]
    let bytes = datafs::take(format!("assets/models/{name}"));
    bytes.map_err(|e| warn!("house models: assets/models/{name}: {e}")).ok()
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

impl Library {
    /// The library, with the scans only (`KIOSK_REAL_HOUSES=0`: the
    /// procedural houses, for a comparison). `None` if the files are missing.
    pub(super) fn load(meshes: &mut Assets<Mesh>) -> Option<Library> {
        crate::models::scans()?;
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::var("KIOSK_REAL_HOUSES").as_deref() == Ok("0") {
            return None;
        }
        let index: Index = serde_json::from_slice(&file("houses_real.json")?).map_err(|e| warn!("house models: {e}")).ok()?;
        let blob = file("houses_real.bin")?;
        let mut models = Vec::new();
        for e in &index.houses {
            if e.lods.len() < 2 {
                continue;
            }
            let mut handles = Vec::new();
            let mut tris = Vec::new();
            for l in e.lods.iter().take(2) {
                let mesh = decode(&blob, l).or_else(|| {
                    warn!("house models: {} is cut short", e.name);
                    None
                })?;
                tris.push(l.indices / 3);
                handles.push(meshes.add(mesh));
            }
            models.push(Model {
                name: e.name.clone(),
                storeys: e.storeys as usize,
                w: e.size[0],
                d: e.size[2],
                meshes: [handles[0].clone(), handles[1].clone()],
                triangles: [tris[0], tris[1]],
            });
        }
        if models.is_empty() {
            return None;
        }
        info!("house models: {} in the library ({} .. {} triangles near)", models.len(), models.iter().map(|m| m.triangles[0]).min().unwrap_or(0), models.iter().map(|m| m.triangles[0]).max().unwrap_or(0));
        Some(Library { models })
    }
}

/// One level of a model from the bake: 13 bytes a vertex (`i16` position in
/// millimetres, `i8` normal, sRGB or tint grey and the role), `u16` indices.
fn decode(blob: &[u8], l: &LodEntry) -> Option<Mesh> {
    let vb = blob.get(l.vertex_offset..l.vertex_offset + l.vertices * 13)?;
    let ib = blob.get(l.index_offset..l.index_offset + l.indices * 2)?;
    let mut positions = Vec::with_capacity(l.vertices);
    let mut normals = Vec::with_capacity(l.vertices);
    let mut colors = Vec::with_capacity(l.vertices);
    for v in vb.chunks_exact(13) {
        let i = |k: usize| i16::from_le_bytes([v[k], v[k + 1]]) as f32 / 1000.0;
        positions.push([i(0), i(2), i(4)]);
        let n = Vec3::new(v[6] as i8 as f32, v[7] as i8 as f32, v[8] as i8 as f32).normalize_or(Vec3::Y);
        normals.push(n.to_array());
        let role = v[12];
        // a tinted role carries its factor / 2 (`house.wesl`), a baked one
        // (9) its own colour
        let rgb = if role == 9 {
            [srgb_to_linear(v[9] as f32 / 255.0), srgb_to_linear(v[10] as f32 / 255.0), srgb_to_linear(v[11] as f32 / 255.0)]
        } else {
            [v[9] as f32 / 256.0; 3]
        };
        colors.push([rgb[0], rgb[1], rgb[2], (40 + role as u32) as f32 / 255.0]);
    }
    let indices: Vec<u16> = ib.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    if indices.iter().any(|&i| i as usize >= l.vertices) {
        return None;
    }
    let n = positions.len();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    // the second uv marks a house model to the shader (`house.wesl`); the
    // first is there because Bevy's PBR input reads a second uv only with a
    // first
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32, 0.0]; n]);
    mesh.insert_indices(Indices::U16(indices));
    Some(mesh)
}

/// A model on a footprint.
pub(super) struct Placement {
    model: usize,
    transform: Transform,
    /// The house's column of the data texture (`materials::house::DATA_ROWS`
    /// texels: wall, trim, roof, shutters, door, frames).
    pub(super) texels: [[f32; 4]; DATA_ROWS as usize],
}

/// The model for footprint `b` (its ring cleaned and counter-clockwise), or
/// `None` for the procedural house. Its stone footing goes into `out`.
pub(super) fn place(lib: &Library, scn: &Scenario, b: &Building, ctx: &Context, ring: &[Pos], out: &mut Builder) -> Option<Placement> {
    if ring.len() != 4 {
        return None;
    }
    let area = b.area();
    let st = houses::style(b, area, ctx.outskirts(b));
    if matches!(st.ty, Type::Rustico | Type::Bottega) {
        return None;
    }
    // the footprint's frame, from its longest edge (as `houses::emit`)
    let mut best = (0.0, [1.0f32, 0.0f32]);
    for i in 0..4 {
        let (a, c) = (ring[i], ring[(i + 1) % 4]);
        let (dx, dy) = (c.x - a.x, c.y - a.y);
        let l = (dx * dx + dy * dy).sqrt();
        if l > best.0 {
            best = (l, [dx / l, dy / l]);
        }
    }
    let u = best.1;
    let v = [-u[1], u[0]];
    let c0 = [ring.iter().map(|p| p.x).sum::<f32>() / 4.0, ring.iter().map(|p| p.y).sum::<f32>() / 4.0];
    let (mut smin, mut smax, mut tmin, mut tmax) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in ring {
        let (s, t) = ((p.x - c0[0]) * u[0] + (p.y - c0[1]) * u[1], (p.x - c0[0]) * v[0] + (p.y - c0[1]) * v[1]);
        smin = smin.min(s);
        smax = smax.max(s);
        tmin = tmin.min(t);
        tmax = tmax.max(t);
    }
    let (sm, tm) = ((smin + smax) * 0.5, (tmin + tmax) * 0.5);
    let c = [c0[0] + u[0] * sm + v[0] * tm, c0[1] + u[1] * sm + v[1] * tm];
    // drawn sizes, with the diorama's magnification
    let (big_a, big_b) = ((smax - smin) * TOY_SCALE, (tmax - tmin) * TOY_SCALE);
    if big_a < 6.0 || big_b < 6.0 {
        return None;
    }

    // the model: nearest in shape, then in size and storeys; among the near
    // ties, the house's own pick
    let mut fits: Vec<(f32, usize, f32, f32, f32)> = Vec::new();
    for (k, m) in lib.models.iter().enumerate() {
        let s = (big_a / m.w * big_b / m.d).sqrt();
        let (cx, cz) = (big_a / (m.w * s), big_b / (m.d * s));
        if cx > STRETCH || cx < 1.0 / STRETCH || cz > STRETCH || cz < 1.0 / STRETCH {
            continue;
        }
        if !(1.3..=2.8).contains(&s) {
            continue;
        }
        let ds = m.storeys as f32 - st.storeys as f32;
        let cost = cx.ln().powi(2) + cz.ln().powi(2) + 0.004 * ds * ds + 0.05 * (s / TOY_SCALE).ln().powi(2);
        fits.push((cost, k, s, cx, cz));
    }
    if fits.is_empty() {
        return None;
    }
    fits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let near: Vec<_> = fits.iter().filter(|f| f.0 <= fits[0].0 + 0.012).collect();
    let (_, k, s, cx, cz) = *near[((hash01(b.id as u64, 0x5100) * near.len() as f32) as usize).min(near.len() - 1)];

    // the front, to the nearer road of the two long sides (the model's
    // front is its +z, which lands on -v)
    let side = |sgn: f32| {
        let d = big_b * 0.5 + 6.0;
        ctx.road_distance([c[0] + v[0] * d * sgn, c[1] + v[1] * d * sgn])
    };
    let uu = if side(-1.0) <= side(1.0) { u } else { [-u[0], -u[1]] };
    let x_axis = Vec3::new(uu[0], 0.0, -uu[1]);
    let z_axis = Vec3::new(uu[1], 0.0, uu[0]);
    let rotation = Quat::from_mat3(&Mat3::from_cols(x_axis, Vec3::Y, z_axis));

    // the ground: the floor on the highest corner, the footing to the lowest
    let ground = |s_: f32, t_: f32| scn.terrain.height_at(Pos { x: c[0] + s_ * u[0] + t_ * v[0], y: c[1] + s_ * u[1] + t_ * v[1] });
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (a, bb) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (0.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        let g = ground(a * big_a * 0.5, bb * big_b * 0.5);
        lo = lo.min(g);
        hi = hi.max(g);
    }
    let transform = Transform {
        translation: Vec3::new(c[0], hi, -c[1]),
        rotation,
        scale: Vec3::new(s * cx, s * RAISE, s * cz),
    };

    // the footing: stone from below the lowest corner to just over the floor,
    // a hair inside the walls
    let base = lo - 1.5;
    let worn = (st.worn * 0.999).clamp(0.0, 0.999);
    out.house_span(base, worn);
    out.part = Part::Stone as u8;
    let plinth = {
        let w = linear([st.wall[0] * 0.55 + 0.2, st.wall[1] * 0.55 + 0.18, st.wall[2] * 0.55 + 0.16]);
        [w[0] * 0.75, w[1] * 0.73, w[2] * 0.70]
    };
    let (ha, hb) = (big_a * 0.5 - 0.04, big_b * 0.5 - 0.04);
    let corner = |a: f32, bb: f32, y: f32| [c[0] + a * u[0] + bb * v[0], y, -(c[1] + a * u[1] + bb * v[1])];
    let top = hi + 0.4;
    if top > base {
        for (a0, b0, a1, b1, n) in [(-ha, -hb, ha, -hb, [-v[0], -v[1]]), (ha, -hb, ha, hb, u), (ha, hb, -ha, hb, v), (-ha, hb, -ha, -hb, [-u[0], -u[1]])] {
            houses::face(out, [corner(a0, b0, base), corner(a1, b1, base), corner(a1, b1, top), corner(a0, b0, top)], [n[0], 0.0, -n[1]], plinth);
        }
    }
    out.part = 0;
    out.house_span(-1.0e4, 0.5);

    let shutter = st.shutters.unwrap_or(houses::SHUTTERS[0]);
    let rgb = |c: [f32; 3], a: f32| {
        let l = linear(c);
        [l[0], l[1], l[2], a]
    };
    let texels = [
        rgb(st.wall, worn),
        rgb(st.trim, base),
        rgb(st.roof_c, if st.slate { 1.0 } else { 0.0 }),
        rgb(shutter, 0.0),
        rgb(st.door, 0.0),
        rgb(st.frame, 0.0),
    ];
    Some(Placement { model: k, transform, texels })
}

/// The entities of the placed houses, house `i` reading column `i`.
pub(super) fn spawn(commands: &mut Commands, lib: &Library, placed: &[Placement], material: &Handle<HouseMaterial>) {
    let default_m = if crate::quality::Graphics::light() { LIGHT_LOD_M } else { LOD_M };
    #[cfg(not(target_arch = "wasm32"))]
    let lod_m = std::env::var("KIOSK_HOUSE_LOD_M").ok().and_then(|v| v.parse().ok()).unwrap_or(default_m);
    #[cfg(target_arch = "wasm32")]
    let lod_m = default_m;
    let mut tris = [0usize; 2];
    let mut uses = vec![0usize; lib.models.len()];
    for (i, p) in placed.iter().enumerate() {
        let m = &lib.models[p.model];
        uses[p.model] += 1;
        tris[0] += m.triangles[0];
        tris[1] += m.triangles[1];
        let tag = MeshTag::new(i as u32);
        commands.spawn((
            (Mesh3d(m.meshes[0].clone()), MeshMaterial3d(material.clone()), p.transform, tag.clone()),
            VisibilityRange { start_margin: 0.0..0.0, end_margin: lod_m..lod_m, use_aabb: true },
            bevy::light::NotShadowCaster,
        ));
        commands.spawn((
            (Mesh3d(m.meshes[1].clone()), MeshMaterial3d(material.clone()), p.transform, tag.clone()),
            VisibilityRange { start_margin: lod_m..lod_m, end_margin: f32::MAX..f32::MAX, use_aabb: true },
            bevy::light::NotShadowCaster,
            // what Mario walks on: the light model, which has the walls and the roof
            crate::terrain_mesh::PauseSolid,
        ));
        // the sun's shadow (a ranged entity casts none): the light model
        commands.spawn((
            (Mesh3d(m.meshes[1].clone()), MeshMaterial3d(material.clone()), p.transform, tag.clone()),
            RenderLayers::layer(crate::terrain_mesh::SHADOW_LAYER),
        ));
    }
    let used: Vec<String> = lib.models.iter().zip(&uses).filter(|(_, &n)| n > 0).map(|(m, n)| format!("{} x{n}", m.name)).collect();
    info!(
        "house models: {} houses, {:.2} M triangles near / {:.2} M far; {}",
        placed.len(),
        tris[0] as f32 / 1e6,
        tris[1] as f32 / 1e6,
        used.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every level of every model decodes, its indices in range, the glass
    /// (role 3) in one run at the end (the lit windows), and the detailed
    /// level within the budget.
    #[test]
    fn the_bake_decodes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models");
        let index: Index = serde_json::from_slice(&std::fs::read(dir.join("houses_real.json")).unwrap()).unwrap();
        let blob = std::fs::read(dir.join("houses_real.bin")).unwrap();
        assert!(index.houses.len() >= 10);
        for e in &index.houses {
            assert_eq!(e.lods.len(), 2, "{}", e.name);
            assert!(e.lods[0].indices / 3 <= 20_000, "{}: {} triangles", e.name, e.lods[0].indices / 3);
            assert!(e.lods[1].indices < e.lods[0].indices / 5, "{}", e.name);
            for l in &e.lods {
                let mesh = decode(&blob, l).unwrap_or_else(|| panic!("{}", e.name));
                let Some(bevy::mesh::VertexAttributeValues::Float32x4(c)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!() };
                let glass: Vec<bool> = c.iter().map(|c| (c[3] * 255.0).round() as u32 == 43).collect();
                let first = glass.iter().position(|&g| g).unwrap_or(glass.len());
                assert!(glass[first..].iter().all(|&g| g), "{}: glass not last", e.name);
            }
        }
    }
}
