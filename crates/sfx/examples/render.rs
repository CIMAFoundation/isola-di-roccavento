//! Writes the sounds to WAV files, to listen to them outside the game:
//! `cargo run --release -p sfx --example render -- <folder>`.
//! Each ambience layer alone for 10 s, a mix that walks from calm to the fire
//! and into a pause, and every cue. The recorded beds are decoded from
//! `data/sounds` with ffmpeg (the game decodes them itself).

use std::sync::Arc;

use sfx::{Ambience, Bed, Cue, Layer, Levels, Loop, RATE};

/// The game's beds, decoded by ffmpeg (mono, 32 kHz).
fn levels() -> Arc<Levels> {
    let levels = Arc::new(Levels::default());
    for b in Bed::ALL {
        let path = format!("data/sounds/{}.mp3", b.file());
        let out = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-i", &path, "-ac", "1", "-ar", "32000", "-f", "f32le", "-"])
            .output()
            .expect("ffmpeg");
        let x: Vec<f32> = out.stdout.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        match Loop::from_bed(&x, 32_000) {
            Some(l) => levels.set_bed(b, l),
            None => eprintln!("{path}: missing or too short"),
        }
    }
    levels
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "results/sfx".into()));
    std::fs::create_dir_all(&dir).unwrap();
    let save = |name: &str, samples: &[f32], channels: u16| {
        let p = dir.join(format!("{name}.wav"));
        std::fs::write(&p, sfx::wav(samples, channels)).unwrap();
        println!("{}", p.display());
    };
    let beds = levels();
    for (name, layer) in [
        ("vento_alberi", Layer::TreeWind),
        ("vento_aperto", Layer::OpenWind),
        ("fuoco", Layer::Fire),
        ("fuoco_forte", Layer::Blaze),
        ("mare", Layer::Sea),
    ] {
        let levels = beds.clone();
        for l in [Layer::TreeWind, Layer::OpenWind, Layer::Fire, Layer::Blaze, Layer::Sea] {
            levels.set(l, 0.0);
        }
        levels.set(Layer::Volume, 1.0);
        levels.set(Layer::Gale, 0.5);
        levels.set(layer, 0.8);
        let mut a = Ambience::new(levels, 11);
        let x: Vec<f32> = (0..RATE * 10).flat_map(|_| a.frame()).collect();
        save(&format!("ambiente_{name}"), &x, 2);
    }
    // 0-8 s calm hillside by the sea; 8-16 s the fire comes near; 16-22 s pause
    let levels = beds;
    levels.set(Layer::Volume, 1.0);
    levels.set(Layer::Gale, 0.5);
    let mut a = Ambience::new(levels.clone(), 5);
    let mut x = Vec::new();
    for i in 0..RATE * 22 {
        if i % 512 == 0 {
            let t = i as f32 / RATE as f32;
            let fire = ((t - 8.0) / 6.0).clamp(0.0, 1.0);
            let blaze = ((t - 11.0) / 4.0).clamp(0.0, 1.0);
            levels.set(Layer::OpenWind, 0.3 * (1.0 - fire));
            levels.set(Layer::TreeWind, 0.3 * fire);
            levels.set(Layer::Sea, 0.5 * (1.0 - fire));
            levels.set(Layer::Fire, fire * (1.0 - blaze).sqrt());
            levels.set(Layer::Blaze, fire * blaze.sqrt());
            levels.set(Layer::Muffle, if t > 16.0 { 1.0 } else { 0.0 });
        }
        x.extend(a.frame());
    }
    save("ambiente_mix", &x, 2);
    // a fire close by, and «case colpite» at 3 s and 7 s: the ambience ducks
    // under it (levels as the game sets them by default, 2026-11-02)
    levels.set(Layer::Muffle, 0.0);
    levels.set(Layer::Volume, 0.35);
    levels.set(Layer::Sea, 0.0);
    levels.set(Layer::TreeWind, 0.2);
    levels.set(Layer::OpenWind, 0.0);
    levels.set(Layer::Fire, 0.6);
    levels.set(Layer::Blaze, 0.6);
    let mut a = Ambience::new(levels.clone(), 8);
    let alert = sfx::render(Cue::Alert);
    let cue_gain = 0.4 * 10f32.powf(3.5 / 20.0);
    let starts = [3 * RATE as usize, 7 * RATE as usize];
    let mut x = Vec::new();
    for i in 0..RATE as usize * 10 {
        if i % 512 == 0 {
            let on = starts.iter().any(|&s| i >= s && i < s + alert.len());
            levels.set(Layer::Duck, if on { 1.0 } else { 0.0 });
        }
        let [l, r] = a.frame();
        let c = starts.iter().filter_map(|&s| alert.get(i.wrapping_sub(s))).sum::<f32>() * cue_gain;
        x.extend([l + c, r + c]);
    }
    save("ambiente_e_avviso", &x, 2);
    // the camera turns: the fire goes from the left ear to the right in 10 s
    levels.set(Layer::Duck, 0.0);
    levels.set(Layer::TreeWind, 0.0);
    let mut a = Ambience::new(levels.clone(), 12);
    let mut x = Vec::new();
    for i in 0..RATE as usize * 12 {
        if i % 512 == 0 {
            let t = i as f32 / RATE as f32;
            levels.set(Layer::FirePan, (0.8 * ((t - 1.0) / 5.0 - 1.0)).clamp(-0.8, 0.8));
        }
        x.extend(a.frame());
    }
    save("fuoco_che_gira", &x, 2);
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
