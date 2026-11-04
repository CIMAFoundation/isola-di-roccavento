//! Structure exposure: how the fire threatens buildings and people.
//!
//! This has to exist as its own layer, and the reason is structural rather
//! than aesthetic. Buildings sit on cells the fuel raster codes as
//! non-vegetated, so a house cell is *never* burnable and can never appear in
//! the core's fire mask. Reading "is this house on fire?" off the CA would
//! return false for every house in every scenario -- verified on the Spotorno
//! run, where a 48 ha fire produced zero burning household cells.
//!
//! So exposure is computed from proximity to burning cells, through the two
//! mechanisms that actually destroy houses in a WUI fire:
//!
//! - **radiant heat** from the flaming front;
//! - **ember attack**, the dominant cause of structure loss in real WUI fires,
//!   which travels far downwind of the front.
//!
//! Crucially the reach of both is **not a constant**: it scales with fireline
//! intensity, which the core reports per cell in kW/m. A creeping grass fire
//! at 200 kW/m and a crowning conifer run at 8,000 kW/m threaten completely
//! different radii, and a fixed interaction distance gets one of them badly
//! wrong. Intensity drives flame length, flame length drives radiant reach,
//! and plume strength drives how far embers loft.

use scenario::{Cell, Scenario, World};

use crate::{CellFire, Weather};

/// Byram's flame-length relation, `L = 0.0775 * I^0.46`, with fireline
/// intensity in kW/m and length in metres (Byram 1959; SI form after
/// Alexander 1982). The formula lives in the core
/// ([`propagator_core::flame_height`]), which also folds it into its output
/// maps: one rule for the game and for PROPAGATOR.
#[inline]
pub fn flame_length_m(fli_kw_m: f32) -> f32 {
    propagator_core::flame_height(fli_kw_m as f64) as f32
}

/// Radiant reach of a flaming cell.
///
/// Uses the safe-separation convention of four flame lengths (Butler & Cohen
/// 1998), which is where radiant flux has fallen to a level people and
/// structures can tolerate, clamped to 10–200 m so a single hot cell cannot
/// reach implausibly far ([`propagator_core::radiant_reach`]). No flame
/// (intensity ≤ 0) has no reach: 0 m.
#[inline]
pub fn radiant_range_m(fli_kw_m: f32) -> f32 {
    propagator_core::radiant_reach(fli_kw_m as f64) as f32
}

/// Downwind ember reach, growing with plume strength and wind.
///
/// Lofting distance rises with intensity and is carried by wind, so this
/// scales as sqrt(intensity) times a wind factor rather than sitting at a
/// fixed radius.
#[inline]
pub fn ember_range_m(fli_kw_m: f32, wind_kmh: f32) -> f32 {
    if fli_kw_m <= 0.0 {
        return 0.0;
    }
    let plume = (fli_kw_m / 500.0).sqrt();
    let wind = 1.0 + wind_kmh / 20.0;
    (120.0 * plume * wind).clamp(0.0, 2500.0)
}

/// Per-building accumulated exposure.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExposureField {
    /// Instantaneous radiant load, 0-1.
    pub radiant: f32,
    /// Instantaneous ember load, 0-1.
    pub ember: f32,
    /// Integrated damage, 0-1. Monotonic: structures do not un-burn.
    pub damage: f32,
    /// True once damage crosses the ignition threshold.
    pub alight: bool,
    /// Fireline intensity of the strongest cell currently reaching this
    /// house, kW/m. Drives the UI readout of *how bad* the threat is.
    pub peak_fli: f32,
}

pub struct StructureExposure {
    fields: Vec<ExposureField>,
    positions: Vec<[f32; 2]>,
    defensible: Vec<f32>,
    /// Structure protection by crews on scene, 0-1 per household, set by the
    /// game each step ([`StructureExposure::set_protection`]). Zero unless set.
    protection: Vec<f32>,
    /// Households bucketed on a fixed grid, so each burning cell only tests
    /// the households that could possibly be in range. Positions never change,
    /// so this is built once.
    buckets: Vec<Vec<u32>>,
    bucket_size: f32,
    bcols: usize,
    brows: usize,
}

/// Seconds of sustained *full* radiant load needed to ignite a structure.
const RADIANT_IGNITION_S: f32 = 600.0;
/// Seconds of sustained *full* ember load needed to ignite a structure.
const EMBER_IGNITION_S: f32 = 1800.0;

/// Share of the ember load a crew on scene removes: spot ignitions on and
/// around a house are what structure protection exists to put out, and embers
/// are the dominant cause of WUI losses (see the module docs).
pub const PROTECTED_EMBER: f32 = 0.85;
/// Share of the radiant load it removes: hose lines and wetting help, but a
/// flaming front next to the wall still radiates.
pub const PROTECTED_RADIANT: f32 = 0.5;

/// Distance over which a cell's ember density falls by e. Most WUI
/// structure losses to embers are within a few hundred metres of the front;
/// long-range spotting starts *fires* (the core's own spotting), it does not
/// shower a town 2 km away with enough brands to ignite every house.
pub const EMBER_DECAY_M: f32 = 700.0;

/// Bucket edge, in metres. Sized so a typical ember radius spans only a few
/// buckets while keeping the grid small.
const BUCKET_M: f32 = 250.0;

impl StructureExposure {
    pub fn new(scn: &Scenario) -> StructureExposure {
        let n = scn.population.households.len();
        let w = scn.world;
        let bcols = (w.width_m / BUCKET_M).ceil() as usize + 1;
        let brows = (w.height_m / BUCKET_M).ceil() as usize + 1;
        let mut buckets = vec![Vec::new(); brows * bcols];

        for (i, h) in scn.population.households.iter().enumerate() {
            let bx = (h.pos[0] / BUCKET_M) as usize;
            let by = (h.pos[1] / BUCKET_M) as usize;
            if let Some(b) = buckets.get_mut(by * bcols + bx) {
                b.push(i as u32);
            }
        }

        StructureExposure {
            fields: vec![ExposureField::default(); n],
            positions: scn.population.households.iter().map(|h| h.pos).collect(),
            defensible: scn
                .population
                .households
                .iter()
                .map(|h| h.defensible_space)
                .collect(),
            protection: vec![0.0; n],
            buckets,
            bucket_size: BUCKET_M,
            bcols,
            brows,
        }
    }

    pub fn fields(&self) -> &[ExposureField] {
        &self.fields
    }

    pub fn get(&self, household: usize) -> ExposureField {
        self.fields[household]
    }

    /// Crews on scene protecting each household, 0-1 (see [`PROTECTED_EMBER`]).
    /// Holds until changed; all zeros is the unprotected model, bit for bit.
    pub fn set_protection(&mut self, protection: &[f32]) {
        for (p, &v) in self.protection.iter_mut().zip(protection) {
            *p = v.clamp(0.0, 1.0);
        }
    }

    pub fn protection(&self) -> &[f32] {
        &self.protection
    }

    /// Households currently taking meaningful heat -- the triage list.
    pub fn threatened(&self, threshold: f32) -> impl Iterator<Item = usize> + '_ {
        self.fields
            .iter()
            .enumerate()
            .filter(move |(_, f)| f.radiant + f.ember > threshold)
            .map(|(i, _)| i)
    }

    /// Recompute exposure from the current active front.
    ///
    /// Scatters from burning cells to households rather than gathering, since
    /// each cell's reach now depends on its own intensity.
    pub fn update(
        &mut self,
        _state: &[CellFire],
        active: &[Cell],
        intensity: &[f32],
        world: World,
        weather: Weather,
        dt_s: f32,
    ) {
        for f in &mut self.fields {
            f.radiant = 0.0;
            f.ember = 0.0;
            f.peak_fli = 0.0;
        }
        if active.is_empty() {
            return;
        }

        // Wind blows *from* wind_dir_deg, so embers travel toward the
        // reciprocal bearing, in the world frame (+x east, +y north).
        let toward = (weather.wind_dir_deg + 180.0).to_radians() as f32;
        let wind_vec = [toward.sin(), toward.cos()];
        let wind_kmh = weather.wind_speed_kmh as f32;

        for cell in active {
            let fli = intensity[cell.row * world.fire_cols + cell.col];
            if fli <= 0.0 {
                continue;
            }
            let r_rad = radiant_range_m(fli);
            let r_emb = ember_range_m(fli, wind_kmh);
            let reach = r_rad.max(r_emb);
            if reach <= 0.0 {
                continue;
            }

            let p = world.centre_of(*cell);
            let span = (reach / self.bucket_size).ceil() as isize;
            let bx = (p.x / self.bucket_size) as isize;
            let by = (p.y / self.bucket_size) as isize;

            let bs = self.bucket_size;
            let reach_m = reach + 2.0;
            let reach2 = reach_m * reach_m;
            let y_lo = (by - span).max(0);
            let y_hi = (by + span).min(self.brows as isize - 1);
            let x_lo = (bx - span).max(0);
            let x_hi = (bx + span).min(self.bcols as isize - 1);
            for ny in y_lo..=y_hi {
                // Distance from the cell to this bucket row's rectangle.
                let ry0 = ny as f32 * bs;
                let gy = (ry0 - p.y).max(p.y - (ry0 + bs)).max(0.0);
                if gy * gy > reach2 {
                    continue;
                }
                for nx in x_lo..=x_hi {
                    let rx0 = nx as f32 * bs;
                    let gx = (rx0 - p.x).max(p.x - (rx0 + bs)).max(0.0);
                    if gx * gx + gy * gy > reach2 {
                        continue;
                    }
                    for &hi in &self.buckets[ny as usize * self.bcols + nx as usize] {
                        let hp = self.positions[hi as usize];
                        let vx = hp[0] - p.x;
                        let vy = hp[1] - p.y;
                        let d = (vx * vx + vy * vy).sqrt().max(1.0);
                        if d > reach {
                            continue;
                        }
                        let f = &mut self.fields[hi as usize];

                        if d < r_rad {
                            // Radiant flux falls off with distance; normalise
                            // by this cell's own radiant range so intensity
                            // sets both the reach and the near-field load.
                            let near = 1.0 - (d / r_rad);
                            f.radiant += near * near * (fli / 4000.0).min(2.0) * 0.35;
                            f.peak_fli = f.peak_fli.max(fli);
                        }
                        if d < r_emb {
                            // Embers need the house downwind of the cell.
                            let dot = (vx * wind_vec[0] + vy * wind_vec[1]) / d;
                            if dot > 0.0 {
                                let downwind = dot.powi(3);
                                // Ember density falls off fast with distance:
                                // the reach above is the longest lofting, not
                                // where houses are lost. Summed over every
                                // burning cell, a linear falloff saturated the
                                // load of whole towns 2-3 km downwind of a
                                // large front (audit, phase 0; measured on
                                // t4_paese: 160/160 homes of Castelvento alight
                                // with the fire 3 km away).
                                let falloff = (1.0 - d / r_emb) * (-d / EMBER_DECAY_M).exp();
                                f.ember += downwind * falloff * 0.02;
                                f.peak_fli = f.peak_fli.max(fli);
                            }
                        }
                    }
                }
            }
        }

        // Defensible space cuts radiant load hard and ember load a little:
        // clearing vegetation removes the fuel next to the wall, but does
        // nothing about embers landing in the gutters.
        for (i, f) in self.fields.iter_mut().enumerate() {
            let ds = self.defensible[i];
            f.radiant = (f.radiant * (1.0 - 0.8 * ds)).min(1.0);
            f.ember = (f.ember * (1.0 - 0.3 * ds)).min(1.0);
            // Structure protection: a crew on scene puts out what lands on the
            // roof and in the gutters, and wets the walls the front faces.
            let pr = self.protection[i];
            if pr > 0.0 {
                f.radiant *= 1.0 - PROTECTED_RADIANT * pr;
                f.ember *= 1.0 - PROTECTED_EMBER * pr;
            }

            if !f.alight {
                // Integrate over *simulated time*, not per call. Accruing per
                // update makes structure loss depend on the caller's step
                // size: the game steps every 2 s and a batch test every 300 s,
                // which would differ by 150x for the same fire.
                //
                // Rates are set so that sustained full radiant exposure
                // ignites a structure in ~10 minutes and sustained full ember
                // attack in ~30, which is the right order for WUI structure
                // loss.
                f.damage = (f.damage
                    + (f.radiant / RADIANT_IGNITION_S + f.ember / EMBER_IGNITION_S) * dt_s)
                    .min(1.0);
                if f.damage >= 1.0 {
                    f.alight = true;
                }
            }
        }
    }
}
