//! Sound (user, 2026-10-30): the island's ambience and short cues for what the
//! player does and what the game says. Nothing here decides anything: the
//! levels follow the camera and the simulation, the cues follow what the
//! screen already shows (the chat, the pause, the orders), so the game stays
//! the same muted (docs/01-GIOCO.md).
//!
//! - Wind, fire and surf are made in code (`sfx::Ambience`), one endless stream.
//! - Cicadas and gulls are recordings (`data/sounds`, `tools/sounds/prepare.py`):
//!   one cicada recording at a time, fading into the next; a gull now and then
//!   when the camera looks at the sea.
//! - The cues: `sfx::render` for the game's events, Kenney's interface sounds
//!   for buttons and orders.
//!
//! The choice (muted, ambience and effects volume) is saved like the graphics
//! level: `~/.rocca_ventosa/audio.txt`, or the browser's storage.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bevy::audio::{AddAudioSource, AudioSink, AudioSinkPlayback, AudioSource, Decodable, PlaybackSettings, Source, Volume};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use rocca::{Axial, Kind};
use sfx::{Cue, Layer, Levels};

use crate::camera::OrbitCamera;
use crate::kiosk::{Kiosk, Phase};
use crate::sim::{Sim, SimRestarted};

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<Stream>()
            .add_audio_source::<Clip>()
            .insert_resource(AudioPrefs::load())
            .init_resource::<Heard>()
            .add_systems(Startup, setup)
            .add_systems(Update, (ambience, cues.after(crate::kiosk::step)))
            .add_systems(bevy_egui::EguiPrimaryContextPass, clicks.after(crate::kiosk::ui::draw));
        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(Last, record_audio.run_if(crate::record::running));
    }
}

/// What the player chose.
#[derive(Resource, Clone, Copy, PartialEq)]
pub struct AudioPrefs {
    pub muted: bool,
    pub ambience: f32,
    pub effects: f32,
}

impl Default for AudioPrefs {
    fn default() -> Self {
        AudioPrefs { muted: false, ambience: 0.7, effects: 0.8 }
    }
}

impl AudioPrefs {
    fn parse(s: &str) -> Option<AudioPrefs> {
        let mut w = s.split_whitespace();
        Some(AudioPrefs { muted: w.next()? == "muto", ambience: w.next()?.parse().ok()?, effects: w.next()?.parse().ok()? })
    }
    fn text(&self) -> String {
        format!("{} {:.2} {:.2}", if self.muted { "muto" } else { "acceso" }, self.ambience, self.effects)
    }
    fn load() -> AudioPrefs {
        saved().and_then(|s| AudioPrefs::parse(&s)).unwrap_or_default()
    }
    pub fn save(&self) {
        store(&self.text());
    }
    fn ambience_gain(&self) -> f32 {
        if self.muted { 0.0 } else { self.ambience }
    }
    fn effects_gain(&self) -> f32 {
        if self.muted { 0.0 } else { self.effects }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn saved_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join(".rocca_ventosa").join("audio.txt")
}

#[cfg(not(target_arch = "wasm32"))]
fn saved() -> Option<String> {
    std::fs::read_to_string(saved_path()).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn store(text: &str) {
    let p = saved_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, text);
}

#[cfg(target_arch = "wasm32")]
fn saved() -> Option<String> {
    web_sys::window()?.local_storage().ok()??.get_item("rocca_ventosa_audio").ok()?
}

#[cfg(target_arch = "wasm32")]
fn store(text: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item("rocca_ventosa_audio", text);
    }
}

/// The ambience made in code, as a Bevy audio source: endless, stereo.
#[derive(Asset, TypePath, Clone)]
struct Stream {
    levels: Arc<Levels>,
}

struct StreamDecoder {
    ambience: sfx::Ambience,
    right: Option<f32>,
}

impl Iterator for StreamDecoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        let [l, r] = self.ambience.frame();
        self.right = Some(r);
        Some(l)
    }
}

impl Source for StreamDecoder {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> bevy::audio::ChannelCount { bevy::audio::ChannelCount::new(2).unwrap() }
    fn sample_rate(&self) -> bevy::audio::SampleRate { bevy::audio::SampleRate::new(sfx::RATE).unwrap() }
    fn total_duration(&self) -> Option<Duration> { None }
}

impl Decodable for Stream {
    type Decoder = StreamDecoder;
    fn decoder(&self) -> StreamDecoder {
        StreamDecoder { ambience: sfx::Ambience::new(self.levels.clone(), 0x5eed), right: None }
    }
}

/// A cue made in code, mono.
#[derive(Asset, TypePath, Clone)]
struct Clip(Arc<[f32]>);

struct ClipDecoder {
    samples: Arc<[f32]>,
    at: usize,
}

impl Iterator for ClipDecoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let s = self.samples.get(self.at).copied();
        self.at += 1;
        s
    }
}

impl Source for ClipDecoder {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> bevy::audio::ChannelCount { bevy::audio::ChannelCount::new(1).unwrap() }
    fn sample_rate(&self) -> bevy::audio::SampleRate { bevy::audio::SampleRate::new(sfx::RATE).unwrap() }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(self.samples.len() as f32 / sfx::RATE as f32))
    }
}

impl Decodable for Clip {
    type Decoder = ClipDecoder;
    fn decoder(&self) -> ClipDecoder {
        ClipDecoder { samples: self.0.clone(), at: 0 }
    }
}

/// The interface sounds (Kenney, CC0), by their file name in `data/sounds`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Ui {
    /// A button.
    Clic,
    /// A team's badge picked up.
    Prendi,
    /// An order the game took.
    Ordine,
    /// A notice: the game refused, or the tutorial asks something else.
    Errore,
    /// A step of the tutorial done.
    Passo,
}

impl Ui {
    const ALL: [Ui; 5] = [Ui::Clic, Ui::Prendi, Ui::Ordine, Ui::Errore, Ui::Passo];
    fn file(self) -> &'static str {
        match self {
            Ui::Clic => "ui_clic",
            Ui::Prendi => "ui_prendi",
            Ui::Ordine => "ui_ordine",
            Ui::Errore => "ui_errore",
            Ui::Passo => "ui_passo",
        }
    }
    fn gain(self) -> f32 {
        match self {
            Ui::Clic => 0.35,
            _ => 0.6,
        }
    }
}

const CUES: [Cue; 11] = [
    Cue::RadioOpen, Cue::RadioClose, Cue::Evacuate, Cue::Alert, Cue::Danger, Cue::Withdraw,
    Cue::Gust, Cue::Done, Cue::Star(0), Cue::Star(1), Cue::Star(2),
];

#[derive(Resource)]
struct Sounds {
    levels: Arc<Levels>,
    cues: HashMap<Cue, Handle<Clip>>,
    ui: HashMap<Ui, Handle<AudioSource>>,
    cicadas: Vec<Handle<AudioSource>>,
    gulls: Vec<Handle<AudioSource>>,
    /// The opening's two cuts: with the logo, then on the island.
    jingles: [Option<Handle<AudioSource>>; 2],
}

/// One recording of cicadas playing (they take turns, see [`ambience`]).
#[derive(Component)]
struct Cicadas {
    started: f32,
    /// The volume it plays at now (before the global one).
    level: f32,
}

/// The length the cicada recordings are cut to (`tools/sounds/prepare.py`)
/// and the fade from one to the next.
const CICADA_S: f32 = 24.0;
const CICADA_FADE_S: f32 = 4.0;

fn setup(mut commands: Commands, data: Res<crate::DataPath>, mut streams: ResMut<Assets<Stream>>, mut clips: ResMut<Assets<Clip>>, mut files: ResMut<Assets<AudioSource>>) {
    let levels = Arc::new(Levels::default());
    let mut load = |name: &str| -> Option<Handle<AudioSource>> {
        match datafs::read(data.0.join("sounds").join(format!("{name}.mp3"))) {
            Ok(bytes) => Some(files.add(AudioSource { bytes: bytes.into() })),
            Err(e) => {
                warn!("suoni: {name}: {e}");
                None
            }
        }
    };
    let ui = Ui::ALL.iter().filter_map(|&u| Some((u, load(u.file())?))).collect();
    let cicadas = (1..=3).filter_map(|k| load(&format!("cicale_{k}"))).collect();
    let gulls = (1..=4).filter_map(|k| load(&format!("gabbiano_{k}"))).collect();
    let jingles = [load("jingle_logo"), load("jingle_isola")];
    let cues = CUES.iter().map(|&c| (c, clips.add(Clip(sfx::render(c).into())))).collect();
    commands.spawn((AudioPlayer(streams.add(Stream { levels: levels.clone() })), PlaybackSettings::LOOP));
    commands.insert_resource(Sounds { levels, cues, ui, cicadas, gulls, jingles });
}

/// What was last heard, to notice what changed.
#[derive(Resource, Default)]
struct Heard {
    ready: bool,
    paused: bool,
    phase: Option<Phase>,
    chat: usize,
    runs: usize,
    hexes: Vec<Option<Axial>>,
    dragging: bool,
    notice: Option<String>,
    step: usize,
    /// How many of the opening's jingles have played.
    jingle: u8,
    /// Real seconds, for the cicadas and the gulls.
    clock: f32,
    next_gull: f32,
    /// Cues waiting their turn: (real time, cue).
    queue: Vec<(f32, Cue)>,
    rng: u32,
    /// Measured from the scene every quarter second.
    scene_at: f32,
    scene: Scene,
}

#[derive(Default, Clone, Copy)]
struct Scene {
    fire: f32,
    water: f32,
    close: f32,
}

/// The camera's distances (m) where the ambience is full and where it is gone:
/// it is heard only close to the action (user, 2026-10-30).
const FULL_M: f32 = 400.0;
const GONE_M: f32 = 1500.0;

/// 1 at [`FULL_M`] or closer, 0 from [`GONE_M`], smooth in between (on a log
/// scale, as the zoom moves).
fn closeness(distance: f32) -> f32 {
    let x = ((GONE_M.ln() - distance.max(1.0).ln()) / (GONE_M / FULL_M).ln()).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Heard {
    fn random(&mut self) -> f32 {
        let mut x = self.rng.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// How much fire and sea there are around what the camera looks at, and how
/// close it is.
fn measure(sim: &Sim, cam: &OrbitCamera) -> Scene {
    let focus = crate::frame::to_world(cam.focus);
    let reach = (cam.distance * 0.4).max(150.0);
    let cells = sim.fire.active_cells();
    let step = (cells.len() / 3000).max(1);
    let heat: f32 = cells.iter().step_by(step).map(|&c| {
        let p = sim.scn.world.centre_of(c);
        let d2 = (p.x - focus.x).powi(2) + (p.y - focus.y).powi(2);
        // louder where it burns harder (user, 2026-10-30): Byram's flame
        // length, 1 at about 1000 kW/m
        let flame = (0.0775 * sim.fire.cell_intensity(c).max(0.0).powf(0.46) / 1.8).clamp(0.15, 4.0);
        flame / (1.0 + d2 / (reach * reach))
    }).sum::<f32>() * step as f32;
    let ring = (0..8).map(|k| {
        let a = k as f32 * std::f32::consts::TAU / 8.0;
        scenario::Pos { x: focus.x + reach * 0.7 * a.cos(), y: focus.y + reach * 0.7 * a.sin() }
    });
    let points: Vec<_> = std::iter::once(focus).chain(ring).collect();
    let water = points.iter().filter(|&&p| !sim.scn.world.contains(p) || crate::sea::is_water(&sim.scn, p)).count() as f32 / points.len() as f32;
    Scene { fire: 1.0 - (-heat / 60.0).exp(), water, close: closeness(cam.distance) }
}

#[allow(clippy::too_many_arguments)]
fn ambience(
    mut commands: Commands,
    time: Res<Time>,
    prefs: Res<AudioPrefs>,
    sounds: Option<Res<Sounds>>,
    sim: Option<Res<Sim>>,
    kiosk: Option<Res<Kiosk>>,
    global: Res<bevy::audio::GlobalVolume>,
    cams: Query<&OrbitCamera, Without<crate::camera::AuxiliaryCamera>>,
    mut heard: ResMut<Heard>,
    mut cicadas: Query<(Entity, &mut Cicadas, &mut AudioSink)>,
) {
    let Some(sounds) = sounds else { return };
    let h = &mut *heard;
    h.clock += time.delta_secs();
    let (Some(sim), Some(kiosk), Ok(cam)) = (sim, kiosk, cams.single()) else {
        sounds.levels.set(Layer::Volume, 0.0);
        return;
    };
    if h.clock >= h.scene_at {
        h.scene_at = h.clock + 0.25;
        h.scene = measure(&sim, cam);
    }
    let Scene { fire, water, close } = h.scene;
    let w = sim.weather();
    // very light, well under the fire (user, 2026-10-30: «ancora troppo forte»)
    let wind = (0.25 + 0.75 * w.wind_speed_kmh as f32 / 35.0).min(1.0) * close * 0.1;
    let sea = water.powf(0.7) * close;
    let pause = sim.paused() && matches!(kiosk.phase, Phase::Tutorial | Phase::Gioco);
    // the ambience stays under the cues: half the chosen volume. Mario hears
    // it too (user, 2026-10-30): his camera is the orbit camera, close to him.
    let gain = 0.5 * prefs.ambience_gain();
    let levels = &sounds.levels;
    levels.set(Layer::Wind, wind);
    levels.set(Layer::Fire, fire * close);
    levels.set(Layer::Sea, sea * 0.8);
    levels.set(Layer::Muffle, if pause { 1.0 } else { 0.0 });
    levels.set(Layer::Volume, gain);

    // Recordings cannot be muffled: in the pause they only get quieter.
    let quiet = if pause { 0.4 } else { 1.0 };
    let life = (1.0 - water) * (1.0 - fire).powi(2) * close * gain * quiet * 0.8;
    let mut newest = f32::NEG_INFINITY;
    for (e, mut c, mut sink) in &mut cicadas {
        let age = h.clock - c.started;
        if age > CICADA_S + 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        newest = newest.max(c.started);
        let env = (age / CICADA_FADE_S).min((CICADA_S - age) / CICADA_FADE_S).clamp(0.0, 1.0);
        // a sink's own volume leaves out the global one (0 under KIOSK_SNAP)
        c.level = env * life;
        sink.set_volume(Volume::Linear(c.level * global.volume.to_linear()));
    }
    if !sounds.cicadas.is_empty() && h.clock - newest > CICADA_S - CICADA_FADE_S && life > 0.01 {
        let k = (h.random() * sounds.cicadas.len() as f32) as usize % sounds.cicadas.len();
        commands.spawn((AudioPlayer(sounds.cicadas[k].clone()), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.0)), Cicadas { started: h.clock, level: 0.0 }));
    }
    if h.next_gull == 0.0 {
        h.next_gull = h.clock + 3.0;
    }
    if h.clock >= h.next_gull {
        let r = h.random();
        h.next_gull = h.clock + (5.0 + 12.0 * r) / sea.max(0.15);
        if sea > 0.15 && gain > 0.0 && !sounds.gulls.is_empty() {
            let k = (h.random() * sounds.gulls.len() as f32) as usize % sounds.gulls.len();
            let speed = 0.92 + 0.16 * h.random();
            commands.spawn((AudioPlayer(sounds.gulls[k].clone()), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(sea * gain * quiet * 0.5)).with_speed(speed)));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// One sound being filmed (`record`): decoded once, at [`sfx::RATE`], stereo.
struct Voice {
    samples: Arc<[f32]>,
    /// Frames into `samples` (fractional: a gull's call plays a little faster
    /// or slower).
    at: f64,
    speed: f64,
    looped: bool,
    volume: f32,
}

#[cfg(not(target_arch = "wasm32"))]
/// Any decoder -> stereo samples at [`sfx::RATE`].
fn stereo<S: Source<Item = f32>>(source: S) -> Arc<[f32]> {
    let ch = usize::from(source.channels().get());
    let rate = f64::from(source.sample_rate().get());
    let raw: Vec<f32> = source.collect();
    let frames = raw.len() / ch.max(1);
    let out_n = (frames as f64 * f64::from(sfx::RATE) / rate) as usize;
    let mut out = Vec::with_capacity(out_n * 2);
    let get = |f: usize, c: usize| raw[f.min(frames.saturating_sub(1)) * ch + c.min(ch - 1)];
    for i in 0..out_n {
        let p = i as f64 * rate / f64::from(sfx::RATE);
        let (f, w) = (p as usize, (p.fract()) as f32);
        for c in 0..2 {
            out.push(get(f, c) * (1.0 - w) + get(f + 1, c) * w);
        }
    }
    out.into()
}

#[cfg(not(target_arch = "wasm32"))]
/// The sound of a recording (`record`): every sound the game is playing,
/// mixed at its own volume for this frame's span of game time. A sound
/// plays to its end once started, as the game's own would: the real playback
/// (silent while filming) may end it sooner or later than the game's clock.
#[allow(clippy::too_many_arguments)]
fn record_audio(
    time: Res<Time>,
    mut rec: ResMut<crate::record::Recorder>,
    files: Res<Assets<AudioSource>>,
    clips: Res<Assets<Clip>>,
    streams: Res<Assets<Stream>>,
    played: Query<(Entity, &PlaybackSettings, Option<&AudioPlayer<AudioSource>>, Option<&AudioPlayer<Clip>>, Option<&AudioPlayer<Stream>>, Option<&Cicadas>)>,
    mut voices: Local<std::collections::HashMap<Entity, Voice>>,
    mut ambience: Local<Option<(Entity, StreamDecoder, f32)>>,
    mut decoded: Local<HashMap<AssetId<AudioSource>, Arc<[f32]>>>,
    mut before: Local<Option<std::collections::HashSet<Entity>>>,
) {
    // what was playing before the recording began is not in it
    if rec.fresh() {
        *before = None;
        voices.clear();
        *ambience = None;
    }
    let before = before.get_or_insert_with(|| played.iter().map(|q| q.0).collect());
    for (e, settings, file, clip, stream, cicadas) in &played {
        let volume = cicadas.map_or(settings.volume.to_linear(), |c| c.level);
        if let Some(s) = stream {
            if ambience.as_ref().is_none_or(|a| a.0 != e) {
                if let Some(st) = streams.get(&s.0) {
                    *ambience = Some((e, st.decoder(), volume));
                }
            }
            continue;
        }
        if let Some(v) = voices.get_mut(&e) {
            v.volume = volume;
            continue;
        }
        if before.contains(&e) {
            continue;
        }
        let samples = if let Some(f) = file {
            match decoded.get(&f.0.id()) {
                Some(s) => Some(s.clone()),
                None => files.get(&f.0).map(|a| {
                    let s = stereo(a.decoder());
                    decoded.insert(f.0.id(), s.clone());
                    s
                }),
            }
        } else {
            clip.and_then(|c| clips.get(&c.0)).map(|c| stereo(c.decoder()))
        };
        if let Some(samples) = samples {
            let looped = matches!(settings.mode, bevy::audio::PlaybackMode::Loop);
            voices.insert(e, Voice { samples, at: 0.0, speed: f64::from(settings.speed), looped, volume });
        }
    }
    rec.audio(time.delta_secs(), |out| {
        let n = out.len() / 2;
        if let Some((_, dec, vol)) = ambience.as_mut() {
            for i in 0..n {
                let l = dec.next().unwrap_or(0.0);
                let r = dec.next().unwrap_or(0.0);
                out[2 * i] += l * *vol;
                out[2 * i + 1] += r * *vol;
            }
        }
        // a voice that has ended stays, silent, while its entity lives: the
        // real playback may still be going, and must not start it again
        voices.retain(|e, _| played.contains(*e));
        for v in voices.values_mut() {
            let frames = v.samples.len() / 2;
            for i in 0..n {
                if v.at as usize >= frames {
                    if !v.looped {
                        break;
                    }
                    v.at = 0.0;
                }
                let f = v.at as usize;
                out[2 * i] += v.samples[2 * f] * v.volume;
                out[2 * i + 1] += v.samples[2 * f + 1] * v.volume;
                v.at += v.speed;
            }
        }
    });
}

fn play_cue(commands: &mut Commands, sounds: &Sounds, prefs: &AudioPrefs, cue: Cue) {
    let gain = prefs.effects_gain();
    if gain <= 0.0 { return; }
    if let Some(h) = sounds.cues.get(&cue) {
        commands.spawn((AudioPlayer(h.clone()), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(gain * 0.5))));
    }
}

fn play_ui(commands: &mut Commands, sounds: &Sounds, prefs: &AudioPrefs, ui: Ui) {
    let gain = prefs.effects_gain();
    if gain <= 0.0 { return; }
    if let Some(h) = sounds.ui.get(&ui) {
        commands.spawn((AudioPlayer(h.clone()), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(gain * ui.gain()))));
    }
}

/// The cues of what changed since the last frame.
#[allow(clippy::too_many_arguments)]
fn cues(
    mut commands: Commands,
    prefs: Res<AudioPrefs>,
    sounds: Option<Res<Sounds>>,
    sim: Option<Res<Sim>>,
    kiosk: Option<Res<Kiosk>>,
    mut restarted: MessageReader<SimRestarted>,
    mut heard: ResMut<Heard>,
) {
    let (Some(sounds), Some(sim), Some(k)) = (sounds, sim, kiosk) else { return };
    let h = &mut *heard;
    let restart = restarted.read().count() > 0 || sim.chat.len() < h.chat || sim.evac.runs.len() < h.runs;
    let hexes: Vec<_> = sim.teams.iter().map(|t| t.hex).collect();
    let playing = matches!(k.phase, Phase::Tutorial | Phase::Gioco);
    if h.ready && !restart && playing {
        if sim.paused() != h.paused {
            play_cue(&mut commands, &sounds, &prefs, if sim.paused() { Cue::RadioOpen } else { Cue::RadioClose });
        }
        if hexes.len() == h.hexes.len() && hexes != h.hexes {
            play_ui(&mut commands, &sounds, &prefs, Ui::Ordine);
        }
        if sim.evac.runs.len() > h.runs {
            play_cue(&mut commands, &sounds, &prefs, Cue::Evacuate);
        }
        for m in &sim.chat[h.chat.min(sim.chat.len())..] {
            let cue = match m.kind {
                Kind::HomesHit | Kind::EvacStuck => Cue::Alert,
                Kind::Lost => Cue::Danger,
                Kind::Withdraw => Cue::Withdraw,
                Kind::EvacDone => Cue::Done,
                Kind::Wind => Cue::Gust,
            };
            play_cue(&mut commands, &sounds, &prefs, cue);
        }
        if k.dragging.is_some() && !h.dragging {
            play_ui(&mut commands, &sounds, &prefs, Ui::Prendi);
        }
        if k.phase == Phase::Tutorial && h.phase == Some(Phase::Tutorial) && k.step > h.step {
            play_ui(&mut commands, &sounds, &prefs, Ui::Passo);
        }
    }
    let notice = k.notice.as_ref().map(|(t, _)| t.clone());
    if h.ready && notice.is_some() && notice != h.notice {
        play_ui(&mut commands, &sounds, &prefs, Ui::Errore);
    }
    // the end: three notes up
    if h.phase == Some(Phase::Gioco) && k.phase == Phase::Fine {
        for s in 0..3u8 {
            h.queue.push((h.clock + 0.4 + 0.3 * f32::from(s), Cue::Star(s)));
        }
    }
    // the jingle, once (user, 2026-10-30): its climax as the CIMA logo fades
    // in, its cadence as the logo fades out on the island. An opening skipped
    // before the logo hears the cadence alone.
    let due = if k.phase == Phase::Intro && k.title { k.jingles_due() } else { 0 };
    // an opening that starts over (a snapshot's or a recording's) hears it again
    if k.opening.is_some() && due == 0 {
        h.jingle = 0;
    }
    if due > h.jingle {
        let part = if h.jingle == 0 && due == 2 { 1 } else { h.jingle };
        h.jingle = due;
        let j = &sounds.jingles[usize::from(part)];
        info!("suoni: jingle {part} ({})", if j.is_some() { "caricato" } else { "mancante" });
        // the jingle is the opening's, not an effect: only muting silences it
        if let (Some(j), false) = (j, prefs.muted) {
            commands.spawn((AudioPlayer(j.clone()), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.9))));
        }
    }
    let now = h.clock;
    let due: Vec<Cue> = h.queue.iter().filter(|(t, _)| *t <= now).map(|&(_, c)| c).collect();
    h.queue.retain(|(t, _)| *t > now);
    for c in due {
        play_cue(&mut commands, &sounds, &prefs, c);
    }
    h.ready = true;
    h.paused = sim.paused();
    h.phase = Some(k.phase);
    h.chat = sim.chat.len();
    h.runs = sim.evac.runs.len();
    h.hexes = hexes;
    h.dragging = k.dragging.is_some();
    h.notice = notice;
    h.step = k.step;
}

/// A click on any button of the interface (egui tells it for accessibility).
fn clicks(mut commands: Commands, mut contexts: EguiContexts, prefs: Res<AudioPrefs>, sounds: Option<Res<Sounds>>) {
    let Some(sounds) = sounds else { return };
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let clicked = ctx.output(|o| o.events.iter().any(|e| matches!(e, egui::output::OutputEvent::Clicked(_))));
    if clicked {
        play_ui(&mut commands, &sounds, &prefs, Ui::Clic);
    }
}

/// The audio controls of the settings menu: muted, and the two volumes.
pub fn settings_ui(ui: &mut egui::Ui, prefs: &mut AudioPrefs) {
    let before = *prefs;
    ui.horizontal(|ui| {
        ui.label(rocca::tr!("Audio:"));
        ui.checkbox(&mut prefs.muted, rocca::tr!("Muto"));
    });
    ui.add_enabled_ui(!prefs.muted, |ui| {
        ui.add(egui::Slider::new(&mut prefs.ambience, 0.0..=1.0).show_value(false).text(rocca::tr!("Ambiente")));
        ui.add(egui::Slider::new(&mut prefs.effects, 0.0..=1.0).show_value(false).text(rocca::tr!("Effetti")));
    });
    if *prefs != before {
        prefs.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip() {
        let p = AudioPrefs { muted: true, ambience: 0.25, effects: 1.0 };
        assert!(closeness(FULL_M) == 1.0 && closeness(100.0) == 1.0);
        assert!(closeness(GONE_M) == 0.0 && closeness(3500.0) == 0.0);
        assert!((0.3..0.7).contains(&closeness((FULL_M * GONE_M).sqrt())));
        assert!(AudioPrefs::parse(&p.text()) == Some(p));
        assert!(AudioPrefs::parse("rotto").is_none());
    }

    #[test]
    fn the_sounds_are_in_the_data() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/sounds");
        let names = Ui::ALL.iter().map(|u| u.file().to_string())
            .chain((1..=3).map(|k| format!("cicale_{k}")))
            .chain((1..=4).map(|k| format!("gabbiano_{k}")))
            .chain(["jingle_logo".to_string(), "jingle_isola".to_string()]);
        for n in names {
            assert!(dir.join(format!("{n}.mp3")).exists(), "{n}");
        }
    }
}
