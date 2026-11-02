//! Blender-authored meshes baked by `scripts/build_models.py`.
//! Compiled into the desktop binary; in the browser they come with the data
//! files (`datafs`), so the wasm stays small (2026-10-30). One mesh and
//! material per symbol.
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use std::{collections::HashMap, sync::OnceLock};

/// The text of a bake in `assets/models/`, read once.
macro_rules! bake {
    ($name:literal) => {{
        #[cfg(not(target_arch = "wasm32"))]
        let text: std::borrow::Cow<'static, str> = include_str!(concat!("../../../assets/models/", $name, ".json")).into();
        #[cfg(target_arch = "wasm32")]
        let text: std::borrow::Cow<'static, str> = datafs::take(concat!("assets/models/", $name, ".json"))
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_else(|| panic!("assets/models/{}.json is not in the page's data", $name))
            .into();
        text
    }};
}

#[derive(serde::Deserialize)]
pub struct Model {
    pub positions: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub wood: Vec<bool>,
    /// The plants (`vegetation.json`): normals as authored (a leaf card
    /// lights as part of its crown) and uvs into the foliage atlas. Empty in
    /// `meshes.json`: normals come from the faces, uvs from the atlas grey.
    #[serde(default)]
    pub normals: Vec<[f32; 3]>,
    #[serde(default)]
    pub uvs: Vec<[f32; 2]>,
}

pub fn model(name: &str) -> &'static Model {
    static MODELS: OnceLock<HashMap<String, Model>> = OnceLock::new();
    &MODELS.get_or_init(|| {
        serde_json::from_str(&bake!("meshes"))
            .expect("valid Blender mesh bake")
    })[name]
}

/// One piece of the town kit (`scripts/build_town_models.py`): landmarks,
/// props, vehicle and people variants. Same layout as [`Model`], no bark mask;
/// `footprint` is the (width, depth) a landmark was authored at.
#[derive(serde::Deserialize)]
pub struct Kit {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub footprint: Option<[f32; 2]>,
    /// Set on the animated parts of a unit (`<unit>.<part>` in `units.json`).
    pub rig: Option<Rig>,
    /// What each vertex is made of (`materials::Surface` ids), where the bake
    /// says: the units. Empty elsewhere.
    #[serde(default)]
    pub surfaces: Vec<u8>,
}

/// How an animated part of a unit moves: about `pivot` (model space, metres)
/// around `axis`, in the way `kind` names (wheel, prop, swing, tool, blink).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Rig {
    #[serde(deserialize_with = "vec3")]
    pub pivot: Vec3,
    #[serde(deserialize_with = "vec3")]
    pub axis: Vec3,
    pub kind: String,
    #[serde(default)]
    pub phase: f32,
}

fn vec3<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Vec3, D::Error> {
    <[f32; 3] as serde::Deserialize>::deserialize(de).map(Vec3::from_array)
}

fn parse_kit(json: &str, what: &str) -> HashMap<String, Kit> {
    // Decode straight into the vertex arrays. A serde_json::Value tree keeps
    // coordinates as generic values in nested arrays; cloning its fields
    // increases that temporary heap. Wasm's linear memory never shrinks after this peak.
    serde_json::from_str(json).unwrap_or_else(|e| panic!("invalid {what} bake: {e}"))
}

pub fn kit(name: &str) -> &'static Kit {
    static KIT: OnceLock<HashMap<String, Kit>> = OnceLock::new();
    KIT.get_or_init(|| parse_kit(&bake!("town"), "town kit"))
        .get(name)
        .unwrap_or_else(|| panic!("town kit has no `{name}`"))
}

/// The island's own models (`scripts/build_island_models.py`): the rocca, the
/// stone tower, the sailing and moored boats. Same layout as the town kit.
pub fn island(name: &str) -> &'static Kit {
    static ISLAND: OnceLock<HashMap<String, Kit>> = OnceLock::new();
    ISLAND
        .get_or_init(|| parse_kit(&bake!("island"), "island kit"))
        .get(name)
        .unwrap_or_else(|| panic!("island kit has no `{name}`"))
}

/// The coast's rocks (`scripts/build_coast_models.py`). Same layout as the town kit.
pub fn coast(name: &str) -> &'static Kit {
    static COAST: OnceLock<HashMap<String, Kit>> = OnceLock::new();
    COAST
        .get_or_init(|| parse_kit(&bake!("coast"), "coast kit"))
        .get(name)
        .unwrap_or_else(|| panic!("coast kit has no `{name}`"))
}

/// An island model as a mesh.
pub fn island_mesh(name: &str) -> Mesh {
    let m = island(name);
    // Landmark meshes are PauseSolid and Mario reads them for collisions.
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, m.colors.iter().map(|c| [c[0], c[1], c[2], 1.0]).collect::<Vec<_>>());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; m.positions.len()]);
    mesh.insert_indices(Indices::U32(m.indices.clone()));
    mesh.compress_indices();
    mesh
}

/// What fire adds to a plant of [`Variety`]: the skeleton of fine twigs inside
/// its crown and the debris on the ground (`debris` flags the latter). The
/// trunk and boughs are the live plant's own, charred in place by the shader.
/// Colours are absolute sRGB.
#[derive(serde::Deserialize)]
pub struct Piece {
    pub positions: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub debris: Vec<bool>,
}

/// One variety of a plant (`scripts/build_vegetation_models.py`): the live
/// model, what the fire adds near the camera, and the same far away;
/// `canopy` and `bark` are the atlas rects (u0, v0, u1, v1) its distant
/// proxy is textured with.
#[derive(serde::Deserialize)]
pub struct Variety {
    pub live: Model,
    pub remains: Piece,
    pub low: Piece,
    pub canopy: [f32; 4],
    pub bark: [f32; 4],
}

/// The foliage atlas the plants' uvs point into (`assets/models/foliage_atlas.png`).
#[derive(serde::Deserialize)]
pub struct Atlas {
    /// A texel of flat linear grey 0.5: the vertex colour unchanged (remains,
    /// grass, the single models of `meshes.json`).
    pub grey: [f32; 2],
}

#[derive(serde::Deserialize)]
struct VegetationBake {
    atlas: Atlas,
    varieties: HashMap<String, Variety>,
}

fn vegetation_bake() -> &'static VegetationBake {
    static BAKE: OnceLock<VegetationBake> = OnceLock::new();
    BAKE.get_or_init(|| serde_json::from_str(&bake!("vegetation")).expect("valid vegetation bake"))
}

pub fn atlas() -> &'static Atlas {
    &vegetation_bake().atlas
}

/// The atlas image file: compiled into the desktop binary, from the page's
/// data in the browser, where it stays: every entry into a game builds the
/// plants again (`vegetation::spawn`), and a `take` failed the second time.
pub fn atlas_png() -> std::borrow::Cow<'static, [u8]> {
    #[cfg(not(target_arch = "wasm32"))]
    let bytes: std::borrow::Cow<'static, [u8]> = include_bytes!("../../../assets/models/foliage_atlas.png").as_slice().into();
    #[cfg(target_arch = "wasm32")]
    let bytes: std::borrow::Cow<'static, [u8]> = datafs::read("assets/models/foliage_atlas.png")
        .expect("assets/models/foliage_atlas.png is not in the page's data")
        .into();
    bytes
}

/// Every species with varieties, as `vegetation.rs` places them.
pub const SPECIES: [&str; 14] = [
    "pine", "aleppo", "cypress", "holm", "cork", "carob", "olive",
    "lentisk", "myrtle", "arbutus", "broom", "heather", "rosemary", "cistus",
];

/// The varieties of each species, in the order of the bake. All of one
/// species share a silhouette family; the species of a fuel group share
/// its outline (conifer, broadleaf, macchia), so the fuel still reads.
pub fn variety_names(species: &str) -> &'static [&'static str] {
    match species {
        "pine" => &["pine_1", "pine_2", "pine_3", "pine_4"],
        "aleppo" => &["aleppo_1", "aleppo_2", "aleppo_3"],
        "cypress" => &["cypress_1", "cypress_2"],
        "holm" => &["holm_1", "holm_2", "holm_3"],
        "cork" => &["cork_1", "cork_2"],
        "carob" => &["carob_1", "carob_2"],
        "olive" => &["olive_1", "olive_2", "olive_3"],
        "lentisk" => &["lentisk_1", "lentisk_2"],
        "myrtle" => &["myrtle_1", "myrtle_2"],
        "arbutus" => &["arbutus_1", "arbutus_2"],
        "broom" => &["broom_1", "broom_2"],
        "heather" => &["heather_1", "heather_2"],
        "rosemary" => &["rosemary_1", "rosemary_2"],
        "cistus" => &["cistus_1", "cistus_2"],
        _ => panic!("no varieties of `{species}`"),
    }
}

/// Which variety a plant is, from a number in `[0, 1)` (its yaw, so the same
/// plants are the same variety on every run).
pub fn pick_variety(species: &str, u: f32) -> &'static str {
    let names = variety_names(species);
    names[((u * 7919.0).fract() * names.len() as f32) as usize % names.len()]
}

pub fn variety(name: &str) -> &'static Variety {
    vegetation_bake()
        .varieties
        .get(name)
        .unwrap_or_else(|| panic!("vegetation has no variety `{name}`"))
}

/// The live model of a plant: a variety, or one of the single models.
pub fn live(name: &str) -> &'static Model {
    if name.contains('_') { &variety(name).live } else { model(name) }
}

fn units() -> &'static HashMap<String, Kit> {
    static UNITS: OnceLock<HashMap<String, Kit>> = OnceLock::new();
    UNITS.get_or_init(|| parse_kit(&bake!("units"), "unit models"))
}

/// The surface of each vertex goes in its colour's alpha
/// (`materials::Surface::alpha`), for the structure shader.
fn kit_to_mesh(m: &Kit, shift: Vec3, usage: RenderAssetUsages) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, usage);
    let positions: Vec<[f32; 3]> = m.positions.iter().map(|p| (Vec3::from_array(*p) - shift).to_array()).collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
    let alpha = |i: usize| m.surfaces.get(i).map_or(1.0, |&s| crate::materials::surface_alpha(s));
    let colors: Vec<[f32; 4]> = m.colors.iter().enumerate().map(|(i, c)| [c[0], c[1], c[2], alpha(i)]).collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; m.positions.len()]);
    mesh.insert_indices(Indices::U32(m.indices.clone()));
    mesh.compress_indices();
    mesh
}

/// The still body of a model of `scripts/build_unit_models.py` (firefighter,
/// engine, tanker), in its real colours (vertex colours, linear). It keeps its
/// CPU copy: Mario reads it, to bump into a team and to greet it.
pub fn unit_mesh(name: &str) -> Mesh {
    let m = units().get(name).unwrap_or_else(|| panic!("unit models have no `{name}`"));
    kit_to_mesh(m, Vec3::ZERO, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
}

/// The animated parts of a unit (wheels, propellers, limbs, beacons), each
/// with its mesh centred on its pivot, sorted by name.
pub fn unit_parts(name: &str) -> Vec<(String, Mesh, Rig)> {
    let prefix = format!("{name}.");
    let mut parts: Vec<_> = units()
        .iter()
        .filter_map(|(k, m)| {
            let rig = m.rig.clone()?;
            Some((k.strip_prefix(&prefix)?.to_string(), kit_to_mesh(m, rig.pivot, RenderAssetUsages::RENDER_WORLD), rig))
        })
        .collect();
    parts.sort_by(|a, b| a.0.cmp(&b.0));
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_models_load_and_stand_on_the_ground() {
        for (name, parts) in [("firefighter", 4), ("engine", 6), ("tanker", 2)] {
            let m = unit_mesh(name);
            assert!(m.count_vertices() > 0, "{name}");
            assert_eq!(unit_parts(name).len(), parts, "{name}: animated parts");
        }
    }

    /// Every variety is a valid pair of live plant and fire additions, within
    /// the vertex budget (the additions are drawn for every plant of a chunk
    /// the fire has touched, burnt or not), starting on the ground and not
    /// taller than the live plant.
    #[test]
    fn the_vegetation_varieties_are_within_budget() {
        for species in SPECIES {
            assert!(variety_names(species).len() >= 2, "{species}: varieties");
            for name in variety_names(species) {
                let v = variety(name);
                let hi = v.live.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
                // as `LIVE_TRIS`, `LIVE_VERTS` in scripts/build_vegetation_models.py
                assert!(v.live.indices.len() / 3 <= 140, "{name}: {} live triangles", v.live.indices.len() / 3);
                assert!(v.live.positions.len() <= 128, "{name}: {} live vertices", v.live.positions.len());
                let n = v.live.positions.len();
                assert!(v.live.normals.len() == n && v.live.uvs.len() == n, "{name}: normals and uvs per vertex");
                assert!(v.live.uvs.iter().flatten().all(|c| (0.0..=1.0).contains(c)), "{name}: uv outside the atlas");
                assert!(v.canopy.iter().chain(&v.bark).all(|c| (0.0..=1.0).contains(c)), "{name}: proxy rects");
                for (piece, tris, verts) in [(&v.remains, 120, 160), (&v.low, 16, 20)] {
                    assert!(!piece.indices.is_empty() && piece.indices.len() % 3 == 0, "{name}");
                    assert!(piece.indices.iter().all(|&i| (i as usize) < piece.positions.len()), "{name}");
                    assert!(piece.positions.iter().flatten().all(|c| c.is_finite()), "{name}");
                    assert!(piece.indices.len() / 3 <= tris && piece.positions.len() <= verts, "{name}: remains too heavy");
                    assert_eq!(piece.debris.len(), piece.positions.len(), "{name}");
                    let (plo, phi) = piece.positions.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])));
                    assert!(plo >= -0.05 && phi <= hi + 0.05, "{name}: additions outside the live plant ({plo}..{phi}, plant up to {hi})");
                }
            }
        }
    }

    #[test]
    fn the_town_kit_is_grounded_and_wound_outward() {
        for name in [
            "church", "chapel", "townhall", "school", "fire_station", "fuel", "water_tower", "substation", "farm", "barn",
            "silo", "mill", "industrial", "lighthouse", "cypress", "street_tree", "fountain", "bench", "umbrella_a",
            "lounger", "beach_hut", "boat", "sailboat", "pc_tent", "camp_tent", "caravan", "goal", "assembly_sign",
            "car_hatch", "car_sedan", "car_suv", "car_van", "car_ape", "car_bus", "car_camper", "person_man",
            "person_woman", "person_child", "person_elder",
        ] {
            let m = kit(name);
            assert!(!m.positions.is_empty(), "{name}");
            assert_eq!(m.positions.len(), m.normals.len(), "{name}");
            assert_eq!(m.indices.len() % 3, 0, "{name}");
            // Finding 11: a back-facing triangle draws nothing. Every face's
            // winding must agree with its baked normal.
            for tri in m.indices.chunks_exact(3) {
                let p: Vec<Vec3> = tri.iter().map(|i| Vec3::from(m.positions[*i as usize])).collect();
                let n = Vec3::from(m.normals[tri[0] as usize]);
                let g = (p[1] - p[0]).cross(p[2] - p[0]);
                assert!(g.dot(n) >= -1e-6, "{name}: a face is wound against its normal");
            }
        }
    }

    #[test]
    fn blender_bakes_are_valid_grounded_triangle_meshes() {
        for name in [
            "pedestrian",
            "firefighter",
            "car",
            "fire_engine",
            "pine",
            "oak",
            "chestnut",
            "bush",
            "olive",
            "cypress",
        ] {
            let m = model(name);
            assert!(!m.positions.is_empty(), "{name}");
            assert_eq!(m.positions.len(), m.colors.len(), "{name}");
            assert_eq!(m.positions.len(), m.wood.len(), "{name}");
            assert_eq!(m.indices.len() % 3, 0, "{name}");
            assert!(
                m.positions.iter().flatten().all(|v| v.is_finite()),
                "{name}"
            );
            let min_y = m
                .positions
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min);
            assert!(min_y >= -0.05 && min_y <= 0.1, "{name}: base {min_y}");
            for triangle in m.indices.chunks_exact(3) {
                let p: Vec<Vec3> = triangle
                    .iter()
                    .map(|i| Vec3::from(m.positions[*i as usize]))
                    .collect();
                assert!(
                    (p[1] - p[0]).cross(p[2] - p[0]).length_squared() > 1e-14,
                    "{name}: degenerate triangle"
                );
            }
            // Vegetation is copied many thousands of times: enforce its budget.
            if matches!(name, "pine" | "oak" | "chestnut" | "bush" | "olive" | "cypress") {
                assert!(
                    m.positions.len() <= 120 && m.indices.len() / 3 <= 160,
                    "{name}"
                );
            }
        }
    }
}
