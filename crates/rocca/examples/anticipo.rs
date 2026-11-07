//! What the drawn front's lookahead costs and how far it is from the fire
//! the game then has (`Game::lookahead_job`, `game::fire_ahead`).
//!
//! The medium case without orders, seeds 1-3: every `EVERY_S` a member runs
//! `HORIZON_S` ahead; then the game itself goes on, and the cells either
//! lights are compared. Per run: ms the member took, cells lit by both, by
//! the guess only, by the game only, and the median and 90th percentile of
//! |guessed - real| arrival (s) over the cells both lit.
//!
//! ```sh
//! cargo run --release -p rocca --example anticipo
//! ```

use std::path::Path;
use std::time::Instant;

use rocca::{Game, STEP_S};

const HORIZON_S: i64 = 180;
const EVERY_S: i64 = 600;
const UNTIL_S: i64 = 4 * 3600;

fn main() -> anyhow::Result<()> {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    println!("seme  t(min)  ms   entrambi  solo_previsti  solo_reali  |dt| mediana  p90");
    for seed in 1..=3u64 {
        let mut game = Game::new(&data, "medium", seed)?;
        while game.time_s() < UNTIL_S && !game.is_over() {
            let now = game.time_s();
            if now > 0 && now % EVERY_S == 0 {
                let t = Instant::now();
                let (snap, member) = game.lookahead_job()?;
                let guess = snap.run(&member, HORIZON_S)?;
                let ms = t.elapsed().as_secs_f64() * 1e3;
                let before: Vec<i32> = game.fire.arrival_times().to_vec();
                // the game the same span on
                while game.time_s() < now + HORIZON_S {
                    game.step()?;
                }
                let real = game.fire.arrival_times();
                let (mut both, mut only_g, mut only_r) = (0, 0, 0);
                let mut err = Vec::new();
                for i in 0..real.len() {
                    if before[i] != i32::MIN {
                        continue;
                    }
                    match (guess[i] != i32::MIN, real[i] != i32::MIN) {
                        (true, true) => {
                            both += 1;
                            err.push((guess[i] - real[i]).abs());
                        }
                        (true, false) => only_g += 1,
                        (false, true) => only_r += 1,
                        _ => {}
                    }
                }
                err.sort_unstable();
                let q = |f: f64| err.get(((err.len() as f64 - 1.0) * f).round() as usize).copied().unwrap_or(0);
                println!("{seed:>4} {:>7} {ms:>4.0} {both:>9} {only_g:>14} {only_r:>11} {:>13} {:>4}", now / 60, q(0.5), q(0.9));
                continue;
            }
            game.step()?;
            debug_assert_eq!(STEP_S, 6);
        }
    }
    Ok(())
}
