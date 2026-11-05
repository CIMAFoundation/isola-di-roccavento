//! Where the plants grow: one archetype per `eu_fuel12` group, scattered
//! deterministically from a hash of the cell they stand in. Deterministic
//! matters more than it sounds: the same seed must produce the same forest on
//! every run, or a debrief replay would show a different landscape than the
//! incident did.
//!
//! The fuel raster is the only vegetation source this scenario has — there is
//! no tree inventory, and there never will be one at 10 km scale. The terrain
//! under the plants is bare ground (see [`crate::terrain_mesh`]), so the land
//! cover a player sees is geometry: macchia as much as trees (the meadow
//! tufts are the terrain shader's). The visual density comes from crown width
//! and overlap rather than from stem count, because a hundred wide crowns
//! close a canopy that a thousand narrow ones would not.

use bevy::prelude::*;
use scenario::{Cell, CoverClass, Pos, Scenario};

use crate::field::noise;

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
    pub(super) fn of_fuel(fuel: i32) -> Option<Species> {
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

/// The plants of the fire cells `(row0, row1, col0, col1)`, end exclusive.
pub(super) fn scatter_tile(scn: &Scenario, density: f32, (r0, r1, c0, c1): (usize, usize, usize, usize)) -> Stand {
    let mut builder = Stand::default();
    for r in r0..r1 {
        for c in c0..c1 {
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
                if scatter_plant(scn, cell, species, &mut rng, &mut builder).is_some() {
                    // (the old per-plant lag: drawn still, so the forest
                    // stays where it was)
                    let _ = rng.unit();
                }
            }
        }
    }
    builder
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
    out: &mut Stand,
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
        // Meadow tufts are drawn by the terrain shader (`shaders/terrain.wgsl`).
        Species::Grass => return None,
    }
    Some(p)
}

/// Which species a plant of fuel class `fuel` at `p` is (`models::SPECIES`).
/// Species come in patches of a few tens of metres (a holm oak grove, a
/// cistus clearing) with single plants mixed in, and every class keeps to
/// its group's outline: what is drawn is what burns.
fn species_for(fuel: i32, p: Pos) -> &'static str {
    let table: &[(&'static str, f32)] = match fuel {
        // no cypresses in the woods (2026-11-05, the user: «possiamo
        // togliere i cipressi»; they had no scanned model)
        10 => &[("pine", 0.60), ("aleppo", 0.40)],
        11 => &[("aleppo", 0.65), ("pine", 0.35)],
        12 => &[("pine", 0.55), ("aleppo", 0.45)],
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
fn conifer(out: &mut Stand, name: &'static str, base: Vec3, scale: f32, yaw: f32,
    leaf: [f32; 3], wood: [f32; 3], rng: &mut Rng) {
    // tall, the crown a little wider than the model
    let height = (13.0 + rng.unit() * 8.0) * scale;
    let width = height * (1.0 + rng.unit() * 0.3);
    out.model(name, base, Vec3::new(width, height, width), yaw, leaf, wood);
}

#[allow(clippy::too_many_arguments)]
fn broadleaf(out: &mut Stand, name: &'static str, base: Vec3, scale: f32, yaw: f32,
    leaf: [f32; 3], wood: [f32; 3], rng: &mut Rng) {
    // lower than the pines, the crown about as wide as tall
    let height = (8.0 + rng.unit() * 5.0) * scale;
    out.model(name, base, Vec3::new(height * 1.1, height, height * 1.1), yaw, leaf, wood);
}

#[allow(clippy::too_many_arguments)]
fn shrub(out: &mut Stand, name: &'static str, base: Vec3, scale: f32, yaw: f32,
    leaf: [f32; 3], wood: [f32; 3], rng: &mut Rng) {
    // low and wide: macchia must not read as young trees
    let r = (2.6 + rng.unit() * 2.2) * scale;
    out.model(name, base, Vec3::new(r, r * 0.8, r), yaw, leaf, wood);
}


/// The plants of a tile as the scatter places them.
#[derive(Default)]
pub(super) struct Stand {
    pub inst: Vec<Inst>,
}

/// A plant: its variety, where and how it stands, its tint.
#[derive(Clone)]
pub(super) struct Inst {
    pub name: &'static str,
    /// Where its foot is (Bevy frame).
    pub base: Vec3,
    pub scale: Vec3,
    pub yaw: f32,
    /// Linear multipliers of the atlas colours (1 = as painted).
    pub leaf: [f32; 3],
    pub wood: f32,
}

impl Stand {
    /// `species` is one of `models::SPECIES`; which of its varieties this
    /// plant is follows from its yaw, so it is the same on every run.
    fn model(&mut self, species: &'static str, base: Vec3, scale: Vec3, yaw: f32,
        foliage: [f32; 3], wood: [f32; 3]) {
        let name = crate::models::pick_variety(species, yaw / std::f32::consts::TAU);
        self.inst.push(Inst { name, base, scale, yaw, leaf: foliage, wood: wood[1] });
    }
}

/// A plant's tint as the instance carries it: leaf red, green, blue and the
/// wood, each multiplier / 2 as a byte (the shader doubles them).
pub(super) fn tint_bytes(i: &Inst) -> [u8; 4] {
    let b = |t: f32| ((t * 0.5).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [b(i.leaf[0]), b(i.leaf[1]), b(i.leaf[2]), b(i.wood)]
}

/// Small deterministic PRNG (SplitMix64), seeded per cell so the same forest
/// grows on every run.
pub(super) struct Rng(u64);

impl Rng {
    pub fn seeded(seed: u64) -> Rng {
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
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }
}
