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
        f.drape(scn, out, [0.0, 0.0], f.lu * TOY_SCALE + 6.0, f.lv * TOY_SCALE + 6.0, [0.80, 0.77, 0.70], 0.0);
        // with the photographed masonry: plaster walls and tiled slopes
        out.part = crate::materials::masonry::Part::Building as u8;
        place(out, m, f.c, low - 0.4, [sx, sy, sz], f.yaw, None);
        out.part = 0;
        return true;
    }
    let h = |salt: u64| hash01(b.id as u64, salt);
    // Open spaces are laid out in the footprint's own frame: the OSM rings are
    // rotated rectangles, and their axis-aligned box is much larger.
    let f = Frame::of(b);
    match kind {
        "plaza" => {
            f.drape(scn, out, [0.0, 0.0], f.lu, f.lv, [0.80, 0.76, 0.68], 0.5);
            f.drape(scn, out, [0.0, 0.0], f.lu - 6.0, f.lv - 6.0, [0.88, 0.84, 0.76], 0.56);
            f.drape(scn, out, [0.0, 0.0], f.lu * 0.5, f.lv * 0.5, [0.80, 0.74, 0.64], 0.62);
            f.put(scn, out, "fountain", [0.0, 0.0], 1.6, 0.0, None);
            for (sa, sb) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                f.put(scn, out, "street_tree", [sa * f.lu * 0.4, sb * f.lv * 0.38], 1.6, h(7) * 6.0, None);
                f.put(scn, out, "bench", [sa * f.lu * 0.2, sb * f.lv * 0.3], 1.8, if sb > 0.0 { 0.0 } else { PI }, None);
                f.put(scn, out, "streetlight", [sa * f.lu * 0.46, sb * f.lv * 0.12], 1.4, 0.0, None);
            }
        }
        "pitch" => {
            let grass = [[0.34, 0.64, 0.31], [0.38, 0.70, 0.35]];
            let line = [0.97, 0.97, 0.95];
            // Paved surround, then the turf in mown bands across the length.
            f.drape(scn, out, [0.0, 0.0], f.lu + 8.0, f.lv + 8.0, [0.62, 0.58, 0.52], 0.5);
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
            f.drape(scn, out, [0.0, 0.0], lu + 2.0, lv + 2.0, [0.60, 0.60, 0.58], 0.5);
            f.drape(scn, out, [0.0, 0.0], lu, lv, [0.36, 0.37, 0.39], 0.56);
            let bay_width = 3.25 * crate::visual_scale::CAR;
            let bays = ((lu / bay_width) as i32).max(1);
            let step = lu / bays as f32;
            for row in [-1.0f32, 1.0] {
                for i in 0..=bays {
                    let a = -lu * 0.5 + i as f32 * step;
                    f.drape(scn, out, [a, row * lv * 0.27], 0.5, lv * 0.42, [0.95, 0.95, 0.92], 0.62);
                }
                for i in 0..bays {
                    let a = -lu * 0.5 + (i as f32 + 0.5) * step;
                    if hash01(b.id as u64 + i as u64 * 31 + (row > 0.0) as u64 * 7, 0x51) < 0.7 {
                        let car = ["car_hatch", "car_sedan", "car_suv", "car_van"][(hash01(b.id as u64 + i as u64, 0x52) * 4.0) as usize % 4];
                        let paint = PAINT[(hash01(b.id as u64 + i as u64 * 3 + row as u64, 0x53) * 7.0) as usize % 7];
                        f.put(scn, out, car, [a, row * lv * 0.27], crate::visual_scale::CAR, if row > 0.0 { 0.0 } else { PI }, Some(paint));
                    }
                }
            }
            // Kerb along the aisle and a couple of lamps.
            f.drape(scn, out, [0.0, 0.0], lu, 0.8, [0.90, 0.88, 0.40], 0.62);
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
            f.drape(scn, out, [0.0, 0.0], lu + 2.0, lv + 2.0, [0.47, 0.70, 0.44], 0.5);
            f.drape(scn, out, [0.0, 0.0], lu, lv, [0.78, 0.86, 0.70], 0.56);
            f.drape(scn, out, [0.0, 0.0], lu - 6.0, lv - 6.0, [0.84, 0.89, 0.77], 0.6);
            // A painted green border, like the sign.
            for s in [-1.0f32, 1.0] {
                f.drape(scn, out, [0.0, s * (lv * 0.5 - 1.5)], lu - 3.0, 1.0, [0.22, 0.55, 0.30], 0.64);
                f.drape(scn, out, [s * (lu * 0.5 - 1.5), 0.0], 1.0, lv - 3.0, [0.22, 0.55, 0.30], 0.64);
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
                place(out, kit("pier"), Pos { x: c.x, y: start - t }, 0.2, [4.0, 1.0, 4.0], 0.0, None);
                t += 4.0;
            }
            for i in 0..6 {
                for s in [-1.0f32, 1.0] {
                    let p = Pos { x: c.x + s * 9.0, y: start - 10.0 - i as f32 * len / 6.5 };
                    if ground_at(scn, p) < -0.5 {
                        let m = if (i + (s > 0.0) as i32) % 3 == 0 { "sailboat" } else { "boat" };
                        place(out, kit(m), p, 0.0, [1.5, 1.5, 1.5], s * std::f32::consts::FRAC_PI_2, None);
                    }
                }
            }
        }
        "cemetery" => {
            f.drape(scn, out, [0.0, 0.0], f.lu, f.lv, [0.50, 0.68, 0.42], 0.5);
            // Gravel path down the middle, graves in rows either side.
            f.drape(scn, out, [0.0, 0.0], f.lu - 4.0, 2.4, [0.86, 0.83, 0.74], 0.56);
            let wall_c = [0.90, 0.88, 0.82];
            let ring = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]].map(|[a, b]| f.at([a * f.lu, b * f.lv]));
            for i in 0..4 {
                wall(scn, out, ring[i], ring[(i + 1) % 4], 2.6, 0.8, wall_c);
            }
            for s in [-1.0f32, 1.0] {
                for i in 0..5 {
                    f.put(scn, out, "cypress", [-f.lu * 0.4 + i as f32 * f.lu * 0.2, s * f.lv * 0.4], 1.1, 0.0, None);
                }
                for i in 0..6 {
                    for j in 0..2 {
                        let at = [-f.lu * 0.4 + 1.5 + i as f32 * f.lu * 0.11, s * (f.lv * 0.12 + j as f32 * f.lv * 0.12 + 2.0)];
                        f.stone(scn, out, at, [0.62, 0.62, 0.60]);
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
fn place(out: &mut Builder, m: &Kit, c: Pos, y: f32, s: [f32; 3], yaw: f32, paint: Option<[f32; 3]>) {
    let (sn, cs) = yaw.sin_cos();
    let start = out.positions.len() as u32;
    let alpha = out.part_alpha();
    let building = out.real && out.part == crate::materials::masonry::Part::Building as u8;
    let frames = if building { window_frames(m) } else { Vec::new() };
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
            }
        }
        out.colors.push([c3[0], c3[1], c3[2], a]);
    }
    out.indices.extend(m.indices.iter().map(|i| start + i));
    let first = start + m.positions.len() as u32;
    out.indices.extend(first..first + frames.len() as u32);
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

/// Frames and a cross of glazing bars on a kit building's windows, in model
/// space, as triangles (position, normal, colour, surface). A window in the
/// bake is a thin box of the glass colour; anything else of that colour (the
/// church's round window) keeps its glass and no frame.
fn window_frames(m: &Kit) -> Vec<([f32; 3], [f32; 3], [f32; 3], u8)> {
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
    let frame = [0.86, 0.83, 0.76];
    let bar_col = [0.70, 0.67, 0.60];
    let mut out = Vec::new();
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
        let sign = if centre >= 0.0 { 1.0 } else { -1.0 };
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
        let mut quad = |u0: f32, v0: f32, u1: f32, v1: f32, d: f32, col: [f32; 3]| {
            let (a, b, c, e) = (at(u0, v0, d), at(u1, v0, d), at(u1, v1, d), at(u0, v1, d));
            // wound to face out along `nrm`, whichever way the axes run
            let cross = {
                let (x, y) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
                [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]]
            };
            let tri = if cross[ax] * sign > 0.0 { [a, b, c, a, c, e] } else { [a, c, b, a, e, c] };
            for p in tri {
                out.push((p, nrm, col, crate::materials::masonry::Part::Frame as u8));
            }
        };
        let (u0, u1, v0, v1) = (lo[ac], hi[ac], lo[1], hi[1]);
        // the frame round it, on the wall just outside the glass
        quad(u0 - w, v0 - w, u1 + w, v0, 0.02, frame);
        quad(u0 - w, v1, u1 + w, v1 + w, 0.02, frame);
        quad(u0 - w, v0, u0, v1, 0.02, frame);
        quad(u1, v0, u1 + w, v1, 0.02, frame);
        // glazing bars over the glass
        let (um, vm, b) = ((u0 + u1) * 0.5, v0 + (v1 - v0) * 0.62, w * 0.5);
        quad(um - b * 0.5, v0, um + b * 0.5, v1, 0.03, bar_col);
        if v1 - v0 > 1.5 * (u1 - u0) {
            quad(u0, vm - b * 0.5, u1, vm + b * 0.5, 0.03, bar_col);
        } else {
            quad(u0, (v0 + v1) * 0.5 - b * 0.5, u1, (v0 + v1) * 0.5 + b * 0.5, 0.03, bar_col);
        }
    }
    out
}

/// A kit prop at `p` on the ground, uniformly scaled.
fn put(scn: &Scenario, out: &mut Builder, name: &str, p: Pos, scale: f32, yaw: f32, paint: Option<[f32; 3]>) {
    // The kit's toy trees would be cartoons among the scanned plants.
    if out.real && matches!(name, "street_tree" | "cypress") {
        return;
    }
    let scale = scale * if matches!(name, "street_tree" | "cypress") {
        crate::visual_scale::TREE
    } else {
        1.0
    };
    let y = ground_at(scn, p).max(0.0);
    place(out, kit(name), p, y - 0.1, [scale; 3], yaw, paint);
}

/// A rectangle draped on the terrain, `lift` metres above it, ~8 m posting.
fn drape(scn: &Scenario, out: &mut Builder, c: Pos, w: f32, d: f32, color: [f32; 3], lift: f32) {
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
