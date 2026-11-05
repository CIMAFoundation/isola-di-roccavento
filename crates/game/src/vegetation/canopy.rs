//! The canopy as the ground sees it from afar: every plant the scatter would
//! grow at full density, its crown stamped as a disc on the terrain's sample
//! grid. The terrain shader paints it as the crowns' colour far from the eye
//! and as their shade near it, where the real plants stand (`terrain.wesl`).
//! Looks only: the fuel, and so the fire, are untouched.

use bevy::prelude::*;
use scenario::{Pos, Scenario};

use super::scatter::{self, Species};

/// Per terrain sample: how much of it the crowns cover (0..1), the crowns'
/// kind (0 conifer, 0.5 broadleaf, 1 macchia, averaged over the crowns) and
/// their brightness against their kind's mean colour.
pub struct Canopy {
    pub cover: Vec<f32>,
    pub kind: Vec<f32>,
    pub light: Vec<f32>,
    /// Mean crown colour of each kind, linear: conifer, broadleaf, macchia.
    pub colours: [[f32; 3]; 3],
}

/// How opaque one crown is seen from above: the sky shows through its edge.
const CROWN_OPACITY: f32 = 0.85;

fn kind_code(s: Species) -> (usize, f32) {
    match s {
        Species::Conifer => (0, 0.0),
        Species::Broadleaf | Species::Grass => (1, 0.5),
        Species::Shrub => (2, 1.0),
    }
}

/// The painted colour of a variety's crown, linear: its canopy tile of the
/// foliage atlas averaged where it is leaf, times the tone its leaves carry.
pub(super) fn crown_colour(name: &'static str) -> [f32; 3] {
    static ATLAS: std::sync::OnceLock<super::atlas::Atlas> = std::sync::OnceLock::new();
    let atlas = ATLAS.get_or_init(super::atlas::atlas);
    let [u0, v0, u1, v1] = crate::models::variety(name).canopy;
    let (w, h) = (atlas.width as usize, atlas.height as usize);
    let (x0, x1) = ((u0 * w as f32) as usize, ((u1 * w as f32) as usize).min(w));
    let (y0, y1) = ((v0 * h as f32) as usize, ((v1 * h as f32) as usize).min(h));
    let lin = |b: u8| Color::srgb_u8(b, 0, 0).to_linear().red;
    let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 4;
            let a = atlas.data[i + 3] as f32 / 255.0;
            for k in 0..3 {
                sum[k] += lin(atlas.data[i + k]) * a;
            }
            n += a;
        }
    }
    let shade = super::meshes::crown_tone(name);
    sum.map(|s| s / n.max(1e-3) * shade)
}

/// Whether the forest's occlusion is drawn (2026-11-04, on trial):
/// `KIOSK_BOSCO=1`. Off, the ground texture has no mips and every plant's
/// stand density is 0: nothing changes.
pub fn forest_shade() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KIOSK_BOSCO").is_ok_and(|v| v == "1"))
}

/// How closed the stand around each fire cell is, 0..1: the woody fuels
/// (trees 1, macchia 0.55) averaged over about [`STAND_RADIUS_CELLS`] cells.
/// From the fuel raster, not from the scatter, so it is the same whatever
/// share of the plants a machine draws.
pub struct StandDensity {
    cols: usize,
    rows: usize,
    cell: f32,
    height_m: f32,
    density: Vec<f32>,
}

/// Each pass of the box blur reaches this many cells; two passes make a tent
/// of twice the reach (some 80 m on 20 m cells).
const STAND_RADIUS_CELLS: usize = 2;

impl StandDensity {
    pub fn new(scn: &Scenario) -> Self {
        let w = scn.world;
        let (cols, rows) = (w.fire_cols, w.fire_rows);
        let mut d: Vec<f32> = scn
            .fuel
            .iter()
            .map(|&f| match Species::of_fuel(f) {
                Some(Species::Conifer | Species::Broadleaf) => 1.0,
                Some(Species::Shrub) => 0.55,
                _ => 0.0,
            })
            .collect();
        for _ in 0..2 {
            d = box_blur(&d, cols, rows, STAND_RADIUS_CELLS);
        }
        StandDensity { cols, rows, cell: w.cellsize, height_m: w.height_m, density: d }
    }

    /// At a point of the scenario frame (bilinear between cell centres).
    pub fn at(&self, p: Pos) -> f32 {
        let fc = (p.x / self.cell - 0.5).clamp(0.0, (self.cols - 1) as f32);
        let fr = ((self.height_m - p.y) / self.cell - 0.5).clamp(0.0, (self.rows - 1) as f32);
        let (c0, r0) = (fc.floor() as usize, fr.floor() as usize);
        let (c1, r1) = ((c0 + 1).min(self.cols - 1), (r0 + 1).min(self.rows - 1));
        let (tx, ty) = (fc - c0 as f32, fr - r0 as f32);
        let v = |r: usize, c: usize| self.density[r * self.cols + c];
        let top = v(r0, c0) * (1.0 - tx) + v(r0, c1) * tx;
        let bottom = v(r1, c0) * (1.0 - tx) + v(r1, c1) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

/// A separable box blur of `r` cells, the edges clamped.
fn box_blur(src: &[f32], cols: usize, rows: usize, r: usize) -> Vec<f32> {
    let n = (2 * r + 1) as f32;
    let mut tmp = vec![0.0f32; src.len()];
    for y in 0..rows {
        for x in 0..cols {
            let mut s = 0.0;
            for k in 0..=2 * r {
                let xx = (x + k).saturating_sub(r).min(cols - 1);
                s += src[y * cols + xx];
            }
            tmp[y * cols + x] = s / n;
        }
    }
    let mut out = vec![0.0f32; src.len()];
    for y in 0..rows {
        for x in 0..cols {
            let mut s = 0.0;
            for k in 0..=2 * r {
                let yy = (y + k).saturating_sub(r).min(rows - 1);
                s += tmp[yy * cols + x];
            }
            out[y * cols + x] = s / n;
        }
    }
    out
}

/// The canopy on a grid of `cols` x `rows` samples `posting` metres apart,
/// row 0 at the north edge (the terrain's textures).
pub fn canopy(scn: &Scenario, cols: usize, rows: usize, posting: f32) -> Canopy {
    let w = scn.world;
    let n = cols * rows;
    let (mut keep, mut kind, mut light) = (vec![1.0f32; n], vec![0.0f32; n], vec![0.0f32; n]);
    let mut weight = vec![0.0f32; n];
    let height_m = (rows - 1) as f32 * posting;
    // the scatter in bands of fire rows, on all cores natively
    let band = 16;
    let bands: Vec<(usize, usize)> = (0..w.fire_rows).step_by(band).map(|r0| (r0, (r0 + band).min(w.fire_rows))).collect();
    let scatter_band = |&(r0, r1): &(usize, usize)| scatter::scatter_tile(scn, 1.0, (r0, r1, 0, w.fire_cols));
    #[cfg(not(target_arch = "wasm32"))]
    let stands: Vec<scatter::Stand> = {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let out = std::sync::Mutex::new((0..bands.len()).map(|_| None).collect::<Vec<Option<scatter::Stand>>>());
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(b) = bands.get(i) else { break };
                    let stand = scatter_band(b);
                    out.lock().unwrap()[i] = Some(stand);
                });
            }
        });
        out.into_inner().unwrap().into_iter().map(Option::unwrap).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let stands: Vec<scatter::Stand> = bands.iter().map(scatter_band).collect();

    let mut colour_of: std::collections::HashMap<&'static str, [f32; 3]> = Default::default();
    let mut kind_sum = [[0.0f32; 3]; 3];
    let mut kind_n = [0.0f32; 3];
    // first the crowns' colours, for each kind's mean
    let mut crowns = Vec::new();
    for i in stands.iter().flat_map(|s| s.inst.iter()) {
        let p = Pos { x: i.base.x, y: -i.base.z };
        let species = Species::of_fuel(scn.fuel_at(w.cell_of(p))).unwrap_or(Species::Broadleaf);
        let (k, code) = kind_code(species);
        let c = *colour_of.entry(i.name).or_insert_with(|| crown_colour(i.name));
        let c = [c[0] * i.leaf[0], c[1] * i.leaf[1], c[2] * i.leaf[2]];
        let r = super::meshes::crown_radius(i.name) * i.scale.x;
        for j in 0..3 {
            kind_sum[k][j] += c[j];
        }
        kind_n[k] += 1.0;
        crowns.push((p, r, k, code, c));
    }
    let colours = [0, 1, 2].map(|k| kind_sum[k].map(|s| s / kind_n[k].max(1.0)));
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    for (p, r, k, code, c) in crowns {
        let bright = lum(c) / lum(colours[k]).max(1e-4);
        let (fc, fr) = (p.x / posting, (height_m - p.y) / posting);
        let reach = (r / posting).ceil() as isize + 1;
        for dr in -reach..=reach {
            for dc in -reach..=reach {
                let (cc, rr) = (fc.round() as isize + dc, fr.round() as isize + dr);
                if cc < 0 || rr < 0 || cc >= cols as isize || rr >= rows as isize {
                    continue;
                }
                let d = ((cc as f32 - fc).powi(2) + (rr as f32 - fr).powi(2)).sqrt() * posting;
                // the share of the sample's square under the crown, roughly
                let a = ((r - d) / posting + 0.5).clamp(0.0, 1.0) * CROWN_OPACITY;
                if a <= 0.0 {
                    continue;
                }
                let s = rr as usize * cols + cc as usize;
                keep[s] *= 1.0 - a;
                kind[s] += a * code;
                light[s] += a * bright;
                weight[s] += a;
            }
        }
    }
    let cover = keep.iter().map(|k| 1.0 - k).collect();
    for s in 0..n {
        if weight[s] > 0.0 {
            kind[s] /= weight[s];
            light[s] /= weight[s];
        } else {
            light[s] = 1.0;
        }
    }
    Canopy { cover, kind, light, colours }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How much of the island the crowns cover, and what it costs.
    #[test]
    fn the_canopy_covers_the_woods() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let Ok(scn) = Scenario::load(&dir) else { return };
        let t = &scn.terrain;
        let started = std::time::Instant::now();
        let c = canopy(&scn, t.cols, t.rows, t.posting);
        let took = started.elapsed().as_secs_f32();
        let land: Vec<usize> = (0..t.cols * t.rows).filter(|&i| t.elev[i] > 2.0).collect();
        let mean = land.iter().map(|&i| c.cover[i]).sum::<f32>() / land.len() as f32;
        let closed = land.iter().filter(|&&i| c.cover[i] > 0.6).count() as f32 / land.len() as f32;
        eprintln!("canopy in {took:.2} s: mean cover on land {mean:.2}, closed (> 0.6) {closed:.2}; colours {:?}", c.colours);
        assert!(mean > 0.1 && mean < 0.9, "{mean}");
    }
}
