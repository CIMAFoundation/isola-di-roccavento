//! Adaptive graphics: the level follows the frame time of the machine it runs on.
//!
//! A ladder of [`Graphics`] is built from the ceiling (the preset the machine
//! would get if it were fast), each rung removing the next most expensive thing
//! that can be changed while the game runs (see `quality_live`). The plant
//! share, the terrain stride and the ground cover are read when the scene is
//! built, so they stay what the launch chose.
//!
//! The [`Controller`] is pure (frame times in, rung out) so it is tested with
//! synthetic frames. Going down is quick: a slow window is enough. Going up is
//! a probe, because under vsync a machine at 60 FPS shows no headroom: after a
//! long calm stretch one rung up is tried, and if the next window is slow it
//! goes back and that rung is never tried again in this run.
//!
//! The resolution of the 3D scene is one of the levers (user, 2026-11-02: at
//! least 30 FPS on the desktop and in the browser): a little lower early, where
//! a dense screen hides it, down to 0.4 per side at the bottom (`render_scale`).
//! A descent that does not make the frames shorter is undone: the time went
//! elsewhere (the CPU, or a browser pacing the page at 30 Hz, as an iPhone in
//! low-power mode does), and going lower would only blur and thin the scene.
//! It is judged over a few rungs, not one: the lever that helps a machine may
//! sit lower down (a browser without a GPU gained nothing from the plants and
//! the resolution, and a lot from the shadows).

use bevy::prelude::*;
use bevy::time::Real;

use crate::quality::Graphics;

/// Median frame time (s) above which the level goes down (about 42 FPS).
const SLOW_S: f32 = 0.024;
/// Above this the machine is far off: two rungs at once.
const VERY_SLOW_S: f32 = 0.050;
/// Median frame time (s) under which a probe upwards is allowed (about 55 FPS).
const FAST_S: f32 = 0.0185;
/// Measuring window (s), and the least frames in it.
const WINDOW_S: f32 = 2.0;
const WINDOW_FRAMES: usize = 20;
/// A machine too slow to fill a window closes it after this long (s) and
/// this many frames.
const WINDOW_LONG_S: f32 = 4.0;
const WINDOW_FEW_FRAMES: usize = 4;
/// No judgement for this long after the start and after every change: shaders
/// compile and caches fill.
const SETTLE_S: f32 = 3.0;
/// A frame longer than this is a hitch (loading, a hidden tab): not measured,
/// unless such frames go on for [`HITCH_RUN_S`]: then the machine is that slow
/// (a browser without a GPU drew the title at 1 FPS, 2026-11-02, and the
/// level never moved).
const HITCH_S: f32 = 0.25;
const HITCH_RUN_S: f32 = 2.0;
/// Calm time (s) before a probe upwards.
const CALM_S: f32 = 12.0;
/// A step down pays when it shortens the median frame by this share.
const GAIN: f32 = 0.08;
/// Rungs tried below the last one that paid before the descent is undone.
const TRIES: usize = 3;
/// After an undone descent, frames this much longer than then (conditions
/// changed: a bigger fire, another window) allow a new one.
const RETRY: f32 = 1.25;

/// The rungs, best first. Consecutive rungs that would be the same are one.
pub fn ladder(top: Graphics) -> Vec<Graphics> {
    let ops: [fn(&mut Graphics); 15] = [
        |g| g.dof = false,
        // the scene a little smaller first: on a dense screen it hardly shows
        |g| g.render_scale = g.render_scale.min(0.85),
        |g| g.veg_keep = g.veg_keep.min(0.6),
        |g| {
            if g.msaa >= 4 {
                g.set_aa("smaa");
            }
        },
        |g| {
            g.cascades = g.cascades.min(2);
            g.shadow_distance = g.shadow_distance.min(1800.0);
            g.shadow_map = g.shadow_map.min(1024);
        },
        |g| g.render_scale = g.render_scale.min(0.7),
        |g| g.veg_keep = g.veg_keep.min(0.35),
        |g| g.bloom = false,
        |g| g.smaa = false,
        |g| g.terrain_shadows = false,
        |g| g.shadows = false,
        |g| g.render_scale = g.render_scale.min(0.6),
        |g| g.veg_keep = g.veg_keep.min(0.2),
        // the floor, for the slowest screens
        |g| g.render_scale = g.render_scale.min(0.5),
        |g| g.render_scale = g.render_scale.min(0.4),
    ];
    let mut out = vec![top];
    let mut g = top;
    for op in ops {
        op(&mut g);
        if out.last() != Some(&g) {
            out.push(g);
        }
    }
    out
}

pub struct Controller {
    len: usize,
    /// The current rung: 0 is the best.
    pub step: usize,
    /// The best rung still allowed: a failed probe moves it down.
    best: usize,
    settle: f32,
    elapsed: f32,
    frames: Vec<f32>,
    calm: f32,
    probing: bool,
    /// During a descent, the last rung that paid and its median frame.
    before: Option<(usize, f32)>,
    /// The median frame when a descent was undone: no new one below `RETRY` times it.
    held: Option<f32>,
    /// Time (s) of the hitches in a row so far.
    hitches: f32,
}

impl Controller {
    pub fn new(len: usize, start: usize) -> Self {
        let step = start.min(len.saturating_sub(1));
        Controller { len, step, best: 0, settle: SETTLE_S, elapsed: 0.0, frames: Vec::new(), calm: 0.0, probing: false, before: None, held: None, hitches: 0.0 }
    }

    /// A new scene: shaders compile and caches fill again, as at the start.
    pub fn resettle(&mut self) {
        self.settle = SETTLE_S;
        self.elapsed = 0.0;
        self.frames.clear();
        self.calm = 0.0;
        self.hitches = 0.0;
        self.before = None;
        self.held = None;
    }

    fn move_to(&mut self, step: usize) {
        self.step = step;
        self.settle = SETTLE_S;
        self.elapsed = 0.0;
        self.frames.clear();
        self.calm = 0.0;
    }

    /// One frame of `dt` seconds of real time; the new rung if it changes.
    pub fn feed(&mut self, dt: f32) -> Option<usize> {
        if self.len < 2 {
            return None;
        }
        if dt > HITCH_S {
            self.hitches += dt;
        } else {
            self.hitches = 0.0;
        }
        if dt > HITCH_S && self.hitches < HITCH_RUN_S {
            // loading, a hidden tab, a window drag: forget the window, wait a little
            self.settle = self.settle.max(1.0);
            self.elapsed = 0.0;
            self.frames.clear();
            self.calm = 0.0;
            return None;
        }
        if self.settle > 0.0 {
            self.settle -= dt;
            return None;
        }
        self.elapsed += dt;
        self.frames.push(dt);
        let full = self.frames.len() >= WINDOW_FRAMES || (self.elapsed >= WINDOW_LONG_S && self.frames.len() >= WINDOW_FEW_FRAMES);
        if self.elapsed < WINDOW_S || !full {
            return None;
        }
        self.frames.sort_by(f32::total_cmp);
        let median = self.frames[self.frames.len() / 2];
        let window = self.elapsed;
        self.elapsed = 0.0;
        self.frames.clear();

        if median > SLOW_S {
            let down = if median > VERY_SLOW_S { 2 } else { 1 };
            let to = (self.step + down).min(self.len - 1);
            if self.probing {
                // the probe failed: this rung is out of reach for this run
                self.best = (self.step + 1).min(self.len - 1);
                self.probing = false;
            } else if self.held.is_some_and(|h| median < h * RETRY) {
                return None;
            } else if let Some((from, was)) = self.before {
                if median <= was * (1.0 - GAIN) {
                    // this rung paid: judge the next ones from here
                    self.before = Some((self.step, median));
                } else if self.step >= from + TRIES || to == self.step {
                    // rungs down and the frames as long: the time is elsewhere
                    self.before = None;
                    self.held = Some(median);
                    if from != self.step {
                        self.move_to(from);
                        return Some(from);
                    }
                    return None;
                }
            } else {
                self.held = None;
                self.before = Some((self.step, median));
            }
            if to != self.step {
                self.move_to(to);
                return Some(to);
            }
            return None;
        }
        // fast enough: a descent is over, and a new one may start some day
        self.before = None;
        self.held = None;
        if self.probing {
            self.probing = false; // survived
        }
        if median <= FAST_S && self.step > self.best {
            self.calm += window;
            if self.calm >= CALM_S {
                let to = self.step - 1;
                self.move_to(to);
                self.probing = true;
                return Some(to);
            }
        } else {
            self.calm = 0.0;
        }
        None
    }
}

/// The adaptive state in the app. Absent when the level was chosen by hand.
#[derive(Resource)]
pub struct Adaptive {
    ctl: Controller,
    ladder: Vec<Graphics>,
    /// What this system last wrote: anything else in `Graphics` is the operator's.
    written: Graphics,
    saved_step: usize,
    /// Whether the rung is read at launch and remembered: not in a snapshot
    /// run (`KIOSK_ADAPT=1`), which starts from the top and leaves the
    /// player's saved rung alone.
    persist: bool,
}

impl Adaptive {
    pub fn new(top: Graphics) -> Self {
        let ladder = ladder(top);
        let persist = std::env::var_os("KIOSK_SNAP").is_none();
        let start = if persist { crate::quality::saved_step().unwrap_or(0) } else { 0 };
        let ctl = Controller::new(ladder.len(), start);
        let written = rung(&ladder, ctl.step, &top);
        Adaptive { ctl, ladder, written, saved_step: start, persist }
    }

    /// The graphics to start with (the saved rung, if any).
    pub fn initial(&self) -> Graphics {
        self.written
    }

    pub fn step(&self) -> usize {
        self.ctl.step
    }

    pub fn rungs(&self) -> usize {
        self.ladder.len()
    }
}

/// The rung `step`, with what is read only at launch taken from `now`.
fn rung(ladder: &[Graphics], step: usize, now: &Graphics) -> Graphics {
    Graphics {
        vegetation: now.vegetation,
        terrain_stride: now.terrain_stride,
        cover_m: now.cover_m,
        lobes: now.lobes,
        ..ladder[step.min(ladder.len() - 1)]
    }
}

/// Entering a game (the first, or after a restart, which goes through the
/// loading state where the controller does not run): wait as at the start.
pub fn resettle(auto: Option<ResMut<Adaptive>>) {
    if let Some(mut auto) = auto {
        auto.ctl.resettle();
    }
}

pub fn adapt(
    time: Res<Time<Real>>,
    auto: Option<ResMut<Adaptive>>,
    mut gfx: ResMut<Graphics>,
    mut commands: Commands,
) {
    let Some(mut auto) = auto else { return };
    if *gfx != auto.written {
        // the operator picked a level or an antialiasing: not ours any more
        commands.remove_resource::<Adaptive>();
        return;
    }
    if let Some(step) = auto.ctl.feed(time.delta_secs()) {
        let g = rung(&auto.ladder, step, &gfx);
        info!("grafica adattiva: livello {}/{} ({g:?})", step + 1, auto.ladder.len());
        auto.written = g;
        *gfx = g;
        if step != auto.saved_step && auto.persist {
            auto.saved_step = step;
            crate::quality::save_step(step);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `seconds` of frames of `dt`, return every rung change.
    fn run(c: &mut Controller, dt: f32, seconds: f32) -> Vec<usize> {
        let mut out = vec![];
        let mut t = 0.0;
        while t < seconds {
            if let Some(s) = c.feed(dt) {
                out.push(s);
            }
            t += dt;
        }
        out
    }

    #[test]
    fn ladder_goes_from_the_top_to_no_shadows_without_repeats() {
        let l = ladder(Graphics::ALTA);
        assert_eq!(l[0], Graphics::ALTA);
        assert!(!l.last().unwrap().shadows && !l.last().unwrap().bloom && !l.last().unwrap().dof);
        assert!(l.windows(2).all(|w| w[0] != w[1]));
        // what is read at launch is never touched
        assert!(l.iter().all(|g| g.vegetation == Graphics::ALTA.vegetation && g.terrain_stride == 1));
        // a ceiling without the expensive things has a shorter ladder
        assert!(ladder(Graphics::WEB).len() < l.len());
        // the scene's resolution only goes down, to the floor, and the depth of
        // field (which reads the depth at screen size) is off before it starts
        for top in [Graphics::ALTA, Graphics::WEB] {
            let l = ladder(top);
            assert_eq!(l[0].render_scale, 1.0);
            assert_eq!(l.last().unwrap().render_scale, 0.4);
            assert!(l.windows(2).all(|w| w[1].render_scale <= w[0].render_scale));
            assert!(l.iter().all(|g| !g.dof || crate::render_scale::whole(g.render_scale)));
        }
    }

    #[test]
    fn a_descent_that_does_not_help_is_undone() {
        // a browser paced at 30 Hz: every rung shows 33 ms
        let mut c = Controller::new(8, 0);
        assert_eq!(run(&mut c, 1.0 / 30.0, 40.0), vec![1, 2, 3, 0], "three rungs down, back, then it holds");
        assert!(run(&mut c, 1.0 / 30.0, 120.0).is_empty());
        // frames a quarter longer than then: worth another look
        assert_eq!(run(&mut c, 0.045, 10.0).first(), Some(&1));
    }

    #[test]
    fn a_lever_further_down_is_reached() {
        // the first two rungs change nothing, the third (the shadows) a lot
        let mut c = Controller::new(8, 0);
        let mut t = 0.0;
        while t < 60.0 {
            let dt = if c.step >= 3 { 0.016 } else { 0.040 };
            c.feed(dt);
            t += dt;
        }
        assert_eq!(c.step, 3);
    }

    #[test]
    fn a_fast_machine_stays_on_top() {
        let mut c = Controller::new(6, 0);
        assert!(run(&mut c, 1.0 / 144.0, 60.0).is_empty());
    }

    #[test]
    fn a_slow_machine_comes_down_to_a_stable_rung() {
        // 40 ms per frame at the top, each rung down saves 12 ms: ok from rung 2 (16 ms)
        let mut c = Controller::new(6, 0);
        let mut seen = vec![];
        for _ in 0..40 {
            let dt = 0.040 - 0.012 * c.step as f32;
            if let Some(s) = c.feed(dt.max(0.008)) {
                seen.push(s);
            }
            for _ in 0..200 {
                if let Some(s) = c.feed(dt.max(0.008)) {
                    seen.push(s);
                }
            }
        }
        assert_eq!(c.step, 2, "{seen:?}");
    }

    #[test]
    fn a_hitch_does_not_count() {
        let mut c = Controller::new(6, 0);
        run(&mut c, 0.016, 5.0);
        // a tab coming back, a window dragged: a frame or two, long
        for _ in 0..2 {
            assert_eq!(c.feed(0.9), None);
        }
        assert!(run(&mut c, 0.016, 20.0).is_empty());
    }

    #[test]
    fn a_machine_at_one_fps_comes_down() {
        // a second per frame at the top, each rung down 40 % shorter
        let mut c = Controller::new(9, 0);
        let mut t = 0.0;
        while t < 90.0 {
            let dt = 1.0 * 0.6f32.powi(c.step as i32);
            c.feed(dt);
            t += dt;
        }
        // 1.0, 0.6, 0.36, 0.22, 0.13, 0.078, 0.047, 0.028, 0.017 s: the first
        // rung under 24 ms is 8; at least it must be well on its way
        assert!(c.step >= 6, "rung {}", c.step);
    }

    #[test]
    fn it_probes_up_after_a_calm_stretch_and_keeps_a_rung_that_holds() {
        let mut c = Controller::new(4, 2);
        let ups = run(&mut c, 0.012, 60.0);
        assert_eq!(ups.first(), Some(&1));
        assert_eq!(c.step, 0); // 12 ms everywhere: it climbs to the top and stays
    }

    #[test]
    fn a_failed_probe_goes_back_and_is_not_repeated() {
        // the top rung costs 30 ms, the others 14 ms
        let mut c = Controller::new(4, 1);
        let mut changes = vec![];
        for _ in 0..8000 {
            let dt = if c.step == 0 { 0.030 } else { 0.014 };
            if let Some(s) = c.feed(dt) {
                changes.push(s);
            }
        }
        assert_eq!(changes, vec![0, 1], "up once, back once, then quiet");
        assert_eq!(c.step, 1);
    }

    #[test]
    fn a_very_slow_machine_skips_a_rung() {
        let mut c = Controller::new(6, 0);
        let first = run(&mut c, 0.07, 30.0);
        assert_eq!(first.first(), Some(&2));
    }

    #[test]
    fn a_single_rung_never_moves() {
        let mut c = Controller::new(1, 0);
        assert!(run(&mut c, 0.1, 30.0).is_empty());
    }
}
