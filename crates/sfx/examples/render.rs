//! Writes the sounds to WAV files, to listen to them outside the game:
//! `cargo run --release -p sfx --example render -- <folder>`.
//! Each ambience layer alone for 10 s, a mix that walks from calm to the fire
//! and into a pause, and every cue.

use std::sync::Arc;

use sfx::{Ambience, Cue, Layer, Levels, RATE};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "results/sfx".into()));
    std::fs::create_dir_all(&dir).unwrap();
    let save = |name: &str, samples: &[f32], channels: u16| {
        let p = dir.join(format!("{name}.wav"));
        std::fs::write(&p, sfx::wav(samples, channels)).unwrap();
        println!("{}", p.display());
    };
    for (name, layer) in [("vento", Layer::Wind), ("fuoco", Layer::Fire), ("mare", Layer::Sea)] {
        let levels = Arc::new(Levels::default());
        levels.set(Layer::Volume, 1.0);
        levels.set(layer, 0.8);
        let mut a = Ambience::new(levels, 11);
        let x: Vec<f32> = (0..RATE * 10).flat_map(|_| a.frame()).collect();
        save(&format!("ambiente_{name}"), &x, 2);
    }
    // 0-8 s calm hillside by the sea; 8-16 s the fire comes near; 16-22 s pause
    let levels = Arc::new(Levels::default());
    levels.set(Layer::Volume, 1.0);
    let mut a = Ambience::new(levels.clone(), 5);
    let mut x = Vec::new();
    for i in 0..RATE * 22 {
        if i % 512 == 0 {
            let t = i as f32 / RATE as f32;
            let fire = ((t - 8.0) / 6.0).clamp(0.0, 1.0);
            levels.set(Layer::Wind, 0.35 + 0.35 * fire);
            levels.set(Layer::Sea, 0.5 * (1.0 - fire));
            levels.set(Layer::Fire, fire);
            levels.set(Layer::Muffle, if t > 16.0 { 1.0 } else { 0.0 });
        }
        x.extend(a.frame());
    }
    save("ambiente_mix", &x, 2);
    for (name, cue) in [
        ("radio_apre", Cue::RadioOpen),
        ("radio_chiude", Cue::RadioClose),
        ("evacuazione", Cue::Evacuate),
        ("case_colpite", Cue::Alert),
        ("persone_in_pericolo", Cue::Danger),
        ("squadra_si_ritira", Cue::Withdraw),
        ("raffica", Cue::Gust),
        ("evacuazione_finita", Cue::Done),
        ("stella_1", Cue::Star(0)),
        ("stella_2", Cue::Star(1)),
        ("stella_3", Cue::Star(2)),
    ] {
        save(&format!("cue_{name}"), &sfx::render(cue), 1);
    }
}
