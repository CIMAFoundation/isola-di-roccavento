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
/// No judgement for this long after the start and after every change: shaders
/// compile and caches fill.
const SETTLE_S: f32 = 3.0;
/// A frame longer than this is a hitch (loading, a hidden tab): not measured.
const HITCH_S: f32 = 0.25;
/// Calm time (s) before a probe upwards.
const CALM_S: f32 = 12.0;

/// The rungs, best first. Consecutive rungs that would be the same are one.
pub fn ladder(top: Graphics) -> Vec<Graphics> {
    let ops: [fn(&mut Graphics); 10] = [
        |g| g.dof = false,
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
        |g| g.veg_keep = g.veg_keep.min(0.35),
        |g| g.bloom = false,
        |g| g.smaa = false,
        |g| g.terrain_shadows = false,
        |g| g.shadows = false,
        |g| g.veg_keep = g.veg_keep.min(0.2),
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
}

impl Controller {
    pub fn new(len: usize, start: usize) -> Self {
        let step = start.min(len.saturating_sub(1));
        Controller { len, step, best: 0, settle: SETTLE_S, elapsed: 0.0, frames: Vec::new(), calm: 0.0, probing: false }
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
        if self.elapsed < WINDOW_S || self.frames.len() < WINDOW_FRAMES {
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
            }
            if to != self.step {
                self.move_to(to);
                return Some(to);
            }
            return None;
        }
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
}

impl Adaptive {
    pub fn new(top: Graphics) -> Self {
        let ladder = ladder(top);
        let start = crate::quality::saved_step().unwrap_or(0);
        let ctl = Controller::new(ladder.len(), start);
        let written = rung(&ladder, ctl.step, &top);
        Adaptive { ctl, ladder, written, saved_step: start }
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
        if step != auto.saved_step {
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
        for _ in 0..10 {
            assert_eq!(c.feed(1.5), None); // a tab coming back
        }
        assert!(run(&mut c, 0.016, 20.0).is_empty());
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
