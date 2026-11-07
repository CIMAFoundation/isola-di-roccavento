//! «Dove andrà il fuoco»: the scientific expert's forecast, issued at each
//! pause (docs/01-GIOCO.md §2, user 2026-10-14).
//!
//! What it knows is what the player could know: the fire **now** (the core's
//! own state, [`fire::ensemble::Snapshot`]), the wind now, and the weather
//! bulletin: the case's change of wind, if it is still to come. Nothing else
//! the simulation will do is read.
//!
//! [`MEMBERS`] runs of the same propagator go forward [`HORIZONS_S`], each
//! with its own seed and a wind perturbed around the forecast: the wind now
//! by ±[`NOW_DIR_DEG`] and ±[`NOW_SPEED`], the announced wind by
//! ±[`LATER_DIR_DEG`] and ±[`LATER_SPEED`], its time by ±[`LATER_AT_S`].
//! **Senza interventi**: the members carry the lines already cut and the
//! water already dropped, but no crew works in them from now on, so the
//! forecast is pessimistic where the units are working. Per cell, the share
//! of members in which it has burnt by +1 and +3 h; the 50 % contour
//! is the isochrone the map draws.
//!
//! Deterministic: the members' seeds and winds come from the game's seed and
//! the time of issue only.

use anyhow::Result;
use fire::ensemble::{Member, Snapshot};
use fire::Weather;

use crate::Game;

/// Members of the ensemble (constant: the forecast's resolution is 1/16).
pub const MEMBERS: usize = 16;
/// The forecast's horizons from the time of issue, simulated seconds.
pub const HORIZONS_S: [i64; 2] = [3600, 3 * 3600];
/// How many horizons, and the last one (what the members run to).
pub const HORIZONS: usize = HORIZONS_S.len();
const LAST_S: i64 = HORIZONS_S[HORIZONS - 1];
/// The share of members a cell must have burnt in to be inside the isochrone.
pub const LIKELY: f32 = 0.5;
/// Spread of the wind now (observed: small), degrees and fraction of speed.
pub const NOW_DIR_DEG: f64 = 10.0;
pub const NOW_SPEED: f64 = 0.10;
/// Spread of the announced wind: direction, speed, and when it comes.
pub const LATER_DIR_DEG: f64 = 20.0;
pub const LATER_SPEED: f64 = 0.20;
pub const LATER_AT_S: i64 = 10 * 60;
/// Which forecast: the simulated second of the pause it was asked at.
pub type Epoch = i64;

/// The forecast: per fire cell, the share of members burnt by each horizon.
#[derive(Debug, Clone)]
pub struct Forecast {
    pub epoch: Epoch,
    /// Simulated second of issue; the horizons count from here.
    pub issued_s: i64,
    pub members: usize,
    pub rows: usize,
    pub cols: usize,
    /// `[+1 h, +3 h]`, row-major on the fire grid (row 0 north), 0-1.
    pub prob: [Vec<f32>; HORIZONS],
    /// Per fire cell, the second after issue by which half the members have
    /// burnt it (the median arrival; 0 where it burns already), infinite
    /// where that is not within the last horizon. `eta_s <= HORIZONS_S[h]`
    /// is exactly [`Forecast::likely`] for `h`.
    pub eta_s: Vec<f32>,
}

impl Forecast {
    /// Whether cell `i` is inside horizon `h`'s isochrone.
    pub fn likely(&self, h: usize, i: usize) -> bool {
        self.prob[h][i] >= LIKELY
    }

    /// Cells inside horizon `h`'s isochrone.
    pub fn likely_cells(&self, h: usize) -> usize {
        self.prob[h].iter().filter(|&&p| p >= LIKELY).count()
    }
}

/// The members still to run, and what the finished ones burnt. Owns its
/// data: run it here ([`Job::run`]), on another thread, or a member per
/// frame ([`Job::step`]) where there are no threads.
pub struct Job {
    pub epoch: Epoch,
    snapshot: Snapshot,
    members: Vec<Member>,
    done: usize,
    counts: [Vec<u16>; HORIZONS],
    /// Each finished member's arrival times, seconds after the snapshot
    /// (`i32::MAX`: not burnt).
    arrivals: Vec<Vec<i32>>,
}

/// splitmix64: a few good bits from a seed, no dependency.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Uniform in [-1, 1).
fn signed(z: u64) -> f64 {
    (z >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// Stratified offsets in [-1, 1): member `k` of `n` draws within its own
/// slice, shuffled per quantity, so 16 members cover the spread evenly.
fn stratified(k: usize, n: usize, key: u64) -> f64 {
    // a permutation of the slices by sorting on hashes
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&j| mix(key ^ (j as u64).wrapping_mul(0x2545_F491_4F6C_DD1D)));
    let slice = order[k] as f64;
    let jitter = (signed(mix(key ^ 0xA5A5 ^ k as u64)) + 1.0) / 2.0;
    (slice + jitter) / n as f64 * 2.0 - 1.0
}

fn turn(w: Weather, dir_deg: f64, speed: f64) -> Weather {
    Weather { wind_dir_deg: (w.wind_dir_deg + dir_deg).rem_euclid(360.0), wind_speed_kmh: (w.wind_speed_kmh * (1.0 + speed)).max(0.0), ..w }
}

impl Game {
    /// The forecast the player should see now: one per pause, while it lasts.
    pub fn forecast_epoch(&self) -> Option<Epoch> {
        self.pause_started_s()
    }

    /// The ensemble for the forecast due now ([`Game::forecast_epoch`]), or
    /// for the wind as it is when none is due. Reads the state, changes
    /// nothing.
    pub fn forecast_job(&self) -> Result<Job> {
        let now = self.time_s();
        let epoch = self.forecast_epoch().unwrap_or(now);
        let snapshot = self.fire.snapshot(&self.scn)?;
        // the wind as it is known now: the game turns the fire's wind at the
        // start of the step after the shift's time, the forecast at the time
        let weather = match self.territory.shifted(&self.case) {
            Some((at, w)) if now >= at => w,
            _ => self.fire.weather(),
        };
        // the change still to come, as announced; none once it has happened
        let later = self.territory.shifted(&self.case).filter(|&(at, _)| at > now);
        let key = mix(self.seed ^ mix(now as u64 ^ 0xF0EC_A57));
        let q = |k: usize, what: u64| stratified(k, MEMBERS, mix(key ^ what));
        let members = (0..MEMBERS)
            .map(|k| Member {
                seed: mix(key ^ (k as u64 + 1)),
                now: turn(weather, q(k, 1) * NOW_DIR_DEG, q(k, 2) * NOW_SPEED),
                later: later.map(|(at, w)| {
                    let at = (at + (q(k, 5) * LATER_AT_S as f64).round() as i64).max(now);
                    (at, turn(w, q(k, 3) * LATER_DIR_DEG, q(k, 4) * LATER_SPEED))
                }),
            })
            .collect();
        let n = snapshot.rows * snapshot.cols;
        Ok(Job { epoch, snapshot, members, done: 0, counts: std::array::from_fn(|_| vec![0; n]), arrivals: vec![] })
    }

    /// The fire a few minutes ahead, for drawing only (2026-11-07, user:
    /// «do lookahead in the sim and interpolate between to have a less jarry
    /// frontline»): the core's snapshot now and one member with the wind as
    /// it is and as announced, unperturbed. Run it for `horizon_s` with
    /// [`Snapshot::run`]: per cell the second it is lit. Its own seed (the
    /// core's draws are not in the checkpoint), so a cell it lights may stay
    /// unlit in the game: what it says is a guess, and nothing the game
    /// decides reads it. Reads the state, changes nothing.
    pub fn lookahead_job(&self) -> Result<(Snapshot, Member)> {
        let now = self.time_s();
        let snapshot = self.fire.snapshot(&self.scn)?;
        let weather = match self.territory.shifted(&self.case) {
            Some((at, w)) if now >= at => w,
            _ => self.fire.weather(),
        };
        let later = self.territory.shifted(&self.case).filter(|&(at, _)| at > now);
        Ok((snapshot, Member { seed: mix(self.seed ^ mix(now as u64 ^ 0x100C_A11E)), now: weather, later }))
    }

    /// The forecast due now, computed here (headless; the kiosk runs
    /// [`Job`] off the frame).
    pub fn forecast(&self) -> Result<Forecast> {
        self.forecast_job()?.run()
    }
}

impl Job {
    pub fn issued_s(&self) -> i64 {
        self.snapshot.time_s
    }

    /// The same ensemble as if no change of wind had been announced: the
    /// baseline that shows what the announcement adds (`examples/previsione.rs`).
    pub fn without_change(mut self) -> Job {
        for m in &mut self.members {
            m.later = None;
        }
        self
    }

    pub fn is_done(&self) -> bool {
        self.done == self.members.len()
    }

    fn add(&mut self, arrival: &[i32]) {
        let t0 = self.snapshot.time_s;
        self.arrivals.push(arrival.iter().map(|&t| if t == i32::MIN { i32::MAX } else { (t as i64 - t0).clamp(0, i32::MAX as i64) as i32 }).collect());
        for (h, &dt) in HORIZONS_S.iter().enumerate() {
            let until = (t0 + dt) as i32;
            for (c, &t) in self.counts[h].iter_mut().zip(arrival) {
                *c += (t != i32::MIN && t <= until) as u16;
            }
        }
    }

    /// Run the next member. Returns whether all are done.
    pub fn step(&mut self) -> Result<bool> {
        if let Some(m) = self.members.get(self.done).copied() {
            let a = self.snapshot.run(&m, LAST_S)?;
            self.add(&a);
            self.done += 1;
        }
        Ok(self.is_done())
    }

    /// Run every member left, on all cores where there are threads; the
    /// result does not depend on how many.
    pub fn run(mut self) -> Result<Forecast> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let left: Vec<Member> = self.members[self.done..].to_vec();
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, left.len().max(1));
            let chunk = left.len().div_ceil(threads).max(1);
            let snap = &self.snapshot;
            let runs: Vec<Result<Vec<Vec<i32>>>> = std::thread::scope(|s| {
                let hs: Vec<_> = left.chunks(chunk).map(|ms| s.spawn(move || ms.iter().map(|m| snap.run(m, LAST_S)).collect())).collect();
                hs.into_iter().map(|h| h.join().expect("forecast member panicked")).collect()
            });
            for r in runs {
                for a in r? {
                    self.add(&a);
                    self.done += 1;
                }
            }
        }
        while !self.step()? {}
        Ok(self.finish())
    }

    /// The forecast from the members run so far.
    pub fn finish(self) -> Forecast {
        let n = self.done.max(1) as f32;
        let p = |c: Vec<u16>| c.into_iter().map(|c| c as f32 / n).collect();
        // the first cell-wise `ceil(LIKELY * n)` arrivals decide: the median
        let k = ((LIKELY * self.done as f32).ceil() as usize).clamp(1, self.done.max(1)) - 1;
        let cells = self.counts[0].len();
        let mut at = Vec::with_capacity(self.done);
        let eta_s = (0..cells)
            .map(|i| {
                at.clear();
                at.extend(self.arrivals.iter().map(|a| a[i]));
                match at.len() {
                    0 => f32::INFINITY,
                    _ => {
                        let t = *at.select_nth_unstable(k).1;
                        if t as i64 <= LAST_S { t as f32 } else { f32::INFINITY }
                    }
                }
            })
            .collect();
        Forecast {
            eta_s,
            epoch: self.epoch,
            issued_s: self.snapshot.time_s,
            members: self.done,
            rows: self.snapshot.rows,
            cols: self.snapshot.cols,
            prob: self.counts.map(p),
        }
    }
}
