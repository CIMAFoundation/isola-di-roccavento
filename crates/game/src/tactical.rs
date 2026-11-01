//! The tactical view: while the world is paused for orders, the scenery that
//! carries no decision (ground, plants, sea, sky) is drawn toward grey and the
//! haze and glow are switched off, so houses, units, fire and the interface
//! stand out. While time runs the same scenery is a little more saturated than
//! its natural colours. One signed amount drives both: negative boosts the
//! colour, positive greys it. The amount eases in and out; the shaders only
//! read it (`terrain.wgsl`, `foliage.wgsl`, `water.wgsl`, and the sky
//! here).

use bevy::post_process::bloom::Bloom;
use bevy::pbr::{FogFalloff, DistanceFog};
use bevy::prelude::*;

use crate::kiosk::{Kiosk, Phase};
use crate::flat::FlatMaterial;
use crate::forecast_zone::ZoneMaterial;
use crate::materials::{FoliageMaterial, TerrainMaterial};
use crate::sea::WaterMaterial;
use crate::sim::Sim;
use crate::sky::{SkyHandle, SkyMaterial};
use crate::tape::TapeMaterial;

/// Desaturation at full effect. `KIOSK_TACTICAL=0..1` overrides it (0 = off).
const DEFAULT_AMOUNT: f32 = 0.55;
/// Saturation gain of the game's own signs in the pause (zones, routes, tape).
const SIGN_GAIN: f32 = 0.75;
/// Extra saturation while time runs (0.15 = 15 % more). `KIOSK_BOOST` overrides.
/// 0 since 2026-11-01: it made up for TonyMcMapface's washed colours, and
/// the tone mapper is now AcesFitted (see `grading` in `main.rs`).
const DEFAULT_BOOST: f32 = 0.0;
/// Real seconds to ease fully in or out.
const EASE_S: f32 = 0.4;

#[derive(Resource)]
pub struct Tactical {
    /// Eased progress of the pause, 0 (running) to 1 (paused for orders).
    level: f32,
    /// Desaturation at `level` = 1, and extra saturation at `level` = 0.
    max: f32,
    boost: f32,
}

impl Tactical {
    /// The eased pause, 0..1.
    fn eased(&self) -> f32 {
        self.level * self.level * (3.0 - 2.0 * self.level)
    }

    /// Signed colour amount for terrain, foliage, sea and sky: negative
    /// boosts saturation, positive desaturates.
    fn amount(&self) -> f32 {
        -self.boost + self.eased() * (self.max + self.boost)
    }
}

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|v| v.parse::<f32>().ok()).map_or(default, |v| v.clamp(0.0, 1.0))
}

pub struct TacticalPlugin;

impl Plugin for TacticalPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Tactical {
            level: 0.0,
            max: env_f32("KIOSK_TACTICAL", DEFAULT_AMOUNT),
            boost: env_f32("KIOSK_BOOST", DEFAULT_BOOST),
        })
        .add_systems(Update, (apply, effects_off.after(crate::sky::update_sky)));
    }
}

fn apply(
    time: Res<Time>,
    kiosk: Option<Res<Kiosk>>,
    sim: Option<Res<Sim>>,
    mario: Option<Res<crate::mario::Playground>>,
    mut tactical: ResMut<Tactical>,
    mut terrain: ResMut<Assets<TerrainMaterial>>,
    mut foliage: ResMut<Assets<FoliageMaterial>>,
    mut cover: ResMut<Assets<crate::near_cover::NearCoverMaterial>>,
    mut water: ResMut<Assets<WaterMaterial>>,
    mut flats: ResMut<Assets<FlatMaterial>>,
    mut zones: ResMut<Assets<ZoneMaterial>>,
    mut tapes: ResMut<Assets<TapeMaterial>>,
) {
    if let (Some(kiosk), Some(sim)) = (kiosk, sim) {
        let playing = matches!(kiosk.phase, Phase::Tutorial | Phase::Gioco);
        // Mario walks the island in full colour, paused or not
        let exploring = mario.is_some_and(|m| m.active);
        let want = if playing && sim.paused() && !exploring { 1.0 } else { 0.0 };
        // a screenshot of a state has no time to ease in
        let step = if std::env::var_os("KIOSK_SNAP").is_some() { 1.0 } else { time.delta_secs() / EASE_S };
        tactical.level = if want > tactical.level { (tactical.level + step).min(want) } else { (tactical.level - step).max(want) };
    }
    let amount = tactical.amount();
    // Writing marks the asset changed and rebuilds its bind group, so only
    // touch the materials whose value differs (new ones start at 0).
    let stale: Vec<_> = terrain.iter().filter(|(_, m)| m.tactical() != amount).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = terrain.get_mut(id) {
            m.set_tactical(amount);
        }
    }
    let stale: Vec<_> = foliage.iter().filter(|(_, m)| m.tactical() != amount).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = foliage.get_mut(id) {
            m.set_tactical(amount);
        }
    }
    let stale: Vec<_> = cover.iter().filter(|(_, m)| m.extension.tactical() != amount).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = cover.get_mut(id) {
            m.extension.set_tactical(amount);
        }
    }
    // The game's own signs (hexes, routes, forecast zones, tape) gain
    // saturation in the pause, so they stand out from the greyed scenery.
    let gain = tactical.eased() * if tactical.max > 0.0 { SIGN_GAIN } else { 0.0 };
    let stale: Vec<_> = flats.iter().filter(|(_, m)| m.saturation() != gain).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = flats.get_mut(id) {
            m.set_saturation(gain);
        }
    }
    let stale: Vec<_> = zones.iter().filter(|(_, m)| m.saturation() != gain).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = zones.get_mut(id) {
            m.set_saturation(gain);
        }
    }
    let stale: Vec<_> = tapes.iter().filter(|(_, m)| m.mode.y != gain).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = tapes.get_mut(id) {
            m.mode.y = gain;
        }
    }
    let stale: Vec<_> = water.iter().filter(|(_, m)| m.uniform.tactical.x != amount).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = water.get_mut(id) {
            m.uniform.tactical.x = amount;
        }
    }
}

/// Haze and glow are not information. The sky resets fog, sky and clouds
/// every frame, so this runs after it: fade the fog out with its sun glow, the
/// bloom, the sky gradient, sun and clouds; and carry the colour amount into
/// the sky. At rest (running, no boost) nothing changes.
fn effects_off(
    tactical: Res<Tactical>,
    mut fog: Query<&mut DistanceFog>,
    mut bloom: Query<&mut Bloom>,
    mut bloom_base: Local<Option<f32>>,
    sky_handle: Option<Res<SkyHandle>>,
    mut skies: ResMut<Assets<SkyMaterial>>,
) {
    let amount = tactical.amount();
    // how far the effects are off: only the pause turns them off
    let k = tactical.eased() * tactical.max;
    if let Some(mut mat) = sky_handle.and_then(|h| skies.get_mut(&h.0)) {
        let u = &mut mat.uniform;
        let luma = |c: Vec4| c.truncate().dot(Vec3::new(0.2126, 0.7152, 0.0722));
        let flat = u.horizon_color.lerp(u.zenith_color, 0.25);
        let grey = |c: Vec4| Vec4::splat(luma(c) * 0.9).with_w(1.0);
        // gradient to one tone, then the signed colour amount around grey
        u.zenith_color = u.zenith_color.lerp(flat, k);
        u.horizon_color = u.horizon_color.lerp(flat, k);
        u.zenith_color = u.zenith_color.lerp(grey(u.zenith_color), amount);
        u.horizon_color = u.horizon_color.lerp(grey(u.horizon_color), amount);
        u.sun_color *= 1.0 - k;
        u.moon_color *= 1.0 - k;
        // coverage below zero pushes the cloud threshold past any density
        u.cloud.z = u.cloud.z * (1.0 - k) - k;
    }
    if k <= 0.0 {
        // nobody else rewrites the bloom, so remember its normal intensity
        *bloom_base = bloom.iter().next().map(|b| b.intensity);
        return;
    }
    for mut f in &mut fog {
        // visibility from 11 km to effectively unlimited
        let c = f.color;
        f.falloff = FogFalloff::from_visibility_colors(11_000.0 + 2_000_000.0 * k * k, c, c);
        f.directional_light_color = Color::NONE;
    }
    for mut b in &mut bloom {
        b.intensity = bloom_base.unwrap_or(b.intensity) * (1.0 - k);
    }
}
