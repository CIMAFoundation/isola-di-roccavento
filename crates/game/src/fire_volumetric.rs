//! Flames and smoke as volumes, ray-marched over the screen at a fraction
//! of its resolution (2026-11-05, the user: «il fumo dall'alto viene
//! visualizzato come delle slices … riprovare con l'impostazione
//! volumetrica … scalando la risoluzione dei volumi in maniera furba e
//! facendo passate di sfocatura»).
//!
//! The billboard puffs are flat: from above they read as stacked slices.
//! The earlier volumes (`fire_view`, boxes ray-marched in the main pass at
//! full resolution, `KIOSK_VOLUMI=scatole`) cost 20 → 76 ms at 4x the
//! pixels. Here the march runs once per pixel of a target `div` times
//! smaller per side, after the main pass, through one field for the whole
//! fire:
//!
//! - the **plume map** ([`PLUME_MAP`]), one texel per fire cell with box
//!   mips: r how much smoke the ground gives off, g how alight it is, b the
//!   flames' height, a the ground's height;
//! - the **smoke** at a point left the ground `h / rise` seconds ago and
//!   drifted with the wind of those seconds since ([`WindMemory`]: a wind
//!   shift bends the column, it does not swing it whole): the map is read
//!   where it left, at a mip that widens with height, torn by 3D noise;
//! - the **flames**: in the bottom tens of metres over the alight band,
//!   rising noise thresholded by the height above the ground, emissive.
//!
//! Each frame jitters the march's first step and blends with the frames
//! before, reprojected (`VolumeSettings::history`): the noise of the coarse
//! grid averages out and the volume stays put as the camera moves. The
//! composite reads the 3x3 low-resolution texels round each screen pixel,
//! weighted by distance and by how alike their depth is (a bilateral blur
//! and upsample in one), and blends the result over the scene before the
//! post effects. Nothing is drawn where the plume's box is off the screen.
//!
//! Off in the game unless `KIOSK_VOLUMI=1`, never on WebGL2; the lab
//! (`fire_lab`, `KIOSK_FUOCO=1`) always has it and sets every knob.

use std::collections::VecDeque;

use bevy::asset::uuid_handle;
use bevy::asset::RenderAssetUsages;
use bevy::camera::MainPassResolutionOverride;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{MeshPipelineViewLayoutKey, MeshPipelineViewLayouts, MeshViewBindGroup, ViewKeyCache};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_3d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_world::RenderEntity;
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;
use fire::CellFire;

use crate::camera::OrbitCamera;
use crate::fire_view::{smoke_wind, vol_flame_m, FLAMING_S};
use crate::sim::Sim;
use crate::terrain_mesh::f16_bits;

const MARCH_SHADER: Handle<Shader> = uuid_handle!("6b0f3e27-9d41-4c8a-a5e2-1f7c3d90b84e");
const COMPOSITE_SHADER: Handle<Shader> = uuid_handle!("e83a51c4-27d6-4f19-9b0e-5c4a72d1f603");
const RESOLVE_SHADER: Handle<Shader> = uuid_handle!("91c2d7e4-5a38-4f0b-b6e1-3d8f20a7c945");

/// The plume map (see the module): `Rgba16Float`, a texel per fire cell,
/// laid out as the fire map (`ground_mask::GRASS_MAP`, row 0 the south edge).
pub const PLUME_MAP: Handle<Image> = uuid_handle!("0d7a9c36-41e8-4b5f-8a13-c62e9f5b07d4");

/// Seconds between rewrites of the plume map.
const PLUME_INTERVAL_S: f32 = 0.1;

/// Whether the game draws the volumes: `KIOSK_VOLUMI=1`, not on WebGL2.
pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !crate::quality::WEBGL2 && std::env::var("KIOSK_VOLUMI").is_ok_and(|v| v == "1"))
}

/// Every knob of the volumes; the lab's panel moves them. The defaults are
/// what the game draws.
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct VolumeSettings {
    /// Screen pixels per march pixel, per side.
    pub div: u32,
    /// Most steps a ray takes.
    pub steps: f32,
    /// Share of the frames before kept each frame (0: none, every frame
    /// on its own; 0.9: smooth, slow to follow).
    pub history: f32,
    /// Seconds a cell gives off smoke after the front reached it.
    pub smoke_s: f32,
    /// Share of the flaming smoke a smouldering cell still gives off.
    pub smoulder: f32,
    /// The plume's top above the ground (m), its rise (m/s), metres of
    /// rise over which it doubles in width.
    pub top_m: f32,
    pub rise_ms: f32,
    pub spread_m: f32,
    /// Extinction per metre of the densest smoke, billow size (m), gain.
    pub sigma: f32,
    pub period_m: f32,
    pub smoke_gain: f32,
    /// Metres up where the column is at full density (thin over the fire).
    pub lift_m: f32,
    /// Where the billows' edge falls in the noise (higher: thinner smoke).
    pub edge: f32,
    /// How much the plume shades itself.
    pub shadow: f32,
    /// Emission gain and absorption per metre of the flames.
    pub flame_gain: f32,
    pub flame_absorb: f32,
    /// The flames grow from their true height near the eye to `flame_toy`
    /// times it far off, between these distances (m), as the tongues and
    /// the houses do (toy scale: from the play camera a true flame is a
    /// few pixels). The volumes draw every flame (2026-11-05).
    pub flame_full_m: f32,
    pub flame_end_m: f32,
    pub flame_toy: f32,
    /// Detail carved into the flames at the screen's resolution, by the
    /// composite: how much (0 off) and the licks' size up close (m).
    pub detail: f32,
    pub detail_m: f32,
    /// Size of the flames' noise (m), sharpness of their edge (smaller:
    /// sharper), height against the sim's flame length, temperature.
    pub flame_period_m: f32,
    pub flame_sharp: f32,
    pub flame_height: f32,
    pub flame_temp: f32,
    /// Seconds of the smoke's own clock a real second (while the sim
    /// runs): the plume rises and drifts on it. On the sim's clock, at
    /// x300, the whole column was new every half second and a wind shift
    /// swung it in that time (user, 2026-11-05: «se il vento gira, il fumo
    /// dovrebbe diradarsi e spandersi, non ruotare e basta»).
    pub pace: f32,
    /// Real seconds over which the smoke's sources follow the fire (the
    /// sim steps a minute at a time at x300: unsmoothed, the plume shook).
    pub smooth_s: f32,
    /// How much smoke that rose through a change of wind spreads (and
    /// thins): metres of width per m/s of the wind's spread, per second.
    pub shear: f32,
    /// The smoke thins out nearer the eye than this (m): from inside the
    /// plume the player would see nothing but brown.
    pub clear_m: f32,
    /// What is drawn (the lab's switches).
    pub show_flames: bool,
    pub show_smoke: bool,
}

impl Default for VolumeSettings {
    fn default() -> Self {
        Self {
            div: 4,
            steps: 80.0,
            history: 0.85,
            smoke_s: FLAMING_S * 3.0,
            smoulder: 0.10,
            top_m: 900.0,
            rise_ms: 6.0,
            spread_m: 45.0,
            sigma: 0.025,
            period_m: 300.0,
            smoke_gain: 1.0,
            lift_m: 300.0,
            edge: 0.44,
            shadow: 1.0,
            flame_gain: 0.7,
            flame_absorb: 0.45,
            // as `fire_view::TRUE_SCALE_NEAR_M`, `TOY_SCALE_FAR_M`
            flame_full_m: 220.0,
            flame_end_m: 750.0,
            flame_toy: 3.0,
            detail: 1.4,
            detail_m: 2.5,
            flame_period_m: 14.0,
            flame_sharp: 0.14,
            flame_height: 1.2,
            flame_temp: 1.0,
            clear_m: 600.0,
            pace: 5.0,
            smooth_s: 1.5,
            shear: 0.4,
            show_flames: true,
            show_smoke: true,
        }
    }
}

impl VolumeSettings {
    /// Sets the knob called `key` (the names of `KIOSK_VOL_SET` and of the
    /// lab's `KIOSK_FUOCO_SET`); false if there is none.
    pub fn set(&mut self, key: &str, x: f32) -> bool {
        match key {
            "div" => self.div = x as u32,
            "passi" => self.steps = x,
            "storia" => self.history = x,
            "fumo_s" => self.smoke_s = x,
            "braci" => self.smoulder = x,
            "cima" => self.top_m = x,
            "risalita" => self.rise_ms = x,
            "allarga" => self.spread_m = x,
            "sigma" => self.sigma = x,
            "volute" => self.period_m = x,
            "fumo_luce" => self.smoke_gain = x,
            "dirada" => self.clear_m = x,
            "ritmo" => self.pace = x,
            "morbidezza_s" => self.smooth_s = x,
            "taglio" => self.shear = x,
            "stacco" => self.lift_m = x,
            "bordo" => self.edge = x,
            "ombra" => self.shadow = x,
            "fiamma_luce" => self.flame_gain = x,
            "fiamma_opaca" => self.flame_absorb = x,
            "fiamma_piena" => self.flame_full_m = x,
            "fiamma_fine" => self.flame_end_m = x,
            "fiamma_giocattolo" => self.flame_toy = x,
            "dettaglio" => self.detail = x,
            "dettaglio_m" => self.detail_m = x,
            "lingue" => self.flame_period_m = x,
            "morbido" => self.flame_sharp = x,
            "altezza" => self.flame_height = x,
            "temp" => self.flame_temp = x,
            "fiamme" => self.show_flames = x != 0.0,
            "fumo" => self.show_smoke = x != 0.0,
            _ => return false,
        }
        true
    }

    /// The defaults, then `KIOSK_VOL_SET=chiave=valore,…` (see [`Self::set`]).
    fn from_env() -> Self {
        let mut s = Self::default();
        // the older names
        if let Some(d) = std::env::var("KIOSK_VOL_DIV").ok().and_then(|v| v.parse().ok()) {
            s.div = d;
        }
        if let Some(n) = std::env::var("KIOSK_VOL_PASSI").ok().and_then(|v| v.parse().ok()) {
            s.steps = n;
        }
        for kv in std::env::var("KIOSK_VOL_SET").unwrap_or_default().split(',').filter(|kv| !kv.is_empty()) {
            match kv.split_once('=').and_then(|(k, v)| Some((k, v.parse::<f32>().ok()?))) {
                Some((k, x)) if s.set(k, x) => {}
                _ => warn!("KIOSK_VOL_SET: {kv}?"),
            }
        }
        s
    }
}

pub struct FireVolumetricPlugin;

impl Plugin for FireVolumetricPlugin {
    fn build(&self, app: &mut App) {
        let lab = crate::fire_lab::wanted();
        if !on() && !lab {
            return;
        }
        crate::load_wesl!(app, MARCH_SHADER, "shaders/fire_march.wesl");
        crate::load_wesl!(app, COMPOSITE_SHADER, "shaders/fire_composite.wesl");
        crate::load_wesl!(app, RESOLVE_SHADER, "shaders/fire_resolve.wesl");
        app.insert_resource(VolumeSettings::from_env()).add_systems(Update, tag_cameras);
        if !lab {
            app.add_systems(Update, update_from_sim.run_if(in_state(crate::AppState::Playing)));
        }
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<Histories>()
            .add_systems(RenderStartup, init_pipelines.after(bevy::pbr::MeshPipelineSystems))
            .add_systems(ExtractSchedule, extract)
            .add_systems(
                Render,
                (
                    depth_usage
                        .in_set(RenderSystems::Prepare)
                        .before(bevy::core_pipeline::core_3d::prepare_core_3d_depth_textures),
                    prepare.in_set(RenderSystems::PrepareResources),
                ),
            )
            .add_systems(Core3d, draw.after(Core3dSystems::MainPass).before(Core3dSystems::EarlyPostProcess));
    }
}

/// The cameras the volumes are drawn for.
#[derive(Component, Default)]
pub struct FireVolumeCamera;

fn tag_cameras(mut commands: Commands, cams: Query<Entity, (With<OrbitCamera>, Without<FireVolumeCamera>)>) {
    for e in &cams {
        commands.entity(e).insert(FireVolumeCamera);
    }
}

// --- main world: the plume map -------------------------------------------

/// The march's uniform (`Params` in both shaders).
#[derive(Clone, Copy, ShaderType, Default)]
struct Params {
    /// 1 / world width, 1 / world height (the map's uv), cell (m), last mip.
    map: Vec4,
    /// The plume's box, min corner (Bevy), and the most steps.
    lo: Vec4,
    /// Max corner, and the height (m) of the flames' zone (0: none).
    hi: Vec4,
    /// The wind now (Bevy x, z, m/s), the rise (m/s), the plume's top (m).
    wind: Vec4,
    /// Extinction (/m), billow size (m), widening (m), gain.
    smoke: Vec4,
    /// Emission gain, absorption (/m), true height up to, toy scale from (m).
    flame: Vec4,
    /// Lift (m), billow edge, self-shadow, flame noise size (m).
    look: Vec4,
    /// Flame edge, height, temperature; the history's share (0: off).
    look2: Vec4,
    /// The march target's size, the main pass's (px).
    size: Vec4,
    /// The main pass's corner in the depth texture (px), the reduction,
    /// the camera's near plane (m).
    origin: Vec4,
    /// The frame's number, radians a march pixel spans, what is drawn,
    /// the smoke's clearing round the eye (m).
    frame: Vec4,
    /// How far smoke of an age has drifted (Bevy x, z), at ages 1/8 … 8/8
    /// of the plume's (`top / rise`), two to a vector.
    drift: [Vec4; 4],
    /// How much wider (in cells) smoke of those ages has spread for the
    /// winds it met, four to a vector.
    shear: [Vec4; 2],
    /// Last frame's clip from world, for the history.
    prev: Mat4,
    /// This frame's world from clip, and the eye (for the resolve, which
    /// has no view bindings).
    world_from_clip: Mat4,
    eye: Vec4,
    /// The flames' toy scale (far off, times the true height), the
    /// carved detail's amount and size (m), the time (s, as the shaders'
    /// `globals.time`).
    more: Vec4,
}

/// The wind of the last few minutes of sim time: smoke aloft drifted with
/// the wind it rose in. A sample holds from its time on.
#[derive(Default, Clone)]
pub struct WindMemory {
    samples: VecDeque<(f32, Vec2)>,
}

impl WindMemory {
    pub fn record(&mut self, now: f32, wind: Vec2) {
        // a restart, or the clock scrubbed back
        if self.samples.back().is_some_and(|&(t, _)| t > now) {
            self.samples.clear();
        }
        if self.samples.back().is_none_or(|&(_, w)| w.distance(wind) > 1e-3) {
            self.samples.push_back((now, wind));
        }
    }

    /// Metres a parcel `age` seconds old drifted by `now`, and how far the
    /// winds it met strayed from their mean (m/s, mean absolute deviation:
    /// after a shift it grows as the time since, not as its square root,
    /// which spread the plume all at once).
    fn drift(&self, now: f32, age: f32) -> (Vec2, f32) {
        let from = now - age;
        let n = self.samples.len();
        let spans = || {
            self.samples.iter().enumerate().filter_map(move |(i, &(t, w))| {
                let start = if i == 0 { f32::NEG_INFINITY } else { t };
                let end = if i + 1 < n { self.samples[i + 1].0 } else { f32::INFINITY };
                let (a, b) = (start.max(from), end.min(now));
                (b > a).then_some((w, b - a))
            })
        };
        let sum: Vec2 = spans().map(|(w, d)| w * d).sum();
        let mean = sum / age.max(1e-3);
        let dev: f32 = spans().map(|(w, d)| w.distance(mean) * d).sum();
        (sum, dev / age.max(1e-3))
    }

    /// Forgets what no smoke still aloft remembers.
    fn trim(&mut self, now: f32, age: f32) {
        while self.samples.len() > 1 && self.samples[1].0 < now - age {
            self.samples.pop_front();
        }
    }
}

/// What the render world needs from this frame: the field's parameters,
/// `None` while nothing smokes or burns.
#[derive(Resource, Clone, Default)]
pub struct PlumeState {
    params: Option<Params>,
    noise: Handle<Image>,
    since: f32,
    generation: u64,
    dims: (usize, usize),
    pub wind: WindMemory,
    settings: Option<VolumeSettings>,
    /// The smoke's own clock (see [`VolumeSettings::pace`]).
    pub clock: f32,
    /// The smoke given off, as last drawn (smoothed), per texel of level 0.
    sources: Vec<f32>,
    /// Whether the sources have caught up with the fire.
    settled: bool,
}

impl PlumeState {
    pub fn new(images: &mut Assets<Image>) -> Self {
        PlumeState { noise: images.add(crate::textures::noise_volume()), since: f32::INFINITY, generation: u64::MAX, ..default() }
    }
}

/// A fire cell for the plume map: when the front reached it (sim s) and
/// its fireline intensity (kW/m); `None` unburnt.
pub type CellBurn = Option<(f32, f32)>;

/// Rewrites the plume map from cells and sizes the plume's box; `wind` is
/// the wind now (Bevy x, z, m/s) and `margin` metres of sway round it.
#[allow(clippy::too_many_arguments)]
pub fn rebuild(
    state: &mut PlumeState,
    images: &mut Assets<Image>,
    world: &scenario::World,
    now: f32,
    dt: f32,
    wind: Vec2,
    margin: f32,
    s: &VolumeSettings,
    cell: impl Fn(usize) -> CellBurn,
    ground: impl Fn(usize) -> f32,
) {
    let (cols, rows) = (world.fire_cols, world.fire_rows);
    if state.dims != (cols, rows) || images.get(&PLUME_MAP).is_none() {
        state.dims = (cols, rows);
        let _ = images.insert(&PLUME_MAP, plume_image(cols, rows));
    }
    let age = s.top_m / s.rise_ms;
    let clock = state.clock;
    state.wind.record(clock, wind);
    state.wind.trim(clock, age);
    state.settings = Some(*s);
    // the sources ease toward the fire's (`dt` real seconds since the last)
    let ease = if s.smooth_s > 0.0 && dt.is_finite() { 1.0 - (-dt / s.smooth_s).exp() } else { 1.0 };
    if state.sources.len() != cols * rows {
        state.sources = vec![0.0; cols * rows];
    }
    let mut settled = true;
    let Some(mut image) = images.get_mut(&PLUME_MAP) else { return };
    let Some(data) = image.data.as_mut() else { return };
    let mut level0 = vec![[0.0f32; 4]; cols * rows];
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let mut any_flame = false;
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            let g = ground(i);
            let at = (rows - 1 - r) * cols + c;
            level0[at][3] = g;
            let mut target = 0.0;
            let burn = cell(i).map(|(arrival, fli)| ((now - arrival).max(0.0), fli)).filter(|(since, _)| *since < s.smoke_s);
            if let Some((since, fli)) = burn {
                let flaming = (1.0 - since / FLAMING_S).clamp(0.0, 1.0);
                // the hot fire smokes most; a share of it as it smoulders out
                let strength = (fli / 2500.0).clamp(0.35, 1.0);
                let smoulder = s.smoulder * (1.0 - (since - FLAMING_S).max(0.0) / (s.smoke_s - FLAMING_S).max(1.0));
                target = strength * if flaming > 0.0 { 0.25 + 0.75 * flaming.sqrt() } else { smoulder };
            }
            let was = state.sources[at];
            let smoke = was + (target - was) * ease;
            let smoke = if smoke < 0.001 && target == 0.0 { 0.0 } else { smoke };
            settled &= (smoke - target).abs() < 0.002;
            state.sources[at] = smoke;
            level0[at][0] = smoke;
            let p = Vec3::new((c as f32 + 0.5) * world.cellsize, g, -(world.height_m - (r as f32 + 0.5) * world.cellsize));
            if smoke > 0.0 {
                lo = lo.min(p);
                hi = hi.max(p);
            }
            let Some((since, fli)) = burn else { continue };
            let flaming = (1.0 - since / FLAMING_S).clamp(0.0, 1.0);
            // tall at the front's edge, low flames and embers behind it
            // the tall flames stand on the front's edge, the band behind it
            // burns low (all of it at full height drew a wall, 2026-11-05)
            let edge = flaming * flaming * flaming;
            let band = smoothstep(0.05, 0.3, flaming) * (0.45 + 0.55 * edge);
            level0[at][1] = band;
            level0[at][2] = if band > 0.0 { vol_flame_m(fli) * (0.15 + 0.85 * edge) } else { 0.0 };
            any_flame |= band > 0.0;
            if band > 0.0 {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
    }
    state.settled = settled;
    write_mips(data, &level0, cols, rows);
    drop(image);
    if !lo.x.is_finite() {
        state.params = None;
        return;
    }
    // how far the smoke of every age has drifted, and the box: the smoking
    // ground, swept by those drifts, as wide as the plume grows
    let mut drift = [Vec4::ZERO; 4];
    let mut shear = [Vec4::ZERO; 2];
    let (mut dlo, mut dhi) = (Vec2::ZERO, Vec2::ZERO);
    let mut widest = 0.0f32;
    for k in 0..8 {
        let a = age * (k + 1) as f32 / 8.0;
        let (d, stray) = state.wind.drift(clock, a);
        // the parcels of this age met winds this far apart: as many metres
        // wider (in cells, as the plume's width is counted)
        let wide = stray * a * s.shear / world.cellsize;
        shear[k / 4][k % 4] = wide;
        widest = widest.max(wide * world.cellsize);
        dlo = dlo.min(d);
        dhi = dhi.max(d);
        let v = &mut drift[k / 2];
        if k % 2 == 0 {
            (v.x, v.y) = (d.x, d.y);
        } else {
            (v.z, v.w) = (d.x, d.y);
        }
    }
    let grow = world.cellsize * (1.0 + s.top_m / s.spread_m) * 0.6 + widest * 0.6 + margin;
    let lo_xz = Vec2::new(lo.x, lo.z) + dlo - grow;
    let hi_xz = Vec2::new(hi.x, hi.z) + dhi + grow;
    let levels = mip_count(cols, rows);
    state.params = Some(Params {
        map: Vec4::new(1.0 / world.width_m, 1.0 / world.height_m, world.cellsize, (levels - 1) as f32),
        lo: Vec4::new(lo_xz.x, lo.y - 15.0, lo_xz.y, s.steps),
        hi: Vec4::new(hi_xz.x, hi.y + s.top_m, hi_xz.y, if any_flame { 45.0 * s.flame_height.max(0.3) } else { 0.0 }),
        wind: Vec4::new(wind.x, wind.y, s.rise_ms, s.top_m),
        smoke: Vec4::new(s.sigma, s.period_m, s.spread_m, s.smoke_gain),
        flame: Vec4::new(s.flame_gain, s.flame_absorb, s.flame_full_m, s.flame_end_m),
        look: Vec4::new(s.lift_m, s.edge, s.shadow, s.flame_period_m),
        look2: Vec4::new(s.flame_sharp, s.flame_height, s.flame_temp, s.history),
        drift,
        shear,
        more: Vec4::new(s.flame_toy, s.detail, s.detail_m, 0.0),
        ..default()
    });
}

/// The game's plume, from the sim.
fn update_from_sim(
    mut commands: Commands,
    sim: Res<Sim>,
    time: Res<Time>,
    settings: Res<VolumeSettings>,
    state: Option<ResMut<PlumeState>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut state) = state else {
        commands.insert_resource(PlumeState::new(&mut images));
        return;
    };
    let dt_real = time.delta_secs();
    state.since += dt_real;
    if !sim.paused() {
        state.clock += dt_real * settings.pace;
    }
    let fresh = state.generation == sim.generation && state.settings == Some(*settings) && state.settled;
    if state.since < PLUME_INTERVAL_S && fresh || state.since < PLUME_INTERVAL_S * 0.25 {
        return;
    }
    let dt = std::mem::replace(&mut state.since, 0.0);
    state.generation = sim.generation;
    let w = sim.scn.world;
    let now = sim.time_s() as f32;
    let (fire_state, arrival, intensity) = (sim.fire.state(), sim.fire.arrival_times(), sim.fire.intensity());
    let wind = smoke_wind(&sim);
    rebuild(
        &mut state,
        &mut images,
        &w,
        now,
        dt,
        Vec2::new(wind.x, wind.y),
        wind.w,
        &settings,
        |i| (fire_state[i] != CellFire::Unburnt && arrival[i] != i32::MIN).then(|| (arrival[i] as f32, intensity[i])),
        |i| sim.scn.dem.get(i).copied().unwrap_or(0.0) as f32,
    );
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mip_count(cols: usize, rows: usize) -> u32 {
    (cols.max(rows) as f32).log2().floor() as u32 + 1
}

fn plume_image(cols: usize, rows: usize) -> Image {
    let levels = mip_count(cols, rows);
    let mut bytes = 0;
    let (mut w, mut h) = (cols, rows);
    for _ in 0..levels {
        bytes += w * h * 8;
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    let mut image = Image::new(
        Extent3d { width: cols as u32, height: rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0u8; bytes],
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    // no COPY_DST: Bevy (0.20) updates a texture that has it in place, and
    // only its first level; the mips, where the smoke is read, kept the
    // first frame's plume. Without it, every rewrite makes a new texture.
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// Level 0 and its box-filtered mips, as half floats, one after another.
fn write_mips(data: &mut [u8], level0: &[[f32; 4]], cols: usize, rows: usize) {
    let mut cur = level0.to_vec();
    let (mut w, mut h) = (cols, rows);
    let mut at = 0;
    loop {
        for px in &cur {
            for (k, v) in px.iter().enumerate() {
                data[at + k * 2..at + k * 2 + 2].copy_from_slice(&f16_bits(*v).to_le_bytes());
            }
            at += 8;
        }
        if (w == 1 && h == 1) || at >= data.len() {
            break;
        }
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![[0.0f32; 4]; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                let mut s = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let v = cur[(2 * y + dy).min(h - 1) * w + (2 * x + dx).min(w - 1)];
                    for k in 0..4 {
                        s[k] += v[k] * 0.25;
                    }
                }
                next[y * nw + x] = s;
            }
        }
        cur = next;
        (w, h) = (nw, nh);
    }
}

// --- render world ----------------------------------------------------------

#[derive(Resource)]
struct Extracted {
    params: Params,
    noise: AssetId<Image>,
    near: f32,
    div: u32,
    /// 1 the flames, 2 the smoke, as bits.
    show: f32,
    clear: f32,
}

/// Marks the volume cameras in the render world.
#[derive(Component)]
struct VolumeView;

fn extract(
    mut commands: Commands,
    state: Extract<Option<Res<PlumeState>>>,
    settings: Extract<Option<Res<VolumeSettings>>>,
    cams: Extract<Query<(RenderEntity, &Projection), With<FireVolumeCamera>>>,
) {
    let params = state.as_ref().and_then(|s| s.params);
    let mut near = 0.1;
    for (e, projection) in &cams {
        let mut cam = commands.entity(e);
        if params.is_some() {
            cam.insert(VolumeView);
        } else {
            cam.remove::<VolumeView>();
        }
        if let Projection::Perspective(p) = projection {
            near = p.near;
        }
    }
    let div = settings.as_ref().map_or(4, |s| s.div.clamp(1, 8));
    let clear = settings.as_ref().map_or(600.0, |s| s.clear_m);
    let show = settings.as_ref().map_or(3.0, |s| f32::from(s.show_flames) + 2.0 * f32::from(s.show_smoke));
    match (params, state.as_ref()) {
        (Some(params), Some(s)) => commands.insert_resource(Extracted { params, noise: s.noise.id(), near, div, show, clear }),
        _ => commands.remove_resource::<Extracted>(),
    }
}

/// The march reads the main pass's depth.
fn depth_usage(mut cams: Query<&mut Camera3d, With<VolumeView>>) {
    for mut cam in &mut cams {
        cam.depth_texture_usages.0 |= TextureUsages::TEXTURE_BINDING.bits();
    }
}

#[derive(Resource)]
struct Pipelines {
    view_layouts: MeshPipelineViewLayouts,
    /// [non-multisampled, multisampled] depth.
    march_layout: [BindGroupLayoutDescriptor; 2],
    resolve_layout: BindGroupLayoutDescriptor,
    composite_layout: [BindGroupLayoutDescriptor; 2],
    fullscreen: FullscreenShader,
    march: HashMap<(MeshPipelineViewLayoutKey, bool), CachedRenderPipelineId>,
    resolve: CachedRenderPipelineId,
    composite: HashMap<(TextureFormat, bool), CachedRenderPipelineId>,
    sampler: Sampler,
    history_sampler: Sampler,
}

fn init_pipelines(
    mut commands: Commands,
    view_layouts: Res<MeshPipelineViewLayouts>,
    fullscreen: Res<FullscreenShader>,
    device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
) {
    let depth = |ms: bool| if ms { texture_depth_2d_multisampled() } else { texture_depth_2d() };
    let march_layout = [false, true].map(|ms| {
        BindGroupLayoutDescriptor::new(
            "fire_march_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<Params>(false),
                    depth(ms),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    texture_3d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        )
    });
    let resolve_layout = BindGroupLayoutDescriptor::new(
        "fire_resolve_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<Params>(false),
                // this frame's march and how far its volume is
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_2d(TextureSampleType::Float { filterable: false }),
                // the frames before
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let composite_layout = [false, true].map(|ms| {
        BindGroupLayoutDescriptor::new(
            "fire_composite_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<Params>(false),
                    depth(ms),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    texture_3d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        )
    });
    let linear = |label: &'static str, mip: MipmapFilterMode| {
        device.create_sampler(&SamplerDescriptor {
            label: Some(label),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: mip,
            ..default()
        })
    };
    let resolve = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("fire_resolve".into()),
        layout: vec![resolve_layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: RESOLVE_SHADER,
            targets: vec![Some(ColorTargetState { format: COLOR_FORMAT, blend: None, write_mask: ColorWrites::ALL })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(Pipelines {
        view_layouts: view_layouts.clone(),
        march_layout,
        resolve_layout,
        composite_layout,
        fullscreen: fullscreen.clone(),
        march: HashMap::default(),
        resolve,
        composite: HashMap::default(),
        sampler: linear("fire_volume_sampler", MipmapFilterMode::Linear),
        history_sampler: linear("fire_volume_history", MipmapFilterMode::Nearest),
    });
}

/// A view's targets: this frame's march (light, the scene's depth, how far
/// the volume is), and two histories written in turn; the last frame's clip
/// from world.
struct History {
    size: UVec2,
    raw: (Texture, TextureView),
    depth: (Texture, TextureView),
    mid: (Texture, TextureView),
    color: [(Texture, TextureView); 2],
    frame: u32,
    prev: Option<Mat4>,
}

impl History {
    fn new(device: &RenderDevice, size: UVec2) -> Self {
        History {
            size,
            raw: target(device, "fire_volume_raw", COLOR_FORMAT, size),
            depth: target(device, "fire_volume_depth", DEPTH_FORMAT, size),
            mid: target(device, "fire_volume_mid", MID_FORMAT, size),
            color: [0, 1].map(|_| target(device, "fire_volume_color", COLOR_FORMAT, size)),
            frame: 0,
            prev: None,
        }
    }
}

#[derive(Resource, Default)]
struct Histories(HashMap<Entity, History>);

/// Per view, this frame: the uniform, the pipelines, which history slot is
/// written.
#[derive(Component)]
struct ViewData {
    uniform: UniformBuffer<Params>,
    march: CachedRenderPipelineId,
    composite: CachedRenderPipelineId,
    multisampled: bool,
    /// The main pass's corner and size in the view (px).
    viewport: (UVec2, UVec2),
    write: usize,
}

const COLOR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const DEPTH_FORMAT: TextureFormat = TextureFormat::R32Float;
/// The volume's distance, the flames' share, distance and height.
const MID_FORMAT: TextureFormat = TextureFormat::Rgba32Float;

fn pass<'a>(encoder: &'a mut CommandEncoder, label: &'static str, targets: &[Option<RenderPassColorAttachment<'_>>]) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: targets,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn attach(view: &TextureView) -> Option<RenderPassColorAttachment<'_>> {
    Some(RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: Operations { load: LoadOp::Clear(Default::default()), store: StoreOp::Store },
    })
}

fn target(device: &RenderDevice, label: &'static str, format: TextureFormat, size: UVec2) -> (Texture, TextureView) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d { width: size.x.max(1), height: size.y.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    (texture, view)
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    mut commands: Commands,
    extracted: Option<Res<Extracted>>,
    mut pipelines: ResMut<Pipelines>,
    mut histories: ResMut<Histories>,
    pipeline_cache: Res<PipelineCache>,
    view_key_cache: Res<ViewKeyCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    views: Query<(Entity, &ExtractedView, &ExtractedCamera, &Msaa, Option<&MainPassResolutionOverride>), With<VolumeView>>,
    time: Res<Time>,
) {
    let Some(extracted) = extracted else {
        histories.0.clear();
        return;
    };
    let Pipelines { view_layouts, march_layout, composite_layout, fullscreen, march: march_ids, composite: composite_ids, .. } =
        &mut *pipelines;
    let mut seen = Vec::new();
    for (entity, view, camera, msaa, over) in &views {
        let Some(key) = view_key_cache.get(&view.retained_view_entity) else { continue };
        let Some(full) = camera.physical_viewport_size else { continue };
        let corner = camera.viewport.as_ref().map_or(UVec2::ZERO, |v| v.physical_position);
        let size = over.map_or(full, |o| o.0.min(full)).max(UVec2::ONE);
        let d = extracted.div;
        let low = UVec2::new(size.x.div_ceil(d), size.y.div_ceil(d));
        let ms = !matches!(msaa, Msaa::Off);
        let view_key: MeshPipelineViewLayoutKey = (*key).into();
        seen.push(entity);

        let march = *march_ids.entry((view_key, ms)).or_insert_with(|| {
            let mut defs: Vec<bevy::shader::ShaderDefVal> = Vec::new();
            if ms {
                defs.push("MULTISAMPLED".into());
            }
            // the view's bindings as the layout has them (the scene's fog)
            if view_key.contains(MeshPipelineViewLayoutKey::DISTANCE_FOG) {
                defs.push("DISTANCE_FOG".into());
            }
            let t = |format| Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL });
            pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("fire_march".into()),
                layout: vec![view_layouts.get_view_layout(view_key).main_layout, march_layout[ms as usize].clone()],
                vertex: fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: MARCH_SHADER,
                    shader_defs: defs,
                    targets: vec![t(COLOR_FORMAT), t(DEPTH_FORMAT), t(MID_FORMAT)],
                    ..default()
                }),
                ..default()
            })
        });
        let composite = *composite_ids.entry((view.target_format, ms)).or_insert_with(|| {
            let mut defs: Vec<bevy::shader::ShaderDefVal> = Vec::new();
            if ms {
                defs.push("MULTISAMPLED".into());
            }
            pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("fire_composite".into()),
                layout: vec![composite_layout[ms as usize].clone()],
                vertex: fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: COMPOSITE_SHADER,
                    shader_defs: defs,
                    targets: vec![Some(ColorTargetState {
                        format: view.target_format,
                        // premultiplied: the volume's light, then the scene
                        // through what it lets pass
                        blend: Some(BlendState {
                            color: BlendComponent {
                                src_factor: BlendFactor::One,
                                dst_factor: BlendFactor::OneMinusSrcAlpha,
                                operation: BlendOperation::Add,
                            },
                            alpha: BlendComponent {
                                src_factor: BlendFactor::Zero,
                                dst_factor: BlendFactor::One,
                                operation: BlendOperation::Add,
                            },
                        }),
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                ..default()
            })
        });

        // the history: new targets when the size changed, and no history
        // in them yet
        let history = histories.0.entry(entity).or_insert_with(|| History::new(&device, low));
        let mut keep = extracted.params.look2.w;
        history.frame = history.frame.wrapping_add(1);
        let write = (history.frame % 2) as usize;
        if history.size != low {
            // a new size (the adaptive resolution moves it, most at the
            // start): new march targets, and the slot about to be written;
            // the frames before are read by uv, whatever their size, so
            // they carry over (dropped, each change flashed the raw march)
            history.size = low;
            history.raw = target(&device, "fire_volume_raw", COLOR_FORMAT, low);
            history.depth = target(&device, "fire_volume_depth", DEPTH_FORMAT, low);
            history.mid = target(&device, "fire_volume_mid", MID_FORMAT, low);
        }
        let slot = &history.color[write].0;
        if (slot.width(), slot.height()) != (low.x, low.y) {
            history.color[write] = target(&device, "fire_volume_color", COLOR_FORMAT, low);
        }
        let world_from_view = view.world_from_view.to_matrix();
        let clip_from_world = view.clip_from_view * world_from_view.inverse();
        let prev = history.prev.unwrap_or_else(|| {
            keep = 0.0;
            clip_from_world
        });
        history.prev = Some(clip_from_world);

        let mut params = extracted.params;
        params.look2.w = keep;
        params.size = Vec4::new(low.x as f32, low.y as f32, size.x as f32, size.y as f32);
        params.origin = Vec4::new(corner.x as f32, corner.y as f32, d as f32, extracted.near);
        // radians a march pixel spans, from the vertical field of view
        let pixel = 2.0 / (view.clip_from_view.y_axis.y.abs().max(1e-3) * size.y as f32) * d as f32;
        // a still march when nothing is kept: the jitter would only boil
        let frame = if keep > 0.0 { (history.frame % 64) as f32 } else { 0.0 };
        params.frame = Vec4::new(frame, pixel, extracted.show, extracted.clear);
        params.prev = prev;
        params.world_from_clip = world_from_view * view.clip_from_view.inverse();
        params.eye = view.world_from_view.translation().extend(1.0);
        params.more.w = time.elapsed_secs_wrapped();
        let mut uniform = UniformBuffer::from(params);
        uniform.write_buffer(&device, &queue);
        commands.entity(entity).insert(ViewData {
            uniform,
            march,
            composite,
            multisampled: ms,
            viewport: (corner, size),
            write,
        });
    }
    histories.0.retain(|e, _| seen.contains(e));
}

fn draw(
    view: ViewQuery<(Entity, &ViewTarget, &ViewDepthStencilTexture, &MeshViewBindGroup, &ViewData), With<VolumeView>>,
    pipeline_cache: Res<PipelineCache>,
    pipelines: Res<Pipelines>,
    histories: Res<Histories>,
    extracted: Option<Res<Extracted>>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (entity, target, depth, view_bind_group, data) = view.into_inner();
    let Some(extracted) = extracted else { return };
    let Some(history) = histories.0.get(&entity) else { return };
    let (Some(march), Some(resolve), Some(composite)) = (
        pipeline_cache.get_render_pipeline(data.march),
        pipeline_cache.get_render_pipeline(pipelines.resolve),
        pipeline_cache.get_render_pipeline(data.composite),
    ) else {
        return;
    };
    let (Some(plume), Some(noise)) = (images.get(PLUME_MAP.id()), images.get(extracted.noise)) else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let Some(uniform) = data.uniform.binding() else { return };
    let ms = data.multisampled as usize;
    let (w, r) = (data.write, 1 - data.write);

    let device = ctx.render_device();
    let march_group = device.create_bind_group(
        "fire_march_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.march_layout[ms]),
        &BindGroupEntries::sequential((
            uniform.clone(),
            depth_view,
            &plume.texture_view,
            &pipelines.sampler,
            &noise.texture_view,
            &noise.sampler,
        )),
    );
    let resolve_group = device.create_bind_group(
        "fire_resolve_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.resolve_layout),
        &BindGroupEntries::sequential((
            uniform.clone(),
            &history.raw.1,
            &history.mid.1,
            &history.color[r].1,
            &pipelines.history_sampler,
        )),
    );
    let composite_group = device.create_bind_group(
        "fire_composite_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.composite_layout[ms]),
        &BindGroupEntries::sequential((
            uniform,
            depth_view,
            &history.color[w].1,
            &history.depth.1,
            &history.mid.1,
            &noise.texture_view,
            &noise.sampler,
        )),
    );

    let encoder = ctx.command_encoder();
    {
        let mut p = pass(encoder, "fire_march", &[attach(&history.raw.1), attach(&history.depth.1), attach(&history.mid.1)]);
        p.set_pipeline(march);
        p.set_bind_group(0, &view_bind_group.main, &view_bind_group.main_offsets);
        p.set_bind_group(1, &march_group, &[]);
        p.draw(0..3, 0..1);
    }
    {
        let mut p = pass(encoder, "fire_resolve", &[attach(&history.color[w].1)]);
        p.set_pipeline(resolve);
        p.set_bind_group(0, &resolve_group, &[]);
        p.draw(0..3, 0..1);
    }
    {
        let load = [Some(RenderPassColorAttachment {
            view: target.main_texture_view(),
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
        })];
        let mut p = pass(encoder, "fire_composite", &load);
        let (corner, size) = data.viewport;
        p.set_viewport(corner.x as f32, corner.y as f32, size.x as f32, size.y as f32, 0.0, 1.0);
        p.set_pipeline(composite);
        p.set_bind_group(0, &composite_group, &[]);
        p.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_steady_wind_drifts_straight_and_spreads_nothing() {
        let mut m = WindMemory::default();
        for t in 0..100 {
            m.record(t as f32, Vec2::new(3.0, 4.0));
        }
        let (d, stray) = m.drift(99.0, 50.0);
        assert!((d - Vec2::new(150.0, 200.0)).length() < 1e-3);
        assert!(stray < 1e-4);
    }

    #[test]
    fn a_shift_bends_the_old_smoke_and_spreads_it_as_the_time_since() {
        let mut m = WindMemory::default();
        m.record(0.0, Vec2::new(5.0, 0.0));
        m.record(100.0, Vec2::new(0.0, 5.0));
        // smoke that rose after the shift went the new way only
        let (d, stray) = m.drift(110.0, 10.0);
        assert!((d - Vec2::new(0.0, 50.0)).length() < 1e-3 && stray < 1e-4);
        // older smoke: both ways, and the spread grows with the time since
        let (d, _) = m.drift(110.0, 50.0);
        assert!((d - Vec2::new(200.0, 50.0)).length() < 1e-3);
        let spread = |since: f32| m.drift(100.0 + since, 100.0).1 * 100.0;
        let (a, b) = (spread(5.0), spread(10.0));
        assert!(a > 0.0 && (b / a - 2.0).abs() < 0.15, "{a} {b}");
    }
}
