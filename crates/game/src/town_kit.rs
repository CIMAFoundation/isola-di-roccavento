//! The town kit on the map: landmarks and dressed open spaces.
//!
//! A building whose `kind` is a landmark (church, town hall, school, fire
//! station, utilities, farms, lighthouse) is drawn with its Blender model
//! (`scripts/build_town_models.py`) scaled to its footprint, instead of being
//! extruded. A footprint that is an open space -- piazza, pitch, car park,
//! assembly area, lido, campsite, harbour, cemetery -- is draped on the ground
//! and dressed with props. Everything lands in the building chunks' merged
//! mesh, so it costs no extra draw calls and inherits the chunk's culling.
//!
//! Open spaces are draped vertex by vertex on `Terrain::height_at`, never laid
//! flat (finding 12: a flat plane on a slope is half buried and half floating).

use std::f32::consts::{FRAC_PI_2, PI};

use scenario::{Building, Pos, Scenario};

use super::{hash01, Builder, TOY_HEIGHT, TOY_SCALE};
use crate::models::{kit, Kit};

/// Landmarks drawn from a kit model of the same name.
const MODELLED: [&str; 14] = [
    "church", "chapel", "townhall", "school", "fire_station", "fuel", "water_tower", "substation", "farm", "barn", "silo", "mill",
    "industrial", "lighthouse",
];

/// Paint for parked cars (their bodies are white in the bake).
const PAINT: [[f32; 3]; 7] = [
    [0.93, 0.93, 0.95],
    [0.55, 0.78, 0.95],
    [0.98, 0.85, 0.40],
    [0.60, 0.85, 0.62],
    [0.95, 0.55, 0.50],
    [0.75, 0.70, 0.92],
    [0.35, 0.40, 0.48],
];

/// With the photographed masonry: car paint as a car park shows it (white,
/// silver, greys, black, a dark blue, a dark red, a beige), linear.
const PAINT_REAL: [[f32; 3]; 7] = [
    [0.72, 0.72, 0.72],
    [0.42, 0.43, 0.45],
    [0.16, 0.17, 0.18],
    [0.025, 0.025, 0.03],
    [0.03, 0.05, 0.12],
    [0.24, 0.03, 0.03],
    [0.46, 0.41, 0.32],
];

/// Draw `b` if it is a landmark or an open space. `false`: not ours, extrude it.
pub(super) fn emit(scn: &Scenario, b: &Building, out: &mut Builder) -> bool {
    let Some(kind) = b.kind.as_deref() else { return false };
    let (min, max) = bbox(b);
    let c = Pos { x: (min.x + max.x) * 0.5, y: (min.y + max.y) * 0.5 };
    let (w, d) = (max.x - min.x, max.y - min.y);
    if MODELLED.contains(&kind) {
        // Scaled and turned to the footprint's own frame, not its bounding box.
        let f = Frame::of(b);
        let m = kit(kind);
        let [fw, fd] = m.footprint.unwrap_or([f.lu, f.lv]);
        let (sx, sz) = (f.lu * TOY_SCALE / fw, f.lv * TOY_SCALE / fd);
        let sy = (sx * sz).sqrt() * TOY_HEIGHT / TOY_SCALE;
        let mut low = f32::MAX;
        for a in [-0.5f32, 0.0, 0.5] {
            for b in [-0.5f32, 0.0, 0.5] {
                low = low.min(ground_at(scn, f.at([a * f.lu * TOY_SCALE, b * f.lv * TOY_SCALE])));
            }
        }
        // A plinth of paving so the model sits on its own lot, not on scrub.
        f.pave(scn, out, [0.0, 0.0], f.lu * TOY_SCALE + 6.0, f.lv * TOY_SCALE + 6.0, [0.80, 0.77, 0.70], 0.0);
        // with the photographed masonry: plaster walls and tiled slopes
        out.part = crate::materials::masonry::Part::Building as u8;
        place(out, kind, f.c, low - 0.4, [sx, sy, sz], f.yaw, None);
        out.part = 0;
        return true;
    }
    let h = |salt: u64| hash01(b.id as u64, salt);
    // Open spaces are laid out in the footprint's own frame: the OSM rings are
    // rotated rectangles, and their axis-aligned box is much larger.
    let f = Frame::of(b);
    match kind {
        "plaza" => {
            f.pave(scn, out, [0.0, 0.0], f.lu, f.lv, [0.80, 0.76, 0.68], 0.5);
            f.pave(scn, out, [0.0, 0.0], f.lu - 6.0, f.lv - 6.0, [0.88, 0.84, 0.76], 0.56);
            f.pave(scn, out, [0.0, 0.0], f.lu * 0.5, f.lv * 0.5, [0.80, 0.74, 0.64], 0.62);
            f.put(scn, out, "fountain", [0.0, 0.0], 1.6, 0.0, None);
            for (sa, sb) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                f.put(scn, out, "street_tree", [sa * f.lu * 0.4, sb * f.lv * 0.38], 1.6, h(7) * 6.0, None);
                f.put(scn, out, "bench", [sa * f.lu * 0.2, sb * f.lv * 0.3], 1.8, if sb > 0.0 { 0.0 } else { PI }, None);
                f.put(scn, out, "streetlight", [sa * f.lu * 0.46, sb * f.lv * 0.12], 1.4, 0.0, None);
            }
        }
        "pitch" => {
            let mut grass = [[0.34, 0.64, 0.31], [0.38, 0.70, 0.35]];
            let mut line = [0.97, 0.97, 0.95];
            if out.real {
                // turf and chalk as photographed: a real pitch is dark
                grass = [[0.075, 0.16, 0.055], [0.09, 0.185, 0.065]];
                line = [0.55, 0.55, 0.53];
            }
            // Paved surround, then the turf in mown bands across the length.
            f.pave(scn, out, [0.0, 0.0], f.lu + 8.0, f.lv + 8.0, [0.62, 0.58, 0.52], 0.5);
            let (pl, pw) = (f.lu - 4.0, f.lv - 4.0);
            let bands = 8;
            for i in 0..bands {
                let a = -pl * 0.5 + (i as f32 + 0.5) * pl / bands as f32;
                f.drape(scn, out, [a, 0.0], pl / bands as f32, pw, grass[i % 2], 0.58);
            }
            let (l, w) = (pl - 3.0, pw - 3.0);
            let t = 0.5;
            for s in [-1.0f32, 1.0] {
                f.drape(scn, out, [0.0, s * w * 0.5], l, t, line, 0.66);
                f.drape(scn, out, [s * l * 0.5, 0.0], t, w, line, 0.66);
                // Penalty box and goal area at each end.
                let (bl, bw) = (l * 0.16, w * 0.6);
                f.drape(scn, out, [s * (l * 0.5 - bl * 0.5), 0.0], bl, t, line, 0.66);
                f.drape(scn, out, [s * (l * 0.5 - bl), 0.0], t, bw, line, 0.66);
                f.drape(scn, out, [s * (l * 0.5 - bl * 0.5), bw * 0.5], bl, t, line, 0.66);
                f.drape(scn, out, [s * (l * 0.5 - bl * 0.5), -bw * 0.5], bl, t, line, 0.66);
                f.put(scn, out, "goal", [s * (l * 0.5 + 0.2), 0.0], 1.8, s * FRAC_PI_2, None);
            }
            f.drape(scn, out, [0.0, 0.0], t, w, line, 0.66);
            f.ring(scn, out, [0.0, 0.0], w * 0.14, t, line, 0.67);
            f.put(scn, out, "bench", [-l * 0.2, -(f.lv * 0.5 + 2.2)], 1.8, PI, None);
            f.put(scn, out, "bench", [l * 0.2, -(f.lv * 0.5 + 2.2)], 1.8, PI, None);
            for (sa, sb) in [(-1.0, -1.0), (1.0, 1.0)] {
                f.put(scn, out, "streetlight", [sa * (f.lu * 0.5 + 1.5), sb * (f.lv * 0.5 + 1.5)], 1.4, 0.0, None);
            }
        }
        "parking" => {
            // Magnified like every other building (`TOY_SCALE`), and bayed to
            // the toy car: the lot is the size of the cars that park in it.
            let (lu, lv) = (f.lu * TOY_SCALE, f.lv * TOY_SCALE);
            f.pave(scn, out, [0.0, 0.0], lu + 2.0, lv + 2.0, [0.60, 0.60, 0.58], 0.5);
            let r = out.real;
            // old asphalt and worn paint, as the roads' (`terrain.wesl`, `ASPHALT_REAL`)
            let pick = |toy: [f32; 3], real: [f32; 3]| if r { real } else { toy };
            f.drape(scn, out, [0.0, 0.0], lu, lv, pick([0.36, 0.37, 0.39], [0.10, 0.10, 0.105]), 0.56);
            let bay_width = 3.25 * crate::visual_scale::CAR;
            let bays = ((lu / bay_width) as i32).max(1);
            let step = lu / bays as f32;
            for row in [-1.0f32, 1.0] {
                for i in 0..=bays {
                    let a = -lu * 0.5 + i as f32 * step;
                    f.drape(scn, out, [a, row * lv * 0.27], 0.5, lv * 0.42, pick([0.95, 0.95, 0.92], [0.50, 0.50, 0.48]), 0.62);
                }
                for i in 0..bays {
                    let a = -lu * 0.5 + (i as f32 + 0.5) * step;
                    if hash01(b.id as u64 + i as u64 * 31 + (row > 0.0) as u64 * 7, 0x51) < 0.7 {
                        let car = ["car_hatch", "car_sedan", "car_suv", "car_van"][(hash01(b.id as u64 + i as u64, 0x52) * 4.0) as usize % 4];
                        let paint = if out.real { PAINT_REAL } else { PAINT }[(hash01(b.id as u64 + i as u64 * 3 + row as u64, 0x53) * 7.0) as usize % 7];
                        f.put(scn, out, car, [a, row * lv * 0.27], crate::visual_scale::CAR, if row > 0.0 { 0.0 } else { PI }, Some(paint));
                    }
                }
            }
            // Kerb along the aisle and a couple of lamps.
            f.drape(scn, out, [0.0, 0.0], lu, 0.8, pick([0.90, 0.88, 0.40], [0.45, 0.42, 0.16]), 0.62);
            for s in [-1.0f32, 1.0] {
                f.put(scn, out, "streetlight", [s * lu * 0.5, 0.0], 1.4, 0.0, None);
            }
        }
        "assembly" => {
            // The Protezione Civile assembly area: a cleared ground with its
            // green sign and the blue tents, the thing the visitor should
            // recognise next time they see one. Inset from the footprint so the
            // lane that borders it stays clear.
            let (lu, lv) = (f.lu - 14.0, f.lv - 14.0);
            let m = |c: [f32; 3]| if out.real { muted(c) } else { c };
            let (edge, ground, inner, paint) = (m([0.47, 0.70, 0.44]), m([0.78, 0.86, 0.70]), m([0.84, 0.89, 0.77]), m([0.22, 0.55, 0.30]));
            f.drape(scn, out, [0.0, 0.0], lu + 2.0, lv + 2.0, edge, 0.5);
            f.drape(scn, out, [0.0, 0.0], lu, lv, ground, 0.56);
            f.drape(scn, out, [0.0, 0.0], lu - 6.0, lv - 6.0, inner, 0.6);
            // A painted green border, like the sign.
            for s in [-1.0f32, 1.0] {
                f.drape(scn, out, [0.0, s * (lv * 0.5 - 1.5)], lu - 3.0, 1.0, paint, 0.64);
                f.drape(scn, out, [s * (lu * 0.5 - 1.5), 0.0], 1.0, lv - 3.0, paint, 0.64);
            }
            f.put(scn, out, "assembly_sign", [0.0, lv * 0.5 + 1.5], 3.2, 0.0, None);
            let n = ((lu * 0.8 / 22.0) as i32).max(2);
            for row in [-1.0f32, 1.0] {
                for i in 0..n {
                    let a = -lu * 0.4 + (i as f32 + 0.5) * lu * 0.8 / n as f32;
                    f.put(scn, out, "pc_tent", [a, row * lv * 0.22], 2.2, 0.0, None);
                }
            }
            for s in [-1.0f32, 1.0] {
                f.put(scn, out, "bench", [s * lu * 0.15, 0.0], 1.8, 0.0, None);
            }
        }
        "lido" => {
            for i in 0..((w / 9.0) as i32) {
                for j in 0..3 {
                    let p = Pos { x: c.x - w * 0.5 + 4.5 + i as f32 * 9.0, y: c.y - d * 0.3 + j as f32 * d * 0.3 };
                    if ground_at(scn, p) < 0.2 {
                        continue;
                    }
                    put(scn, out, if (i + j) % 2 == 0 { "umbrella_a" } else { "umbrella_b" }, p, 1.8, 0.0, None);
                    put(scn, out, "lounger", Pos { x: p.x + 2.4, y: p.y }, 1.8, 0.0, None);
                }
            }
            put(scn, out, "beach_hut", Pos { x: c.x + w * 0.5, y: c.y + d * 0.4 }, 2.2, 0.0, None);
        }
        "campsite" => {
            drape(scn, out, c, w, d, [0.74, 0.70, 0.52], 0.3);
            for i in 0..34u64 {
                let p = Pos { x: c.x + (hash01(b.id as u64 + i, 0x61) - 0.5) * w * 0.9, y: c.y + (hash01(b.id as u64 + i, 0x62) - 0.5) * d * 0.9 };
                let r = hash01(b.id as u64 + i, 0x63);
                let what = if r < 0.35 { "caravan" } else if r < 0.6 { "camp_tent" } else if r < 0.85 { "camp_tent_b" } else { "car_camper" };
                let paint = (what == "car_camper").then_some([0.95, 0.95, 0.92]);
                put(scn, out, what, p, if what.starts_with("camp_tent") { 2.2 } else { 1.4 }, r * 6.3, paint);
            }
        }
        "harbour" => {
            // A pier out into the sea from the middle of the waterfront, and
            // boats moored either side of it.
            let mut y = c.y + d * 0.5;
            let mut k = 0;
            while k < 40 && ground_at(scn, Pos { x: c.x, y }) > -2.5 {
                y -= 4.0;
                k += 1;
            }
            let start = c.y + d * 0.5;
            let len = (start - y).max(20.0);
            let mut t = 0.0;
            while t < len {
                place(out, "pier", Pos { x: c.x, y: start - t }, 0.2, [4.0, 1.0, 4.0], 0.0, None);
                t += 4.0;
            }
            for i in 0..6 {
                for s in [-1.0f32, 1.0] {
                    let p = Pos { x: c.x + s * 9.0, y: start - 10.0 - i as f32 * len / 6.5 };
                    if ground_at(scn, p) < -0.5 {
                        let m = if (i + (s > 0.0) as i32) % 3 == 0 { "sailboat" } else { "boat" };
                        place(out, m, p, 0.0, [1.5, 1.5, 1.5], s * std::f32::consts::FRAC_PI_2, None);
                    }
                }
            }
        }
        "cemetery" => {
            // grass between the graves (with the photographed masonry, as dry as the meadows)
            let lawn = if out.real { [0.16, 0.17, 0.09] } else { [0.50, 0.68, 0.42] };
            f.drape(scn, out, [0.0, 0.0], f.lu, f.lv, lawn, 0.5);
            // Gravel path down the middle, graves in rows either side.
            f.pave(scn, out, [0.0, 0.0], f.lu - 4.0, 2.4, [0.86, 0.83, 0.74], 0.56);
            let wall_c = [0.90, 0.88, 0.82];
            let ring = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]].map(|[a, b]| f.at([a * f.lu, b * f.lv]));
            // with the photographed masonry: lime-washed walls, stone graves
            let (wall_c, grave) = if out.real { ([0.48, 0.45, 0.39], [0.30, 0.29, 0.27]) } else { (wall_c, [0.62, 0.62, 0.60]) };
            out.part = if out.real { crate::materials::masonry::Part::Plaster as u8 } else { 0 };
            for i in 0..4 {
                wall(scn, out, ring[i], ring[(i + 1) % 4], 2.6, 0.8, wall_c);
            }
            out.part = 0;
            for s in [-1.0f32, 1.0] {
                for i in 0..5 {
                    f.put(scn, out, "cypress", [-f.lu * 0.4 + i as f32 * f.lu * 0.2, s * f.lv * 0.4], 1.1, 0.0, None);
                }
                for i in 0..6 {
                    for j in 0..2 {
                        let at = [-f.lu * 0.4 + 1.5 + i as f32 * f.lu * 0.11, s * (f.lv * 0.12 + j as f32 * f.lv * 0.12 + 2.0)];
                        out.part = if out.real { crate::materials::masonry::Part::Stone as u8 } else { 0 };
                        f.stone(scn, out, at, grave);
                        out.part = 0;
                    }
                }
            }
            f.put(scn, out, "chapel", [f.lu * 0.36, 0.0], 0.75, 0.0, None);
        }
        _ => return false,
    }
    true
}

/// Extra lift on every draped layer: the terrain mesh is finer than the 4 m posting
/// and pokes through a thinner skin on bumps.
const POKE: f32 = 0.7;

/// A footprint's own frame: centre `c`, unit axis `u` along the long side (east
/// of north-south), `v` across it, half-free sizes `lu` x `lv` in metres. A
/// local `[a, b]` is the world point `c + a*u + b*v`; a kit model turned by
/// `yaw` (plus `self.yaw`) has its local x along `u` and its local z along `v`.
struct Frame {
    c: Pos,
    u: [f32; 2],
    v: [f32; 2],
    lu: f32,
    lv: f32,
    yaw: f32,
}

impl Frame {
    fn of(b: &Building) -> Frame {
        let r = &b.ring;
        let edge = |i: usize| [r[(i + 1) % r.len()][0] - r[i][0], r[(i + 1) % r.len()][1] - r[i][1]];
        let (e0, e1) = (edge(0), edge(1));
        let len = |e: [f32; 2]| (e[0] * e[0] + e[1] * e[1]).sqrt();
        let (long, short) = if len(e0) >= len(e1) { (e0, e1) } else { (e1, e0) };
        let (ll, sl) = (len(long), len(short));
        let mut u = [long[0] / ll, long[1] / ll];
        if u[0] < 0.0 {
            u = [-u[0], -u[1]];
        }
        let n = r.len() as f32;
        let c = Pos { x: r.iter().map(|p| p[0]).sum::<f32>() / n, y: r.iter().map(|p| p[1]).sum::<f32>() / n };
        Frame { c, u, v: [u[1], -u[0]], lu: ll, lv: sl, yaw: u[1].atan2(u[0]) }
    }

    fn at(&self, [a, b]: [f32; 2]) -> Pos {
        Pos { x: self.c.x + a * self.u[0] + b * self.v[0], y: self.c.y + a * self.u[1] + b * self.v[1] }
    }

    /// A rectangle `w` x `d` (along `u`, `v`) centred at local `at`, draped on
    /// the terrain `lift` metres above it, 4 m posting. Layers stacked on one
    /// spot differ by at least 0.04 m; below ~0.45 the terrain mesh pokes through.
    fn drape(&self, scn: &Scenario, out: &mut Builder, at: [f32; 2], w: f32, d: f32, color: [f32; 3], lift: f32) {
        let nx = ((w / 4.0).ceil() as usize).max(1);
        let ny = ((d / 4.0).ceil() as usize).max(1);
        let pt = |i: usize, j: usize| {
            let p = self.at([at[0] - w * 0.5 + w * i as f32 / nx as f32, at[1] - d * 0.5 + d * j as f32 / ny as f32]);
            [p.x, ground_at(scn, p).max(0.0) + lift + POKE, -p.y]
        };
        for i in 0..nx {
            for j in 0..ny {
                let (a, b, c, d) = (pt(i, j), pt(i + 1, j), pt(i + 1, j + 1), pt(i, j + 1));
                // Face up whichever way the frame is handed.
                let up = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
                if up < 0.0 { out.quad(a, d, c, b, color) } else { out.quad(a, b, c, d, color) }
            }
        }
    }

    /// [`Frame::drape`] of paving. With the photographed masonry, where the
    /// terrain paves the spot already (most of it is road, building or yard
    /// in the cover raster: its setts, `terrain.wesl`) nothing, so square,
    /// streets and lots are one stone and the benches stand on it, not
    /// buried in a slab lifted a metre; elsewhere stone slabs
    /// (`masonry::Part::Stone`) of a weathered tone, laid flat (normals up:
    /// the photograph keeps one frame across the drape's facets).
    fn pave(&self, scn: &Scenario, out: &mut Builder, at: [f32; 2], w: f32, d: f32, color: [f32; 3], lift: f32) {
        if !out.real {
            return self.drape(scn, out, at, w, d, color, lift);
        }
        if self.paved_share(scn, at, w, d) >= 0.5 {
            return;
        }
        out.part = crate::materials::masonry::Part::Stone as u8;
        out.up_normals = true;
        let k = 0.30;
        self.drape(scn, out, at, w, d, [color[0] * k, color[1] * k * 0.98, color[2] * k * 0.96], lift);
        out.up_normals = false;
        out.part = 0;
    }

    /// The share of a `w` x `d` rectangle at local `at` the cover raster
    /// calls paved (road, building, yard), on a 9 x 9 grid.
    fn paved_share(&self, scn: &Scenario, at: [f32; 2], w: f32, d: f32) -> f32 {
        use scenario::CoverClass;
        let Some(cover) = &scn.cover else { return 0.0 };
        let mut n = 0;
        for i in 0..9 {
            for j in 0..9 {
                let p = self.at([at[0] + w * (i as f32 / 8.0 - 0.5), at[1] + d * (j as f32 / 8.0 - 0.5)]);
                n += matches!(cover.at(p), CoverClass::Road | CoverClass::Building | CoverClass::Yard) as u32;
            }
        }
        n as f32 / 81.0
    }

    /// A thin ring of radius `r` and line width `t`, draped as short segments.
    fn ring(&self, scn: &Scenario, out: &mut Builder, at: [f32; 2], r: f32, t: f32, color: [f32; 3], lift: f32) {
        let n = 24;
        for k in 0..n {
            let ang = (k as f32 + 0.5) / n as f32 * 2.0 * PI;
            let p = [at[0] + r * ang.cos(), at[1] + r * ang.sin()];
            let seg = 2.0 * PI * r / n as f32 + 0.2;
            // A short strip turned to the tangent, in a throwaway frame.
            let to_world = |l: [f32; 2]| [l[0] * self.u[0] + l[1] * self.v[0], l[0] * self.u[1] + l[1] * self.v[1]];
            let local = Frame { c: self.at(p), u: to_world([-ang.sin(), ang.cos()]), v: to_world([ang.cos(), ang.sin()]), lu: seg, lv: t, yaw: 0.0 };
            local.drape(scn, out, [0.0, 0.0], seg, t, color, lift);
        }
    }

    /// A kit prop at local `at`, turned `yaw` about the frame.
    fn put(&self, scn: &Scenario, out: &mut Builder, name: &str, at: [f32; 2], scale: f32, yaw: f32, paint: Option<[f32; 3]>) {
        put(scn, out, name, self.at(at), scale, self.yaw + yaw, paint);
    }

    /// A small headstone.
    fn stone(&self, scn: &Scenario, out: &mut Builder, at: [f32; 2], color: [f32; 3]) {
        let ring: Vec<Pos> = [[-0.8, -0.35], [0.8, -0.35], [0.8, 0.35], [-0.8, 0.35]].iter().map(|[a, b]| self.at([at[0] + a, at[1] + b])).collect();
        let ring = if super::signed_area(&ring) < 0.0 { ring.into_iter().rev().collect() } else { ring };
        let g = ground_at(scn, self.at(at)).max(0.0);
        out.prism(&ring, g + 0.3, g + 1.5, color, [color[0] * 0.92, color[1] * 0.92, color[2] * 0.92]);
    }
}

fn bbox(b: &Building) -> (Pos, Pos) {
    let (mut lo, mut hi) = (Pos { x: f32::MAX, y: f32::MAX }, Pos { x: f32::MIN, y: f32::MIN });
    for v in &b.ring {
        lo.x = lo.x.min(v[0]);
        lo.y = lo.y.min(v[1]);
        hi.x = hi.x.max(v[0]);
        hi.y = hi.y.max(v[1]);
    }
    (lo, hi)
}

fn ground_at(scn: &Scenario, p: Pos) -> f32 {
    scn.terrain.height_at(p)
}

/// A kit model at world position `c` (ground height `y`), scaled per axis and
/// turned `yaw` radians about the vertical. `paint` recolours its white parts.
/// With the photographed masonry (`out.real`), each vertex also says what it
/// is (`real_surface`), and a building's windows get frames and glazing bars
/// (`window_frames`).
fn place(out: &mut Builder, name: &str, c: Pos, y: f32, s: [f32; 3], yaw: f32, paint: Option<[f32; 3]>) {
    let m = kit(name);
    let (sn, cs) = yaw.sin_cos();
    let start = out.positions.len() as u32;
    let alpha = out.part_alpha();
    let building = out.real && out.part == crate::materials::masonry::Part::Building as u8;
    let frames = if building { window_frames(m, name) } else { Vec::new() };
    // the gables the bake paints in the roof's colour: the walls' plaster
    let gable = if building { main_wall_colour(m) } else { None };
    let extra = frames.iter().map(|(p, n, col, id)| (p, n, col, Some(*id)));
    for (p, n, col, id) in m.positions.iter().zip(&m.normals).zip(&m.colors).map(|((p, n), c)| (p, n, c, None)).chain(extra) {
        let (x, z) = (p[0] * s[0], p[2] * s[2]);
        let (rx, rz) = (x * cs + z * sn, -x * sn + z * cs);
        out.positions.push([c.x + rx, y + p[1] * s[1], -c.y + rz]);
        // Normals under non-uniform scale: divide by the scale, renormalise.
        let (nx, ny, nz) = (n[0] / s[0], n[1] / s[1], n[2] / s[2]);
        let (rnx, rnz) = (nx * cs + nz * sn, -nx * sn + nz * cs);
        let l = (rnx * rnx + ny * ny + rnz * rnz).sqrt().max(1e-6);
        out.normals.push([rnx / l, ny / l, rnz / l]);
        let white = col[0] > 0.99 && col[1] > 0.99 && col[2] > 0.99;
        let mut c3 = match paint {
            Some(pc) if white => pc,
            _ => *col,
        };
        let mut a = alpha;
        if out.real {
            let id = id.or_else(|| real_surface(*col, building));
            if let Some(id) = id {
                a = crate::materials::surface_alpha(id);
            }
            if id == Some(crate::materials::masonry::surface::GLASS) {
                // the bake's pale blue panes as dark glass, like the houses'
                c3 = [0.05, 0.06, 0.08];
            } else if building && id.is_none() {
                if let Some(g) = gable.filter(|_| roof_colour(*col) && n[1].abs() < 0.2) {
                    c3 = g;
                }
                c3 = lime_and_clay(c3);
            } else if !building && id == Some(crate::materials::masonry::Part::Stone as u8) {
                c3 = [col[0] * 0.36, col[1] * 0.36, col[2] * 0.36];
            }
        }
        out.colors.push([c3[0], c3[1], c3[2], a]);
    }
    out.indices.extend(m.indices.iter().map(|i| start + i));
    let first = start + m.positions.len() as u32;
    out.indices.extend(first..first + frames.len() as u32);
}

/// The bake's terracotta, roofs' and gables' (not a flag's red).
fn roof_colour(c: [f32; 3]) -> bool {
    (c[0] - 0.82).abs() + (c[1] - 0.40).abs() + (c[2] - 0.25).abs() < 0.06
}

/// The colour of most of a kit building's upright plaster: a bake colour
/// that is not glass, wood, roof or dark, by the faces' area.
fn main_wall_colour(m: &Kit) -> Option<[f32; 3]> {
    let mut best: Vec<([f32; 3], f32)> = Vec::new();
    for t in 0..m.indices.len() / 3 {
        let i = m.indices[3 * t] as usize;
        let (c, n) = (m.colors[i], m.normals[i]);
        let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        if n[1].abs() > 0.2 || roof_colour(c) || luma < 0.3 || real_surface(c, true).is_some() {
            continue;
        }
        let p = |v: usize| m.positions[m.indices[3 * t + v] as usize];
        let x = cross3(sub3(p(1), p(0)), sub3(p(2), p(0)));
        let area = 0.5 * (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
        match best.iter_mut().find(|(b, _)| *b == c) {
            Some(e) => e.1 += area,
            None => best.push((c, area)),
        }
    }
    best.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(c, _)| c)
}

/// With the photographed masonry, a kit building's plaster and tiles in the
/// tones of the houses around (`buildings::palette::real`): the bake's
/// colours are sRGB picks stored as they are, and as linear they drew the
/// church nearly white and its roof a pale salmon, twice as bright as any
/// house's. Read as sRGB, and a quarter less saturated.
fn lime_and_clay(c: [f32; 3]) -> [f32; 3] {
    let lin = c.map(|v| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) });
    let l = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
    lin.map(|v| l + (v - l) * 0.75)
}

/// The kit's palette is a handful of flat colours, shared by every model
/// (`scripts/build_town_models.py`): what each is made of, for the
/// photographed look. `None`: the part the vertex is in (plaster and tiles on
/// a building, a prop's own flat colour).
fn real_surface(c: [f32; 3], building: bool) -> Option<u8> {
    use crate::materials::masonry::surface;
    let near = |r: [f32; 3]| (c[0] - r[0]).abs() + (c[1] - r[1]).abs() + (c[2] - r[2]).abs() < 0.04;
    let wood = c[0] > c[1] && c[1] > c[2] && c[0] < 0.6 && c[0] - c[2] > 0.12 && c[0] - c[2] < 0.36;
    if near([0.16, 0.24, 0.33]) {
        Some(surface::GLASS)
    } else if wood {
        Some(surface::WOOD)
    } else if building {
        None
    } else if near([0.35, 0.65, 0.88]) {
        // a fountain's water: dark and glossy, as glass is drawn
        Some(surface::GLASS)
    } else if near([0.8, 0.76, 0.68]) {
        // the kit's stone (a fountain's basin)
        Some(crate::materials::masonry::Part::Stone as u8)
    } else if near([0.1, 0.1, 0.12]) {
        // tyres, bench legs, lamp posts: dark and matte
        Some(surface::RUBBER)
    } else if c[0] > 0.99 && c[1] > 0.99 && c[2] > 0.99 {
        // the body a car is painted on
        Some(surface::PAINT)
    } else {
        None
    }
}

/// A window of a kit building in model space: its glass box `lo`..`hi`, on
/// a facade whose normal is the axis `ax` (0: x, 2: z), facing `sign`.
#[derive(Clone, Copy, Debug)]
struct Pane {
    lo: [f32; 3],
    hi: [f32; 3],
    ax: usize,
    sign: f32,
}

impl Pane {
    /// The axis along the facade.
    fn across(&self) -> usize {
        2 - self.ax
    }
}

/// Frames and a cross of glazing bars on a kit building's windows, in model
/// space, as triangles (position, normal, colour, surface). A window in the
/// bake is a thin box of the glass colour; anything else of that colour (the
/// church's round window) keeps its glass and no frame. The walls the bake
/// left blind (the flanks of church and town hall, 2026-11-06) get windows
/// like the model's own (`blind_wall_panes`): glass, frame, bars, a sill, and
/// on the town hall open shutters on every window.
fn window_frames(m: &Kit, name: &str) -> Vec<([f32; 3], [f32; 3], [f32; 3], u8)> {
    use crate::materials::masonry::{surface, Part};
    let baked = glass_panes(m);
    let added = blind_wall_panes(m, name, &baked);
    let frame = [0.86, 0.83, 0.76];
    let bar_col = [0.70, 0.67, 0.60];
    // Ligurian green louvres, as on the houses (`palette::SHUTTERS[0]`, linear)
    let shutters = matches!(name, "townhall").then_some([0.033, 0.147, 0.064]);
    let mut out = Vec::new();
    for (k, pane) in baked.iter().chain(&added).enumerate() {
        let is_new = k >= baked.len();
        let (ax, ac, sign) = (pane.ax, pane.across(), pane.sign);
        let (lo, hi) = (pane.lo, pane.hi);
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let face = if sign > 0.0 { hi[ax] } else { lo[ax] };
        let mut nrm = [0.0; 3];
        nrm[ax] = sign;
        // a point on the outer face: `u` across the window, `v` up it, `d` out
        let at = |u: f32, v: f32, d: f32| {
            let mut p = [0.0; 3];
            p[ax] = face + sign * d;
            p[ac] = u;
            p[1] = v;
            p
        };
        let w = (ext[ac].min(ext[1]) * 0.12).clamp(0.06, 0.25);
        let quad = |out: &mut Vec<_>, u0: f32, v0: f32, u1: f32, v1: f32, d: f32, col: [f32; 3], id: u8| {
            facing(out, [at(u0, v0, d), at(u1, v0, d), at(u1, v1, d), at(u0, v1, d)], nrm, col, id);
        };
        let fr = Part::Frame as u8;
        let (u0, u1, v0, v1) = (lo[ac], hi[ac], lo[1], hi[1]);
        if is_new {
            // the glass itself, which the bake has only on its own windows
            quad(&mut out, u0, v0, u1, v1, 0.0, [0.16, 0.24, 0.33], surface::GLASS);
        }
        // the frame round it, on the wall just outside the glass
        quad(&mut out, u0 - w, v0 - w, u1 + w, v0, 0.02, frame, fr);
        quad(&mut out, u0 - w, v1, u1 + w, v1 + w, 0.02, frame, fr);
        quad(&mut out, u0 - w, v0, u0, v1, 0.02, frame, fr);
        quad(&mut out, u1, v0, u1 + w, v1, 0.02, frame, fr);
        // glazing bars over the glass
        let (um, vm, b) = ((u0 + u1) * 0.5, v0 + (v1 - v0) * 0.62, w * 0.5);
        quad(&mut out, um - b * 0.5, v0, um + b * 0.5, v1, 0.03, bar_col, fr);
        if v1 - v0 > 1.5 * (u1 - u0) {
            quad(&mut out, u0, vm - b * 0.5, u1, vm + b * 0.5, 0.03, bar_col, fr);
        } else {
            quad(&mut out, u0, (v0 + v1) * 0.5 - b * 0.5, u1, (v0 + v1) * 0.5 + b * 0.5, 0.03, bar_col, fr);
        }
        if v0 > 0.3 {
            // a stone sill, proud of the wall: its top and its front
            let (s0, s1, depth) = (u0 - w * 1.6, u1 + w * 1.6, 0.16);
            let top = v0 - w;
            let up = [0.0, 1.0, 0.0];
            facing(&mut out, [at(s0, top, 0.0), at(s1, top, 0.0), at(s1, top, depth), at(s0, top, depth)], up, frame, fr);
            facing(&mut out, [at(s0, top - 0.14, depth), at(s1, top - 0.14, depth), at(s1, top, depth), at(s0, top, depth)], nrm, frame, fr);
        }
        if let Some(col) = shutters {
            // open, against the wall either side
            let half = (u1 - u0) * 0.5;
            for (a, b) in [(u0 - half * 0.92 - w, u0 - w), (u1 + w, u1 + half * 0.92 + w)] {
                quad(&mut out, a, v0, b, v1, -0.02, col, SHUTTER_SURFACE);
            }
        }
    }
    out
}

/// A quad of corners `q` (in order round it) as two triangles wound to face
/// along `n`.
fn facing(out: &mut Vec<([f32; 3], [f32; 3], [f32; 3], u8)>, q: [[f32; 3]; 4], n: [f32; 3], col: [f32; 3], id: u8) {
    let c = cross3(sub3(q[1], q[0]), sub3(q[2], q[0]));
    let tri = if c[0] * n[0] + c[1] * n[1] + c[2] * n[2] > 0.0 { [q[0], q[1], q[2], q[0], q[2], q[3]] } else { [q[0], q[2], q[1], q[0], q[3], q[2]] };
    for p in tri {
        out.push((p, n, col, id));
    }
}

/// Vertex surface of louvred shutters (`buildings::SHUTTER`).
const SHUTTER_SURFACE: u8 = 11;

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross3(x: [f32; 3], y: [f32; 3]) -> [f32; 3] {
    [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]]
}

/// The bake's own windows: thin boxes of the glass colour.
fn glass_panes(m: &Kit) -> Vec<Pane> {
    use std::collections::HashMap;
    let is_glass = |i: usize| real_surface(m.colors[i], true) == Some(crate::materials::masonry::surface::GLASS);
    // the glass triangles, joined where they share a corner
    let key = |p: [f32; 3]| [(p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64, (p[2] * 1000.0).round() as i64];
    let tris: Vec<usize> = (0..m.indices.len() / 3).filter(|&t| is_glass(m.indices[3 * t] as usize)).collect();
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        p[i] = r;
        r
    }
    let mut by_corner: HashMap<[i64; 3], usize> = HashMap::new();
    for (k, &t) in tris.iter().enumerate() {
        for v in 0..3 {
            let at = key(m.positions[m.indices[3 * t + v] as usize]);
            if let Some(&o) = by_corner.get(&at) {
                let (a, b) = (find(&mut parent, k), find(&mut parent, o));
                parent[a] = b;
            } else {
                by_corner.insert(at, k);
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for k in 0..tris.len() {
        let r = find(&mut parent, k);
        groups.entry(r).or_default().push(tris[k]);
    }
    let mut panes = Vec::new();
    for g in groups.values() {
        let mut corners: Vec<[i64; 3]> = g.iter().flat_map(|&t| (0..3).map(move |v| (t, v))).map(|(t, v)| key(m.positions[m.indices[3 * t + v] as usize])).collect();
        corners.sort();
        corners.dedup();
        if corners.len() != 8 {
            continue; // not a box
        }
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for &t in g {
            for v in 0..3 {
                let p = m.positions[m.indices[3 * t + v] as usize];
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
        }
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        // the thin horizontal axis is the facade's normal
        let (ax, ac) = if ext[0] < ext[2] { (0, 2) } else { (2, 0) };
        if ext[ax] > 0.35 * ext[ac].min(ext[1]) || ext[ac] < 0.2 || ext[1] < 0.2 {
            continue;
        }
        let centre = (lo[ax] + hi[ax]) * 0.5;
        panes.push(Pane { lo, hi, ax, sign: if centre >= 0.0 { 1.0 } else { -1.0 } });
    }
    // in a fixed order: the hash map's is not
    panes.sort_by(|a, b| (a.ax, a.lo[1], a.lo[a.across()]).partial_cmp(&(b.ax, b.lo[1], b.lo[b.across()])).unwrap_or(std::cmp::Ordering::Equal));
    panes
}

/// The kit's landmarks whose blind walls get windows.
const WINDOWED: [&str; 4] = ["church", "townhall", "school", "chapel"];

/// Windows for the walls of `m` the bake left blind: every large, vertical,
/// plaster-coloured face of the model without a window of its own. They are
/// the model's own windows, as to size and storeys: on the face opposite a
/// windowed one at the same places, elsewhere spaced as the model spaces
/// them; a tall face (a bell tower) gets smaller ones up it. A window is kept
/// only where the face is in plain view: a ray from in front of each corner
/// and of its middle must reach that face first (not a tower against it, not
/// the belfry's openings, not the inside of another part).
fn blind_wall_panes(m: &Kit, name: &str, baked: &[Pane]) -> Vec<Pane> {
    use std::collections::HashMap;
    if !WINDOWED.contains(&name) {
        return Vec::new();
    }
    let ntri = m.indices.len() / 3;
    let corner = |t: usize, v: usize| m.positions[m.indices[3 * t + v] as usize];
    let wall_colour = |c: [f32; 3]| {
        real_surface(c, true).is_none() && !roof_colour(c) && 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] > 0.3
    };
    // a vertical face on the model's axes, as (axis, sign, plane in cm)
    let face_of = |t: usize| -> Option<(usize, i32, i64)> {
        let n = m.normals[m.indices[3 * t] as usize];
        let ax = if n[0].abs() > 0.99 { 0 } else if n[2].abs() > 0.99 { 2 } else { return None };
        Some((ax, n[ax].signum() as i32, (corner(t, 0)[ax] * 100.0).round() as i64))
    };
    let mut faces: HashMap<(usize, i32, i64), ([f32; 2], [f32; 2])> = HashMap::new();
    for t in 0..ntri {
        if !wall_colour(m.colors[m.indices[3 * t] as usize]) {
            continue;
        }
        let Some(f) = face_of(t) else { continue };
        let e = faces.entry(f).or_insert(([f32::MAX; 2], [f32::MIN; 2]));
        for v in 0..3 {
            let p = corner(t, v);
            let (u, y) = (p[2 - f.0], p[1]);
            e.0 = [e.0[0].min(u), e.0[1].min(y)];
            e.1 = [e.1[0].max(u), e.1[1].max(y)];
        }
    }
    // the model's window: its size, its storeys and its spacing
    let size = |a: usize| {
        let mut v: Vec<f32> = baked.iter().map(|p| p.hi[a] - p.lo[a]).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v.get(v.len() / 2).copied()
    };
    let width = {
        let mut v: Vec<f32> = baked.iter().map(|p| p.hi[p.across()] - p.lo[p.across()]).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v.get(v.len() / 2).copied().unwrap_or(0.9)
    };
    let height = size(1).unwrap_or(2.2);
    let mut rows: Vec<f32> = baked.iter().map(|p| p.lo[1]).collect();
    rows.sort_by(|a, b| a.partial_cmp(b).unwrap());
    rows.dedup_by(|a, b| (*a - *b).abs() < 0.2);
    let mut pitch = f32::MAX;
    for a in baked {
        for b in baked {
            let d = (a.lo[a.across()] - b.lo[b.across()]).abs();
            if a.ax == b.ax && a.sign == b.sign && (a.lo[1] - b.lo[1]).abs() < 0.2 && d > width {
                pitch = pitch.min(d);
            }
        }
    }
    if pitch == f32::MAX {
        pitch = (width * 3.2).max(3.0);
    }

    // the first thing a ray from `o` along `d` meets: (distance, triangle)
    let hit = |o: [f32; 3], d: [f32; 3]| -> Option<(f32, usize)> {
        let mut best: Option<(f32, usize)> = None;
        for t in 0..ntri {
            let (a, b, c) = (corner(t, 0), corner(t, 1), corner(t, 2));
            let (e1, e2) = (sub3(b, a), sub3(c, a));
            let pv = cross3(d, e2);
            let det = e1[0] * pv[0] + e1[1] * pv[1] + e1[2] * pv[2];
            if det.abs() < 1e-9 {
                continue;
            }
            let tv = sub3(o, a);
            let u = (tv[0] * pv[0] + tv[1] * pv[1] + tv[2] * pv[2]) / det;
            if !(-1e-4..=1.0001).contains(&u) {
                continue;
            }
            let qv = cross3(tv, e1);
            let v = (d[0] * qv[0] + d[1] * qv[1] + d[2] * qv[2]) / det;
            if v < -1e-4 || u + v > 1.0001 {
                continue;
            }
            let s = (e2[0] * qv[0] + e2[1] * qv[1] + e2[2] * qv[2]) / det;
            if s > 1e-4 && best.is_none_or(|(bs, _)| s < bs) {
                best = Some((s, t));
            }
        }
        best
    };
    const REACH: f32 = 3.0;
    let in_view = |key: (usize, i32, i64), u: f32, y: f32| {
        let (ax, sign) = (key.0, key.1 as f32);
        let mut o = [0.0; 3];
        o[ax] = key.2 as f32 / 100.0 + sign * REACH;
        o[2 - ax] = u;
        o[1] = y;
        let mut d = [0.0; 3];
        d[ax] = -sign;
        // not from inside another part (the tower, the attic under the
        // nave's roof): looking out from there, the first face is seen
        // from behind
        let out_dir = d.map(|v| -v);
        if let Some((_, t)) = hit(o, out_dir) {
            let n = m.normals[m.indices[3 * t] as usize];
            if n[0] * out_dir[0] + n[1] * out_dir[1] + n[2] * out_dir[2] > 0.0 {
                return false;
            }
        }
        match hit(o, d) {
            Some((s, t)) => (s - REACH).abs() < 0.03 && face_of(t) == Some(key) && wall_colour(m.colors[m.indices[3 * t] as usize]),
            None => false,
        }
    };

    let mut keys: Vec<_> = faces.keys().copied().collect();
    keys.sort();
    let mut out = Vec::new();
    for key in keys {
        let ([u0, y0], [u1, y1]) = faces[&key];
        let (ax, sign, plane) = (key.0, key.1 as f32, key.2 as f32 / 100.0);
        let ac = 2 - ax;
        if u1 - u0 < 3.0 || y1 - y0 < 3.0 {
            continue;
        }
        // a face with windows of its own keeps them
        let on_face = |p: &Pane| p.ax == ax && p.sign == sign && (if sign > 0.0 { p.lo[ax] } else { p.hi[ax] } - plane).abs() < 0.3;
        if baked.iter().any(|p| on_face(p) && p.hi[ac] > u0 && p.lo[ac] < u1) {
            continue;
        }
        // where the windows go along the face: as on the face opposite, or
        // spaced as the model spaces them, centred
        let opposite: Vec<f32> = baked.iter().filter(|p| p.ax == ax && p.sign == -sign && (p.lo[1] - rows.first().copied().unwrap_or(0.0)).abs() < 0.2).map(|p| (p.lo[ac] + p.hi[ac]) * 0.5).filter(|c| *c - width > u0 && *c + width < u1).collect();
        let columns: Vec<f32> = if opposite.len() >= 2 {
            opposite
        } else {
            let margin = width.max(1.0);
            let n = (((u1 - u0 - 2.0 * margin) / pitch).floor() as i32 + 1).max(1);
            let mid = (u0 + u1) * 0.5;
            (0..n).map(|i| mid + (i as f32 - (n - 1) as f32 * 0.5) * pitch).collect()
        };
        // the storeys: the model's rows that fit, and up a tall face smaller
        // windows every few metres
        let mut levels: Vec<(f32, f32, f32)> = rows.iter().filter(|&&r| r > y0 + 0.3 && r + height < y1 - 0.6).map(|&r| (r, width, height)).collect();
        if levels.is_empty() && rows.is_empty() {
            let r = y0 + (y1 - y0) * 0.3;
            if r + height < y1 - 0.6 {
                levels.push((r, width, height));
            }
        }
        if let Some(&(top, ..)) = levels.last() {
            let (w2, h2) = (width * 0.6, height * 0.55);
            let step = (height * 2.2).max(4.5);
            let mut r = top + height + step - h2;
            while rows.len() == 1 && r + h2 < y1 - 1.0 {
                levels.push((r, w2, h2));
                r += step;
            }
        }
        for &(r, w, h) in &levels {
            for &c in &columns {
                let (a, b) = (c - w * 0.5, c + w * 0.5);
                if a < u0 + 0.4 || b > u1 - 0.4 {
                    continue;
                }
                let m_ = 0.25;
                let probes = [(c, r + h * 0.5), (a - m_, r - m_), (b + m_, r - m_), (a - m_, r + h + m_), (b + m_, r + h + m_)];
                if !probes.iter().all(|&(u, y)| in_view(key, u, y)) {
                    continue;
                }
                let (mut lo, mut hi) = ([0.0; 3], [0.0; 3]);
                // a box 10 cm proud of the wall, as the bake's are
                let (near, far) = (plane - sign * 0.02, plane + sign * 0.10);
                lo[ax] = near.min(far);
                hi[ax] = near.max(far);
                lo[ac] = a;
                hi[ac] = b;
                lo[1] = r;
                hi[1] = r + h;
                out.push(Pane { lo, hi, ax, sign });
            }
        }
    }
    out
}

/// A kit prop at `p` on the ground, uniformly scaled.
fn put(scn: &Scenario, out: &mut Builder, name: &str, p: Pos, scale: f32, yaw: f32, paint: Option<[f32; 3]>) {
    // The kit's toy trees would be cartoons among the scanned plants.
    if matches!(name, "street_tree" | "cypress") && KIT_TREES.with(|t| t.borrow_mut().as_mut().map(|v| v.push(KitTree { cypress: name == "cypress", at: p, scale, yaw })).is_some()) {
        return;
    }
    if out.real && matches!(name, "street_tree" | "cypress") {
        return;
    }
    let scale = scale * if matches!(name, "street_tree" | "cypress") {
        crate::visual_scale::TREE
    } else {
        1.0
    };
    let y = ground_at(scn, p).max(0.0);
    place(out, name, p, y - 0.1, [scale; 3], yaw, paint);
}

/// A rectangle draped on the terrain, `lift` metres above it, ~8 m posting.
fn drape(scn: &Scenario, out: &mut Builder, c: Pos, w: f32, d: f32, color: [f32; 3], lift: f32) {
    let color = if out.real { muted(color) } else { color };
    let nx = ((w / 8.0).ceil() as usize).max(1);
    let ny = ((d / 8.0).ceil() as usize).max(1);
    let at = |i: usize, j: usize| {
        let p = Pos { x: c.x - w * 0.5 + w * i as f32 / nx as f32, y: c.y - d * 0.5 + d * j as f32 / ny as f32 };
        [p.x, ground_at(scn, p).max(0.0) + lift, -p.y]
    };
    for i in 0..nx {
        for j in 0..ny {
            // Counter-clockwise seen from above (Bevy -Z is north), so the face
            // points up (finding 11).
            out.quad(at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1), color);
        }
    }
}

/// With the photographed masonry: a flat colour of the kit's ground (the
/// assembly area's, a campsite's) less saturated and less bright, as paint
/// and grass look in the photographs around them.
fn muted(c: [f32; 3]) -> [f32; 3] {
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    c.map(|v| (l + (v - l) * 0.6) * 0.5)
}

/// A low wall from `a` to `b`, following the ground.
fn wall(scn: &Scenario, out: &mut Builder, a: Pos, b: Pos, h: f32, t: f32, color: [f32; 3]) {
    let len = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
    let n = ((len / 6.0).ceil() as usize).max(1);
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    let (px, py) = (-uy * t * 0.5, ux * t * 0.5);
    for k in 0..n {
        let p0 = Pos { x: a.x + ux * len * k as f32 / n as f32, y: a.y + uy * len * k as f32 / n as f32 };
        let p1 = Pos { x: a.x + ux * len * (k + 1) as f32 / n as f32, y: a.y + uy * len * (k + 1) as f32 / n as f32 };
        let ring = [
            Pos { x: p0.x - px, y: p0.y - py },
            Pos { x: p1.x - px, y: p1.y - py },
            Pos { x: p1.x + px, y: p1.y + py },
            Pos { x: p0.x + px, y: p0.y + py },
        ];
        let g = ground_at(scn, p0).min(ground_at(scn, p1)).max(0.0);
        // Wind CCW for `prism`, whichever way the wall runs.
        let ring: Vec<Pos> = if super::signed_area(&ring) < 0.0 { ring.iter().rev().copied().collect() } else { ring.to_vec() };
        out.prism(&ring, g - 0.5, g + h, color, [color[0] * 0.9, color[1] * 0.9, color[2] * 0.9]);
    }
}

/// A garden tree behind about half the houses, and a hedge-side bush: the
/// greenery between houses that a town is drawn with.
pub(super) fn garden(scn: &Scenario, b: &Building, out: &mut Builder) {
    if !matches!(b.kind.as_deref(), Some("house" | "villa" | "terrace")) {
        return;
    }
    let (min, max) = bbox(b);
    let c = Pos { x: (min.x + max.x) * 0.5, y: (min.y + max.y) * 0.5 };
    let h = hash01(b.id as u64, 0xA1);
    if h < 0.55 {
        let side = if hash01(b.id as u64, 0xA2) < 0.5 { -1.0 } else { 1.0 };
        let p = Pos { x: c.x + side * (max.x - min.x) * 0.95, y: c.y + (hash01(b.id as u64, 0xA3) - 0.5) * 8.0 };
        put(scn, out, if h < 0.15 { "cypress" } else { "street_tree" }, p, 1.3 + h, h * 6.3, None);
    }
}

/// A toy tree of the kit (`street_tree` or `cypress`): where it stands, its
/// kit scale and yaw. With the scanned plants the toys are not drawn and
/// the vegetation puts a scanned tree in their place (`vegetation::scatter`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct KitTree {
    pub cypress: bool,
    pub at: Pos,
    pub scale: f32,
    pub yaw: f32,
}

thread_local! {
    /// While [`kit_trees`] runs: the toy trees `put` is asked for.
    static KIT_TREES: std::cell::RefCell<Option<Vec<KitTree>>> = const { std::cell::RefCell::new(None) };
}

/// Every toy tree the kit would draw in the scenario's open spaces and
/// gardens, in the same places (same conditions as `buildings::emit_building`).
pub(crate) fn kit_trees(scn: &Scenario) -> Vec<KitTree> {
    KIT_TREES.with(|t| *t.borrow_mut() = Some(Vec::new()));
    let mut scratch = Builder::default();
    for b in &scn.vectors.buildings {
        if b.ring.len() < 3 || b.kind.as_deref().is_some_and(|k| MODELLED.contains(&k)) {
            continue;
        }
        if !emit(scn, b, &mut scratch) && b.area() >= 6.0 {
            garden(scn, b, &mut scratch);
        }
        scratch = Builder::default();
    }
    KIT_TREES.with(|t| t.borrow_mut().take()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every blind flank of church and town hall gets windows, none on a
    /// face that has its own, and none inside another part (the tower).
    #[test]
    fn blind_flanks_get_windows() {
        for name in ["church", "townhall"] {
            let m = kit(name);
            let baked = glass_panes(m);
            let added = blind_wall_panes(m, name, &baked);
            let sides = |ps: &[Pane]| {
                let mut s: Vec<(usize, i32)> = ps.iter().map(|p| (p.ax, p.sign as i32)).collect();
                s.sort();
                s.dedup();
                s
            };
            let mut all = sides(&baked);
            all.extend(sides(&added));
            all.sort();
            all.dedup();
            assert_eq!(all.len(), 4, "{name}: windows on {all:?}");
            for a in &added {
                for b in &baked {
                    let overlap = (0..3).all(|k| a.lo[k] < b.hi[k] && b.lo[k] < a.hi[k]);
                    assert!(!overlap, "{name}: {a:?} over the bake's {b:?}");
                }
            }
        }
        // the church's tower stands against the nave's east end: no window
        // of the nave behind it
        let m = kit("church");
        let added = blind_wall_panes(m, "church", &glass_panes(m));
        assert!(!added.iter().any(|p| p.ax == 0 && (p.lo[0] - 8.0).abs() < 0.3 && p.lo[2] < 3.0 && p.hi[2] > -3.0 && p.hi[1] < 11.0));
    }
}
