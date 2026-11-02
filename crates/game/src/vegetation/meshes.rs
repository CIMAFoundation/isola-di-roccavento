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
//!
//! The colour of a leaf or wood vertex is the model's own shading, halved;
//! the plant's tint (also halved) multiplies it and the atlas paints it. What
//! the fire adds (twigs, debris) has absolute colours and points at the
//! atlas's flat grey.

use bevy::prelude::*;

/// Bytes per vertex (see the table above).
pub const VERTEX_BYTES: u64 = 28;

/// The levels of detail of a variety.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    /// The authored model with its leaf cards (desktop, near the camera).
    Full = 0,
    /// A six-sided crown and a trunk wearing the species' canopy tile.
    Proxy = 1,
    /// The crown as a bipyramid, for the far distance.
    Far = 2,
    /// What fire adds (twig skeleton, ash and fallen wood), near and far.
    RemainsNear = 3,
    RemainsFar = 4,
}

pub const TIERS: usize = 5;

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
    match tier {
        Tier::Full => b.model(crate::models::live(name)),
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
}

impl Builder {
    fn vertex(&mut self, p: Vec3, n: Option<Vec3>, kind: Kind, uv: [f32; 2], color: [f32; 4]) -> u32 {
        let i = self.positions.len() as u32;
        self.positions.push(p);
        self.normals.push(n);
        self.kinds.push(kind);
        self.uvs.push(uv);
        self.colors.push(color);
        i
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
        for i in 0..self.positions.len() {
            let p = self.positions[i];
            let n = self.normals[i].unwrap_or_else(|| faces[i].normalize_or_zero());
            let n = if n == Vec3::ZERO { Vec3::Y } else { n };
            for v in [p.x, p.y, p.z] {
                vertices.extend_from_slice(&v.to_le_bytes());
            }
            vertices.extend_from_slice(&[snorm(n.x), snorm(n.y), snorm(n.z), self.kinds[i].snorm() as u8]);
            for v in self.uvs[i] {
                vertices.extend_from_slice(&unorm16(v));
            }
            for v in self.colors[i] {
                vertices.extend_from_slice(&unorm16(v));
            }
        }
        PlantMesh { vertices, indices: self.indices.iter().map(|&i| i as u16).collect(), cutout, double_sided }
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
            for tier in [Tier::Full, Tier::Proxy, Tier::Far, Tier::RemainsNear, Tier::RemainsFar] {
                let m = variety_mesh(name, tier);
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
                        assert!(kinds.iter().any(|&k| k == Kind::Twig.snorm()), "{name} {tier:?}: a skeleton");
                    }
                    _ => assert!(kinds.iter().any(|&k| k == Kind::Leaf.snorm()), "{name} {tier:?}: leaves"),
                }
            }
        }
    }

    /// The proxies stay under half of the models they stand in for (or 30
    /// triangles) and face outward: their back faces are culled, and an
    /// inward crown would vanish and be lit from the wrong side.
    #[test]
    fn proxies_are_cheap_and_face_out() {
        for name in every_variety() {
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
