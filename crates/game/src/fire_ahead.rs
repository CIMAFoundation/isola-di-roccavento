//! The front drawn between now and a few minutes ahead (2026-11-07, user:
//! «do lookahead in the sim and interpolate between to have a less jarry
//! frontline»).
//!
//! The fire grid is 20 m. A cell went from unburnt to burnt in one rewrite
//! of the fire map, and the scar's edge, the bilinear contour of that flag,
//! jumped half a cell at once; the map was rewritten every 0.15 s of real
//! time, 45 simulated seconds at ×300. Two things here:
//!
//! - **the lookahead**: every [`REFRESH_S`] simulated seconds a member of
//!   the core runs [`HORIZON_S`] ahead from the game's snapshot
//!   (`rocca::Game::lookahead_job`, off the frame on the desktop), and says
//!   when the cells just ahead of the front will be lit. Measured
//!   (`examples/anticipo.rs`, medium, seeds 1-3, every 10 min): about 1 ms a
//!   run; where both it and the game light a cell, the same second (median
//!   and 90th percentile 0 s); a cell in eight or so lit by one only;
//! - **the ramp**: each cell's share of «burnt» rises from 0 to 1 over the
//!   time the front takes to cross it (from its neighbours' arrivals),
//!   crossing ½ at its own arrival. The bilinear contour of those shares
//!   then moves across the gap between two cells continuously. The shaders
//!   evaluate it every frame on the drawing clock ([`Sim::draw_time_s`]):
//!   the map holds a line per cell (its value at [`FireAhead::base`], its
//!   slope and where it stops), not the value.
//!
//! A cell the lookahead lit and the game has not stops at [`GUESS_CAP`] (the
//! front at its centre, no further). Where the game and the guess part (a
//! cell lit unforeseen, a guess dropped), the line eases to the new
//! target over [`CATCH_S`] instead of jumping.
//!
//! Only the drawing reads this: the guess decides nothing (the scar, the
//! glow and the volumes' flames; not the plants, which catch on the game's
//! own arrivals, `vegetation::plant_fire`). `KIOSK_ANTICIPO=0` (in the
//! browser `?anticipo=0`) draws the fire map as before.

use bevy::asset::uuid_handle;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use fire::CellFire;

use crate::materials::TerrainMaterial;
use crate::sim::{Sim, SimRestarted};
use crate::terrain_mesh::f16_bits;

/// The front map: `Rgba16Float`, a texel per fire cell laid out as the fire
/// map (`ground_mask::GRASS_MAP`, row 0 the south edge). r the cell's
/// arrival less [`FireAhead::base`] (s; [`NEVER_REL`] none foreseen), g its
/// share burnt at `base`, b how fast that changes (/s), a where it stops:
/// at `base + dt` the share is `clamp(g + b dt, 0, a)` when b ≥ 0, else
/// `clamp(g + b dt, a, 1)`.
pub const FRONT_MAP: Handle<Image> = uuid_handle!("c7e2a4f1-58b3-4d0e-9a6c-2f81d3b5e907");

/// How far ahead the lookahead runs, simulated seconds.
pub const HORIZON_S: i64 = 180;
/// How often it runs again, simulated seconds: two game steps, off the
/// frame; in the browser, where it runs in the frame (a few ms), ten.
pub const REFRESH_S: i64 = if cfg!(target_arch = "wasm32") { 60 } else { 12 };
/// Real seconds between rewrites of the map; between them the shaders
/// extend each line.
const WRITE_S: f32 = 0.1;
/// Simulated seconds a line takes to reach a target that moved, at least.
const CATCH_S: f32 = 20.0;
/// How far a cell the game has not lit may go: a little past the front at
/// its centre (the game lights it within a step of its arrival, when the
/// guess was right).
pub const GUESS_CAP: f32 = 0.6;
/// The time the front takes to cross a cell, bounds and the default where
/// no neighbour burnt before (an ignition, a spot fire), simulated s.
const GAP_MIN_S: f32 = 12.0;
const GAP_MAX_S: f32 = 600.0;
const GAP_NEW_S: f32 = 90.0;
/// Seconds after which the base moves on (and the whole map is written):
/// the half floats keep a second or better within ±2000 s of it.
const REBASE_S: f64 = 900.0;
/// No arrival foreseen, as the map holds it.
pub const NEVER_REL: f32 = 30_000.0;
/// Long burnt cells (no arrival in the core), as the map holds them.
const LONG_AGO_REL: f32 = -60_000.0;
/// Past this age (s) a burnt cell's line has long stopped and its glow is
/// out: it is not rewritten until the base moves.
const SETTLED_S: f32 = 2.0 * GAP_MAX_S + 2400.0;

/// Whether the front is drawn with the lookahead (on unless asked off).
pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let asked = if cfg!(target_arch = "wasm32") { crate::quality::web_param("anticipo") } else { std::env::var("KIOSK_ANTICIPO").ok() };
        asked.as_deref() != Some("0")
    })
}

#[cfg(not(target_arch = "wasm32"))]
type Running = std::sync::Arc<std::sync::Mutex<Option<Result<(i64, Vec<i32>), String>>>>;

/// The lookahead and the lines the map holds.
#[derive(Resource)]
pub struct FireAhead {
    /// Per cell the second the last lookahead lit it, `i32::MIN` none.
    guess: Vec<i32>,
    /// The game's second it ran from.
    guess_at: Option<i64>,
    #[cfg(not(target_arch = "wasm32"))]
    running: Option<Running>,
    /// Per cell (share at `base`, slope, stop), as the map holds them.
    lines: Vec<[f32; 3]>,
    /// The simulated second the lines count from.
    pub base: f64,
    /// The drawing clock this frame (simulated s).
    pub clock: f64,
    /// Cells whose lines may still change: written each time.
    live: Vec<usize>,
    since: f32,
    /// The guess changed, or the game stepped, since the last write.
    dirty: bool,
    /// Write everything anew, with no easing (a new game, the clock back).
    fresh: bool,
    last_time: i64,
    dims: (usize, usize),
}

impl Default for FireAhead {
    fn default() -> Self {
        FireAhead {
            guess: Vec::new(),
            guess_at: None,
            #[cfg(not(target_arch = "wasm32"))]
            running: None,
            lines: Vec::new(),
            base: 0.0,
            clock: 0.0,
            live: Vec::new(),
            since: f32::INFINITY,
            dirty: true,
            fresh: true,
            last_time: 0,
            dims: (0, 0),
        }
    }
}

/// The front map's image, every cell unburnt.
pub fn front_image(cols: usize, rows: usize) -> Image {
    let mut data = Vec::with_capacity(cols * rows * 8);
    for _ in 0..cols * rows {
        for v in [NEVER_REL, 0.0, 0.0, 0.0] {
            data.extend_from_slice(&f16_bits(v).to_le_bytes());
        }
    }
    Image::new(
        Extent3d { width: cols as u32, height: rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

/// The share burnt the line `l` gives `dt` seconds after its base.
pub fn share(l: [f32; 3], dt: f32) -> f32 {
    let v = l[0] + l[1] * dt;
    if l[1] >= 0.0 { v.clamp(0.0, l[2]) } else { v.clamp(l[2], 1.0) }
}

/// The share a cell should have at `x`: lit at `t` (∞ never), crossed in
/// `gap` seconds, at most `cap`.
fn target(t: f32, gap: f32, cap: f32, x: f32) -> f32 {
    if !t.is_finite() { 0.0 } else { (0.5 + (x - t) / gap).clamp(0.0, cap) }
}

/// The line for a cell, `now` seconds after the base, given the one it had:
/// the target's own where the two agree (or `fresh`), else one that reaches
/// the target `catch` seconds on.
fn line_for(was: [f32; 3], t: f32, gap: f32, cap: f32, now: f32, catch: f32, fresh: bool) -> [f32; 3] {
    let follow = if !t.is_finite() { [0.0, 0.0, 0.0] } else { [0.5 - t / gap, 1.0 / gap, cap] };
    if fresh {
        return follow;
    }
    let shown = share(was, now);
    let aim_now = target(t, gap, cap, now);
    if (shown - aim_now).abs() < 0.02 {
        return follow;
    }
    let aim = target(t, gap, cap, now + catch);
    let rate = (aim - shown) / catch;
    [shown - rate * now, rate, aim]
}

/// The lookahead's run, and the map's lines rewritten from the game and the
/// guess. After the game's step, before anything draws the fire.
#[allow(clippy::too_many_arguments)]
pub fn update(
    sim: Res<Sim>,
    time: Res<Time>,
    mut ahead: ResMut<FireAhead>,
    mut images: ResMut<Assets<Image>>,
    mut terrain: ResMut<Assets<TerrainMaterial>>,
    mut restarted: MessageReader<SimRestarted>,
) {
    let w = sim.scn.world;
    let (cols, rows) = (w.fire_cols, w.fire_rows);
    let a = &mut *ahead;
    let now = sim.time_s();
    if restarted.read().count() > 0 || now < a.last_time || a.dims != (cols, rows) {
        *a = FireAhead { dims: (cols, rows), ..default() };
        a.guess = vec![i32::MIN; cols * rows];
        a.lines = vec![[0.0; 3]; cols * rows];
        let _ = images.insert(&FRONT_MAP, front_image(cols, rows));
    }
    if a.last_time != now {
        a.dirty = true;
    }
    a.last_time = now;
    a.clock = sim.draw_time_s();
    poll(a, &sim);
    a.since += time.delta_secs();

    // the base moves on now and then, and the whole map with it
    if a.fresh || a.clock - a.base > REBASE_S {
        a.base = a.clock.floor();
        a.fresh = true;
    }
    let rel = (a.clock - a.base) as f32;
    // the clock to the terrain, every frame it moves
    // (writing marks the asset changed and rebuilds its bind group: only
    // when the value differs, so not while the world stands still)
    let stale: Vec<_> = terrain.iter().filter(|(_, m)| m.front_clock() != rel || m.front_on() != on()).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = terrain.get_mut(id) {
            m.set_front_clock(rel, on());
        }
    }
    if !on() || !(a.fresh || a.dirty && a.since >= WRITE_S) {
        return;
    }
    // simulated seconds since the last write, and how long a catch-up takes
    let catch = CATCH_S.max(1.5 * a.since.min(1.0) * sim.speed);
    a.since = 0.0;
    let Some(mut image) = images.get_mut(&FRONT_MAP) else { return };
    let Some(data) = image.data.as_mut() else { return };
    a.write(&sim.game, catch, data);
}

impl FireAhead {
    /// The arrival a cell is drawn with: the game's where it burns, the
    /// guess where the guess lights it within `within_s` of the drawing
    /// clock; `None` else.
    pub fn drawn_arrival(&self, game: &rocca::Game, i: usize, within_s: f32) -> Option<f32> {
        let (state, arrival) = (game.fire.state(), game.fire.arrival_times());
        if state[i] != CellFire::Unburnt {
            return (arrival[i] != i32::MIN).then_some(arrival[i] as f32);
        }
        let g = *self.guess.get(i)?;
        (on() && self.guess_at.is_some() && g != i32::MIN && f64::from(g) <= self.clock + f64::from(within_s)).then_some(g as f32)
    }

    /// Rewrites the lines of the cells that may change into `data` (the
    /// front map's bytes) from the game and the guess, on [`FireAhead::clock`].
    pub fn write(&mut self, game: &rocca::Game, catch: f32, data: &mut [u8]) {
        let a = self;
        let w = game.scn.world;
        let (cols, rows) = (w.fire_cols, w.fire_rows);
        a.dirty = false;
        let fresh = std::mem::replace(&mut a.fresh, false);
        let base = a.base;
        let rel = (a.clock - a.base) as f32;
        let sim = game;
        // each cell's arrival relative to the base (∞ none foreseen) and its cap
        let (state, arrival) = (sim.fire.state(), sim.fire.arrival_times());
        let guess_on = a.guess_at.is_some();
        let arrive = |i: usize| -> (f32, f32) {
            if state[i] != CellFire::Unburnt {
                let t = if arrival[i] == i32::MIN { LONG_AGO_REL } else { (f64::from(arrival[i]) - base) as f32 };
                (t, 1.0)
            } else if guess_on && a.guess[i] != i32::MIN {
                ((f64::from(a.guess[i]) - base) as f32, GUESS_CAP)
            } else {
                (f32::INFINITY, 0.0)
            }
        };
        // the cells to write: all of them anew, else those near the front and
        // those whose line has not stopped
        let mut cells: Vec<usize> = if fresh {
            (0..cols * rows).collect()
        } else {
            let mut v = std::mem::take(&mut a.live);
            for c in sim.fire.active_cells() {
                v.push(c.row * cols + c.col);
            }
            if guess_on {
                v.extend(a.guess.iter().enumerate().filter(|(i, g)| **g != i32::MIN && state[*i] == CellFire::Unburnt).map(|(i, _)| i));
            }
            v.sort_unstable();
            v.dedup();
            v
        };
        // their neighbours too: a ramp starts before its cell is lit
        if !fresh {
            let mut more = Vec::with_capacity(cells.len() * 2);
            for &i in &cells {
                let (r, c) = (i / cols, i % cols);
                for (dr, dc) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (rr, cc) = (r as i32 + dr, c as i32 + dc);
                    if rr >= 0 && cc >= 0 && (rr as usize) < rows && (cc as usize) < cols {
                        more.push(rr as usize * cols + cc as usize);
                    }
                }
            }
            cells.extend(more);
            cells.sort_unstable();
            cells.dedup();
        }
        let mut live = Vec::new();
        for i in cells {
            let (t, cap) = arrive(i);
            let (r, c) = (i / cols, i % cols);
            // the time the front took to cross into it: the latest neighbour lit
            // before it, by distance
            let mut gap = f32::INFINITY;
            if t.is_finite() {
                for dr in -1i32..=1 {
                    for dc in -1i32..=1 {
                        let (rr, cc) = (r as i32 + dr, c as i32 + dc);
                        if (dr, dc) == (0, 0) || rr < 0 || cc < 0 || rr as usize >= rows || cc as usize >= cols {
                            continue;
                        }
                        let (tn, _) = arrive(rr as usize * cols + cc as usize);
                        if tn < t {
                            let d = if dr != 0 && dc != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
                            gap = gap.min((t - tn) / d);
                        }
                    }
                }
            }
            let gap = if gap.is_finite() { gap.clamp(GAP_MIN_S, GAP_MAX_S) } else { GAP_NEW_S };
            let line = line_for(a.lines[i], t, gap, cap, rel, catch, fresh);
            a.lines[i] = line;
            // still moving: written next time as well
            let settled = !t.is_finite() && share(line, rel + 1e4) == 0.0 && line[1] == 0.0
                || cap >= 1.0 && rel - t > SETTLED_S;
            if !settled {
                live.push(i);
            }
            let at = ((rows - 1 - r) * cols + c) * 8;
            let tr = if t.is_finite() { t.clamp(LONG_AGO_REL, NEVER_REL) } else { NEVER_REL };
            for (k, v) in [tr, line[0], line[1], line[2]].into_iter().enumerate() {
                data[at + 2 * k..at + 2 * k + 2].copy_from_slice(&f16_bits(v).to_le_bytes());
            }
        }
        a.live = live;
    }
}

/// Starts the lookahead when it is due, takes its result when it is done.
fn poll(a: &mut FireAhead, sim: &Sim) {
    if !on() {
        return;
    }
    let now = sim.time_s();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(r) = &a.running {
        let done = r.lock().ok().and_then(|mut s| s.take());
        match done {
            Some(Ok((at, guess))) => {
                a.running = None;
                if at <= now && guess.len() == a.guess.len() {
                    a.guess = guess;
                    a.guess_at = Some(at);
                    a.dirty = true;
                }
            }
            Some(Err(e)) => {
                a.running = None;
                warn!("lookahead: {e}");
            }
            None => return,
        }
    }
    let due = a.guess_at.is_none_or(|at| now - at >= REFRESH_S || at > now);
    if !due || sim.fire.active_cells().is_empty() {
        return;
    }
    let Ok((snapshot, member)) = sim.lookahead_job() else { return };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let slot: Running = Default::default();
        let out = slot.clone();
        std::thread::spawn(move || {
            let r = snapshot.run(&member, HORIZON_S).map(|g| (snapshot.time_s, g)).map_err(|e| format!("{e:#}"));
            if let Ok(mut s) = out.lock() {
                *s = Some(r);
            }
        });
        a.running = Some(slot);
    }
    #[cfg(target_arch = "wasm32")]
    if let Ok(guess) = snapshot.run(&member, HORIZON_S) {
        a.guess = guess;
        a.guess_at = Some(now);
        a.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cell followed with no surprise: ½ at its arrival, 0 and 1 half a
    /// gap either side.
    #[test]
    fn a_line_crosses_half_at_arrival() {
        let l = line_for([0.0; 3], 100.0, 40.0, 1.0, 0.0, 20.0, true);
        assert!((share(l, 100.0) - 0.5).abs() < 1e-5);
        assert_eq!(share(l, 80.0), 0.0);
        assert_eq!(share(l, 120.0), 1.0);
        assert_eq!(share(l, 1e5), 1.0);
    }

    /// A target that jumps is reached over the catch-up, not at once, and
    /// never overshot.
    #[test]
    fn a_surprise_eases() {
        let was = line_for([0.0; 3], f32::INFINITY, 40.0, 0.0, 0.0, 20.0, true);
        // lit unforeseen, 30 s ago
        let l = line_for(was, -30.0, 40.0, 1.0, 0.0, 20.0, false);
        assert_eq!(share(l, 0.0), 0.0);
        assert!(share(l, 10.0) > 0.3 && share(l, 10.0) < 0.7);
        assert!((share(l, 20.0) - 1.0).abs() < 1e-5);
        assert!((share(l, 60.0) - 1.0).abs() < 1e-5);
        // a guess dropped: back down to 0 the same way
        let guessed = line_for([0.0; 3], -10.0, 40.0, GUESS_CAP, 0.0, 20.0, true);
        let shown = share(guessed, 0.0);
        let l = line_for(guessed, f32::INFINITY, 40.0, 0.0, 0.0, 20.0, false);
        assert!((share(l, 0.0) - shown).abs() < 1e-5);
        assert!(share(l, 10.0) < shown && share(l, 10.0) > 0.0);
        assert_eq!(share(l, 20.0), 0.0);
        assert_eq!(share(l, 100.0), 0.0);
    }

    /// How the drawn front moves from frame to frame, the fire map as it was
    /// (a burnt flag a cell, rewritten every 0.15 s) against the lines: the
    /// medium case, seed 1, from minute 40 for 20 s of real time at 60
    /// frames a second and ×300. Per frame, the burnt area drawn grows by
    /// some m²; a jarring front grows in bursts. Prints the table and
    /// checks that the lines grow more evenly and keep within a few cells
    /// of the game's own area.
    #[test]
    fn the_front_grows_evenly() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut sim = Sim::new(&data, "medium", 1).unwrap();
        while sim.time_s() < 40 * 60 {
            sim.game.step().unwrap();
        }
        sim.speed = 300.0;
        let w = sim.scn.world;
        let n = w.fire_cols * w.fire_rows;
        let cell_m2 = w.cellsize * w.cellsize;
        let mut a = FireAhead { dims: (w.fire_cols, w.fire_rows), guess: vec![i32::MIN; n], lines: vec![[0.0; 3]; n], ..default() };
        let mut bytes = vec![0u8; n * 8];
        let frame = 1.0 / 60.0;
        let (mut old_area, mut since_old) = (None::<f32>, f32::INFINITY);
        let (mut old, mut new, mut real) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..(20.0 / frame) as usize {
            sim.tick(frame).unwrap();
            let now = sim.time_s();
            a.clock = sim.draw_time_s();
            if a.guess_at.is_none_or(|at| now - at >= REFRESH_S) {
                let (snap, member) = sim.lookahead_job().unwrap();
                a.guess = snap.run(&member, HORIZON_S).unwrap();
                a.guess_at = Some(now);
                a.dirty = true;
            }
            if a.last_time != now {
                a.dirty = true;
            }
            a.last_time = now;
            a.since += frame;
            if a.fresh || a.clock - a.base > REBASE_S {
                a.base = a.clock.floor();
                a.fresh = true;
            }
            if a.fresh || a.dirty && a.since >= WRITE_S {
                let catch = CATCH_S.max(1.5 * a.since.min(1.0) * sim.speed);
                a.since = 0.0;
                a.write(&sim.game, catch, &mut bytes);
            }
            let rel = (a.clock - a.base) as f32;
            new.push(a.lines.iter().map(|l| share(*l, rel)).sum::<f32>() * cell_m2);
            let burnt = sim.fire.state().iter().filter(|s| **s != CellFire::Unburnt).count() as f32 * cell_m2;
            real.push(burnt);
            since_old += frame;
            if since_old >= 0.15 {
                since_old = 0.0;
                old_area = Some(burnt);
            }
            old.push(old_area.unwrap_or(burnt));
        }
        // per-frame growth, past the first second (the lines start fresh)
        let stats = |v: &[f32]| {
            let d: Vec<f32> = v.windows(2).skip(60).map(|p| (p[1] - p[0]).max(0.0)).collect();
            let mean = d.iter().sum::<f32>() / d.len() as f32;
            let still = d.iter().filter(|x| **x < 1.0).count() as f32 / d.len() as f32;
            let max = d.iter().cloned().fold(0.0, f32::max);
            let sd = (d.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / d.len() as f32).sqrt();
            (mean, sd / mean, still, max / mean)
        };
        let (o, l) = (stats(&old), stats(&new));
        println!("              m²/frame  scarto/media  fotogrammi fermi  massimo/media");
        println!("prima         {:>8.0}  {:>12.2}  {:>15.0} %  {:>13.1}", o.0, o.1, o.2 * 100.0, o.3);
        println!("anticipo      {:>8.0}  {:>12.2}  {:>15.0} %  {:>13.1}", l.0, l.1, l.2 * 100.0, l.3);
        let lag = new.iter().zip(&real).skip(60).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        println!("scarto massimo dall'area del gioco: {:.0} m² ({:.1} celle)", lag, lag / cell_m2);
        assert!(l.1 < o.1 * 0.5, "the lines grow no more evenly");
        assert!(l.2 < o.2 * 0.5);
    }

    /// An unconfirmed cell stops at its cap.
    #[test]
    fn a_guess_stops_at_the_cap() {
        let l = line_for([0.0; 3], 0.0, 40.0, GUESS_CAP, 0.0, 20.0, true);
        assert!((share(l, 1e4) - GUESS_CAP).abs() < 1e-5);
    }
}
