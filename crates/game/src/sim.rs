//! The kiosk's handle on the one simulation and gameplay authority,
//! [`rocca::Game`]. Nothing here steps a model or decides anything: it only
//! turns frame time into game steps at the chosen speed, and counts
//! generations so views rebuild when the state changed.

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use rocca::{Game, STEP_S};

#[derive(Resource)]
pub struct Sim {
    pub game: Game,
    pub data: PathBuf,
    /// Simulated seconds per real second; 0 = the world stands still.
    pub speed: f32,
    accumulator: f32,
    /// Bumped whenever the game state changes, so views rebuild only then.
    /// Monotonic across restarts: a staleness token, not a step count.
    pub generation: u64,
}

/// Raised on the frame a new game started, so views holding state the game no
/// longer explains (a burning roof, a vehicle entity) can drop it.
#[derive(Message)]
pub struct SimRestarted;

impl Deref for Sim {
    type Target = Game;
    fn deref(&self) -> &Game {
        &self.game
    }
}

impl DerefMut for Sim {
    fn deref_mut(&mut self) -> &mut Game {
        &mut self.game
    }
}

/// Most game steps a frame may run, so a slow step cannot spiral.
const MAX_STEPS_PER_FRAME: usize = 40;

impl Sim {
    pub fn new(data: &Path, case: &str, seed: u64) -> anyhow::Result<Sim> {
        Ok(Sim { game: Game::new(data, case, seed)?, data: data.to_path_buf(), speed: 0.0, accumulator: 0.0, generation: 0 })
    }

    /// A new game on the same territory: `case` and `seed` from scratch. The
    /// operator's manual restart; nothing else calls it.
    pub fn restart(&mut self, case: &str, seed: u64) -> anyhow::Result<()> {
        self.game = Game::new(&self.data, case, seed)?;
        self.speed = 0.0;
        self.accumulator = 0.0;
        self.generation += 1;
        Ok(())
    }

    /// The simulated second to draw at: the game's time and the part of a
    /// step the clock has run since (less than [`STEP_S`]). Only the drawing
    /// reads it (`fire_ahead`); the game is where its last step left it.
    pub fn draw_time_s(&self) -> f64 {
        self.game.time_s() as f64 + f64::from(self.accumulator.clamp(0.0, STEP_S as f32))
    }

    /// Fast automation clock: fixed model steps, stopping exactly at a decision
    /// or the end. Does not accumulate wall time or skip simulation work.
    #[cfg(any(test, target_arch = "wasm32"))]
    pub fn advance_steps(&mut self, max_steps: usize) -> anyhow::Result<usize> {
        let mut count = 0;
        while count < max_steps && !self.game.is_over() {
            self.game.step()?;
            count += 1;
        }
        if count > 0 { self.generation += 1; }
        Ok(count)
    }

    /// Advance by `real_s` of wall time at the current speed. Effects depend on
    /// simulated time only: steps are always [`STEP_S`] long.
    pub fn tick(&mut self, real_s: f32) -> anyhow::Result<bool> {
        if self.speed <= 0.0 {
            return Ok(false);
        }
        self.accumulator += real_s * self.speed;
        let mut stepped = false;
        for _ in 0..MAX_STEPS_PER_FRAME {
            if self.accumulator < STEP_S as f32 || self.game.is_over() {
                break;
            }
            self.game.step()?;
            self.accumulator -= STEP_S as f32;
            stepped = true;
        }
        // Behind by more than a frame's cap: drop the backlog, keep real time.
        self.accumulator = self.accumulator.min(STEP_S as f32);
        if stepped {
            self.generation += 1;
        }
        Ok(stepped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tutorial's orders, then the game to its end at `speed`, `frame_s`
    /// of wall time a frame (or `batch` steps a frame).
    fn play(speed: f32, frame_s: f32, batch: Option<usize>) -> rocca::Outcome {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut sim = Sim::new(&data, "tutorial", 1).unwrap();
        let script = rocca::tutorial::Script::of(&sim).unwrap();
        let orders = rocca::tutorial::Script { steps: script.steps.into_iter().filter(|s| s.expect != rocca::tutorial::Expect::Over).collect() };
        orders.play(&mut sim).unwrap();
        sim.speed = speed;
        while !sim.is_over() {
            if let Some(batch) = batch { sim.advance_steps(batch).unwrap(); } else { sim.tick(frame_s).unwrap(); }
        }
        sim.outcome()
    }

    /// Effects depend on simulated time, not on the speed or the frame rate.
    #[test]
    fn the_speed_does_not_change_the_game() {
        let slow = play(60.0, 1.0 / 40.0, None);
        assert_eq!(play(300.0, 1.0 / 40.0, None), slow);
        assert_eq!(play(300.0, 0.7, None), slow);
        assert_eq!(play(0.0, 0.0, Some(10_000)), slow);
    }
}
