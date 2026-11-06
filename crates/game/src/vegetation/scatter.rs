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

/// How wide a town cypress is for its height, against the scanned fir it
/// is drawn with (its crown about two thirds as wide as tall): a column a
/// quarter as wide as tall, like the toy.
const CYPRESS_SLIM: f32 = 0.4;
/// Scanned firs drawn in one cypress, and how much darker its leaves.
const CYPRESS_LAYERS: u32 = 3;
const CYPRESS_DARK: f32 = 0.7;

/// The trees of the towns, with the scanned plants: where the kit's toy
/// trees stand (`buildings::kit_trees`: piazzas, cemeteries, gardens), a
/// scanned tree of about the toy's size. Without the scans, none: the toys
/// are drawn. The piazzas get holm oaks, the gardens olives with a few holm
/// and cork oaks, the cypresses the scanned fir drawn slim (a dark column,
/// as a cypress reads from the game's view; without that scan, a slim holm
/// oak; its column `meshes::Builder::cypress_core`). They are plants like
/// the others: same levels, shadows and fire.
/// `KIOSK_TOWN_TREES=0` leaves the towns bare, for a before/after;
/// `KIOSK_TOWN_VARIETY=1` (a trial, off: [`town_variety`]) more species,
/// each plant its own size and lean, and rows where a town is bare.
pub(super) fn town_trees(scn: &Scenario) -> Vec<Inst> {
    let off = cfg!(not(target_arch = "wasm32")) && std::env::var("KIOSK_TOWN_TREES").as_deref() == Ok("0");
    if crate::models::scans().is_none() || off {
        return Vec::new();
    }
    let variety = town_variety();
    // the cypress: the scanned fir drawn slim, or (without it) a holm oak
    let (cypress, slim) = if crate::models::scan_of("cypress_1").is_some() { ("cypress", CYPRESS_SLIM) } else { ("holm", 0.3) };
    let kit: Vec<TownSpot> = crate::buildings::kit_trees(scn).into_iter().map(|t| TownSpot { cypress: t.cypress, at: t.at, scale: t.scale, yaw: t.yaw }).collect();
    let mut spots: Vec<(TownSpot, Option<&'static str>)> = kit.iter().map(|t| (*t, None)).collect();
    if variety {
        spots.extend(town_rows(scn, &kit));
    }
    let mut out = Stand::default();
    for (k, (t, row)) in spots.into_iter().enumerate() {
        let mut rng = Rng::seeded((0x7EE5_0000 + k as u64) ^ ((t.at.x.to_bits() as u64) << 20) ^ t.at.y.to_bits() as u64);
        let ground = scn.terrain.height_at(t.at);
        let base = Vec3::new(t.at.x, ground, -t.at.y);
        let shade = 0.9 + rng.unit() * 0.2;
        let leaf = [shade * (0.95 + rng.unit() * 0.1), shade, shade * (0.95 + rng.unit() * 0.1)];
        let wood = [lin1(0.88 + rng.unit() * 0.2); 3];
        // the scanned trees turn as they like: their variety follows the yaw
        let yaw = (t.yaw + rng.unit() * std::f32::consts::TAU).rem_euclid(std::f32::consts::TAU);
        let toy = crate::visual_scale::TREE * t.scale;
        // with the variety: each plant its own height, width and lean (drawn
        // after the rest, so without it every plant is as it was)
        let (mut tall, mut wide, mut lean) = (1.0, 1.0, [0.0, 0.0]);
        if variety {
            tall = 0.8 + rng.unit() * 0.45;
            wide = 0.85 + rng.unit() * 0.5;
            lean = [rng.unit() * std::f32::consts::TAU, rng.unit()];
        }
        let first = out.inst.len();
        if t.cypress || row == Some("cypress") {
            // the toy cypress is 10.5 m tall and a quarter as wide
            let height = 10.5 * toy * (0.9 + rng.unit() * 0.2) * tall;
            // a cypress keeps its column: less wider or narrower, hardly leaning
            let (slim, lean_max) = (slim * (0.85 + 0.3 * (wide - 0.85) / 0.5), 0.15);
            // a sapling's whorls are open: three of them in one column, each
            // turned, close it into the dense dark spindle of a cypress
            let layers = if cypress == "cypress" { CYPRESS_LAYERS } else { 1 };
            for l in 0..layers {
                let turn = (yaw + l as f32 * std::f32::consts::TAU / layers as f32 + 0.4).rem_euclid(std::f32::consts::TAU);
                let h = height * (1.0 - 0.06 * l as f32);
                out.model(cypress, base, Vec3::new(h * slim, h, h * slim), turn, mul(leaf, CYPRESS_DARK), wood);
            }
            lean[1] *= lean_max;
        } else {
            // the toy street tree is 6.2 m tall, about as wide as tall
            let height = 6.2 * toy * (0.9 + rng.unit() * 0.2) * tall;
            let u = rng.unit();
            let species = match row {
                Some(s) => s,
                None if !variety => match (u * 10.0) as u32 {
                    _ if t.scale >= 1.6 => "holm",
                    0..=5 => "olive",
                    6..=8 => "holm",
                    _ => "cork",
                },
                // the piazzas: holm oaks, a stone pine or a carob among them
                None if t.scale >= 1.6 => match (u * 20.0) as u32 {
                    0..=12 => "holm",
                    13..=16 => "pine",
                    _ => "carob",
                },
                // the gardens: olives first, then the island's other trees
                None => match (u * 20.0) as u32 {
                    0..=6 => "olive",
                    7..=9 => "holm",
                    10..=11 => "cork",
                    12..=14 => "carob",
                    15..=16 => "arbutus",
                    17 => "aleppo",
                    _ => "pine",
                },
            };
            // a strawberry tree is small, a pine tall and spread
            let (h, w) = match species {
                "arbutus" => (0.6, 1.0),
                "pine" | "aleppo" => (1.5, 0.9),
                _ => (1.0, 1.1),
            };
            let height = height * h;
            out.model(species, base, Vec3::new(height * w * wide, height, height * w * wide), yaw, leaf, wood);
            // an olive or a strawberry tree leans; an oak or a pine a little less
            lean[1] *= if matches!(species, "olive" | "arbutus") { 1.0 } else { 0.6 };
        }
        if variety {
            for i in &mut out.inst[first..] {
                i.yaw = lean_yaw(i.yaw, lean[0], lean[1]);
            }
        }
    }
    out.inst
}

/// A town tree's place: the kit's (`town_kit::KitTree`) or a row's.
#[derive(Clone, Copy)]
struct TownSpot {
    cypress: bool,
    at: Pos,
    scale: f32,
    yaw: f32,
}

/// `KIOSK_TOWN_VARIETY=1` (2026-11-06, the user: «fai più varietà»; a
/// trial, off until confirmed): the towns' trees of more species, each its
/// own size and lean, and rows of trees along the streets of the towns
/// with few ([`town_rows`]). The desktop only.
fn town_variety() -> bool {
    cfg!(not(target_arch = "wasm32")) && std::env::var("KIOSK_TOWN_VARIETY").as_deref() == Ok("1")
}

/// A plant's lean in its yaw (2026-11-06, `KIOSK_TOWN_VARIETY`): whole turns
/// past [`LEAN_TURNS`] carry one of 16 bearings and of 8 steps up to
/// [`LEAN_MAX`] (`share` of it, 0..1) of shear with the height; the sine
/// and cosine of the yaw stay those of `yaw`, so all else reads it as it
/// was. Below `LEAN_TURNS` (every other plant) no lean (`plant_common::place`).
pub(super) fn lean_yaw(yaw: f32, bearing: f32, share: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let dir = ((bearing.rem_euclid(TAU) / TAU * 16.0).round() as u32) % 16;
    let step = (share.clamp(0.0, 1.0) * 7.0).round() as u32;
    if step == 0 {
        return yaw;
    }
    let code = (step * 16 + dir) as f32;
    // a half turn on, the yaw within the half turn either side: the whole
    // turns stay whole whichever way the division rounds
    let around = (yaw.rem_euclid(TAU) - PI).clamp(-PI + 1e-3, PI - 1e-3);
    TAU * (LEAN_TURNS + code + 0.5) + around
}

/// As `plant_common.wesl`.
pub(super) const LEAN_TURNS: f32 = 64.0;
pub(super) const LEAN_MAX: f32 = 0.12;

/// Rows of trees along the streets of the towns that have few
/// (`KIOSK_TOWN_VARIETY`): of a town's houses, those with fewer trees than
/// half their number in the kit get, along the `ROW_ROADS` drivable roads
/// with room for the longest, a tree every `ROW_STEP_M`, `ROW_OFFSET_M` off
/// the road's axis, where it stands in the town (within `ROW_TOWN_M` of a
/// house) and clear of houses, other roads, other trees and the sea. One
/// species a road; cypresses on both sides, the others on one.
fn town_rows(scn: &Scenario, kit: &[TownSpot]) -> Vec<(TownSpot, Option<&'static str>)> {
    const ROW_STEP_M: f32 = 13.0;
    const ROW_OFFSET_M: f32 = 6.5;
    const ROW_TOWN_M: f32 = 45.0;
    const ROW_ROADS: usize = 2;
    const SPECIES: [&str; 5] = ["holm", "pine", "olive", "cypress", "holm"];
    let v = &scn.vectors;
    let homes: Vec<&scenario::Building> = v.buildings.iter().filter(|b| matches!(b.kind.as_deref(), Some("house" | "villa" | "terrace" | "shop"))).collect();
    // which towns are bare: trees per house of the kit, by the nearest house's town
    let mut count: std::collections::HashMap<&str, (usize, usize)> = std::collections::HashMap::new();
    for b in &homes {
        count.entry(b.locality.as_deref().unwrap_or("")).or_default().0 += 1;
    }
    let near_home = |p: Vec2| homes.iter().map(|b| (Vec2::from(b.centroid).distance(p), b)).min_by(|a, b| a.0.total_cmp(&b.0));
    for t in kit {
        if let Some((d, b)) = near_home(Vec2::new(t.at.x, t.at.y)) {
            if d < 60.0 {
                count.entry(b.locality.as_deref().unwrap_or("")).or_default().1 += 1;
            }
        }
    }
    let bare = |town: &str| count.get(town).is_some_and(|&(houses, trees)| (trees as f32) < 0.5 * houses as f32);
    let seg_dist = |p: Vec2, a: Vec2, b: Vec2| {
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        p.distance(a + ab * t)
    };
    let in_building = |p: Vec2| {
        v.buildings.iter().any(|b| {
            let ring: Vec<Vec2> = b.ring.iter().map(|q| Vec2::from(*q)).collect();
            let near = (0..ring.len()).any(|i| seg_dist(p, ring[i], ring[(i + 1) % ring.len()]) < 3.5);
            let mut inside = false;
            for i in 0..ring.len() {
                let (a, c) = (ring[i], ring[(i + 1) % ring.len()]);
                if (a.y > p.y) != (c.y > p.y) && p.x < a.x + (p.y - a.y) / (c.y - a.y) * (c.x - a.x) {
                    inside = !inside;
                }
            }
            near || inside
        })
    };
    // the places a row could take: (town, road, the side, where)
    let mut could: Vec<(&str, usize, bool, Vec2)> = Vec::new();
    for (r, road) in v.roads.iter().enumerate().filter(|(_, r)| r.drivable && r.line.len() >= 2) {
        let line: Vec<Vec2> = road.line.iter().map(|q| Vec2::from(*q)).collect();
        let mut walked = 0.0f32;
        for seg in line.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let len = a.distance(b);
            if len < 1e-3 {
                continue;
            }
            let dir = (b - a) / len;
            let side = Vec2::new(-dir.y, dir.x);
            let mut s = (ROW_STEP_M - walked % ROW_STEP_M) % ROW_STEP_M;
            while s < len {
                let on = a + dir * s;
                for left in [false, true] {
                    let p = on + side * if left { ROW_OFFSET_M } else { -ROW_OFFSET_M };
                    let Some((d, home)) = near_home(p) else { continue };
                    let town = home.locality.as_deref().unwrap_or("");
                    if d > ROW_TOWN_M || !bare(town) || scn.terrain.height_at(Pos { x: p.x, y: p.y }) < 1.0 {
                        continue;
                    }
                    let near_road = v.roads.iter().any(|r| r.line.windows(2).any(|w| seg_dist(p, Vec2::from(w[0]), Vec2::from(w[1])) < ROW_OFFSET_M - 0.5));
                    if !near_road && !in_building(p) {
                        could.push((town, r, left, p));
                    }
                }
                s += ROW_STEP_M;
            }
            walked += len;
        }
    }
    // of a bare town, its `ROW_ROADS` roads with the longest rows; an
    // avenue of cypresses on both sides, any other row on one
    let mut by_road: std::collections::BTreeMap<(&str, usize), usize> = std::collections::BTreeMap::new();
    for &(town, r, _, _) in &could {
        *by_road.entry((town, r)).or_default() += 1;
    }
    let mut chosen: Vec<(&str, usize, usize)> = by_road.iter().map(|(&(t, r), &n)| (t, r, n)).collect();
    chosen.sort_by(|a, b| a.0.cmp(b.0).then(b.2.cmp(&a.2)).then(a.1.cmp(&b.1)));
    let mut kept: Vec<(&str, usize)> = Vec::new();
    for (town, r, _) in chosen {
        if kept.iter().filter(|k| k.0 == town).count() < ROW_ROADS {
            kept.push((town, r));
        }
    }
    let mut placed: Vec<Vec2> = kit.iter().map(|t| Vec2::new(t.at.x, t.at.y)).collect();
    let mut out = Vec::new();
    for (town, r, left, p) in could {
        let road = &v.roads[r];
        let species = SPECIES[(road.id.unsigned_abs() % SPECIES.len() as u64) as usize];
        if !kept.contains(&(town, r)) || (species != "cypress" && left != (road.id % 2 == 0)) || placed.iter().any(|q| q.distance(p) < 8.0) {
            continue;
        }
        placed.push(p);
        let yaw = hash01(p.x, p.y) * std::f32::consts::TAU;
        out.push((TownSpot { cypress: false, at: Pos { x: p.x, y: p.y }, scale: 1.5, yaw }, Some(species)));
    }
    out
}

fn hash01(x: f32, y: f32) -> f32 {
    let mut r = Rng::seeded(((x.to_bits() as u64) << 32) ^ y.to_bits() as u64 ^ 0x2077_5EED);
    r.unit()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A lean travels in the yaw's whole turns and leaves its sine and
    /// cosine as they were; no lean, the yaw itself (every other plant
    /// stays below `LEAN_TURNS`, which the shader reads as none).
    #[test]
    fn a_lean_rides_on_the_yaw() {
        use std::f32::consts::TAU;
        for k in 0..200 {
            let yaw = k as f32 / 200.0 * TAU;
            assert_eq!(lean_yaw(yaw, 1.0, 0.0), yaw);
            let (bearing, share) = ((k * 7 % 16) as f32 / 16.0 * TAU, 1.0 - (k % 7) as f32 / 7.0);
            let w = lean_yaw(yaw, bearing, share);
            assert!((w.cos() - yaw.cos()).abs() < 2e-3 && (w.sin() - yaw.sin()).abs() < 2e-3, "{yaw} {w}");
            // as `plant_common::place` reads it
            let code = (w / TAU).floor() - LEAN_TURNS;
            assert!(code >= 16.0, "{yaw}: {code}");
            assert_eq!(code % 16.0, (k * 7 % 16) as f32);
            let amount = (code / 16.0).floor() / 7.0 * LEAN_MAX;
            assert!((amount - share * LEAN_MAX).abs() < 1e-4, "{amount}");
        }
    }
}
