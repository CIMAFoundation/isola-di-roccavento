//! The game's sounds made in code (user, 2026-10-30): the ambience follows
//! the simulation. Gulls and cicadas are recordings (`data/sounds`), the game
//! plays them beside this. Pure DSP, std only:
//! the game plays [`Ambience`] as an endless stream and the [`Cue`]s as short
//! clips; `examples/render.rs` writes them to WAV files to listen to.
//!
//! Every sound is also shown on screen: the game must stay playable muted
//! (docs/01-GIOCO.md).

use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub const RATE: u32 = 44_100;
const SR: f32 = RATE as f32;

/// Small, fast, deterministic noise.
#[derive(Clone)]
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }
    /// White noise in [-1, 1).
    fn white(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// One-pole low-pass.
#[derive(Clone, Default)]
struct OnePole {
    a: f32,
    y: f32,
}

impl OnePole {
    fn new(fc: f32) -> OnePole {
        let mut f = OnePole::default();
        f.set(fc);
        f
    }
    fn set(&mut self, fc: f32) {
        self.a = 1.0 - (-TAU * fc / SR).exp();
    }
    fn run(&mut self, x: f32) -> f32 {
        self.y += self.a * (x - self.y);
        self.y
    }
}

/// State-variable filter (topology-preserving): low, band and high at once.
#[derive(Clone, Default)]
struct Svf {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    ic1: f32,
    ic2: f32,
}

struct Bands {
    low: f32,
    band: f32,
    high: f32,
}

impl Svf {
    fn new(fc: f32, q: f32) -> Svf {
        let mut f = Svf::default();
        f.set(fc, q);
        f
    }
    fn set(&mut self, fc: f32, q: f32) {
        let g = (PI * fc.clamp(20.0, SR * 0.45) / SR).tan();
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    fn run(&mut self, x: f32) -> Bands {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        Bands { low: v2, band: v1, high: x - self.k * v1 - v2 }
    }
}

/// A value that wanders: a new random target every `lo..hi` seconds, reached
/// smoothly. Gusts, flicker, the swell of the cicadas.
#[derive(Clone)]
struct Wander {
    now: f32,
    target: f32,
    left: f32,
    glide: OnePole,
    lo: f32,
    hi: f32,
}

impl Wander {
    fn new(lo: f32, hi: f32, glide_hz: f32) -> Wander {
        Wander { now: 0.5, target: 0.5, left: 0.0, glide: OnePole { a: 1.0 - (-TAU * glide_hz / SR).exp(), y: 0.5 }, lo, hi }
    }
    fn run(&mut self, rng: &mut Rng) -> f32 {
        self.left -= 1.0 / SR;
        if self.left <= 0.0 {
            self.left = rng.range(self.lo, self.hi);
            self.target = rng.unit();
        }
        self.now = self.glide.run(self.target);
        self.now
    }
}

/// The levels the game asks for, each 0..1, written from the main thread and
/// read by the audio thread (lock-free: one `f32` per atomic).
#[derive(Default)]
pub struct Levels([AtomicU32; Layer::COUNT]);

#[derive(Clone, Copy, Debug)]
pub enum Layer {
    /// The wind in the trees: the weather's speed, louder from high up.
    Wind,
    /// The fire near what the camera looks at.
    Fire,
    /// The surf, near the coast.
    Sea,
    /// The pause: the world holds its breath, muffled and lower.
    Muffle,
    /// The ambience's own volume (the player's choice).
    Volume,
}

impl Layer {
    const COUNT: usize = 5;
}

impl Levels {
    pub fn set(&self, layer: Layer, v: f32) {
        self.0[layer as usize].store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn get(&self, layer: Layer) -> f32 {
        f32::from_bits(self.0[layer as usize].load(Ordering::Relaxed))
    }
}

/// Frames between reads of [`Levels`] and filter updates.
const BLOCK: usize = 64;

/// A crackle of the fire: a few clicks of filtered noise, close together.
#[derive(Clone, Default)]
struct Crackle {
    left: usize,
    amp: f32,
    decay: f32,
    hp: Svf,
    pan: f32,
    clicks: u8,
    gap: usize,
}

/// The ambience of the island, endless and stereo: wind, fire and sea
/// mixed at the [`Levels`] the game sets.
pub struct Ambience {
    levels: Arc<Levels>,
    rng: Rng,
    /// Levels as heard: they glide to the asked ones in about a second.
    heard: [f32; Layer::COUNT],
    n: usize,
    // wind
    gust: Wander,
    wind_bp: [Svf; 2],
    wind_low: [OnePole; 2],
    whistle: Svf,
    // fire
    flicker: Wander,
    roar_lp: OnePole,
    roar_bp: Svf,
    crackles: Vec<Crackle>,
    // sea
    waves: [Wave; 2],
    // the pause
    muffle: [OnePole; 2],
}

#[derive(Clone)]
struct Wave {
    t: f32,
    period: f32,
    rise: f32,
    size: f32,
    lp: Svf,
    fizz: Svf,
    pan: f32,
}

impl Wave {
    fn new(rng: &mut Rng, pan: f32) -> Wave {
        let mut w = Wave { t: rng.range(0.0, 6.0), period: 8.0, rise: 2.0, size: 1.0, lp: Svf::new(400.0, 0.7), fizz: Svf::new(3000.0, 0.7), pan };
        w.next(rng);
        w
    }
    fn next(&mut self, rng: &mut Rng) {
        self.period = rng.range(6.5, 11.0);
        self.rise = rng.range(1.4, 2.6);
        self.size = rng.range(0.55, 1.0);
    }
    /// The swell of this wave now, 0..1.
    fn envelope(&self) -> f32 {
        if self.t < self.rise {
            let x = self.t / self.rise;
            x * x
        } else {
            (-(self.t - self.rise) / (0.28 * self.period)).exp()
        }
    }
}

impl Ambience {
    pub fn new(levels: Arc<Levels>, seed: u32) -> Ambience {
        let mut rng = Rng(seed.max(1));
        let waves = [Wave::new(&mut rng, -0.35), Wave::new(&mut rng, 0.35)];
        Ambience {
            levels,
            heard: [0.0; Layer::COUNT],
            n: 0,
            gust: Wander::new(1.2, 4.5, 0.35),
            wind_bp: [Svf::new(500.0, 1.1), Svf::new(560.0, 1.1)],
            wind_low: [OnePole::new(140.0), OnePole::new(150.0)],
            whistle: Svf::new(1100.0, 9.0),
            flicker: Wander::new(0.15, 0.6, 2.0),
            roar_lp: OnePole::new(180.0),
            roar_bp: Svf::new(700.0, 0.8),
            crackles: vec![Crackle::default(); 10],
            waves,
            muffle: [OnePole::new(18_000.0), OnePole::new(18_000.0)],
            rng,
        }
    }

    fn block(&mut self) {
        let glide = 1.0 - (-(BLOCK as f32) / (0.8 * SR)).exp();
        for (i, h) in self.heard.iter_mut().enumerate() {
            let want = f32::from_bits(self.levels.0[i].load(Ordering::Relaxed));
            *h += glide * (want - *h);
        }
        let g = self.gust.now;
        self.wind_bp[0].set(320.0 + 650.0 * g, 1.1);
        self.wind_bp[1].set(360.0 + 700.0 * g, 1.1);
        self.whistle.set(900.0 + 700.0 * g, 9.0);
        let m = self.heard[Layer::Muffle as usize];
        // 18 kHz open, 700 Hz muffled; exponential in between
        let fc = 18_000.0 * (700.0_f32 / 18_000.0).powf(m);
        for f in &mut self.muffle {
            f.set(fc);
        }
        for w in &mut self.waves {
            let e = w.envelope();
            w.lp.set(250.0 + 1600.0 * e, 0.7);
        }
    }

    /// The next stereo frame.
    pub fn frame(&mut self) -> [f32; 2] {
        if self.n % BLOCK == 0 {
            self.block();
        }
        self.n = self.n.wrapping_add(1);
        let [wind, fire, sea, muffle, volume] = self.heard;
        let rng = &mut self.rng;
        let mut out = [0.0f32; 2];

        // Wind: band-passed noise whose pitch and loudness follow the gusts.
        let g = self.gust.run(rng);
        if wind > 0.002 {
            let amp = 2.5 * wind * (0.25 + 0.75 * g * g);
            for c in 0..2 {
                let x = rng.white();
                out[c] += amp * (0.9 * self.wind_bp[c].run(x).band + 1.6 * self.wind_low[c].run(x));
            }
            out[0] += amp * g * g * 0.25 * self.whistle.run(rng.white()).band;
            out[1] += amp * g * g * 0.25 * self.whistle.ic1;
        }

        // Fire: a low roar that flickers, and crackles.
        let fl = self.flicker.run(rng);
        if fire > 0.002 {
            let x = rng.white();
            let roar = fire * (0.5 + 0.7 * fl) * (2.2 * self.roar_lp.run(x) + 0.35 * self.roar_bp.run(x).band);
            out[0] += roar;
            out[1] += roar;
            let rate = 4.0 + 70.0 * fire * (0.6 + 0.8 * fl);
            if rng.unit() < rate / SR {
                if let Some(c) = self.crackles.iter_mut().find(|c| c.clicks == 0) {
                    let big = rng.unit().powi(3);
                    c.clicks = 1 + (rng.unit() * 4.0) as u8;
                    c.amp = fire.sqrt() * (0.15 + 0.85 * big);
                    c.hp = Svf::new(rng.range(1200.0, 4500.0) * (1.0 - 0.6 * big), 0.9);
                    c.pan = rng.range(-0.8, 0.8);
                    c.left = 0;
                    c.gap = 0;
                }
            }
            for c in self.crackles.iter_mut().filter(|c| c.clicks > 0) {
                if c.gap > 0 {
                    c.gap -= 1;
                    continue;
                }
                if c.left == 0 {
                    c.left = (rng.range(0.6, 5.0) * SR / 1000.0) as usize;
                    c.decay = (-1.0 / c.left as f32 * 4.0).exp();
                }
                let y = c.hp.run(rng.white() * c.amp).high;
                c.amp *= c.decay;
                out[0] += y * (1.0 - c.pan) * 1.2;
                out[1] += y * (1.0 + c.pan) * 1.2;
                c.left -= 1;
                if c.left == 0 {
                    c.clicks -= 1;
                    c.amp *= rng.range(0.4, 1.1);
                    c.gap = (rng.range(2.0, 25.0) * SR / 1000.0) as usize;
                }
            }
        }

        // Sea: waves that rise and draw back, each with its fizz.
        for w in &mut self.waves {
            w.t += 1.0 / SR;
            if w.t > w.period {
                w.t = 0.0;
                w.next(rng);
            }
            if sea > 0.002 {
                let e = w.envelope() * w.size;
                let x = rng.white();
                let y = sea * (0.25 + e) * 0.9 * w.lp.run(x).low + sea * e * e * 0.12 * w.fizz.run(x).high;
                out[0] += y * (1.0 - w.pan);
                out[1] += y * (1.0 + w.pan);
            }
        }

        let gain = volume * (1.0 - 0.55 * muffle);
        [soft_clip(gain * self.muffle[0].run(out[0])), soft_clip(gain * self.muffle[1].run(out[1]))]
    }
}

fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

/// The short sounds that go with what the game shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cue {
    /// The pause: the expert's radio opens.
    RadioOpen,
    /// The world runs again: «passo», and the radio closes.
    RadioClose,
    /// An evacuation is ordered: three notes, up.
    Evacuate,
    /// A district's homes are hit: two notes falling, twice.
    Alert,
    /// People caught by the fire: three low pulses.
    Danger,
    /// A team pulls back: three quick pulses.
    Withdraw,
    /// The wind turns: a gust.
    Gust,
    /// An evacuation is over: two notes, up.
    Done,
    /// One star of the final score: a note, higher for each star.
    Star(u8),
}

/// The cue as mono samples at [`RATE`].
pub fn render(cue: Cue) -> Vec<f32> {
    let mut rng = Rng(0x9e37_79b9 ^ cue_seed(cue));
    let mut v = match cue {
        Cue::RadioOpen => {
            let mut v = squelch(&mut rng, 0.07);
            v.extend(silence(0.02));
            v.extend(beep(1000.0, 0.07, 0.35));
            v.extend(silence(0.03));
            v.extend(beep(1500.0, 0.09, 0.35));
            v
        }
        Cue::RadioClose => {
            let mut v = beep(1500.0, 0.06, 0.3);
            v.extend(silence(0.03));
            v.extend(beep(1000.0, 0.09, 0.3));
            v.extend(squelch(&mut rng, 0.12));
            v
        }
        Cue::Evacuate => {
            let mut v = Vec::new();
            for (k, hz) in [523.3, 659.3, 784.0].into_iter().enumerate() {
                add_at(&mut v, &tone(hz, 0.5, 0.6), (k as f32 * 0.11 * SR) as usize, 0.6);
            }
            v
        }
        Cue::Alert => {
            let mut v = Vec::new();
            for (k, hz) in [440.0, 370.0, 440.0, 370.0].into_iter().enumerate() {
                add_at(&mut v, &tone(hz, 0.32, 0.9), (k as f32 * 0.2 * SR) as usize, 0.6);
            }
            v
        }
        Cue::Danger => {
            let mut v = Vec::new();
            for k in 0..3 {
                add_at(&mut v, &tone(293.7, 0.24, 0.8), (k as f32 * 0.3 * SR) as usize, 0.6);
            }
            v
        }
        Cue::Withdraw => {
            let mut v = Vec::new();
            for k in 0..3 {
                add_at(&mut v, &tone(880.0, 0.09, 0.5), (k as f32 * 0.13 * SR) as usize, 0.6);
            }
            v
        }
        Cue::Gust => gust(&mut rng),
        Cue::Done => {
            let mut v = vec![0.0; (1.6 * SR) as usize];
            add_at(&mut v, &chime(784.0), 0, 0.5);
            add_at(&mut v, &chime(1046.5), (0.16 * SR) as usize, 0.5);
            v
        }
        Cue::Star(k) => {
            let hz = [1046.5, 1318.5, 1568.0][usize::from(k.min(2))];
            chime(hz)
        }
    };
    normalise(&mut v, 0.8);
    v
}

fn cue_seed(cue: Cue) -> u32 {
    match cue {
        Cue::RadioOpen => 1,
        Cue::RadioClose => 2,
        Cue::Evacuate => 3,
        Cue::Alert => 4,
        Cue::Danger => 5,
        Cue::Withdraw => 6,
        Cue::Gust => 7,
        Cue::Done => 8,
        Cue::Star(k) => 100 + u32::from(k),
    }
}

fn silence(s: f32) -> Vec<f32> {
    vec![0.0; (s * SR) as usize]
}

fn add_at(v: &mut Vec<f32>, x: &[f32], at: usize, gain: f32) {
    if v.len() < at + x.len() {
        v.resize(at + x.len(), 0.0);
    }
    for (i, s) in x.iter().enumerate() {
        v[at + i] += s * gain;
    }
}

fn normalise(v: &mut [f32], peak: f32) {
    let m = v.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if m > 0.0 {
        for s in v {
            *s *= peak / m;
        }
    }
}

/// A sine with soft edges.
fn beep(hz: f32, s: f32, amp: f32) -> Vec<f32> {
    let n = (s * SR) as usize;
    let edge = 0.006 * SR;
    (0..n)
        .map(|i| {
            let e = (i as f32 / edge).min(1.0).min((n - i) as f32 / edge);
            amp * e * (TAU * hz * i as f32 / SR).sin()
        })
        .collect()
}

/// The hiss of a radio opening or closing.
fn squelch(rng: &mut Rng, s: f32) -> Vec<f32> {
    let n = (s * SR) as usize;
    let mut f = Svf::new(2200.0, 0.9);
    (0..n).map(|i| 0.5 * (1.0 - i as f32 / n as f32).powi(2) * f.run(rng.white()).band * 3.0).collect()
}

/// A glockenspiel note.
fn chime(hz: f32) -> Vec<f32> {
    let n = (1.3 * SR) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let a = (t / 0.002).min(1.0);
            a * ((-t / 0.45).exp() * (TAU * hz * t).sin()
                + 0.35 * (-t / 0.12).exp() * (TAU * hz * 2.756 * t).sin()
                + 0.15 * (-t / 0.05).exp() * (TAU * hz * 5.404 * t).sin())
        })
        .collect()
}

/// A soft synth note: two slightly detuned voices, a few harmonics, a quick
/// attack and an exponential fall. `bright` 0..1 adds the upper harmonics.
fn tone(hz: f32, s: f32, bright: f32) -> Vec<f32> {
    let n = (s * SR) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let voice = |f: f32| (1..5).map(|k| (TAU * f * k as f32 * t).sin() * bright.powi(k - 1) / k as f32).sum::<f32>();
            let env = (t / 0.008).min(1.0) * (-t / (0.35 * s)).exp() * ((s - t) / 0.01).clamp(0.0, 1.0);
            env * (voice(hz) + voice(hz * 1.004)) * 0.5
        })
        .collect()
}

/// A gust: noise swelling and sweeping up and back.
fn gust(rng: &mut Rng) -> Vec<f32> {
    let s = 2.6;
    let n = (s * SR) as usize;
    let mut f = Svf::new(300.0, 1.4);
    (0..n)
        .map(|i| {
            let x = i as f32 / n as f32;
            let env = (PI * x).sin().powi(2);
            if i % BLOCK == 0 {
                f.set(280.0 + 1100.0 * env, 1.4);
            }
            env * f.run(rng.white()).band
        })
        .collect()
}

/// 16-bit PCM WAV bytes, for listening to the sounds outside the game.
pub fn wav(samples: &[f32], channels: u16) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * u32::from(channels) * 2).to_le_bytes());
    b.extend_from_slice(&(channels * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_when_every_layer_is_off() {
        let levels = Arc::new(Levels::default());
        levels.set(Layer::Volume, 1.0);
        let mut a = Ambience::new(levels, 7);
        assert!((0..RATE).all(|_| a.frame() == [0.0, 0.0]));
    }

    #[test]
    fn layers_are_heard_and_bounded() {
        for layer in [Layer::Wind, Layer::Fire, Layer::Sea] {
            let levels = Arc::new(Levels::default());
            levels.set(Layer::Volume, 1.0);
            levels.set(layer, 1.0);
            let mut a = Ambience::new(levels, 3);
            let x: Vec<f32> = (0..RATE * 8).flat_map(|_| a.frame()).collect();
            let rms = (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
            assert!(rms > 0.01, "{layer:?}: rms {rms}");
            assert!(x.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{layer:?}");
        }
    }

    #[test]
    fn cues_are_short_and_normalised() {
        for cue in [Cue::RadioOpen, Cue::RadioClose, Cue::Evacuate, Cue::Alert, Cue::Danger, Cue::Withdraw, Cue::Gust, Cue::Done, Cue::Star(0), Cue::Star(2)] {
            let v = render(cue);
            let peak = v.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!((peak - 0.8).abs() < 1e-3, "{cue:?}: peak {peak}");
            assert!(v.len() < 8 * RATE as usize, "{cue:?}");
        }
    }
}
