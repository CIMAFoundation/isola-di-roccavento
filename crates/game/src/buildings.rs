//! Buildings, extruded from their real OSM footprints.
//!
//! Every one of the 7,629 footprints in the window is drawn as actual
//! geometry: walls following the traced outline, an overhanging eave, and a
//! hipped or flat roof depending on what the building is. That matters beyond
//! looks. The player is asked to judge which streets are defensible and where
//! a fire can run into the town, and a field of identical boxes hides exactly
//! the things that decision turns on — how tightly packed the old centre is,
//! how the ridge-line houses stand alone in the fuel, where the industrial
//! sheds are.
//!
//! Heights come from data where there is data. OSM `building:levels` is
//! present on very few of these, so the storey count baked into the population
//! file is used where a dwelling exists, and otherwise a height is inferred
//! from the building's tag and footprint area. That inference is visible and
//! wrong in individual cases; it is not used by any model, only drawn.
//!
//! Colour is per-building, hashed from the OSM id off a Ligurian palette —
//! ochre, terracotta, cream, faded rose — so the town reads as a town rather
//! than as one material. The palette is the point where this stops being
//! documentation of data and starts being art direction, and it is confined to
//! [`palette`] so it can be replaced wholesale.
//!
//! Geometry is merged per 512 m chunk, the same trade [`crate::vegetation`]
//! makes: 7,629 entities would cost more in transform propagation and draw
//! calls than the triangles do, while chunks still cull.
//!
//! Buildings respond to the fire through [`fire::StructureExposure`], never
//! through the fire mask — a house sits on a non-burnable cell and can never
//! appear in it (see `fire::exposure`). A structure the exposure model has
//! ignited flares, then goes to charcoal.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use scenario::{Building, Pos, Scenario};

use crate::sim::Sim;
use crate::materials::{self, Finish, StructureMaterial};
use crate::materials::masonry::{self, Masonry, MasonryMaterial, Part};

#[path = "town_kit.rs"]
mod town_kit;
// the kit's toy trees, for the scanned trees put in their place (`vegetation`)
pub(crate) use town_kit::kit_trees;

/// Unlit window glass — dark enough that a daylit building reads as an
/// ordinary wall with punched openings, not a lattice of black holes.
const WINDOW_GLASS: [f32; 4] = [0.06, 0.07, 0.10, 1.0];

/// A lit window after dark. Pushed above 1.0 like the fire's own emissive
/// tricks (`Damage::Alight`), so the bloom pass catches it and a lit house
/// actually reads as a small light in the dark rather than a beige square.
const WINDOW_LIT: [f32; 4] = [2.1, 1.55, 0.85, 1.0];

/// Below this fraction of `SunState::brightness`, windows switch on. Not a
/// fade: a lit window is a binary fact about a house (is anyone home, is it
/// dark out), and a threshold crossed twice a run is a cheaper and just as
/// honest a signal as a continuous glow ramp.

/// Chunk edge in metres.
const CHUNK_M: f32 = 512.0;

/// Metres per storey.
const STOREY_M: f32 = 3.2;

/// How far the roof overhangs the wall. Small, but it is what gives every
/// building a shadow line under the eave instead of a flat silhouette.
const EAVE_M: f32 = 0.55;

/// Simulated seconds a structure flames before it is a ruin. Short, so a
/// three-minute session shows the ruin and not only the flare.
const BURN_DOWN_S: f32 = 8.0 * 60.0;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Damage {
    None = 0,
    /// Taking enough heat to be worth watching, not yet alight.
    Threatened = 1,
    Alight = 2,
    Destroyed = 3,
}

struct Structure {
    /// Households living here, for the exposure lookup.
    households: Vec<u32>,
    pos: Pos,
    vert_start: u32,
    vert_end: u32,
    /// Window quads, a contiguous sub-range of `vert_start..vert_end` — see
    /// `emit_building`. `window_start == window_end` for a shed or
    /// industrial hall, which gets none.
    window_start: u32,
    window_end: u32,
    drawn: u8,
    /// Whether this structure's windows are currently lit. Cached rather
    /// than recomputed on every read, because both the damage pass and the
    /// hover pass need to know it and only the former has the sim handy.
    lit: bool,
    /// Simulated time this structure caught, if it has.
    alight_at_s: f32,
}

struct Chunk {
    mesh: Handle<Mesh>,
    structures: Vec<Structure>,
    /// Colours as built, before any fire damage.
    base: Vec<[f32; 4]>,
}

#[derive(Resource)]
pub struct Buildings {
    chunks: Vec<Chunk>,
    /// Sim generation `damage` last ran for (`u64::MAX` = none, after a restart).
    damage_gen: u64,
}

pub fn spawn(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StructureMaterial>>,
    mut masonry_materials: ResMut<Assets<MasonryMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let scn = &sim.scn;
    // With the scanned plants, photographed plaster, tiles and stone
    // (`materials::masonry`, 2026-11-06); without, exactly as before.
    let real = masonry::textures(&mut images);

    // Storeys, where the population bake knows them.
    let mut levels: HashMap<i64, u8> = HashMap::new();
    for d in &scn.population.dwellings {
        levels.insert(d.osm_id, d.levels);
    }
    // Which households live in which building, for the damage layer.
    let mut residents: HashMap<i64, Vec<u32>> = HashMap::new();
    for h in &scn.population.households {
        residents.entry(h.building).or_default().push(h.id as u32);
    }

    let base_material = StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.75,
        metallic: 0.0,
        reflectance: 0.35,
        ..default()
    };
    let masonry_material = real.as_ref().map(|t| masonry_materials.add(masonry::material(base_material, t, Masonry::Town)));
    let material = materials.add(materials::structure(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.75,
        metallic: 0.0,
        // Sunbaked lime-washed plaster and glazed terracotta both carry a
        // faint sheen a matte material (the terrain's own 0.12) does not —
        // enough to pick out a highlight along a wall at low sun, not enough
        // to look wet.
        reflectance: 0.35,
        // VR-training dev scenarios are flat unlit geometry, not sunlit
        // plaster.
        unlit: false,
        ..default()
    }, Finish::Town));

    let cols = (scn.world.width_m / CHUNK_M).ceil() as usize + 1;
    let rows = (scn.world.height_m / CHUNK_M).ceil() as usize + 1;
    let mut buckets: Vec<Vec<&Building>> = vec![Vec::new(); rows * cols];
    for b in &scn.vectors.buildings {
        let cx = (b.centroid[0] / CHUNK_M).max(0.0) as usize;
        let cy = (b.centroid[1] / CHUNK_M).max(0.0) as usize;
        if cx < cols && cy < rows {
            buckets[cy * cols + cx].push(b);
        }
    }

    let mut chunks = Vec::new();
    let (mut count, mut tris) = (0usize, 0usize);

    for bucket in buckets {
        if bucket.is_empty() {
            continue;
        }
        let mut builder = Builder { real: real.is_some(), ..default() };
        let mut structures = Vec::new();

        for b in bucket {
            let start = builder.positions.len() as u32;
            let Some((win_start, win_end)) =
                emit_building(scn, b, levels.get(&b.id).copied(), &mut builder)
            else {
                continue;
            };
            let households = residents.get(&b.id).cloned().unwrap_or_default();
            structures.push(Structure {
                households,
                pos: Pos {
                    x: b.centroid[0],
                    y: b.centroid[1],
                },
                vert_start: start,
                vert_end: builder.positions.len() as u32,
                window_start: win_start,
                window_end: win_end,
                drawn: Damage::None as u8,
                lit: false,
                alight_at_s: f32::INFINITY,
            });
            count += 1;
        }

        if structures.is_empty() {
            continue;
        }
        tris += builder.indices.len() / 3;
        let base = builder.colors.clone();
        let mesh = meshes.add(builder.finish());
        match &masonry_material {
            Some(m) => commands.spawn(((Mesh3d(mesh.clone()), MeshMaterial3d(m.clone())), crate::terrain_mesh::PauseSolid)),
            None => commands.spawn(((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())), crate::terrain_mesh::PauseSolid)),
        };
        chunks.push(Chunk {
            mesh,
            structures,
            base,
        });
    }

    info!(
        "buildings: {count} footprints in {} chunks, {:.2} M triangles",
        chunks.len(),
        tris as f32 / 1e6
    );
    commands.insert_resource(Buildings {
        chunks,
        damage_gen: u64::MAX,
    });
}

/// What kind of thing this is, which sets its height, its roof and its palette.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    House,
    Apartments,
    Industrial,
    Civic,
    Shed,
}

impl Kind {
    fn of(tag: Option<&str>, area: f32) -> Kind {
        match tag.unwrap_or("yes") {
            "house" | "detached" | "residential" | "cabin" | "bungalow" | "villa" | "terrace" | "shop" => Kind::House,
            "apartments" | "hotel" | "dormitory" => Kind::Apartments,
            "industrial" | "warehouse" | "commercial" | "retail" | "supermarket" => {
                Kind::Industrial
            }
            "church" | "chapel" | "school" | "public" | "civic" | "hospital" | "train_station" => {
                Kind::Civic
            }
            "roof" | "garage" | "garages" | "shed" | "hut" | "construction" | "ruins" => Kind::Shed,
            // The overwhelming majority are tagged `building=yes`, so the
            // footprint has to do the work: a 40 m² box is a garage, a
            // 600 m² one on this coast is a block of flats.
            _ if area < 45.0 => Kind::Shed,
            _ if area > 420.0 => Kind::Apartments,
            _ => Kind::House,
        }
    }

    fn flat_roof(self) -> bool {
        matches!(self, Kind::Industrial | Kind::Shed)
    }
}

/// Ligurian coastal palette: lime-washed ochres and pinks with terracotta
/// roofs. Chosen by eye against photographs of Spotorno and Noli, and the one
/// place in this file that is taste rather than data.
mod palette {
    pub const WALLS: [[f32; 3]; 8] = [
        [0.93, 0.76, 0.50], // ochre
        [0.95, 0.66, 0.52], // apricot
        [0.96, 0.93, 0.84], // cream
        [0.90, 0.58, 0.55], // rose
        [0.97, 0.88, 0.58], // pale yellow
        [0.70, 0.82, 0.90], // Ligurian pale blue
        [0.76, 0.88, 0.76], // mint
        [0.86, 0.48, 0.38], // terracotta red
    ];
    pub const ROOFS: [[f32; 3]; 6] = [
        [0.80, 0.38, 0.24],
        [0.86, 0.46, 0.28],
        [0.72, 0.30, 0.24],
        [0.90, 0.56, 0.30],
        [0.55, 0.42, 0.50],
        [0.34, 0.50, 0.58],
    ];
    /// Louvred shutters: Ligurian green, a darker green, chestnut, grey.
    pub const SHUTTERS: [[f32; 3]; 4] = [[0.20, 0.42, 0.28], [0.13, 0.30, 0.20], [0.42, 0.26, 0.14], [0.55, 0.58, 0.55]];
    /// Window sills and the cornice: pale stone and lime.
    pub const SILL: [f32; 3] = [0.90, 0.87, 0.80];
    pub const INDUSTRIAL_WALL: [f32; 3] = [0.74, 0.74, 0.72];
    pub const INDUSTRIAL_ROOF: [f32; 3] = [0.52, 0.55, 0.57];
    pub const CIVIC_WALL: [f32; 3] = [0.90, 0.89, 0.85];

    /// With the photographed masonry (`materials::masonry`): the lime washes
    /// of Liguria as photographs show them, sun-faded (ochre, Genoa red,
    /// salmon, cream, straw, a grey-green), and the clay of the tiles. The
    /// photographs add their own shade, so these are a little lighter.
    pub mod real {
        pub const WALLS: [[f32; 3]; 8] = [
            [0.88, 0.72, 0.50], // ochre
            [0.86, 0.60, 0.48], // salmon
            [0.92, 0.88, 0.80], // cream
            [0.80, 0.52, 0.44], // faded Genoa red
            [0.92, 0.82, 0.60], // straw
            [0.78, 0.78, 0.70], // grey-green
            [0.90, 0.78, 0.66], // apricot
            [0.84, 0.66, 0.56], // rose
        ];
        pub const ROOFS: [[f32; 3]; 5] = [
            [0.70, 0.36, 0.24],
            [0.76, 0.42, 0.28],
            [0.62, 0.32, 0.24],
            [0.74, 0.48, 0.34],
            [0.58, 0.38, 0.30],
        ];
        pub const CIVIC_WALL: [f32; 3] = [0.88, 0.85, 0.78];
    }
}

/// Footprint magnification (see `emit_building`).
const TOY_SCALE: f32 = crate::visual_scale::BUILDING_FOOTPRINT;

/// Toy houses are taller than real ones so they stand up out of the street.
const TOY_HEIGHT: f32 = crate::visual_scale::BUILDING_HEIGHT;

/// Emit one building, and return the vertex range of its window quads
/// (`start == end` if it has none). `None` for footprints too degenerate to
/// draw.
fn emit_building(
    scn: &Scenario,
    b: &Building,
    baked_levels: Option<u8>,
    out: &mut Builder,
) -> Option<(u32, u32)> {
    let n = b.ring.len();
    if n < 3 {
        return None;
    }
    // Landmarks and open spaces come from the town kit.
    if town_kit::emit(scn, b, out) {
        let end = out.positions.len() as u32;
        return Some((end, end));
    }
    let area = b.area();
    if area < 6.0 {
        return None;
    }
    // The kit's toy trees beside the houses would be cartoons among the
    // scanned plants: not drawn with them.
    if !out.real {
        town_kit::garden(scn, b, out);
    }

    // OSM rings repeat the first point as the last; drop it, and drop any
    // vertex that duplicates its predecessor, or the wall quads degenerate.
    let mut ring: Vec<Pos> = Vec::with_capacity(n);
    for v in &b.ring {
        let p = Pos { x: v[0], y: v[1] };
        if ring
            .last()
            .map(|q: &Pos| (q.x - p.x).abs() < 0.05 && (q.y - p.y).abs() < 0.05)
            .unwrap_or(false)
        {
            continue;
        }
        ring.push(p);
    }
    if ring.len() > 2 {
        let (f, l) = (ring[0], *ring.last().unwrap());
        if (f.x - l.x).abs() < 0.05 && (f.y - l.y).abs() < 0.05 {
            ring.pop();
        }
    }
    if ring.len() < 3 {
        return None;
    }
    // Wind counter-clockwise so wall normals face outward.
    if signed_area(&ring) < 0.0 {
        ring.reverse();
    }

    // Toy scale: a diorama house is bigger than a real one, so the town reads
    // at 2 m. About the footprint's own centroid.
    {
        let k = TOY_SCALE;
        let (cx, cy) = (
            ring.iter().map(|p| p.x).sum::<f32>() / ring.len() as f32,
            ring.iter().map(|p| p.y).sum::<f32>() / ring.len() as f32,
        );
        for p in ring.iter_mut() {
            p.x = cx + (p.x - cx) * k;
            p.y = cy + (p.y - cy) * k;
        }
    }
    // Infer the building type and storeys from the real footprint, so visual
    // magnification cannot turn a garage into a house or add storeys.
    let kind = Kind::of(b.kind.as_deref(), area);
    let h = hash01(b.id as u64, 0x1F);
    // The population bake's storey count is itself synthetic and sits at 2 for
    // 98% of dwellings, which draws the old town as a field of bungalows. It
    // is used as a floor, not as truth: a 400 m² block on this coast is four
    // or five storeys whatever the file says.
    let by_area: f32 = match area {
        a if a < 60.0 => 1.0,
        a if a < 130.0 => 2.0,
        a if a < 260.0 => 3.0,
        a if a < 500.0 => 4.0,
        _ => 5.0,
    };
    let storeys = match kind {
        Kind::Shed => 1.0,
        Kind::Industrial => 1.0,
        Kind::Civic => by_area.max(2.0),
        // A detached house stays a house however wide its footprint is.
        Kind::House => baked_levels.map(f32::from).unwrap_or(2.0).max(2.0).min(3.0),
        Kind::Apartments => {
            baked_levels
                .map(f32::from)
                .unwrap_or(0.0)
                .max(by_area)
                .max(3.0)
                + (h * 2.0).floor()
        }
    };
    let wall_h = match kind {
        // A shed's storey is not 3.2 m, and an industrial hall's is much more.
        Kind::Shed => 2.6 + h * 0.8,
        Kind::Industrial => 6.5 + h * 3.0,
        Kind::Civic => storeys * STOREY_M * 1.35,
        _ => storeys * STOREY_M * (0.95 + 0.1 * h),
    } * TOY_HEIGHT;

    // Ground: sit the base below the lowest corner and the eave above the
    // highest, so a building on a slope is cut into the hill rather than
    // floating off it on the downhill side.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for p in &ring {
        let g = scn.terrain.height_at(*p);
        lo = lo.min(g);
        hi = hi.max(g);
    }
    let base = lo - 1.5;
    let eave = hi + wall_h;

    let centroid = centroid(&ring);
    // The palettes are sRGB, as picked; vertex colours are linear.
    let (wall, roof) = (
            match kind {
                Kind::Industrial => palette::INDUSTRIAL_WALL,
                Kind::Civic => palette::CIVIC_WALL,
                _ => palette::WALLS[(hash01(b.id as u64, 0x7C) * 8.0) as usize % 8],
            },
            match kind {
                Kind::Industrial | Kind::Shed => palette::INDUSTRIAL_ROOF,
                _ => palette::ROOFS[(hash01(b.id as u64, 0x3D) * 6.0) as usize % 6],
            },
        );
    // With the photographed masonry, the palette of real Ligurian plaster
    // (a cartoon's mint and sky blue go) and of clay tiles only.
    let (wall, roof) = if out.real {
        (
            match kind {
                Kind::Industrial => palette::INDUSTRIAL_WALL,
                Kind::Civic => palette::real::CIVIC_WALL,
                _ => palette::real::WALLS[(hash01(b.id as u64, 0x7C) * 8.0) as usize % 8],
            },
            match kind {
                Kind::Industrial | Kind::Shed => palette::INDUSTRIAL_ROOF,
                _ => palette::real::ROOFS[(hash01(b.id as u64, 0x3D) * 5.0) as usize % 5],
            },
        )
    } else {
        (wall, roof)
    };
    let (wall, roof) = (linear(wall), linear(roof));
    // A plinth: the darker, damper base course every masonry building on this
    // coast has. It is also what stops a wall reading as an untextured plane.
    let plinth_top = base + (eave - base).min(1.0) * 0.9 + 0.6;
    let plinth = [wall[0] * 0.78, wall[1] * 0.76, wall[2] * 0.74];

    let m = ring.len();
    for i in 0..m {
        let a = ring[i];
        let c = ring[(i + 1) % m];
        out.part = Part::Stone as u8;
        out.quad(
            [a.x, base, -a.y],
            [c.x, base, -c.y],
            [c.x, plinth_top, -c.y],
            [a.x, plinth_top, -a.y],
            if out.real { [plinth[0] * 1.1, plinth[1] * 1.1, plinth[2] * 1.1] } else { plinth },
        );
        out.part = Part::Plaster as u8;
        out.quad(
            [a.x, plinth_top, -a.y],
            [c.x, plinth_top, -c.y],
            [c.x, eave, -c.y],
            [a.x, eave, -a.y],
            wall,
        );
    }

    // Windows: only where someone plausibly lives or works, punched into the
    // wall just emitted above. Emitted as their own small quads (not a
    // texture) so they can be recoloured independently of the wall behind
    // them — dark glass by day, lit or dark by night depending on whether the
    // household is home. Contiguous in the builder, so the whole run is one
    // vertex range.
    let window_start = out.positions.len() as u32;
    if matches!(kind, Kind::House | Kind::Apartments | Kind::Civic) {
        let rows = (storeys.round() as usize).clamp(1, 6);
        for i in 0..m {
            let a = ring[i];
            let c = ring[(i + 1) % m];
            out.part = GLASS;
            emit_windows(a, c, plinth_top, eave, rows, out);
            out.part = Part::Plaster as u8;
        }
    }
    let window_end = out.positions.len() as u32;
    if matches!(kind, Kind::House | Kind::Apartments | Kind::Civic) {
        let rows = (storeys.round() as usize).clamp(1, 6);
        // Shutters on most homes (green or brown, as on this coast), stone
        // sills under every window, and a pale cornice under the eave.
        let shutters = (kind != Kind::Civic && hash01(b.id as u64, 0x5A) < 0.8)
            .then(|| linear(palette::SHUTTERS[(hash01(b.id as u64, 0x5B) * 4.0) as usize % 4]));
        for i in 0..m {
            let (a, c) = (ring[i], ring[(i + 1) % m]);
            emit_window_dressing(a, c, plinth_top, eave, rows, shutters, out);
            emit_cornice(a, c, eave, out);
        }
    }

    // A shop: a striped awning over the street side and a sign board, so the
    // main street reads as shops with flats above from across the gazebo.
    // Both hang on the footprint edge that faces most nearly south (weighted by
    // its length), not on the bounding box: on a rotated or L-shaped footprint
    // the box edge floats in the air beside the wall.
    out.part = 0;
    if b.kind.as_deref() == Some("shop") {
        let mut best: Option<(f32, Pos, Pos, (f32, f32))> = None;
        for i in 0..m {
            let (a, c) = (ring[i], ring[(i + 1) % m]);
            let (dx, dy) = (c.x - a.x, c.y - a.y);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 3.0 {
                continue;
            }
            let mut n = (dy / len, -dx / len);
            if n.0 * ((a.x + c.x) * 0.5 - centroid.x) + n.1 * ((a.y + c.y) * 0.5 - centroid.y) < 0.0 {
                n = (-n.0, -n.1);
            }
            let score = len * (-n.1).max(0.0);
            if -n.1 > 0.5 && best.map_or(true, |b| score > b.0) {
                best = Some((score, a, c, n));
            }
        }
        if let Some((_, a, c, n)) = best {
            // Along the wall, so that the quads wind as they did on the
            // south edge (t = +x when n = -y).
            let t = (-n.1, n.0);
            let (pa, pc) = (a.x * t.0 + a.y * t.1, c.x * t.0 + c.y * t.1);
            let (lo_t, hi_t) = (pa.min(pc), pa.max(pc));
            let (x0, x1) = (lo_t + 0.6, hi_t - 0.6);
            let mid = Pos { x: (a.x + c.x) * 0.5, y: (a.y + c.y) * 0.5 };
            let g = scn.terrain.height_at(mid);
            // Never above the eave of a low shop.
            let top = (g + 4.6).min(eave - 0.8).max(g + 2.2);
            let low = top - 1.2;
            // The point at distance `d` out from the wall, `u` along it.
            let wall_d = mid.x * n.0 + mid.y * n.1;
            let at = |u: f32, d: f32, y: f32| {
                let o = wall_d + d;
                [u * t.0 + o * n.0, y, -(u * t.1 + o * n.1)]
            };
            let stripes = [[0.92, 0.25, 0.22], [0.97, 0.95, 0.90], [0.20, 0.55, 0.35], [0.97, 0.95, 0.90], [0.25, 0.45, 0.85]];
            let pick = stripes[(hash01(b.id as u64, 0x91) * 3.0) as usize % 3 * 2 % 5];
            let cnt = 6;
            for k in 0..cnt {
                let xa = x0 + (x1 - x0) * k as f32 / cnt as f32;
                let xb = x0 + (x1 - x0) * (k + 1) as f32 / cnt as f32;
                let col = if k % 2 == 0 { pick } else { [0.97, 0.95, 0.90] };
                // Wound so the face points up and out over the street (finding 11).
                out.quad(at(xa, 0.05, top), at(xa, 2.4, low), at(xb, 2.4, low), at(xb, 0.05, top), col);
            }
            out.quad(
                at(x0 + 1.0, 0.12, top + 1.6),
                at(x0 + 1.0, 0.12, top + 0.4),
                at(x1 - 1.0, 0.12, top + 0.4),
                at(x1 - 1.0, 0.12, top + 1.6),
                [0.98, 0.90, 0.55],
            );
        }
    }

    // The eave ring is the footprint pushed outward, which both casts the
    // shadow line and hides the seam where roof meets wall.
    let eaves: Vec<Pos> = offset_ring(&ring, centroid, EAVE_M);
    // Underside of the overhang, in shadow.
    let soffit = [roof[0] * 0.35, roof[1] * 0.35, roof[2] * 0.35];
    out.part = Part::Tiles as u8;
    for i in 0..m {
        let a = ring[i];
        let c = ring[(i + 1) % m];
        let ea = eaves[i];
        let ec = eaves[(i + 1) % m];
        out.quad(
            [ea.x, eave, -ea.y],
            [ec.x, eave, -ec.y],
            [c.x, eave, -c.y],
            [a.x, eave, -a.y],
            soffit,
        );
    }

    if kind.flat_roof() {
        // Flat roof with a low parapet: an industrial shed silhouette.
        out.part = Part::Plaster as u8;
        let top = eave + 0.5;
        for i in 0..m {
            let a = eaves[i];
            let c = eaves[(i + 1) % m];
            out.quad(
                [a.x, eave, -a.y],
                [c.x, eave, -c.y],
                [c.x, top, -c.y],
                [a.x, top, -a.y],
                [roof[0] * 0.8, roof[1] * 0.8, roof[2] * 0.8],
            );
        }
        out.fan(&eaves, top, centroid, roof);
    } else {
        // Hipped roof: the eave ring pulled in toward the centroid and lifted.
        // Not a straight skeleton — on a concave footprint the inset ring can
        // pinch — but at this scale the silhouette is right and the cost is a
        // few triangles.
        let inset = (area.sqrt() * 0.22).clamp(1.0, 4.5);
        let ridge_h = (inset * 1.25).clamp(2.2, 6.0);
        let cap: Vec<Pos> = offset_ring(&eaves, centroid, -inset);
        let ridge = eave + ridge_h;
        for i in 0..m {
            let a = eaves[i];
            let c = eaves[(i + 1) % m];
            let ca = cap[i];
            let cc = cap[(i + 1) % m];
            out.quad(
                [a.x, eave, -a.y],
                [c.x, eave, -c.y],
                [cc.x, ridge, -cc.y],
                [ca.x, ridge, -ca.y],
                roof,
            );
        }
        // Slightly lighter along the ridge, as weathered tile is.
        out.fan(
            &cap,
            ridge,
            centroid,
            [roof[0] * 1.12, roof[1] * 1.1, roof[2] * 1.08],
        );
        // A chimney on most houses: the silhouette cue that says "home".
        if matches!(kind, Kind::House | Kind::Apartments) && hash01(b.id as u64, 0x55) < 0.7 {
            let k = (area.sqrt() * 0.07).clamp(0.9, 1.8);
            let c = Pos { x: centroid.x + k * 1.6, y: centroid.y + k * 0.6 };
            let sq = [
                Pos { x: c.x - k * 0.5, y: c.y - k * 0.5 },
                Pos { x: c.x + k * 0.5, y: c.y - k * 0.5 },
                Pos { x: c.x + k * 0.5, y: c.y + k * 0.5 },
                Pos { x: c.x - k * 0.5, y: c.y + k * 0.5 },
            ];
            out.part = Part::Plaster as u8;
            out.prism(&sq, ridge - 0.8, ridge + k * 2.2, [0.78, 0.70, 0.62], [0.35, 0.30, 0.28]);
        }
    }
    out.part = 0;
    Some((window_start, window_end))
}

/// Punch up to two windows per storey into the wall segment `a -> c`, pushed
/// a few centimetres proud of the wall plane so they do not z-fight with it.
/// Skips edges too short to hold one convincingly — a gable end on a shed-
/// sized footprint gets no window rather than one spanning the whole wall.
fn emit_windows(a: Pos, c: Pos, plinth_top: f32, eave: f32, rows: usize, out: &mut Builder) {
    let elen = ((c.x - a.x).powi(2) + (c.y - a.y).powi(2)).sqrt();
    if elen < 3.0 {
        return;
    }
    let cols: &[f32] = if elen > 7.0 { &[0.32, 0.68] } else { &[0.5] };
    let half_w = (elen * 0.09).clamp(0.35, 0.9);
    // The wall's own outward normal, from the same three corners its quad
    // was built from — reusing it keeps the window coplanar-but-proud rather
    // than needing its own (and possibly inward-facing) offset.
    let wn = normal(
        [a.x, plinth_top, -a.y],
        [c.x, plinth_top, -c.y],
        [c.x, eave, -c.y],
    );
    let push = |p: [f32; 3]| {
        [
            p[0] + wn[0] * 0.04,
            p[1] + wn[1] * 0.04,
            p[2] + wn[2] * 0.04,
        ]
    };

    let band = (eave - plinth_top) / rows as f32;
    for r in 0..rows {
        let band_lo = plinth_top + band * r as f32;
        let win_h = band * 0.5;
        let y_lo = band_lo + band * 0.25;
        let y_hi = y_lo + win_h;
        for &t in cols {
            let cx = a.x + (c.x - a.x) * t;
            let cy = a.y + (c.y - a.y) * t;
            let (tx, ty) = ((c.x - a.x) / elen, (c.y - a.y) / elen);
            let (lx, ly) = (cx - tx * half_w, cy - ty * half_w);
            let (rx, ry) = (cx + tx * half_w, cy + ty * half_w);
            out.quad(
                push([lx, y_lo, -ly]),
                push([rx, y_lo, -ry]),
                push([rx, y_hi, -ry]),
                push([lx, y_hi, -ly]),
                [WINDOW_GLASS[0], WINDOW_GLASS[1], WINDOW_GLASS[2]],
            );
        }
    }
}

/// The shutters and sills of the windows `emit_windows` punches in the same
/// wall: the same layout, so they frame them. Outside the window range, so a
/// lit window does not light its shutters.
fn emit_window_dressing(a: Pos, c: Pos, plinth_top: f32, eave: f32, rows: usize, shutters: Option<[f32; 3]>, out: &mut Builder) {
    let elen = ((c.x - a.x).powi(2) + (c.y - a.y).powi(2)).sqrt();
    if elen < 3.0 {
        return;
    }
    let cols: &[f32] = if elen > 7.0 { &[0.32, 0.68] } else { &[0.5] };
    let half_w = (elen * 0.09).clamp(0.35, 0.9);
    let wn = normal([a.x, plinth_top, -a.y], [c.x, plinth_top, -c.y], [c.x, eave, -c.y]);
    let off = |p: [f32; 3], d: f32| [p[0] + wn[0] * d, p[1] + wn[1] * d, p[2] + wn[2] * d];
    let (tx, ty) = ((c.x - a.x) / elen, (c.y - a.y) / elen);
    let sill = linear(palette::SILL);
    let band = (eave - plinth_top) / rows as f32;
    for r in 0..rows {
        let y_lo = plinth_top + band * r as f32 + band * 0.25;
        let y_hi = y_lo + band * 0.5;
        for &t in cols {
            let (cx, cy) = (a.x + (c.x - a.x) * t, a.y + (c.y - a.y) * t);
            let at = |k: f32, y: f32| [cx + tx * half_w * k, y, -(cy + ty * half_w * k)];
            if let Some(col) = shutters {
                // open, against the wall either side
                for (k0, k1) in [(-2.0, -1.08), (1.08, 2.0)] {
                    out.quad_on(off(at(k0, y_lo), 0.07), off(at(k1, y_lo), 0.07), off(at(k1, y_hi), 0.07), off(at(k0, y_hi), 0.07), col, SHUTTER);
                }
            }
            if out.real {
                // A painted frame around the glass and a cross of glazing
                // bars: what makes a dark rectangle a window from near.
                let part = std::mem::replace(&mut out.part, Part::Frame as u8);
                let frame = linear(palette::SILL);
                let bar = 0.09;
                let (fy0, fy1) = (y_lo - 0.02, y_hi + 0.06);
                let (k0, k1) = (-1.0 - 0.12 / half_w, 1.0 + 0.12 / half_w);
                // jambs and head, a little proud of the wall
                for (ka, kb) in [(k0, -1.0), (1.0, k1)] {
                    out.quad(off(at(ka, fy0), 0.06), off(at(kb, fy0), 0.06), off(at(kb, fy1), 0.06), off(at(ka, fy1), 0.06), frame);
                }
                out.quad(off(at(-1.0, y_hi), 0.06), off(at(1.0, y_hi), 0.06), off(at(1.0, fy1), 0.06), off(at(-1.0, fy1), 0.06), frame);
                // glazing bars over the glass (which is 4 cm proud)
                let dark = [frame[0] * 0.8, frame[1] * 0.8, frame[2] * 0.8];
                let bw = bar / half_w;
                out.quad(off(at(-bw * 0.5, y_lo), 0.07), off(at(bw * 0.5, y_lo), 0.07), off(at(bw * 0.5, y_hi), 0.07), off(at(-bw * 0.5, y_hi), 0.07), dark);
                let ym = y_lo + (y_hi - y_lo) * 0.62;
                out.quad(off(at(-1.0, ym - bar * 0.5), 0.07), off(at(1.0, ym - bar * 0.5), 0.07), off(at(1.0, ym + bar * 0.5), 0.07), off(at(-1.0, ym + bar * 0.5), 0.07), dark);
                out.part = part;
            }
            // sill: a top and a front face, proud of the wall
            let (s0, s1) = (at(-1.15, y_lo - 0.02), at(1.15, y_lo - 0.02));
            out.quad(off(s0, 0.0), off(s0, 0.22), off(s1, 0.22), off(s1, 0.0), sill);
            let (f0, f1) = (at(-1.15, y_lo - 0.18), at(1.15, y_lo - 0.18));
            out.quad(off(f0, 0.22), off(f1, 0.22), off(s1, 0.22), off(s0, 0.22), sill);
        }
    }
}

/// A pale moulded band under the eave, proud of the wall: the line that
/// closes a Ligurian facade at the top.
fn emit_cornice(a: Pos, c: Pos, eave: f32, out: &mut Builder) {
    let elen = ((c.x - a.x).powi(2) + (c.y - a.y).powi(2)).sqrt();
    if elen < 0.5 {
        return;
    }
    let wn = normal([a.x, eave, -a.y], [c.x, eave, -c.y], [c.x, eave + 1.0, -c.y]);
    let off = |p: [f32; 3], d: f32| [p[0] + wn[0] * d, p[1], p[2] + wn[2] * d];
    let col = linear(palette::SILL);
    let (lo, hi) = (eave - 0.7, eave - 0.05);
    let (a0, c0) = ([a.x, lo, -a.y], [c.x, lo, -c.y]);
    let (a1, c1) = ([a.x, hi, -a.y], [c.x, hi, -c.y]);
    out.quad(off(a0, 0.18), off(c0, 0.18), off(c1, 0.18), off(a1, 0.18), col);
    // its underside, in shadow
    out.quad(off(a0, 0.0), off(c0, 0.0), off(c0, 0.18), off(a0, 0.18), col);
}

/// sRGB (as the palettes are picked) to the linear vertex colour.
fn linear(c: [f32; 3]) -> [f32; 3] {
    let l = Color::srgb(c[0], c[1], c[2]).to_linear();
    [l.red, l.green, l.blue]
}

/// Vertex surface of the shutters (`materials::surface_alpha`): louvres.
const SHUTTER: u8 = 11;

/// Vertex surface of window glass, with the photographed masonry.
const GLASS: u8 = 4;

fn signed_area(ring: &[Pos]) -> f32 {
    let n = ring.len();
    let mut acc = 0.0;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        acc += a.x * b.y - b.x * a.y;
    }
    acc * 0.5
}

fn centroid(ring: &[Pos]) -> Pos {
    let n = ring.len() as f32;
    Pos {
        x: ring.iter().map(|p| p.x).sum::<f32>() / n,
        y: ring.iter().map(|p| p.y).sum::<f32>() / n,
    }
}

/// Push every vertex `d` metres away from the centroid (negative pulls in).
/// A true polygon offset would use the edge bisectors; radial offset is
/// stable on the concave rings OSM is full of, and the difference at 0.5 m on
/// a 10 m building is not visible.
fn offset_ring(ring: &[Pos], c: Pos, d: f32) -> Vec<Pos> {
    ring.iter()
        .map(|p| {
            let (vx, vy) = (p.x - c.x, p.y - c.y);
            let len = (vx * vx + vy * vy).sqrt().max(1e-3);
            // Never let an inward offset cross the centroid.
            let k = if d < 0.0 { d.max(-len * 0.75) } else { d };
            Pos {
                x: p.x + vx / len * k,
                y: p.y + vy / len * k,
            }
        })
        .collect()
}

#[derive(Default)]
struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    /// The photographed masonry is on (`materials::masonry`): the vertices
    /// carry what they are made of, `part`.
    real: bool,
    /// What the quads emitted now are (`masonry::Part`, or a structure
    /// surface id), when `real`; 0: a prop, as it is.
    part: u8,
    /// Quads emitted now face straight up whatever their slope (paving
    /// draped on the ground, `town_kit`).
    up_normals: bool,
}

impl Builder {
    /// Flat-shaded quad, wound a-b-c-d. Vertices are not shared between faces:
    /// buildings want hard edges, and sharing would smooth the corners into
    /// something inflatable.
    fn quad(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], color: [f32; 3]) {
        self.quad_on(a, b, c, d, color, 0);
    }

    /// The vertex alpha of what is emitted now, for a vertex of no surface
    /// of its own: always 1.0 (none) without the photographed masonry.
    fn part_alpha(&self) -> f32 {
        materials::surface_alpha(if self.real { self.part } else { 0 })
    }

    /// A quad of a given vertex surface (`materials::surface_alpha`).
    fn quad_on(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], color: [f32; 3], surface: u8) {
        let n = if self.up_normals { [0.0, 1.0, 0.0] } else { normal(a, b, c) };
        let start = self.positions.len() as u32;
        let alpha = if surface == 0 { self.part_alpha() } else { materials::surface_alpha(surface) };
        for v in [a, b, c, d] {
            self.positions.push(v);
            self.normals.push(n);
            self.colors.push([color[0], color[1], color[2], alpha]);
        }
        self.indices
            .extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    }

    /// A CCW ring extruded from `y0` to `y1`, with a flat cap.
    fn prism(&mut self, ring: &[Pos], y0: f32, y1: f32, wall: [f32; 3], top: [f32; 3]) {
        let m = ring.len();
        for i in 0..m {
            let (a, c) = (ring[i], ring[(i + 1) % m]);
            self.quad([a.x, y0, -a.y], [c.x, y0, -c.y], [c.x, y1, -c.y], [a.x, y1, -a.y], wall);
        }
        self.fan(ring, y1, centroid(ring), top);
    }

    /// Cap a ring at height `y` with a fan around its centroid.
    fn fan(&mut self, ring: &[Pos], y: f32, c: Pos, color: [f32; 3]) {
        let start = self.positions.len() as u32;
        let alpha = self.part_alpha();
        self.positions.push([c.x, y, -c.y]);
        self.normals.push([0.0, 1.0, 0.0]);
        self.colors.push([color[0], color[1], color[2], alpha]);
        for p in ring {
            self.positions.push([p.x, y, -p.y]);
            self.normals.push([0.0, 1.0, 0.0]);
            self.colors.push([color[0], color[1], color[2], alpha]);
        }
        let n = ring.len() as u32;
        for i in 0..n {
            self.indices
                .extend_from_slice(&[start, start + 1 + i, start + 1 + (i + 1) % n]);
        }
    }

    fn finish(self) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

fn normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
    [n[0] / len, n[1] / len, n[2] / len]
}

fn hash01(id: u64, salt: u64) -> f32 {
    let mut h = id.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u32 << 24) as f32
}

/// Recolour structures the fire has reached.
///
/// Threat comes from two places, and both are proximity models rather than the
/// fire mask: the household exposure model for anything with residents, and
/// the agent threat field for the rest. Only chunks containing a changed
/// structure are re-uploaded.
impl Buildings {
    /// Structures the books count as lost and still burning down, with the
    /// seconds since they caught. The fire view dresses these in tall flame,
    /// smoke and embers: a flat orange tint reads as a label, not a house on
    /// fire.
    pub fn burning(&self, now: f32) -> Vec<(Pos, f32)> {
        self.chunks
            .iter()
            .flat_map(|c| &c.structures)
            .filter(|s| s.alight_at_s.is_finite())
            .map(|s| (s.pos, (now - s.alight_at_s).max(0.0)))
            .filter(|(_, age)| *age <= BURN_DOWN_S)
            .collect()
    }
}

/// Forget what the fire did to the town.
///
/// `alight_at_s` is the one piece of state here that is *not* recomputed from
/// the sim each frame — it is a latch, deliberately, so a structure keeps
/// burning down after the front has moved on. That makes it also the one piece
/// a restart has to clear by hand, or the new run opens with the old run's
/// charred buildings still standing in it.
pub fn reset(
    mut restarted: MessageReader<crate::sim::SimRestarted>,
    mut buildings: ResMut<Buildings>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if restarted.is_empty() {
        return;
    }
    restarted.clear();
    buildings.damage_gen = u64::MAX;
    for chunk in &mut buildings.chunks {
        for s in &mut chunk.structures {
            s.alight_at_s = f32::INFINITY;
            s.drawn = Damage::None as u8;
            s.lit = false;
        }
        if let Some(mut mesh) = meshes.get_mut(&chunk.mesh) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, chunk.base.clone());
        }
    }
}

/// Recompute one structure's colours from scratch: `base` plus whatever
/// `s.drawn`/`s.lit` currently say. Shared by the damage pass (which decides
/// those two fields) and the hover pass (which only ever replays them) so
/// the two can never disagree about what an un-hovered, undamaged building
/// looks like.
/// The alpha (the vertex surface) is kept whatever the state.
fn recolor_structure(colors: &mut [[f32; 4]], base: &[[f32; 4]], s: &Structure) {
    let range = s.vert_start as usize..s.vert_end as usize;
    match s.drawn {
        x if x == Damage::Threatened as u8 => {
            // Lit by the fire rather than damaged by it: warm, and just
            // enough to pick the building out of the street.
            for i in range {
                let c = base[i];
                // A warm cast, not a colour change: the street still reads.
                colors[i] = [(c[0] * 0.85 + 0.18).min(1.3), c[1] * 0.80 + 0.06, c[2] * 0.6, c[3]];
            }
        }
        x if x == Damage::Alight as u8 => {
            // Above 1.0 so the bloom pass catches it, like the vegetation
            // flare.
            for i in range {
                colors[i] = [2.4, 0.75, 0.14, base[i][3]];
            }
        }
        x if x == Damage::Destroyed as u8 => {
            for i in range {
                let c = base[i];
                colors[i] = [
                    0.10 + c[0] * 0.08,
                    0.09 + c[1] * 0.07,
                    0.09 + c[2] * 0.07,
                    c[3],
                ];
            }
        }
        _ => {
            for i in range {
                colors[i] = base[i];
            }
            if s.lit {
                for i in (s.window_start as usize)..(s.window_end as usize) {
                    colors[i] = WINDOW_LIT;
                }
            }
        }
    }
}

/// Building states are the game's own facts, the same the outcome counts
/// (`rocca::Outcome::homes_hit`): a home the structure exposure model has
/// ignited flares and then chars; a home taking heat now takes a warm cast.
pub fn damage(
    sim: Res<Sim>,
    mut buildings: ResMut<Buildings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut frame: ResMut<crate::streamed_mesh::StreamedFrame>,
) {
    // The sim is flagged changed every frame (kiosk systems take it mutably);
    // the buildings only change when it has stepped.
    if buildings.damage_gen == sim.generation {
        return;
    }
    buildings.damage_gen = sim.generation;
    let now = sim.time_s() as f32;
    let fields = sim.fire.exposure().fields();
    let warm: Vec<bool> = fields.iter().map(|f| f.radiant + f.ember > 0.05).collect();
    let lost: Vec<bool> = fields.iter().map(|f| f.alight).collect();

    let Buildings { chunks, .. } = &mut *buildings;

    for chunk in chunks.iter_mut() {
        // the vertices of the structures that change
        let mut dirty = usize::MAX..0;
        for s in &mut chunk.structures {
            let lost = s.households.iter().any(|&h| lost.get(h as usize) == Some(&true));
            let threatened = s.households.iter().any(|&h| warm.get(h as usize) == Some(&true));
            if lost && s.alight_at_s.is_infinite() {
                s.alight_at_s = now;
            }
            let want = if s.alight_at_s.is_finite() {
                if now - s.alight_at_s > BURN_DOWN_S {
                    Damage::Destroyed
                } else {
                    Damage::Alight
                }
            } else if threatened {
                Damage::Threatened
            } else {
                Damage::None
            } as u8;
            if want != s.drawn {
                s.drawn = want;
                dirty.start = dirty.start.min(s.vert_start as usize);
                dirty.end = dirty.end.max(s.vert_end as usize);
            }
        }
        if dirty.is_empty() {
            continue;
        }

        // Only those vertices, in place (`streamed_mesh::write_range`): the
        // mesh prepared again was a new copy of the whole block of houses,
        // indices too, several times a second while a village burns.
        let Some(mesh) = meshes.get_mut_untracked(&chunk.mesh) else { continue };
        if let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR) {
            for s in &chunk.structures {
                if (s.vert_start as usize) < dirty.end && s.vert_end as usize > dirty.start {
                    recolor_structure(colors, &chunk.base, s);
                }
            }
        }
        crate::streamed_mesh::write_range(&mut frame, chunk.mesh.id(), mesh, dirty);
    }
}
