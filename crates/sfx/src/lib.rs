//! The game's ambience and cues (user, 2026-10-30): the ambience follows the
//! simulation. The sea and the cues are made in code; the fire and the wind are
//! recordings (user, 2026-11-02, `data/sounds`, `tools/sounds/prepare.py`)
//! that the game decodes and hands over as [`Loop`]s, played here as beds.
//! Gulls and cicadas are recordings the game plays beside this. Pure DSP, std
//! only: the game plays [`Ambience`] as an endless stream and the [`Cue`]s as
//! short clips; `examples/render.rs` writes them to WAV files to listen to.
//!
//! Every sound is also shown on screen: the game must stay playable muted
//! (docs/01-GIOCO.md).

use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

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

/// The levels the game asks for, each 0..1, written from the main thread and
/// read by the audio thread (lock-free: one `f32` per atomic), and the
/// recorded beds, handed over once each as the game decodes them.
#[derive(Default)]
pub struct Levels {
    values: [AtomicU32; Layer::COUNT],
    beds: [OnceLock<Loop>; Bed::COUNT],
}

#[derive(Clone, Copy, Debug)]
pub enum Layer {
    /// The wind in the trees ([`Bed::TreeWind`]).
    TreeWind,
    /// The wind over open ground, grass and sea ([`Bed::OpenWind`]).
    OpenWind,
    /// How hard the wind blows, 0..1: the wind beds play a little faster
    /// (higher) when it is strong.
    Gale,
    /// A fire burning near what the camera looks at ([`Bed::Fire`]).
    Fire,
    /// A fire burning hard: the front's roar ([`Bed::Blaze`]).
    Blaze,
    /// Where the fire is heard, -1 (left) to 1 (right), 0 in the middle
    /// (user, 2026-11-02): the two fire beds lean to the side that burns.
    FirePan,
    /// The surf, near the coast.
    Sea,
    /// The pause: the world holds its breath, muffled and lower.
    Muffle,
    /// The ambience's own volume (the player's choice).
    Volume,
    /// An important cue is playing: the whole ambience steps 6 dB down, in
    /// 50 ms, and comes back in about half a second (it must not pump).
    Duck,
}

impl Layer {
    const COUNT: usize = 10;
}

/// How far [`Layer::Duck`] lowers the ambience: 6 dB.
pub const DUCK: f32 = 0.5;

/// The recordings the ambience loops (`data/sounds/<file>.mp3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bed {
    Fire,
    Blaze,
    TreeWind,
    OpenWind,
}

impl Bed {
    pub const ALL: [Bed; 4] = [Bed::Fire, Bed::Blaze, Bed::TreeWind, Bed::OpenWind];
    const COUNT: usize = 4;
    /// The file in `data/sounds`, without `.mp3`.
    pub fn file(self) -> &'static str {
        match self {
            Bed::Fire => "fuoco",
            Bed::Blaze => "fuoco_forte",
            Bed::TreeWind => "vento_alberi",
            Bed::OpenWind => "vento_aperto",
        }
    }
    /// The layer that sets its level.
    fn layer(self) -> Layer {
        match self {
            Bed::Fire => Layer::Fire,
            Bed::Blaze => Layer::Blaze,
            Bed::TreeWind => Layer::TreeWind,
            Bed::OpenWind => Layer::OpenWind,
        }
    }
    /// The files are all at -23 LUFS (`prepare.py`); this brings level 1 to
    /// about the loudness the sounds made in code had at level 1 (fire
    /// -14 LUFS, wind -13 LUFS; measured with `examples/render.rs`), so the
    /// game's mix stays where it was.
    fn gain(self) -> f32 {
        match self {
            Bed::Fire | Bed::Blaze => 2.8,
            Bed::TreeWind | Bed::OpenWind => 2.1,
        }
    }
    /// Whether the wind's strength speeds it up.
    fn windy(self) -> bool {
        matches!(self, Bed::TreeWind | Bed::OpenWind)
    }
}

impl Levels {
    pub fn set(&self, layer: Layer, v: f32) {
        let lo = if matches!(layer, Layer::FirePan) { -1.0 } else { 0.0 };
        self.values[layer as usize].store(v.clamp(lo, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn get(&self, layer: Layer) -> f32 {
        f32::from_bits(self.values[layer as usize].load(Ordering::Relaxed))
    }
    /// Hands a decoded recording to the ambience; the first one given stays.
    pub fn set_bed(&self, bed: Bed, l: Loop) {
        let _ = self.beds[bed as usize].set(l);
    }
    pub fn has_bed(&self, bed: Bed) -> bool {
        self.beds[bed as usize].get().is_some()
    }
}

/// What `prepare.py` cuts beyond the loop, in seconds: a margin at each end
/// (room for the padding an MP3 decoder adds) and the fade from the end of the
/// loop into its start. Must match `BED_MARGIN_S` and `BED_FADE_S` there.
pub const BED_MARGIN_S: f32 = 0.25;
pub const BED_FADE_S: f32 = 4.0;

/// A recording that loops with no seam: mono, 16 bit, at its own rate. The
/// ambience reads it twice, half a loop apart, for the two ears.
pub struct Loop {
    samples: Box<[i16]>,
    rate: u32,
}

impl Loop {
    /// A loop from a decoded bed (mono, at `rate`): the margins dropped, the
    /// last [`BED_FADE_S`] faded into the start at equal power. `None` if the
    /// recording is too short.
    pub fn from_bed(mono: &[f32], rate: u32) -> Option<Loop> {
        let margin = (BED_MARGIN_S * rate as f32) as usize;
        let fade = (BED_FADE_S * rate as f32) as usize;
        let s = mono.get(margin..mono.len().checked_sub(margin)?)?;
        let n = s.len().checked_sub(fade).filter(|&n| n >= fade)?;
        let samples = (0..n)
            .map(|i| {
                let x = if i < fade {
                    let r = i as f32 / fade as f32;
                    s[i] * r.sqrt() + s[n + i] * (1.0 - r).sqrt()
                } else {
                    s[i]
                };
                (x.clamp(-1.0, 1.0) * 32767.0) as i16
            })
            .collect();
        Some(Loop { samples, rate })
    }
    pub fn len(&self) -> usize {
        self.samples.len()
    }
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
    /// The sample at a fractional position, wrapping.
    fn at(&self, pos: f64) -> f32 {
        let n = self.samples.len();
        let i = pos as usize % n;
        let w = pos.fract() as f32;
        let a = f32::from(self.samples[i]);
        let b = f32::from(self.samples[(i + 1) % n]);
        (a + (b - a) * w) / 32768.0
    }
}

/// Frames between reads of [`Levels`] and filter updates.
const BLOCK: usize = 64;

/// The ambience of the island, endless and stereo: the recorded beds (fire,
/// wind) and the sea mixed at the [`Levels`] the game sets.
pub struct Ambience {
    levels: Arc<Levels>,
    rng: Rng,
    /// Levels as heard: they glide to the asked ones in about a second.
    heard: [f32; Layer::COUNT],
    n: usize,
    /// Where each bed is, in its own samples (each starts at its own place).
    at: [f64; Bed::COUNT],
    /// Each bed's step per output frame, set every block.
    step: [f64; Bed::COUNT],
    /// The fire's gain in each ear, from [`Layer::FirePan`], set every block.
    fire_ears: [f32; 2],
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
            at: std::array::from_fn(|_| f64::from(rng.range(0.0, 1.0e6))),
            step: [1.0; Bed::COUNT],
            fire_ears: [1.0; 2],
            waves,
            muffle: [OnePole::new(18_000.0), OnePole::new(18_000.0)],
            rng,
        }
    }

    fn block(&mut self) {
        let glide = |tau: f32| 1.0 - (-(BLOCK as f32) / (tau * SR)).exp();
        for (i, h) in self.heard.iter_mut().enumerate() {
            let want = f32::from_bits(self.levels.values[i].load(Ordering::Relaxed));
            let k = if i == Layer::Duck as usize {
                glide(if want > *h { 0.05 } else { 0.4 })
            } else if i == Layer::FirePan as usize {
                // the camera turns quickly: the fire follows in a quarter second
                glide(0.25)
            } else {
                glide(0.8)
            };
            *h += k * (want - *h);
        }
        // equal power, 1 in each ear in the middle; at ±0.8 (the game's
        // widest) the far ear is at −13 dB and the near one at +3 dB
        let a = (self.heard[Layer::FirePan as usize] + 1.0) * PI / 4.0;
        self.fire_ears = [a.cos() * std::f32::consts::SQRT_2, a.sin() * std::f32::consts::SQRT_2];
        // a strong wind plays its recordings up to 8 % faster (a little higher)
        let gale = self.heard[Layer::Gale as usize];
        for b in Bed::ALL {
            if let Some(l) = self.levels.beds[b as usize].get() {
                let speed = if b.windy() { 0.96 + 0.12 * gale } else { 1.0 };
                self.step[b as usize] = f64::from(l.rate) / f64::from(RATE) * f64::from(speed);
            }
        }
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
        let sea = self.heard[Layer::Sea as usize];
        let muffle = self.heard[Layer::Muffle as usize];
        let volume = self.heard[Layer::Volume as usize];
        let rng = &mut self.rng;
        let mut out = [0.0f32; 2];

        // Fire and wind: the recordings, each ear half a loop from the other.
        for b in Bed::ALL {
            let i = b as usize;
            let Some(l) = self.levels.beds[i].get() else { continue };
            let level = self.heard[b.layer() as usize];
            if level > 0.002 {
                let amp = level * b.gain();
                let [el, er] = if matches!(b, Bed::Fire | Bed::Blaze) { self.fire_ears } else { [1.0, 1.0] };
                let half = (l.len() / 2) as f64;
                out[0] += amp * el * l.at(self.at[i]);
                out[1] += amp * er * l.at(self.at[i] + half);
            }
            // it runs on silent too: a level that comes back finds it elsewhere
            self.at[i] = (self.at[i] + self.step[i]) % l.len() as f64;
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

        let duck = self.heard[Layer::Duck as usize];
        let gain = volume * (1.0 - 0.55 * muffle) * (1.0 - DUCK * duck);
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
        levels.set_bed(Bed::Fire, Loop::from_bed(&noise_bed(32_000, 10.0), 32_000).unwrap());
        levels.set(Layer::Volume, 1.0);
        let mut a = Ambience::new(levels, 7);
        assert!((0..RATE).all(|_| a.frame() == [0.0, 0.0]));
    }

    /// A bed of white noise, as `prepare.py` cuts it.
    fn noise_bed(rate: u32, seconds: f32) -> Vec<f32> {
        let mut rng = Rng(9);
        (0..(seconds * rate as f32) as usize).map(|_| 0.1 * rng.white()).collect()
    }

    #[test]
    fn a_bed_loops_with_no_seam() {
        let rate = 32_000;
        let x: Vec<f32> = (0..rate * 12).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let l = Loop::from_bed(&x, rate).unwrap();
        let margin = (BED_MARGIN_S * rate as f32) as usize;
        let fade = (BED_FADE_S * rate as f32) as usize;
        assert_eq!(l.len(), x.len() - 2 * margin - fade);
        // the wrap goes on as the recording did: from the end to the sample after it
        let (end, start) = (l.at((l.len() - 1) as f64), l.at(l.len() as f64));
        let (a, b) = (x[margin + l.len() - 1], x[margin + l.len()]);
        assert!((end - a).abs() < 1e-3 && (start - b).abs() < 1e-3, "{end} {start} / {a} {b}");
        assert!(Loop::from_bed(&x[..rate as usize * 5], rate).is_none());
    }

    #[test]
    fn layers_are_heard_and_bounded() {
        for layer in [Layer::TreeWind, Layer::OpenWind, Layer::Fire, Layer::Blaze, Layer::Sea] {
            let levels = Arc::new(Levels::default());
            for b in Bed::ALL {
                levels.set_bed(b, Loop::from_bed(&noise_bed(32_000, 10.0), 32_000).unwrap());
            }
            levels.set(Layer::Volume, 1.0);
            levels.set(Layer::Gale, 1.0);
            levels.set(layer, 1.0);
            let mut a = Ambience::new(levels, 3);
            let x: Vec<f32> = (0..RATE * 8).flat_map(|_| a.frame()).collect();
            let rms = (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
            assert!(rms > 0.01, "{layer:?}: rms {rms}");
            assert!(x.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{layer:?}");
        }
    }

    #[test]
    fn the_fire_leans_to_its_side() {
        let ears = |pan: f32| {
            let levels = Arc::new(Levels::default());
            levels.set_bed(Bed::Fire, Loop::from_bed(&noise_bed(32_000, 10.0), 32_000).unwrap());
            levels.set(Layer::Volume, 0.5);
            levels.set(Layer::Fire, 1.0);
            levels.set(Layer::FirePan, pan);
            let mut a = Ambience::new(levels, 2);
            (0..RATE * 3).for_each(|_| drop(a.frame()));
            let x: Vec<[f32; 2]> = (0..RATE * 5).map(|_| a.frame()).collect();
            let rms = |c: usize| (x.iter().map(|f| f[c] * f[c]).sum::<f32>() / x.len() as f32).sqrt();
            20.0 * (rms(1) / rms(0)).log10()
        };
        assert!(ears(0.0).abs() < 1.0, "{}", ears(0.0));
        assert!(ears(0.8) > 10.0 && ears(-0.8) < -10.0, "{} {}", ears(0.8), ears(-0.8));
    }

    #[test]
    fn ducking_steps_6_db_down_and_comes_back() {
        let levels = Arc::new(Levels::default());
        levels.set(Layer::Volume, 0.3);
        levels.set(Layer::Sea, 1.0);
        let mut a = Ambience::new(levels.clone(), 4);
        let mut rms = |a: &mut Ambience, s: f32| {
            let x: Vec<f32> = (0..(s * SR) as usize).flat_map(|_| a.frame()).collect();
            (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
        };
        rms(&mut a, 3.0);
        let open = rms(&mut a, 20.0);
        levels.set(Layer::Duck, 1.0);
        rms(&mut a, 0.2);
        let ducked = rms(&mut a, 20.0);
        let db = 20.0 * (ducked / open).log10();
        assert!((-7.5..-4.5).contains(&db), "{db} dB");
        levels.set(Layer::Duck, 0.0);
        rms(&mut a, 2.0);
        let back = 20.0 * (rms(&mut a, 20.0) / open).log10();
        assert!(back.abs() < 1.5, "{back} dB");
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
