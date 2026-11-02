//! Reading the fire as a continuous field instead of as 20 m cells.
//!
//! The CA computes on a 20 m raster, but nothing says the fire has to be
//! *drawn* on one — and if it is, the result looks like a spreadsheet: square
//! flame fronts, staircase scar edges, whole blocks of trees igniting in
//! lockstep. The renderers on the CPU sample the fire through this module
//! instead, which bilinearly interpolates the cell fields; the scar on the
//! ground does the same in the terrain's shader, from the fire map
//! (`fire_view::update_fire_map`, `shaders/fire_ground.wesl`), with the sample
//! warped by ~7 m of value noise so the boundary meanders at a scale finer
//! than the grid it came from.
//!
//! This is a rendering device and nothing more: the model is unchanged, and no
//! decision — exposure, intervention, agent behaviour — is ever taken from
//! these interpolated values.

use scenario::{Pos, World};

pub struct FireField<'a> {
    pub arrival: &'a [i32],
    pub intensity: &'a [f32],
    /// Cells a crew on foot has cleared to bare ground.
    pub cleared: &'a [bool],
    pub world: World,
}

impl<'a> FireField<'a> {
    /// Fractional cell coordinates of a world position, in the raster's frame
    /// (col to the east, row to the south), relative to *cell centres*.
    fn cell_coords(&self, p: Pos) -> (f32, f32) {
        let w = &self.world;
        (
            (p.x / w.cellsize - 0.5).clamp(0.0, (w.fire_cols - 1) as f32),
            ((w.height_m - p.y) / w.cellsize - 0.5).clamp(0.0, (w.fire_rows - 1) as f32),
        )
    }

    fn corners(&self, p: Pos) -> ([usize; 4], f32, f32) {
        let (fx, fy) = self.cell_coords(p);
        let (x0, y0) = (fx.floor(), fy.floor());
        let x1 = (x0 as usize + 1).min(self.world.fire_cols - 1);
        let y1 = (y0 as usize + 1).min(self.world.fire_rows - 1);
        let (x0, y0) = (x0 as usize, y0 as usize);
        let cols = self.world.fire_cols;
        (
            [
                y0 * cols + x0,
                y0 * cols + x1,
                y1 * cols + x0,
                y1 * cols + x1,
            ],
            fx.fract(),
            fy.fract(),
        )
    }

    /// How much of this point is cleared ground, in `[0, 1]`, continuous
    /// across the edge of the line.
    pub fn cleared_fraction(&self, p: Pos) -> f32 {
        let (idx, tx, ty) = self.corners(p);
        let v = |i: usize| f32::from(self.cleared[i]);
        bilinear(v(idx[0]), v(idx[1]), v(idx[2]), v(idx[3]), tx, ty)
    }

    /// Fireline intensity at a point, interpolated over cells that have one.
    pub fn intensity(&self, p: Pos) -> f32 {
        let (idx, tx, ty) = self.corners(p);
        bilinear(
            self.intensity[idx[0]],
            self.intensity[idx[1]],
            self.intensity[idx[2]],
            self.intensity[idx[3]],
            tx,
            ty,
        )
    }

    /// Simulated time the fire reaches this *point*, interpolated over the
    /// cells that have already burnt.
    ///
    /// This is what lets a stand of trees inside one cell catch over several
    /// seconds in the direction the fire is travelling, rather than all at the
    /// same instant. `None` when no neighbouring cell has burnt at all.
    pub fn arrival(&self, p: Pos) -> Option<f32> {
        let (idx, tx, ty) = self.corners(p);
        let weights = [
            (1.0 - tx) * (1.0 - ty),
            tx * (1.0 - ty),
            (1.0 - tx) * ty,
            tx * ty,
        ];
        let (mut sum, mut total) = (0.0, 0.0);
        for (i, w) in idx.iter().zip(weights) {
            let t = self.arrival[*i];
            if t == i32::MIN || w <= 0.0 {
                continue;
            }
            sum += t as f32 * w;
            total += w;
        }
        // Unburnt corners drag the estimate later rather than being ignored:
        // without this, a point next to a single burnt cell would inherit that
        // cell's arrival time exactly and the whole cell would flip at once.
        (total > 0.0).then(|| sum / total + (1.0 - total) * 90.0)
    }
}

#[inline]
fn bilinear(v00: f32, v10: f32, v01: f32, v11: f32, tx: f32, ty: f32) -> f32 {
    let top = v00 * (1.0 - tx) + v10 * tx;
    let bot = v01 * (1.0 - tx) + v11 * tx;
    top * (1.0 - ty) + bot * ty
}

/// Smooth value noise in `[0, 1]`, on a unit lattice.
pub fn noise(x: f32, y: f32, seed: u64) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let corner = |ix: f32, iy: f32| {
        let mut h = seed
            ^ (ix as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (iy as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        (h >> 40) as f32 / (1u32 << 24) as f32
    };
    bilinear(
        corner(x0, y0),
        corner(x0 + 1.0, y0),
        corner(x0, y0 + 1.0),
        corner(x0 + 1.0, y0 + 1.0),
        sx,
        sy,
    )
}
