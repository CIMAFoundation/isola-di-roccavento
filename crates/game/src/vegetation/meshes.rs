//! The plants' meshes: for each variety one mesh per level of detail, in
//! model space (the plant one unit tall, unrotated, untinted: the instances
//! place and tint it), in the vertex format the plant pipeline reads
//! (`render.rs`, `shaders/plant_common.wesl`). They never go through Bevy's
//! `Mesh` assets: built once, uploaded once, never touched again.
//!
//! | bytes  | attribute                                             |
//! |--------|-------------------------------------------------------|
//! | 0..12  | position, `Float32x3`                                 |
//! | 12..16 | normal and what the vertex is ([`Kind`]), `Snorm8x4`  |
//! | 16..20 | uv into the foliage atlas, `Unorm16x2`                |
//! | 20..28 | colour (linear) and occlusion, `Unorm16x4`            |
//! | 28..32 | a scanned card's right axis, its stretch / 2, `Snorm8x4` |
//! | 32..36 | a scanned card's up axis, `Snorm8x4`                  |
//!
//! The colour of a leaf or wood vertex is the model's own shading, halved;
//! the plant's tint (also halved) multiplies it and the atlas paints it. What
//! the fire adds (twigs, debris) has absolute colours and points at the
//! atlas's flat grey. On the leaves of a lobed stand-in ([`lobed_mesh`]) the
//! uv is only the corner of the canopy tile: the shader paints the tile over
//! them (`PLANT_CANOPY`).

use std::collections::BTreeMap;

use bevy::prelude::*;

/// Bytes per vertex (see the table above).
pub const VERTEX_BYTES: u64 = 36;

/// The levels of detail of a variety.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    /// The authored model with its leaf cards (desktop, near the camera).
    Full = 0,
    /// A six-sided crown and a trunk wearing the species' canopy tile; in
    /// the browser the lobed stand-in instead ([`lobed_mesh`]).
    Proxy = 1,
    /// The crown as a bipyramid, for the far distance.
    Far = 2,
    /// What fire adds (twig skeleton, ash and fallen wood), near and far.
    RemainsNear = 3,
    RemainsFar = 4,
    /// A scanned plant's nearest level (`models::scans`, `lod0`), where it
    /// has one; then [`Tier::Mid`] (`lod1`), [`Tier::Full`] (`lod2`), and
    /// boxes made of its `lod2` in place of the proxies ([`Tier::Proxy`]
    /// small ones, [`Tier::Far`] fewer and larger: `Builder::scan_voxels`).
    Near = 5,
    Mid = 6,
}

pub const TIERS: usize = 7;

/// What a vertex is to the fire, in the normal's fourth byte (`kind` in the
/// shader): leaves burn away, wood chars and stays, twigs and debris appear
/// where the fire has been.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Leaf,
    Wood,
    Twig,
    Debris,
}

impl Kind {
    fn snorm(self) -> i8 {
        match self {
            Kind::Leaf => 127,
            Kind::Wood => 0,
            Kind::Twig => -64,
            Kind::Debris => -127,
        }
    }
}

/// One variety's mesh for one tier.
pub struct PlantMesh {
    /// [`VERTEX_BYTES`] per vertex.
    pub vertices: Vec<u8>,
    pub indices: Vec<u16>,
    /// Drawn with the cut-out pipeline: leaf cards cut from the atlas's
    /// alpha, burn holes, remains uncovered by the passing fire.
    pub cutout: bool,
    /// Both faces drawn (leaf cards, twigs); a closed crown culls its back.
    pub double_sided: bool,
    /// Its leaves are painted with the canopy tile by the shader, in three
    /// projections (`PLANT_CANOPY`): the lobed stand-in.
    pub canopy: bool,
}

impl PlantMesh {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    #[cfg(test)]
    pub fn vertex_count(&self) -> usize {
        self.vertices.len() / VERTEX_BYTES as usize
    }

    /// The position of vertex `i` (for the tests).
    #[cfg(test)]
    pub fn position(&self, i: usize) -> Vec3 {
        let o = i * VERTEX_BYTES as usize;
        let f = |k: usize| f32::from_le_bytes(self.vertices[o + 4 * k..o + 4 * k + 4].try_into().unwrap());
        Vec3::new(f(0), f(1), f(2))
    }

    /// The normal of vertex `i` (for the tests).
    #[cfg(test)]
    pub fn normal(&self, i: usize) -> Vec3 {
        let o = i * VERTEX_BYTES as usize + 12;
        let s = |k: usize| self.vertices[o + k] as i8 as f32 / 127.0;
        Vec3::new(s(0), s(1), s(2))
    }
}

/// `name`'s mesh for `tier`.
pub fn variety_mesh(name: &'static str, tier: Tier) -> PlantMesh {
    let mut b = Builder::default();
    let scan = crate::models::scan_of(name).and_then(|plant| {
        // the nearest level there is, from the tier's on (a level not built
        // yet); the proxies only their own
        let levels: &[&str] = match tier {
            Tier::Near => &["lod0", "lod1", "lod2"],
            Tier::Mid => &["lod1", "lod2"],
            Tier::Full => &["lod2"],
            Tier::Proxy => &["lod3"],
            Tier::Far | Tier::RemainsNear | Tier::RemainsFar => return None,
        };
        let scans = crate::models::scans()?;
        levels.iter().find_map(|level| scans.level(plant, level))
    });
    let level = |l: &str| crate::models::scan_of(name).and_then(|p| crate::models::scans()?.level(p, l));
    // a scanned plant past its `lod2`: in boxes, fine then coarse, made of
    // that level; `KIOSK_SCAN_FAR=grumi`: the cards and the lumps it was
    // before, to compare
    if let (Tier::Proxy | Tier::Far, false) = (tier, far_lumps()) {
        if let (Some((vertices, indices, leaf_rgb)), Some(cards)) = (level("lod2"), cards()) {
            let grain = if tier == Tier::Proxy { VOXELS_FINE } else { VOXELS_COARSE };
            b.scan_voxels(&vertices, &indices, leaf_tint(leaf_rgb, super::canopy::crown_colour(name)), scan_stretch(name), nominal_height_m(name), grain, cards);
            if !b.indices.is_empty() {
                return b.finish(false, false);
            }
            b = Builder::default();
        }
    }
    if tier == Tier::Far {
        if let Some((vertices, indices, _)) = level("lod3") {
            let colour = scan_leaf_colour(name).unwrap_or_else(|| super::canopy::crown_colour(name));
            b.scan_lumps(&vertices, &indices, colour, scan_stretch(name));
            return b.finish(false, false);
        }
    }
    if let Some((vertices, indices, leaf_rgb)) = scan {
        b.scan(&vertices, &indices, leaf_rgb, super::canopy::crown_colour(name), scan_stretch(name));
        return b.finish(true, true);
    }
    // a scanned plant's own wood stays, charred: of the remains only the
    // debris at its foot
    let debris_only = crate::models::scan_of(name).is_some();
    match tier {
        Tier::RemainsNear if debris_only => b.debris(&crate::models::variety(name).remains),
        // the far remains have no debris of their own: the near ones'
        // (none, for some: then no remains, `vegetation::spawn`)
        Tier::RemainsFar if debris_only => b.debris(&crate::models::variety(name).remains),
        Tier::Near | Tier::Mid | Tier::Full => b.model(crate::models::live(name)),
        Tier::Proxy => b.proxy(name, false),
        Tier::Far => b.proxy(name, true),
        Tier::RemainsNear => b.remains(&crate::models::variety(name).remains),
        Tier::RemainsFar => b.remains(&crate::models::variety(name).low),
    }
    let closed = matches!(tier, Tier::Proxy | Tier::Far);
    b.finish(!closed, !closed)
}

/// Half the width of a variety's crown seen from above, model units.
pub fn crown_radius(name: &str) -> f32 {
    let (lo, hi) = proxy_of(name).crown;
    0.5 * (hi.x - lo.x).max(hi.z - lo.z)
}

/// The linear tone a variety's leaves carry over the atlas colour, averaged
/// over the crown.
pub fn crown_tone(name: &str) -> f32 {
    let p = proxy_of(name);
    (0.5 * (p.shade_low + p.shade_high)).powf(2.2)
}

/// What a scanned plant's leaves carry over their cards, halved (the
/// colour travels halved and is doubled): it brings their mean colour
/// (`leaf_rgb`) to the variety's crown (`target`), as far as doubling them
/// goes. The scans are darker than the palette: a pine's needles reach a
/// third of its green and stay a dark olive.
fn leaf_tint(leaf_rgb: [f32; 3], target: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|k| (0.5 * target[k] / leaf_rgb[k].max(1e-3)).clamp(0.05, 1.0))
}

/// The colour a scanned plant's leaves are drawn in, their mean (of its
/// `lod2`, the level it has up to where its far model begins); `None` for
/// an authored plant. What stands for the plant far away is in this
/// colour, not in the crown's own: the canopy painted on the ground
/// (`canopy`) and its lumps ([`Builder::scan_lumps`]). In the palette's
/// green (until 2026-11-06) the far woods were another forest, bright
/// behind the dark near one (user: «vorrei che i modelli a grande distanza
/// fossero sempre quelli derivati da scansioni, non i vecchi modelli»).
pub fn scan_leaf_colour(name: &'static str) -> Option<[f32; 3]> {
    let plant = crate::models::scan_of(name)?;
    let scans = crate::models::scans()?;
    let leaves = scans.leaf_rgb(plant, "lod2").or_else(|| scans.leaf_rgb(plant, "lod3"))?;
    let tint = leaf_tint(leaves, super::canopy::crown_colour(name));
    Some([0, 1, 2].map(|k| 2.0 * tint[k] * leaves[k]))
}

/// `KIOSK_SCAN_FAR=grumi`: a scanned plant far away as the lumps it was
/// until 2026-11-06 ([`Builder::scan_lumps`]), not in boxes
/// ([`Builder::scan_voxels`]): to compare the two.
fn far_lumps() -> bool {
    static LUMPS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LUMPS.get_or_init(|| std::env::var("KIOSK_SCAN_FAR").is_ok_and(|v| v == "grumi"))
}

/// About how tall the scatter plants a variety (m, `scatter`): a shrub, a
/// broadleaf, a conifer. The models are a unit tall.
fn nominal_height_m(name: &str) -> f32 {
    match name.rsplit_once('_').map_or(name, |(species, _)| species) {
        "pine" | "aleppo" | "cypress" => 17.0,
        "holm" | "cork" | "carob" | "olive" => 10.0,
        _ => 3.5,
    }
}

/// The scans' cards (`models::Scans::atlas_png`) at a quarter of their
/// side or less, to read what a card shows where a mesh is made of it
/// ([`Builder::scan_voxels`]): linear colour and how much of the texel is
/// leaf.
pub struct Cards {
    width: usize,
    height: usize,
    /// Linear red, green, blue of the leaf in the texel, and its share.
    texels: Vec<[f32; 4]>,
}

impl Cards {
    fn at(&self, uv: [f32; 2]) -> [f32; 4] {
        let x = ((uv[0].clamp(0.0, 1.0) * self.width as f32) as usize).min(self.width - 1);
        let y = ((uv[1].clamp(0.0, 1.0) * self.height as f32) as usize).min(self.height - 1);
        self.texels[y * self.width + x]
    }
}

fn cards() -> Option<&'static Cards> {
    static CARDS: std::sync::OnceLock<Option<Cards>> = std::sync::OnceLock::new();
    CARDS
        .get_or_init(|| {
            let scans = crate::models::scans()?;
            let (data, w, h) = super::atlas::decode(&scans.atlas_png);
            let (w, h) = (w as usize, h as usize);
            let k = w.max(h).div_ceil(1024).max(1);
            let (width, height) = (w / k, h / k);
            let lin: Vec<f32> = (0..256).map(|i| Color::srgb_u8(i as u8, 0, 0).to_linear().red).collect();
            let mut texels = vec![[0.0f32; 4]; width * height];
            for (i, t) in texels.iter_mut().enumerate() {
                let (x0, y0) = (i % width * k, i / width * k);
                let mut sum = [0.0f32; 4];
                for y in y0..y0 + k {
                    for x in x0..x0 + k {
                        let p = &data[(y * w + x) * 4..(y * w + x) * 4 + 4];
                        let a = p[3] as f32 / 255.0;
                        for c in 0..3 {
                            sum[c] += lin[p[c] as usize] * a;
                        }
                        sum[3] += a;
                    }
                }
                let a = sum[3].max(1e-6);
                *t = [sum[0] / a, sum[1] / a, sum[2] / a, sum[3] / (k * k) as f32];
            }
            Some(Cards { width, height, texels })
        })
        .as_ref()
}

/// How much wider a scanned plant is drawn than it was scanned
/// (2026-11-04): its crown brought to the width of the variety's own
/// (the leaves' span across, over the height), so a stand covers the
/// ground as the island's fuels were drawn: Poly Haven's Aleppo pine is a
/// slim tree where ours spread an umbrella, its *Searsia* half as wide as
/// our lentisk. Measured on the scan's `lod2` (all levels alike), within
/// 0.6..1.8.
fn scan_stretch(name: &'static str) -> f32 {
    let span = |points: &mut dyn Iterator<Item = [f32; 3]>| {
        let (lo, hi) = points.fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| {
            ([lo[0].min(p[0]), lo[1].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[2])])
        });
        0.5 * ((hi[0] - lo[0]) + (hi[1] - lo[1]))
    };
    let m = crate::models::live(name);
    let ours = span(&mut m.positions.iter().zip(&m.wood).filter(|(_, w)| !**w).map(|(p, _)| *p));
    let Some((vertices, _, _)) = crate::models::scan_of(name).and_then(|p| crate::models::scans()?.level(p, "lod2")) else {
        return 1.0;
    };
    let scanned = span(&mut vertices.iter().filter(|v| v.leaf).map(|v| v.position));
    (ours / scanned.max(1e-3)).clamp(0.6, 1.8)
}

/// `name`'s lobed stand-in, the browser's plant (2026-11-02): the authored
/// model without the leaf cards, which need a cut-out (on WebGL2 without
/// MSAA a `discard` with jagged edges, and tile-based GPUs lose their
/// hidden-surface removal). The trunk and limbs as they are; each crown part
/// a closed lump, its core (a lumpy icosahedron) grown to the reach of the
/// part's cards. So a pine keeps its umbrella pads on a forked trunk and a
/// holm oak its three lobes, where the six-sided proxy is one crown on a
/// stick. Opaque and single-sided.
pub fn lobed_mesh(name: &'static str) -> PlantMesh {
    let mut b = Builder::default();
    b.lobes(name);
    let mut m = b.finish(false, false);
    m.canopy = true;
    m
}

/// Size (u, v) of a canopy tile in the atlas: every variety's is the same
/// (`scripts/build_vegetation_models.py`, `canopy_rect`), and the shader
/// repeats it over the lobes from its corner.
pub fn canopy_tile() -> [f32; 2] {
    let [u0, v0, u1, v1] = crate::models::variety(crate::models::variety_names(crate::models::SPECIES[0])[0]).canopy;
    super::atlas::layout().authored_uv([u1 - u0, v1 - v0])
}

/// The most lumps a scanned plant has far away ([`Builder::scan_lumps`]).
const MAX_LUMPS: usize = 6;

/// How fine a scanned plant in boxes is ([`Builder::scan_voxels`]): the
/// cells up its height, the smallest cell at the size it is planted (m),
/// and the most faces it may have.
#[derive(Clone, Copy)]
struct Grain {
    layers: f32,
    least_m: f32,
    faces: usize,
}

/// From where the boxes begin (755 m off a pixel is 0.6 m: a pine's cell
/// about three) to 1.5 km, and beyond.
const VOXELS_FINE: Grain = Grain { layers: 10.0, least_m: 0.7, faces: 180 };
const VOXELS_COARSE: Grain = Grain { layers: 4.0, least_m: 2.0, faces: 44 };

/// The share of a cell's face in wood for a cell of wood to count, and how
/// much of what is behind it a face's worth of leaves hides (as one minus
/// the exponential of so many times the leaves in the cell). Tuned on a
/// pine wood from 800 m against the plants' `lod2` drawn that far: at 0.9
/// the crowns were solid and the wood darker by a third; at 0.12 loose
/// cubes.
const VOXEL_FILL: f32 = 0.2;
const VOXEL_COVER: f32 = 0.2;

/// How much of its cards' reach a lump of [`lobed_mesh`] fills: the cards'
/// leaves do not reach their corners, and a lump as wide as their box read
/// as a fat cushion.
const LOBE_FILL: f32 = 0.82;

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

fn proxy_of(name: &str) -> &'static Proxy {
    static PROXIES: std::sync::OnceLock<std::collections::HashMap<&'static str, Proxy>> = std::sync::OnceLock::new();
    &PROXIES.get_or_init(|| {
        crate::models::SPECIES
            .into_iter()
            .flat_map(|species| crate::models::variety_names(species).iter().copied())
            .map(|name| {
                let m = crate::models::live(name);
                let leaves = || (0..m.positions.len()).filter(|&i| !m.wood[i]);
                let (lo, hi) = leaves().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), i| {
                    (lo.min(Vec3::from(m.positions[i])), hi.max(Vec3::from(m.positions[i])))
                });
                let mid = (lo.y + hi.y) * 0.5;
                // the tone the full model's leaf vertices carry
                let mean = |upper: bool| {
                    let (sum, n) = leaves()
                        .filter(|&i| (m.positions[i][1] > mid) == upper)
                        .fold((0.0, 0), |(s, n), i| (s + m.colors[i][1], n + 1));
                    if n == 0 { 1.0 } else { sum / n as f32 }
                };
                let bare: Vec<usize> = (0..m.positions.len()).filter(|&i| m.wood[i] && m.positions[i][1] < lo.y).collect();
                let trunk = (!bare.is_empty()).then(|| {
                    let r = bare.iter().map(|&i| Vec2::new(m.positions[i][0], m.positions[i][2]).length()).fold(0.0, f32::max);
                    r.max(0.01)
                });
                let v = crate::models::variety(name);
                (name, Proxy { crown: (lo, hi), shade_low: mean(false), shade_high: mean(true), trunk, canopy: v.canopy, bark: v.bark })
            })
            .collect()
    })[name]
}

#[derive(Default)]
struct Builder {
    positions: Vec<Vec3>,
    /// `None`: averaged from the faces around the vertex in `finish`.
    normals: Vec<Option<Vec3>>,
    kinds: Vec<Kind>,
    uvs: Vec<[f32; 2]>,
    /// Linear colour and occlusion.
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    /// A scanned card's right and up axes (zero elsewhere).
    cards: Vec<[Vec3; 2]>,
    /// How much a scanned plant is stretched across ([`scan_stretch`]).
    stretch: Vec<f32>,
    /// The uv points into the scans' cards (`atlas::Layout::scan_uv`), not
    /// into the authored atlas.
    scan_uv: Vec<bool>,
}

impl Builder {
    fn vertex(&mut self, p: Vec3, n: Option<Vec3>, kind: Kind, uv: [f32; 2], color: [f32; 4]) -> u32 {
        let i = self.positions.len() as u32;
        self.positions.push(p);
        self.normals.push(n);
        self.kinds.push(kind);
        self.uvs.push(uv);
        self.colors.push(color);
        self.cards.push([Vec3::ZERO; 2]);
        self.stretch.push(1.0);
        self.scan_uv.push(false);
        i
    }

    /// A scanned plant's level (`models::Scans::level`): the cards leaves
    /// painted by the scans' atlas, their mean colour (`leaf_rgb`) brought
    /// to the variety's crown (`target`, the palette the island's fuels
    /// are read by; the plant's tint over it); the wood the bark's own
    /// colour (over the atlas grey, which halves it: the colour travels
    /// halved and is doubled). The leaves occluded toward the bottom and
    /// the inside of the crown.
    /// Stretched across by `stretch` ([`scan_stretch`]): positions and
    /// the cards' axes by it, normals by its inverse.
    fn scan(&mut self, vertices: &[crate::models::ScanVertex], indices: &[u32], leaf_rgb: [f32; 3], target: [f32; 3], stretch: f32) {
        let s = Vec3::new(stretch, 1.0, stretch);
        let leaf = leaf_tint(leaf_rgb, target);
        let (lo, hi) = vertices.iter().filter(|v| v.leaf).fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| {
            (lo.min(Vec3::from(v.position) * s), hi.max(Vec3::from(v.position) * s))
        });
        let grey = crate::models::atlas().grey;
        let start = self.positions.len() as u32;
        for v in vertices {
            let p = Vec3::from(v.position) * s;
            let n = Some((Vec3::from(v.normal) / s).normalize_or_zero());
            let i = if v.leaf {
                let i = self.vertex(p, n, Kind::Leaf, v.uv, [leaf[0], leaf[1], leaf[2], crown_occlusion(p, lo, hi)]);
                // as scanned: the shader tests them against the view
                // unstretched (`plant_common::card_hidden`)
                self.cards[i as usize] = [Vec3::from(v.card_a), Vec3::from(v.card_b)];
                self.stretch[i as usize] = stretch;
                self.scan_uv[i as usize] = true;
                i
            } else {
                self.vertex(p, n, Kind::Wood, grey, [v.color[0], v.color[1], v.color[2], 0.8])
            };
            let _ = i;
        }
        self.indices.extend(indices.iter().map(|&i| start + i));
    }

    /// The authored model: its normals (a leaf card lights as part of its
    /// crown), uvs, and the crown's shading in the colour; the leaves
    /// occluded toward the bottom and the inside of the crown.
    fn model(&mut self, m: &crate::models::Model) {
        let leaves = || (0..m.positions.len()).filter(|&i| !m.wood[i]);
        let (lo, hi) = leaves().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), i| {
            (lo.min(Vec3::from(m.positions[i])), hi.max(Vec3::from(m.positions[i])))
        });
        let grey = crate::models::atlas().grey;
        for (i, p) in m.positions.iter().enumerate() {
            let p = Vec3::from(*p);
            let wood = m.wood[i];
            let shade = if wood { 1.0 } else { m.colors[i][1].powf(2.2) };
            let occlusion = if wood { 0.8 } else { crown_occlusion(p, lo, hi) };
            let n = m.normals.get(i).map(|n| Vec3::from(*n).normalize_or_zero());
            let uv = m.uvs.get(i).copied().unwrap_or(grey);
            let c = (shade * 0.5).min(1.0);
            self.vertex(p, n, if wood { Kind::Wood } else { Kind::Leaf }, uv, [c, c, c, occlusion]);
        }
        self.indices.extend(m.indices.iter().copied());
    }

    /// The distant stand-in for the model: a six-sided crown (a bottom
    /// point, two rings, a top point; `far`: one ring as wide as the crown
    /// at its widest) over the model's leaves, and a three-sided trunk where
    /// the model shows bare wood. The crown wears the species' canopy tile,
    /// mirrored on alternate faces (six is even: no seam), the trunk its
    /// bark. Counter-clockwise from outside: the back faces are culled.
    fn proxy(&mut self, name: &str, far: bool) {
        const SIDES: u32 = 6;
        let p = proxy_of(name);
        // halved (the shader doubles it), and a few percent darker: the
        // authored crown shades itself in its lobes; the far ring a little
        // lighter, its faces are flatter to the sun
        let shade = |k: f32| (0.5 * 0.9 * k.powf(2.2)).min(1.0);
        let [cu0, cv0, cu1, cv1] = p.canopy;
        let [bu0, bv0, bu1, bv1] = p.bark;
        let (lo, hi) = p.crown;
        let (centre, radius) = ((lo + hi) * 0.5, (hi - lo) * 0.5);

        if let Some(r) = p.trunk {
            let start = self.positions.len() as u32;
            let top = lo.y + radius.y * 0.4;
            let top_v = bv0 + (bv1 - bv0) * top.min(1.0);
            for i in 0..3 {
                let a = i as f32 * std::f32::consts::TAU / 3.0;
                let d = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                let u = if i == 1 { bu1 } else { bu0 };
                self.vertex(Vec3::new(centre.x, 0.0, centre.z) + d, None, Kind::Wood, [u, bv0], [0.5, 0.5, 0.5, 0.7]);
                self.vertex(Vec3::new(centre.x, top, centre.z) + d, None, Kind::Wood, [u, top_v], [0.5, 0.5, 0.5, 0.85]);
            }
            for i in 0..3 {
                let (b0, b1) = (start + 2 * i, start + 2 * ((i + 1) % 3));
                self.indices.extend_from_slice(&[b0, b0 + 1, b1, b1, b0 + 1, b1 + 1]);
            }
        }

        let y = |t: f32| lo.y + (hi.y - lo.y) * t;
        let cv = |t: f32| cv1 + (cv0 - cv1) * t;
        let mid_u = (cu0 + cu1) * 0.5;
        let side_u = |i: u32| if i.is_multiple_of(2) { cu0 } else { cu1 };
        let leaf = |k: f32, occlusion: f32| {
            let c = shade(k);
            [c, c, c, occlusion]
        };
        let bottom = self.vertex(Vec3::new(centre.x, y(0.04), centre.z), None, Kind::Leaf, [mid_u, cv(0.0)], leaf(p.shade_low * 0.8, 0.6));
        let ring = self.positions.len() as u32;
        let rings: &[(f32, f32, f32, f32)] = if far {
            &[(0.45, 0.97, p.shade_high * 1.08, 0.85)]
        } else {
            &[(0.3, 0.85, p.shade_low, 0.75), (0.62, 1.0, p.shade_high, 0.95)]
        };
        for (r, &(t, k, sh, occlusion)) in rings.iter().enumerate() {
            for i in 0..SIDES {
                // the upper ring turned half a side: the faces between read as leaves, not facets
                let a = (i as f32 + 0.5 * r as f32) * std::f32::consts::TAU / SIDES as f32;
                let v = Vec3::new(centre.x + a.cos() * radius.x * k, y(t), centre.z + a.sin() * radius.z * k);
                self.vertex(v, None, Kind::Leaf, [side_u(i), cv(t)], leaf(sh, occlusion));
            }
        }
        let top = self.vertex(Vec3::new(centre.x, hi.y, centre.z), None, Kind::Leaf, [mid_u, cv(1.0)], leaf(p.shade_high * 1.05, 1.0));
        for i in 0..SIDES {
            let (a, b) = (ring + i, ring + (i + 1) % SIDES);
            if far {
                self.indices.extend_from_slice(&[bottom, a, b, a, top, b]);
            } else {
                let (c, d) = (a + SIDES, b + SIDES);
                self.indices.extend_from_slice(&[bottom, a, b, a, c, b, c, d, b, c, top, d]);
            }
        }
    }

    /// The lobed stand-in ([`lobed_mesh`]). The leaves of the live model
    /// come apart in pieces, triangles joined by their vertices: a core is
    /// a piece painted with the canopy tile, the others are cards, each
    /// hung round the core it is nearest to. A lump is its core stretched,
    /// about the middle of its cards' box, to [`LOBE_FILL`] of that box, in
    /// the cards' tone (the core is the dark inside of the crown). Wood
    /// hidden inside a lump is left out.
    fn lobes(&mut self, name: &str) {
        let m = crate::models::live(name);
        let [u0, v0, u1, v1] = crate::models::variety(name).canopy;
        let n = m.positions.len();
        let at = |i: usize| Vec3::from(m.positions[i]);
        let shade_of = |i: usize| m.colors[i][1];
        let on_canopy = |i: usize| {
            m.uvs.get(i).is_some_and(|uv| (u0 - 1e-3..=u1 + 1e-3).contains(&uv[0]) && (v0 - 1e-3..=v1 + 1e-3).contains(&uv[1]))
        };
        let bounds = |vs: &[usize]| {
            vs.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), &i| (lo.min(at(i)), hi.max(at(i))))
        };
        // the pieces of the leaves (union-find over their triangles)
        fn root(of: &mut [usize], mut i: usize) -> usize {
            while of[i] != i {
                of[i] = of[of[i]];
                i = of[i];
            }
            i
        }
        let mut of: Vec<usize> = (0..n).collect();
        let tris: Vec<[usize; 3]> = m.indices.chunks_exact(3).map(|t| [t[0] as usize, t[1] as usize, t[2] as usize]).collect();
        for t in tris.iter().filter(|t| t.iter().all(|&i| !m.wood[i])) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2])] {
                let (ra, rb) = (root(&mut of, a), root(&mut of, b));
                of[ra] = rb;
            }
        }
        let mut pieces: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in (0..n).filter(|&i| !m.wood[i]) {
            let r = root(&mut of, i);
            pieces.entry(r).or_default().push(i);
        }
        struct Part {
            core: Vec<usize>,
            centre: Vec3,
            half: Vec3,
            /// The box of the core and its cards.
            lo: Vec3,
            hi: Vec3,
            /// The cards' tone: sum and count of their vertices' shade.
            card_shade: (f32, usize),
        }
        let (cores, cards): (Vec<Vec<usize>>, Vec<Vec<usize>>) = pieces.into_values().partition(|p| p.iter().all(|&i| on_canopy(i)));
        let mut parts: Vec<Part> = cores
            .into_iter()
            .map(|core| {
                let (lo, hi) = bounds(&core);
                Part { centre: (lo + hi) * 0.5, half: ((hi - lo) * 0.5).max(Vec3::splat(1e-3)), lo, hi, card_shade: (0.0, 0), core }
            })
            .collect();
        for card in &cards {
            let mid = card.iter().map(|&i| at(i)).sum::<Vec3>() / card.len() as f32;
            let reach = |p: &Part| ((mid - p.centre) / p.half).length();
            if let Some(p) = parts.iter_mut().min_by(|a, b| reach(a).total_cmp(&reach(b))) {
                let (lo, hi) = bounds(card);
                (p.lo, p.hi) = (p.lo.min(lo), p.hi.max(hi));
                p.card_shade.0 += card.iter().map(|&i| shade_of(i)).sum::<f32>();
                p.card_shade.1 += card.len();
            }
        }
        // the whole crown, for the occlusion toward its heart and its bottom
        let (crown_lo, crown_hi) = parts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p.lo), hi.max(p.hi)));
        // each lump: where it stands and how far it reaches (an ellipsoid, to hide the wood inside it)
        let mut lumps: Vec<(Vec3, Vec3)> = Vec::new();
        for p in &parts {
            let centre = (p.lo + p.hi) * 0.5;
            let reach = (p.hi - p.lo) * 0.5 * LOBE_FILL;
            let k = reach / p.half;
            let core_shade = p.core.iter().map(|&i| shade_of(i)).sum::<f32>() / p.core.len() as f32;
            let tone = if p.card_shade.1 == 0 { 1.0 } else { p.card_shade.0 / p.card_shade.1 as f32 / core_shade.max(1e-3) };
            let mut index = std::collections::HashMap::new();
            for &i in &p.core {
                let q = centre + (at(i) - p.centre) * k;
                let normal = m.normals.get(i).map(|nm| (Vec3::from(*nm) / k).normalize_or_zero());
                // halved, and a few percent darker, as the proxies: the
                // cards let the crown's shade show between them
                let c = (0.5 * 0.9 * (shade_of(i) * tone).powf(2.2)).min(1.0);
                let occlusion = crown_occlusion(q, crown_lo, crown_hi);
                index.insert(i, self.vertex(q, normal, Kind::Leaf, [u0, v0], [c, c, c, occlusion]));
            }
            // counter-clockwise seen from outside, as the closed crowns are culled
            let start = self.indices.len();
            for t in tris.iter().filter(|t| t.iter().all(|i| index.contains_key(i))) {
                self.indices.extend(t.iter().map(|i| index[i]));
            }
            let volume: f32 = self.indices[start..]
                .chunks_exact(3)
                .map(|t| {
                    let [a, b, c] = [0, 1, 2].map(|k| self.positions[t[k] as usize] - centre);
                    a.dot(b.cross(c))
                })
                .sum();
            if volume < 0.0 {
                for t in self.indices[start..].chunks_exact_mut(3) {
                    t.swap(1, 2);
                }
            }
            lumps.push((centre, reach));
        }
        // the trunk and limbs, without what the lumps hide
        let hidden = |i: usize| lumps.iter().any(|&(c, r)| ((at(i) - c) / r.max(Vec3::splat(1e-3))).length_squared() < 0.81);
        let mut index = std::collections::HashMap::new();
        for t in tris.iter().filter(|t| t.iter().all(|&i| m.wood[i]) && !t.iter().all(|&i| hidden(i))) {
            for &i in t {
                let v = *index.entry(i).or_insert_with(|| {
                    let normal = m.normals.get(i).map(|nm| Vec3::from(*nm).normalize_or_zero());
                    let uv = m.uvs.get(i).copied().unwrap_or(crate::models::atlas().grey);
                    self.vertex(at(i), normal, Kind::Wood, uv, [0.5, 0.5, 0.5, 0.8])
                });
                self.indices.push(v);
            }
        }
    }

    /// What fire adds to a plant: its twig skeleton and the debris at its
    /// foot, in absolute colours (sRGB in the bake).
    fn remains(&mut self, piece: &crate::models::Piece) {
        let grey = crate::models::atlas().grey;
        let start = self.positions.len() as u32;
        for ((p, c), &debris) in piece.positions.iter().zip(&piece.colors).zip(&piece.debris) {
            let l = Color::srgb(c[0], c[1], c[2]).to_linear();
            let kind = if debris { Kind::Debris } else { Kind::Twig };
            self.vertex(Vec3::from(*p), None, kind, grey, [l.red, l.green, l.blue, 1.0]);
        }
        self.indices.extend(piece.indices.iter().map(|i| start + i));
    }

    /// A scanned plant from far away, in boxes (2026-11-06, the user: «non
    /// mi piacciono i blob da lontano. non si potrebbero usare delle
    /// approssimazioni tipo modelli voxelizzati?»). The plant's `lod2` is
    /// laid in a grid of cells (`grain`), and a cell with enough leaf in
    /// it is a cube of the crown, drawn by the faces it shows: those with
    /// no cube beyond them, and not the one toward the ground, never seen
    /// from so far. A cell of wood alone (the trunk) is a post as thick as
    /// the wood in it. Each in the colour its cards show there (`cards`,
    /// under the leaves' tint `tint`, halved as it travels) or its bark's.
    /// Opaque and single-sided, as the lumps were: no cut-out, the far
    /// forest's cost. The plant keeps its outline, its trunk and the
    /// colours of its scan; the lumps were one colour and a cushion for
    /// each crown part.
    #[allow(clippy::too_many_arguments)]
    fn scan_voxels(&mut self, vertices: &[crate::models::ScanVertex], indices: &[u32], tint: [f32; 3], stretch: f32, height_m: f32, grain: Grain, cards: &Cards) {
        let s = Vec3::new(stretch, 1.0, stretch);
        let at = |i: u32| Vec3::from(vertices[i as usize].position) * s;
        let (lo, hi) = vertices.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| {
            let p = Vec3::from(v.position) * s;
            (lo.min(p), hi.max(p))
        });
        if vertices.is_empty() || hi.y - lo.y < 1e-3 {
            return;
        }
        #[derive(Clone, Copy)]
        struct Cell {
            leaf: f32,
            wood: f32,
            colour: Vec3,
            /// The wood's reach in the cell.
            lo: Vec3,
            hi: Vec3,
        }
        let empty = Cell { leaf: 0.0, wood: 0.0, colour: Vec3::ZERO, lo: Vec3::splat(f32::MAX), hi: Vec3::splat(f32::MIN) };
        const SIDES: [(Vec3, Vec3, Vec3); 6] = [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z, Vec3::X),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        ];
        // coarser until the faces are few enough
        let mut edge = ((hi.y - lo.y) / grain.layers).max(grain.least_m / height_m);
        loop {
            let n = ((hi - lo) / edge).ceil().max(Vec3::ONE);
            let (nx, ny, nz) = (n.x as usize, n.y as usize, n.z as usize);
            let index = |x: usize, y: usize, z: usize| (y * nz + z) * nx + x;
            let mut cells = vec![empty; nx * ny * nz];
            for t in indices.chunks_exact(3) {
                let (a, b, c) = (at(t[0]), at(t[1]), at(t[2]));
                let area = 0.5 * (b - a).cross(c - a).length();
                let v: [&crate::models::ScanVertex; 3] = [0, 1, 2].map(|k| &vertices[t[k] as usize]);
                let leaf = v.iter().all(|v| v.leaf);
                // points spread over the triangle, a dozen to a cell's face
                let count = ((area / (edge * edge) * 12.0).ceil() as usize).clamp(1, 400);
                for k in 0..count {
                    let (mut u, mut w) = ((k as f32 + 0.5) * 0.754_877_7 % 1.0, (k as f32 + 0.5) * 0.569_840_3 % 1.0);
                    if u + w > 1.0 {
                        (u, w) = (1.0 - u, 1.0 - w);
                    }
                    let p = a + (b - a) * u + (c - a) * w;
                    let g = ((p - lo) / edge).floor().max(Vec3::ZERO);
                    let cell = &mut cells[index((g.x as usize).min(nx - 1), (g.y as usize).min(ny - 1), (g.z as usize).min(nz - 1))];
                    let share = area / count as f32;
                    let mix = |f: fn(&crate::models::ScanVertex) -> Vec3| f(v[0]) * (1.0 - u - w) + f(v[1]) * u + f(v[2]) * w;
                    if leaf {
                        let uv = mix(|v| Vec3::new(v.uv[0], v.uv[1], 0.0));
                        let texel = cards.at([uv.x, uv.y]);
                        let shown = share * texel[3];
                        cell.leaf += shown;
                        cell.colour += Vec3::new(texel[0] * 2.0 * tint[0], texel[1] * 2.0 * tint[1], texel[2] * 2.0 * tint[2]) * shown;
                    } else {
                        cell.wood += share;
                        cell.colour += mix(|v| Vec3::from(v.color)) * share;
                        (cell.lo, cell.hi) = (cell.lo.min(p), cell.hi.max(p));
                    }
                }
            }
            // A cell of wood counts when it holds enough of it. A cell of
            // leaves is a cube as often as its leaves hide what is behind
            // them (so much leaf to a face of the cell, seen from any side),
            // by a number of its own: all of them cubes, the crown was
            // solid, where the scan lets the ground through.
            let chance = |i: usize| ((i as u32).wrapping_mul(2_654_435_761) >> 8) as f32 / 16_777_216.0;
            let hides = |c: &Cell| 1.0 - (-VOXEL_COVER * c.leaf / (edge * edge)).exp();
            let leaves: Vec<bool> = cells.iter().map(|c| c.leaf >= c.wood).collect();
            let mut kept: Vec<bool> = cells.iter().enumerate().map(|(i, c)| if leaves[i] { chance(i) < hides(c) } else { c.wood / (edge * edge) >= VOXEL_FILL }).collect();
            // a plant is never nothing: its fullest cell, if none came out
            if !kept.contains(&true) {
                let fullest = (0..cells.len()).max_by(|&a, &b| (cells[a].leaf + cells[a].wood).total_cmp(&(cells[b].leaf + cells[b].wood)));
                match fullest.filter(|&i| cells[i].leaf + cells[i].wood > 0.0) {
                    Some(i) => kept[i] = true,
                    None => return,
                }
            }
            let leafy = |i: usize| kept[i] && leaves[i];
            // the faces: (cell, side)
            let mut faces = Vec::new();
            for y in 0..ny {
                for z in 0..nz {
                    for x in 0..nx {
                        let i = index(x, y, z);
                        if !kept[i] {
                            continue;
                        }
                        for (side, (n, _, _)) in SIDES.iter().enumerate().take(5) {
                            let (bx, by, bz) = (x as isize + n.x as isize, y as isize + n.y as isize, z as isize + n.z as isize);
                            let inside = bx >= 0 && by >= 0 && bz >= 0 && (bx as usize) < nx && (by as usize) < ny && (bz as usize) < nz;
                            // a cube of the crown hides the face it touches; a post hides none and shows all of its own
                            if leafy(i) && inside && leafy(index(bx as usize, by as usize, bz as usize)) {
                                continue;
                            }
                            faces.push((i, side));
                        }
                    }
                }
            }
            if faces.len() > grain.faces {
                edge *= 1.15;
                continue;
            }
            // a cell's box, no farther than the plant reaches
            let corners = |i: usize| {
                let a = lo + Vec3::new((i % nx) as f32, (i / (nx * nz)) as f32, (i / nx % nz) as f32) * edge;
                (a, (a + edge).min(hi))
            };
            let (crown_lo, crown_hi) = (0..cells.len()).filter(|&i| leafy(i)).fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), i| (a.min(corners(i).0), b.max(corners(i).1)));
            let grey = crate::models::atlas().grey;
            for (i, side) in faces {
                let c = &cells[i];
                let colour = (c.colour / (c.leaf + c.wood)).clamp(Vec3::ZERO, Vec3::ONE);
                // the cell; or the wood in it, as tall as the cell and no
                // thinner than a sixth of it
                let (a, b) = corners(i);
                let (centre, half) = if leafy(i) {
                    ((a + b) * 0.5, (b - a) * 0.5)
                } else {
                    let (mid, reach) = ((c.lo + c.hi) * 0.5, ((c.hi - c.lo) * 0.5).max(Vec3::splat(edge / 12.0)));
                    (Vec3::new(mid.x, (a.y + b.y) * 0.5, mid.z), Vec3::new(reach.x, (b.y - a.y) * 0.5, reach.z))
                };
                let (n, u, w) = SIDES[side];
                let start = self.positions.len() as u32;
                // its sides lit a little from above too, as a crown's leaves are
                let normal = (n + Vec3::Y * 0.25).normalize();
                for (su, sw) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let p = centre + (n + u * su + w * sw) * half;
                    let occlusion = if leafy(i) { crown_occlusion(p, crown_lo, crown_hi) } else { 0.8 };
                    self.vertex(p, Some(normal), if leafy(i) { Kind::Leaf } else { Kind::Wood }, grey, [colour.x, colour.y, colour.z, occlusion]);
                }
                self.indices.extend([start, start + 1, start + 2, start, start + 2, start + 3]);
            }
            return;
        }
    }

    /// A scanned plant from far away (2026-11-04): each cluster of its
    /// `lod3` (three crossed cards about one centre) a closed lump, opaque
    /// and single-sided, as wide as its cards reach (`LOBE_FILL`), in the
    /// variety's crown colour over the atlas grey (which halves it, as the
    /// colour is doubled: the colour itself), darker toward the bottom and
    /// the heart of the crown. No cut-out, so no fragment is drawn twice for
    /// a leaf's edge: the far forest's cost.
    fn scan_lumps(&mut self, vertices: &[crate::models::ScanVertex], indices: &[u32], colour: [f32; 3], stretch: f32) {
        let s = Vec3::new(stretch, 1.0, stretch);
        // the cards: the leaf vertices joined by triangles
        let n = vertices.len();
        let mut of: Vec<usize> = (0..n).collect();
        fn root(of: &mut [usize], mut i: usize) -> usize {
            while of[i] != i {
                of[i] = of[of[i]];
                i = of[i];
            }
            i
        }
        for t in indices.chunks_exact(3) {
            let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
            if vertices[a].leaf && vertices[b].leaf && vertices[c].leaf {
                for (x, y) in [(a, b), (b, c)] {
                    let (rx, ry) = (root(&mut of, x), root(&mut of, y));
                    of[rx] = ry;
                }
            }
        }
        let mut cards: BTreeMap<usize, Vec<Vec3>> = BTreeMap::new();
        for i in (0..n).filter(|&i| vertices[i].leaf) {
            let r = root(&mut of, i);
            cards.entry(r).or_default().push(Vec3::from(vertices[i].position) * s);
        }
        // the clusters: cards about one centre
        let mut clusters: Vec<(Vec3, Vec3, Vec3)> = Vec::new();
        for pts in cards.values() {
            let mid = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
            let (lo, hi) = pts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), &p| (lo.min(p), hi.max(p)));
            match clusters.iter_mut().find(|c| c.0.distance(mid) < 1e-3) {
                Some(c) => (c.1, c.2) = (c.1.min(lo), c.2.max(hi)),
                None => clusters.push((mid, lo, hi)),
            }
        }
        // at most `MAX_LUMPS`: the two nearest merged, their boxes joined
        while clusters.len() > MAX_LUMPS {
            let mut best = (f32::MAX, 0, 1);
            for i in 0..clusters.len() {
                for j in i + 1..clusters.len() {
                    let d = ((clusters[i].1 + clusters[i].2) - (clusters[j].1 + clusters[j].2)).length_squared();
                    if d < best.0 {
                        best = (d, i, j);
                    }
                }
            }
            let (_, i, j) = best;
            let b = clusters.swap_remove(j);
            clusters[i].1 = clusters[i].1.min(b.1);
            clusters[i].2 = clusters[i].2.max(b.2);
        }
        let (crown_lo, crown_hi) = clusters.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), c| (lo.min(c.1), hi.max(c.2)));
        let grey = crate::models::atlas().grey;
        let colour = colour.map(|c| c.clamp(0.0, 1.0));
        // an octahedron, its corners pushed in and out a little per lump,
        // turned per lump so that the lumps do not line up
        let corners: [Vec3; 6] = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
        const FACES: [[u32; 3]; 8] = [[2, 4, 0], [2, 0, 5], [2, 5, 1], [2, 1, 4], [3, 0, 4], [3, 5, 0], [3, 1, 5], [3, 4, 1]];
        for (k, &(_, lo, hi)) in clusters.iter().enumerate() {
            let centre = (lo + hi) * 0.5;
            let reach = ((hi - lo) * 0.5 * LOBE_FILL).max(Vec3::splat(1e-3));
            let start = self.positions.len() as u32;
            let turn = Quat::from_rotation_y(k as f32 * 0.7);
            for (j, c) in corners.iter().enumerate() {
                let h = ((k * 6 + j) as u32).wrapping_mul(2654435761) >> 24;
                let bump = 0.9 + 0.2 * h as f32 / 255.0;
                let c = &(turn * *c);
                let p = centre + *c * reach * bump;
                let normal = ((*c / reach).normalize_or_zero() + Vec3::Y * 0.3).normalize();
                let occlusion = crown_occlusion(p, crown_lo, crown_hi);
                self.vertex(p, Some(normal), Kind::Leaf, grey, [colour[0], colour[1], colour[2], occlusion]);
            }
            for f in FACES {
                self.indices.extend([start + f[0], start + f[1], start + f[2]]);
            }
        }
    }

    /// Of the remains only the debris at the plant's foot (whole
    /// triangles: all three vertices debris).
    fn debris(&mut self, piece: &crate::models::Piece) {
        let grey = crate::models::atlas().grey;
        let mut index = std::collections::HashMap::new();
        for t in piece.indices.chunks_exact(3).filter(|t| t.iter().all(|&i| piece.debris[i as usize])) {
            for &i in t {
                let v = *index.entry(i).or_insert_with(|| {
                    let c = piece.colors[i as usize];
                    let l = Color::srgb(c[0], c[1], c[2]).to_linear();
                    self.vertex(Vec3::from(piece.positions[i as usize]), None, Kind::Debris, grey, [l.red, l.green, l.blue, 1.0])
                });
                self.indices.push(v);
            }
        }
    }

    /// Area-weighted normals where none was given, then the packed mesh.
    fn finish(self, cutout: bool, double_sided: bool) -> PlantMesh {
        let mut faces = vec![Vec3::ZERO; self.positions.len()];
        for t in self.indices.chunks_exact(3) {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let n = (self.positions[b] - self.positions[a]).cross(self.positions[c] - self.positions[a]);
            for i in [a, b, c] {
                faces[i] += n;
            }
        }
        assert!(self.positions.len() < u16::MAX as usize, "a plant mesh is small");
        let mut vertices = Vec::with_capacity(self.positions.len() * VERTEX_BYTES as usize);
        let snorm = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8;
        let unorm16 = |v: f32| ((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes();
        let layout = super::atlas::layout();
        for i in 0..self.positions.len() {
            let p = self.positions[i];
            let n = self.normals[i].unwrap_or_else(|| faces[i].normalize_or_zero());
            let n = if n == Vec3::ZERO { Vec3::Y } else { n };
            for v in [p.x, p.y, p.z] {
                vertices.extend_from_slice(&v.to_le_bytes());
            }
            vertices.extend_from_slice(&[snorm(n.x), snorm(n.y), snorm(n.z), self.kinds[i].snorm() as u8]);
            let uv = if self.scan_uv[i] { layout.scan_uv(self.uvs[i]) } else { layout.authored_uv(self.uvs[i]) };
            for v in uv {
                vertices.extend_from_slice(&unorm16(v));
            }
            for v in self.colors[i] {
                vertices.extend_from_slice(&unorm16(v));
            }
            let [a, b] = self.cards[i];
            vertices.extend_from_slice(&[snorm(a.x), snorm(a.y), snorm(a.z), snorm(self.stretch[i] * 0.5)]);
            vertices.extend_from_slice(&[snorm(b.x), snorm(b.y), snorm(b.z), 0]);
        }
        PlantMesh { vertices, indices: self.indices.iter().map(|&i| i as u16).collect(), cutout, double_sided, canopy: false }
    }
}

/// Ambient occlusion of a leaf at `p` in a crown spanning `lo..hi`: darker
/// toward its bottom and its heart, where the sky is hidden by the leaves
/// around.
fn crown_occlusion(p: Vec3, lo: Vec3, hi: Vec3) -> f32 {
    let t = ((p.y - lo.y) / (hi.y - lo.y).max(1e-3)).clamp(0.0, 1.0);
    let centre = (lo + hi) * 0.5;
    let half = ((hi - lo) * 0.5).max(Vec3::splat(1e-3));
    let r = (Vec2::new((p.x - centre.x) / half.x, (p.z - centre.z) / half.z)).length().min(1.0);
    0.55 + 0.45 * (0.35 + 0.65 * t) * (0.7 + 0.3 * r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variety() -> impl Iterator<Item = &'static str> {
        crate::models::SPECIES.into_iter().flat_map(|s| crate::models::variety_names(s).iter().copied())
    }

    /// Every tier of every variety builds, indexes inside its vertices,
    /// stays within about a unit of its foot, and marks what it is.
    #[test]
    fn every_tier_builds() {
        for name in every_variety() {
            for tier in [Tier::Full, Tier::Proxy, Tier::Far, Tier::RemainsNear, Tier::RemainsFar, Tier::Near, Tier::Mid] {
                let m = variety_mesh(name, tier);
                let scanned_remains = crate::models::scan_of(name).is_some() && matches!(tier, Tier::RemainsNear | Tier::RemainsFar);
                if scanned_remains && m.triangles() == 0 {
                    continue;
                }
                assert!(m.triangles() > 0, "{name} {tier:?}");
                assert_eq!(m.vertices.len() % VERTEX_BYTES as usize, 0);
                assert!(m.indices.iter().all(|&i| (i as usize) < m.vertex_count()), "{name} {tier:?}");
                for i in 0..m.vertex_count() {
                    let p = m.position(i);
                    assert!(p.y >= -0.5 && p.y <= 1.3 && p.x.abs() <= 1.3 && p.z.abs() <= 1.3, "{name} {tier:?}: {p}");
                }
                let kinds: Vec<i8> = (0..m.vertex_count()).map(|i| m.vertices[i * VERTEX_BYTES as usize + 15] as i8).collect();
                match tier {
                    Tier::RemainsNear | Tier::RemainsFar => {
                        assert!(kinds.iter().all(|&k| k < 0), "{name} {tier:?}: remains are twigs and debris");
                        // a scanned plant keeps its own charred wood: no skeleton
                        if crate::models::scan_of(name).is_none() {
                            assert!(kinds.iter().any(|&k| k == Kind::Twig.snorm()), "{name} {tier:?}: a skeleton");
                        }
                    }
                    _ => assert!(kinds.iter().any(|&k| k == Kind::Leaf.snorm()), "{name} {tier:?}: leaves"),
                }
            }
        }
    }

    /// The lobed stand-ins: opaque, single-sided and painted by the shader;
    /// a lump for each crown part of the model and no cards, so fewer
    /// triangles than the model; their leaves face out, as their normals do
    /// (the back faces are culled), and stay within the model's reach.
    #[test]
    fn lobed_stand_ins_are_closed_lumps_facing_out() {
        assert!(canopy_tile()[0] > 0.0 && canopy_tile()[1] > 0.0);
        for name in every_variety() {
            let [u0, v0, u1, v1] = crate::models::variety(name).canopy;
            // in the plants' atlas (with the scans, the authored one is a part of it)
            let [su, sv] = super::super::atlas::layout().authored_uv([u1 - u0, v1 - v0]);
            assert!((su - canopy_tile()[0]).abs() < 1e-4 && (sv - canopy_tile()[1]).abs() < 1e-4, "{name}: a canopy tile of its own size");
            let m = lobed_mesh(name);
            assert!(!m.cutout && !m.double_sided && m.canopy);
            // against the authored model (a scanned variety's `lod2` may be
            // lighter than the browser's lumps, where the scans are off)
            let full = variety_mesh(name, Tier::Full);
            if crate::models::scan_of(name).is_none() {
                assert!(m.triangles() < full.triangles(), "{name}: {} triangles for the model's {}", m.triangles(), full.triangles());
            }
            assert!(m.triangles() >= 20, "{name}: at least a lump");
            assert!(m.indices.iter().all(|&i| (i as usize) < m.vertex_count()));
            let kind = |i: usize| m.vertices[i * VERTEX_BYTES as usize + 15] as i8;
            let mut leaves = 0;
            for t in m.indices.chunks_exact(3) {
                let [a, b, c] = [0, 1, 2].map(|k| t[k] as usize);
                if kind(a) != Kind::Leaf.snorm() {
                    continue;
                }
                leaves += 1;
                let face = (m.position(b) - m.position(a)).cross(m.position(c) - m.position(a));
                let normals = m.normal(a) + m.normal(b) + m.normal(c);
                assert!(face.dot(normals) > 0.0, "{name}: a lump's face winds against its normals");
            }
            assert!(leaves >= 20 && leaves % 20 == 0, "{name}: {leaves} leaf triangles, lumps of 20");
            for i in 0..m.vertex_count() {
                let p = m.position(i);
                assert!(p.y >= -0.5 && p.y <= 1.3 && p.x.abs() <= 1.3 && p.z.abs() <= 1.3, "{name}: {p}");
            }
        }
    }

    /// The proxies stay under half of the models they stand in for (or 30
    /// triangles) and face outward: their back faces are culled, and an
    /// inward crown would vanish and be lit from the wrong side.
    #[test]
    fn proxies_are_cheap_and_face_out() {
        for name in every_variety() {
            // a scanned plant's: boxes, fine and coarse (with `KIOSK_SCANS=1`)
            if crate::models::scan_of(name).is_some() {
                let model = variety_mesh(name, Tier::Full).triangles();
                for (tier, grain) in [(Tier::Proxy, VOXELS_FINE), (Tier::Far, VOXELS_COARSE)] {
                    let m = variety_mesh(name, tier);
                    assert!(!m.double_sided && !m.cutout, "{name} {tier:?}: opaque and single-sided");
                    assert!(m.triangles() >= 2 && m.triangles() <= grain.faces * 2, "{name} {tier:?}: {} triangles", m.triangles());
                    assert!(tier == Tier::Proxy || m.triangles() <= model.max(60), "{name}: {} far triangles for a model of {model}", m.triangles());
                    // every face winds the way its normal points, outward
                    for t in m.indices.chunks_exact(3) {
                        let [a, b, c] = [0, 1, 2].map(|k| m.position(t[k] as usize));
                        let face = (b - a).cross(c - a);
                        assert!(face.length() > 0.0 && face.dot(m.normal(t[0] as usize)) > 0.0, "{name} {tier:?}: a face winds against its normal");
                    }
                }
                continue;
            }
            let full = variety_mesh(name, Tier::Full).triangles();
            for tier in [Tier::Proxy, Tier::Far] {
                let m = variety_mesh(name, tier);
                assert!(m.triangles() * 2 <= full.max(60), "{name}: {} proxy triangles for {full}", m.triangles());
                assert!(!m.double_sided);
                let (lo, hi) = proxy_of(name).crown;
                let centre = (lo + hi) * 0.5;
                for t in m.indices.chunks_exact(3) {
                    let [a, b, c] = [0, 1, 2].map(|k| m.position(t[k] as usize));
                    let face = (b - a).cross(c - a);
                    if face.length() < 1e-6 {
                        continue;
                    }
                    let mid = (a + b + c) / 3.0;
                    // the trunk's faces point away from its axis, the crown's from its centre
                    let axis = if mid.y < lo.y { Vec3::new(centre.x, mid.y, centre.z) } else { centre };
                    assert!(face.dot(mid - axis) > 0.0, "{name} {tier:?}: a face winds inward at {mid}");
                }
                for i in 0..m.vertex_count() {
                    let (p, n) = (m.position(i), m.normal(i));
                    if p.y >= lo.y {
                        assert!(n.dot(p - centre) > 0.0, "{name} {tier:?}: normal {n} at {p} points in");
                    }
                }
            }
        }
    }
}
