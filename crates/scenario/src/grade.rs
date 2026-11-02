//! Road grading: the ground under every road is a flat bench, its long
//! profile smoothed, with cut and fill slopes back to the natural ground.
//!
//! The published render DEM knows nothing of the roads: a 9.6 m carriageway
//! painted on a 30° hillside leaned across with it. This is the one rule that
//! levels it, applied as a function of position over the raw heightfield, so
//! every consumer of [`crate::Terrain::height_at`] — the terrain mesh, the
//! engines, the houses, the plants, the agents' slopes — sees the same ground.
//!
//! The rule:
//! - each road is resampled every ≤ 5 m, and its centreline height is the raw
//!   ground smoothed *along the network* (a diffusion over the graph of
//!   resampled points, so two roads meeting at a junction meet at one height);
//! - across the road the ground is level out to the road's half-width plus a
//!   shoulder ([`flat_half_width`]);
//! - beyond, it returns to the raw ground over a width that grows with the cut
//!   or fill (slopes of about 37°, up to [`MAX_BLEND_M`]);
//! - where several roads are near, the nearest dominates.
//!
//! `ROCCA_STRADE_PIANE=0` switches it off (the raw DEM, for comparisons).

use std::collections::HashMap;

use crate::Road;

/// Half-widths of the drawn carriageway, metres, by road role (the game's
/// road mask and vegetation clearings use these too).
pub const DRIVABLE_HALF_W: f32 = 4.8;
pub const TRACK_HALF_W: f32 = 1.6;

/// Longest resampling step along a road, metres.
const STEP_M: f32 = 5.0;
/// Width over which the ground goes back to the raw DEM, at most, metres.
pub const MAX_BLEND_M: f32 = 30.0;
/// Least blend width, and the run per metre of cut or fill (2: ~37° at the
/// steepest point of the smoothstep, which is 1.5 times the mean slope).
const MIN_BLEND_M: f32 = 3.0;
const RUN_PER_RISE: f32 = 2.0;
/// Diffusion steps of the long profile: a lazy random walk spreads by
/// `sqrt(k / 2) · step`, so 50 is about 25 m on a highway, 12 about 12 m on a
/// track.
const SMOOTH_DRIVABLE: usize = 50;
const SMOOTH_TRACK: usize = 12;
/// A DEM sample's diagonal and then some (the render DEM is 5 m).
const SAMPLE_MARGIN_M: f32 = 7.5;
/// How fast a farther road's say falls off past its own bench, metres.
const HANDOVER_M: f32 = 3.0;
/// Spatial hash cell, metres.
const CELL_M: f32 = 20.0;

/// Half-width of the level ground: the carriageway plus a shoulder.
pub fn flat_half_width(drivable: bool) -> f32 {
    if drivable { DRIVABLE_HALF_W + 1.7 } else { TRACK_HALF_W + 0.8 }
}

#[derive(Clone, Copy, Debug)]
pub struct GradeSeg {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub za: f32,
    pub zb: f32,
    /// Half-width of the level ground.
    pub flat: f32,
    pub drivable: bool,
}

impl GradeSeg {
    /// Distance from `p` to the centreline, and the road's height there.
    #[inline]
    pub fn nearest(&self, p: [f32; 2]) -> (f32, f32) {
        let (dx, dy) = (self.b[0] - self.a[0], self.b[1] - self.a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 < 1e-6 { 0.0 } else { (((p[0] - self.a[0]) * dx + (p[1] - self.a[1]) * dy) / len2).clamp(0.0, 1.0) };
        let (cx, cy) = (self.a[0] + dx * t - p[0], self.a[1] + dy * t - p[1]);
        ((cx * cx + cy * cy).sqrt(), self.za + (self.zb - self.za) * t)
    }
}

pub struct RoadGrade {
    pub segs: Vec<GradeSeg>,
    nx: usize,
    ny: usize,
    /// CSR: segments within reach of each cell.
    start: Vec<u32>,
    items: Vec<u32>,
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Whether grading is on (`ROCCA_STRADE_PIANE=0` turns it off).
pub fn enabled() -> bool {
    std::env::var("ROCCA_STRADE_PIANE").map_or(true, |v| v != "0")
}

impl RoadGrade {
    /// `raw` is the ungraded ground (bilinear on the DEM).
    pub fn build(roads: &[Road], width_m: f32, height_m: f32, raw: impl Fn([f32; 2]) -> f32) -> RoadGrade {
        // graph of resampled points; original vertices welded by position
        let mut index: HashMap<(i64, i64), u32> = HashMap::new();
        let mut pos: Vec<[f32; 2]> = Vec::new();
        let mut drivable_node: Vec<bool> = Vec::new();
        let mut edges: Vec<(u32, u32, bool)> = Vec::new();
        let key = |p: [f32; 2]| ((p[0] / 0.05).round() as i64, (p[1] / 0.05).round() as i64);
        for road in roads {
            let drivable = road.drivable;
            if !(drivable || road.track) || road.line.len() < 2 {
                continue;
            }
            let mut node = |p: [f32; 2], keyed: bool, pos: &mut Vec<[f32; 2]>, dn: &mut Vec<bool>| -> u32 {
                let id = if keyed {
                    *index.entry(key(p)).or_insert_with(|| {
                        pos.push(p);
                        dn.push(false);
                        (pos.len() - 1) as u32
                    })
                } else {
                    pos.push(p);
                    dn.push(false);
                    (pos.len() - 1) as u32
                };
                dn[id as usize] |= drivable;
                id
            };
            let mut prev = node(road.line[0], true, &mut pos, &mut drivable_node);
            for w in road.line.windows(2) {
                let (a, b) = (w[0], w[1]);
                let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                let n = (len / STEP_M).ceil().max(1.0) as usize;
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                    let id = node(p, k == n, &mut pos, &mut drivable_node);
                    if id != prev {
                        edges.push((prev, id, drivable));
                    }
                    prev = id;
                }
            }
        }

        // long profile: diffuse the raw heights over the network, highways
        // first, then the tracks with the highway nodes they touch held
        let raw0: Vec<f32> = pos.iter().map(|&p| raw(p)).collect();
        let mut z = raw0.clone();
        let diffuse = |z: &mut Vec<f32>, which: bool, steps: usize, held: &dyn Fn(usize) -> bool| {
            let mut nbr: Vec<Vec<u32>> = vec![Vec::new(); z.len()];
            for &(a, b, d) in &edges {
                if d == which {
                    nbr[a as usize].push(b);
                    nbr[b as usize].push(a);
                }
            }
            let mut next = z.clone();
            for _ in 0..steps {
                for i in 0..z.len() {
                    if nbr[i].is_empty() || held(i) {
                        continue;
                    }
                    let mean = nbr[i].iter().map(|&j| z[j as usize]).sum::<f32>() / nbr[i].len() as f32;
                    next[i] = 0.5 * (z[i] + mean);
                }
                std::mem::swap(z, &mut next);
                next.copy_from_slice(z);
            }
        };
        diffuse(&mut z, true, SMOOTH_DRIVABLE, &|_| false);
        diffuse(&mut z, false, SMOOTH_TRACK, &|i| drivable_node[i]);

        let segs: Vec<GradeSeg> = edges
            .iter()
            .map(|&(a, b, d)| GradeSeg {
                a: pos[a as usize],
                b: pos[b as usize],
                za: z[a as usize],
                zb: z[b as usize],
                flat: flat_half_width(d),
                drivable: d,
            })
            .collect();

        // each cell lists the segments within reach of any point in it
        let (nx, ny) = ((width_m / CELL_M).ceil() as usize + 1, (height_m / CELL_M).ceil() as usize + 1);
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); nx * ny];
        for (k, s) in segs.iter().enumerate() {
            let reach = s.flat + MAX_BLEND_M + SAMPLE_MARGIN_M;
            let cx = |v: f32| ((v / CELL_M).floor().max(0.0) as usize).min(nx - 1);
            let cy = |v: f32| ((v / CELL_M).floor().max(0.0) as usize).min(ny - 1);
            for j in cy(s.a[1].min(s.b[1]) - reach)..=cy(s.a[1].max(s.b[1]) + reach) {
                for i in cx(s.a[0].min(s.b[0]) - reach)..=cx(s.a[0].max(s.b[0]) + reach) {
                    // distance from the segment to the cell's box, bounded by its centre's
                    let c = [(i as f32 + 0.5) * CELL_M, (j as f32 + 0.5) * CELL_M];
                    // plus a DEM sample's diagonal: a point whose bilinear
                    // corners were graded must take the exact path too
                    if s.nearest(c).0 <= reach + CELL_M * std::f32::consts::FRAC_1_SQRT_2 + SAMPLE_MARGIN_M {
                        lists[j * nx + i].push(k as u32);
                    }
                }
            }
        }
        let mut start = Vec::with_capacity(nx * ny + 1);
        let mut items = Vec::new();
        for l in &lists {
            start.push(items.len() as u32);
            items.extend_from_slice(l);
        }
        start.push(items.len() as u32);
        RoadGrade { segs, nx, ny, start, items }
    }

    /// Segments that may affect `p`.
    #[inline]
    pub fn near(&self, p: [f32; 2]) -> &[u32] {
        if !(p[0] >= 0.0 && p[1] >= 0.0) {
            return &[];
        }
        let (i, j) = ((p[0] / CELL_M) as usize, (p[1] / CELL_M) as usize);
        if i >= self.nx || j >= self.ny {
            return &[];
        }
        let k = j * self.nx + i;
        &self.items[self.start[k] as usize..self.start[k + 1] as usize]
    }

    /// The graded ground at `p`, over the raw ground `z_raw` there.
    #[inline]
    pub fn apply(&self, p: [f32; 2], z_raw: f32) -> f32 {
        self.apply_with(p, z_raw, |_, _, _, _, _| ())
    }

    /// [`Self::apply`], reporting each road's say: `(segment, distance past
    /// its bench, its height, reach weight, weight)`.
    pub fn apply_with(&self, p: [f32; 2], z_raw: f32, mut each: impl FnMut(u32, f32, f32, f32, f32)) -> f32 {
        // the roads in reach: (segment, distance past its bench, height, reach)
        const MAX: usize = 96;
        let mut hit = [(0u32, 0.0f32, 0.0f32, 0.0f32); MAX];
        let mut n = 0;
        for &k in self.near(p) {
            let s = &self.segs[k as usize];
            let (d, zs) = s.nearest(p);
            let out = d - s.flat;
            if out >= MAX_BLEND_M {
                continue;
            }
            let wt = ((z_raw - zs).abs() * RUN_PER_RISE + MIN_BLEND_M).min(MAX_BLEND_M);
            let a = 1.0 - smooth(out / wt);
            if a > 0.0 && n < MAX {
                hit[n] = (k, out, zs, a);
                n += 1;
            }
        }
        let (mut wsum, mut zsum, mut amax) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..n {
            let (k, out, zs, a) = hit[i];
            // A road at another height, nearer its own bench, holds its ground:
            // the cut or fill of a hairpin's lower leg must not reach over the
            // upper one. Roads at the same height (a junction, the next
            // segment) do not hold each other back.
            let mut a = a;
            for (j, &(_, out_j, zs_j, a_j)) in hit[..n].iter().enumerate() {
                if j != i {
                    // (softly: which road is nearer swaps over a metre)
                    let nearer = smooth((out - out_j) * 0.5 + 0.5);
                    a *= 1.0 - a_j * nearer * smooth(((zs - zs_j).abs() - 0.5) / 2.0);
                }
            }
            if a <= 0.0 {
                continue;
            }
            // the nearest road dominates (exponential, not a power: two legs
            // of a hairpin hand over across a bank, not a step)
            let w = a * a * (-out.max(0.0) / HANDOVER_M).exp();
            each(k, out, zs, a, w);
            wsum += w;
            zsum += w * zs;
            amax = amax.max(a);
        }
        if wsum <= 0.0 {
            return z_raw;
        }
        z_raw + (zsum / wsum - z_raw) * amax
    }

    /// How much of `p` is road bench (1 on the level ground, 0 on the raw).
    pub fn bench(&self, p: [f32; 2]) -> f32 {
        self.near(p)
            .iter()
            .map(|&k| {
                let s = &self.segs[k as usize];
                1.0 - smooth((s.nearest(p).0 - s.flat) / MIN_BLEND_M)
            })
            .fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Pos, Scenario};

    fn scenario() -> Scenario {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        Scenario::load_by_id(&dir, crate::ID).unwrap()
    }

    /// Across the carriageway the ground is level, away from junctions and
    /// other roads; far from any road it is the published DEM.
    #[test]
    fn the_bench_is_level_and_the_rest_untouched() {
        let scn = scenario();
        let t = &scn.terrain;
        let g = t.grade.as_ref().expect("grading on by default");
        let mut tilts: Vec<f32> = Vec::new();
        for s in &g.segs {
            let (dx, dy) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.5 {
                continue;
            }
            let m = [(s.a[0] + s.b[0]) * 0.5, (s.a[1] + s.b[1]) * 0.5];
            let w = s.flat - 0.5;
            let l = Pos { x: m[0] - dy / len * w, y: m[1] + dx / len * w };
            let r = Pos { x: m[0] + dy / len * w, y: m[1] - dx / len * w };
            tilts.push((t.height_at(l) - t.height_at(r)).abs());
        }
        tilts.sort_by(|a, b| a.total_cmp(b));
        let p90 = tilts[tilts.len() * 9 / 10];
        assert!(p90 < 0.05, "kerb to kerb, 90 % of the road within 5 cm (got {p90:.3} m)");

        let mut far = 0;
        for k in 0..20_000u32 {
            let p = Pos { x: (k as f32 * 397.3) % t.width_m, y: (k as f32 * 911.7) % t.height_m };
            if g.near([p.x, p.y]).is_empty() {
                assert_eq!(t.height_at(p), t.raw_height_at(p));
                far += 1;
            }
        }
        assert!(far > 10_000);
    }

    /// No jumps: the graded ground is continuous everywhere near the roads.
    #[test]
    fn the_graded_ground_has_no_steps() {
        let scn = scenario();
        let t = &scn.terrain;
        let g = t.grade.as_ref().unwrap();
        let mut worst = 0.0f32;
        for (i, s) in g.segs.iter().enumerate().step_by(3) {
            for k in 0..24 {
                let ang = (i * 7 + k) as f32 * 0.9;
                let r = (k as f32 * 1.7) % (s.flat + 34.0);
                let p = Pos { x: s.a[0] + r * ang.cos(), y: s.a[1] + r * ang.sin() };
                let q = Pos { x: p.x + 0.05 * ang.sin(), y: p.y - 0.05 * ang.cos() };
                worst = worst.max((t.height_at(q) - t.height_at(p)).abs());
            }
        }
        // 5 cm apart: even a 45° fill slope is 5 cm; a step would be metres
        assert!(worst < 0.6, "largest change over 5 cm: {worst:.2} m");
    }
}
