//! The rendering heightfield: finer than the fire grid, and independent of it.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{read_raw, Pos};

#[derive(Debug, Deserialize)]
struct TerrainMeta {
    rows: usize,
    cols: usize,
    posting_m: f32,
    world_size_m: [f32; 2],
    elev_min: f32,
    elev_max: f32,
}

/// Smoothed elevation samples for rendering. Row 0 is the north edge.
pub struct Terrain {
    pub rows: usize,
    pub cols: usize,
    /// Sample spacing in metres (5 m at present, vs the fire grid's 20 m).
    pub posting: f32,
    pub width_m: f32,
    pub height_m: f32,
    pub elev_min: f32,
    pub elev_max: f32,
    /// The ground at each sample: the published DEM with the roads graded
    /// into it ([`crate::grade`]), when grading is on.
    pub elev: Vec<f32>,
    /// The road grading, and the published (ungraded) height of every sample
    /// it changed, sorted by sample index: [`Terrain::height_at`] applies the
    /// rule to the raw ground at any position, so the road bench is exact
    /// between samples too.
    pub grade: Option<crate::grade::RoadGrade>,
    raw_changed: Vec<(u32, f32)>,
}

impl Terrain {
    pub fn load(dir: &Path) -> Result<Terrain> {
        let meta: TerrainMeta = serde_json::from_slice(
            &datafs::read(dir.join("render_terrain.json"))
                .context("render_terrain.json")?,
        )?;
        let elev = read_raw::<f32>(
            &dir.join("render_terrain.f32"),
            meta.rows * meta.cols,
        )?;
        Ok(Terrain {
            rows: meta.rows,
            cols: meta.cols,
            posting: meta.posting_m,
            width_m: meta.world_size_m[0],
            height_m: meta.world_size_m[1],
            elev_min: meta.elev_min,
            elev_max: meta.elev_max,
            elev,
            grade: None,
            raw_changed: Vec::new(),
        })
    }

    /// Grade the roads into the ground ([`crate::grade`]).
    pub fn grade_roads(&mut self, roads: &[crate::Road]) {
        let grade = crate::grade::RoadGrade::build(roads, self.width_m, self.height_m, |p| self.height_at(Pos { x: p[0], y: p[1] }));
        let mut changed = Vec::new();
        for r in 0..self.rows {
            let y = self.height_m - r as f32 * self.posting;
            for c in 0..self.cols {
                let p = [c as f32 * self.posting, y];
                if grade.near(p).is_empty() {
                    continue;
                }
                let i = r * self.cols + c;
                let raw = self.elev[i];
                let z = grade.apply(p, raw);
                if z != raw {
                    changed.push((i as u32, raw));
                    self.elev[i] = z;
                }
            }
        }
        self.raw_changed = changed;
        self.grade = Some(grade);
    }

    /// The published height of sample `i`, before grading.
    #[inline]
    fn raw_sample(&self, i: usize) -> f32 {
        match self.raw_changed.binary_search_by_key(&(i as u32), |e| e.0) {
            Ok(k) => self.raw_changed[k].1,
            Err(_) => self.elev[i],
        }
    }

    /// Bilinear on the published DEM, before the roads are graded.
    pub fn raw_height_at(&self, p: Pos) -> f32 {
        self.bilinear(p, |i| self.raw_sample(i))
    }

    /// Bilinear elevation at a world position. Agents, buildings and the
    /// camera all sit on this, so it must be continuous -- sampling the 20 m
    /// fire DEM instead would make everything visibly step.
    pub fn height_at(&self, p: Pos) -> f32 {
        match &self.grade {
            Some(g) if !g.near([p.x, p.y]).is_empty() => g.apply([p.x, p.y], self.raw_height_at(p)),
            _ => self.bilinear(p, |i| self.elev[i]),
        }
    }

    #[inline]
    fn bilinear(&self, p: Pos, h: impl Fn(usize) -> f32) -> f32 {
        let fx = (p.x / self.posting).clamp(0.0, (self.cols - 1) as f32);
        // world +y is north, row 0 is the north edge
        let fy = ((self.height_m - p.y) / self.posting).clamp(0.0, (self.rows - 1) as f32);

        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(self.cols - 1);
        let y1 = (y0 + 1).min(self.rows - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        let h = |r: usize, c: usize| h(r * self.cols + c);
        let top = h(y0, x0) * (1.0 - tx) + h(y0, x1) * tx;
        let bot = h(y1, x0) * (1.0 - tx) + h(y1, x1) * tx;
        top * (1.0 - ty) + bot * ty
    }

    /// Surface normal from central differences, for lighting and for slope
    /// effects on agent walking speed.
    pub fn normal_at(&self, p: Pos) -> [f32; 3] {
        let d = self.posting;
        let hl = self.height_at(Pos { x: p.x - d, ..p });
        let hr = self.height_at(Pos { x: p.x + d, ..p });
        let hd = self.height_at(Pos { y: p.y - d, ..p });
        let hu = self.height_at(Pos { y: p.y + d, ..p });
        let nx = hl - hr;
        let nz = hd - hu;
        let ny = 2.0 * d;
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        [nx / len, ny / len, nz / len]
    }

    /// Slope in degrees -- drives how fast people can move off-road. On the
    /// published ground: the road grading is drawn, not simulated (the user
    /// decides whether the model sees it; `docs/TODO.md`).
    pub fn slope_deg_at(&self, p: Pos) -> f32 {
        let d = self.posting;
        let h = |q: Pos| self.raw_height_at(q);
        let nx = h(Pos { x: p.x - d, ..p }) - h(Pos { x: p.x + d, ..p });
        let nz = h(Pos { y: p.y - d, ..p }) - h(Pos { y: p.y + d, ..p });
        let ny = 2.0 * d;
        let up = ny / (nx * nx + ny * ny + nz * nz).sqrt();
        up.clamp(-1.0, 1.0).acos().to_degrees()
    }
}
