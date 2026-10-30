//! God rays (user, 2026-10-30), «alta» only: when the sun is in the picture,
//! what is bright around it (the disc, the lit cloud edges, the sky between
//! the island's ridges) is smeared outward along the lines from the sun, a
//! screen-space radial blur after the scene and before tonemapping. Cheap and
//! honest enough at this scale: the sun's shafts break on the clouds and the
//! island's profile, which the shadow maps would not see.
//!
//! The pass exists only while it shows: the camera carries [`GodRays`] when
//! the profile is «alta», the sun is up and on screen or near its edge.

use bevy::asset::uuid_handle;
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::core_pipeline::Core3dSystems;
use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::BoxedSystem;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::RenderApp;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;

use crate::camera::OrbitCamera;

const GODRAYS_SHADER_HANDLE: Handle<Shader> = uuid_handle!("3c9e51d2-8a47-4f0b-b6e1-7d2c90a4e115");

pub struct GodRaysPlugin;

impl Plugin for GodRaysPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, GODRAYS_SHADER_HANDLE, "shaders/godrays.wesl");
        app.add_plugins(FullscreenMaterialPlugin::<GodRays>::default())
            .add_systems(PostUpdate, aim.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// The pass's settings, as the shader reads them.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default)]
#[extract_app(RenderApp)]
pub struct GodRays {
    /// The sun on the screen, uv (0,0 top left); may lie a little outside.
    sun_uv: Vec2,
    /// 0 = no rays.
    strength: f32,
    /// Share of the sun's luminance under which nothing makes rays.
    threshold: f32,
    /// Width / height of the screen, to keep the glow round.
    aspect: f32,
    _pad: Vec3,
}

impl FullscreenMaterial for GodRays {
    fn fragment_shader() -> ShaderRef {
        GODRAYS_SHADER_HANDLE.into()
    }

    /// Before the bloom, so before the depth of field (which follows the bloom):
    /// «alta» has both, and unordered against them this pass read a
    /// half-written texture. The rays then glow and blur with the scene.
    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system.in_set(Core3dSystems::PostProcess).before(bevy::post_process::bloom::bloom)
    }
}

/// Full strength; tuned on the opening's sunrise (2026-10-30).
const STRENGTH: f32 = 2.5;
/// Share of the sun disc's luminance from which a pixel makes rays.
const THRESHOLD: f32 = 0.15;

/// Puts the sun on the screen, or takes the pass away.
fn aim(
    mut commands: Commands,
    gfx: Res<crate::quality::Graphics>,
    sun: Res<crate::sky::SunState>,
    cams: Query<(Entity, &Camera, &GlobalTransform, Option<&GodRays>), With<OrbitCamera>>,
) {
    let alta = gfx.level() == Some("alta") && !cfg!(target_arch = "wasm32");
    for (e, cam, gt, had) in &cams {
        let rays = alta.then(|| {
            let at = gt.translation() + sun.direction * 10_000.0;
            let ndc = cam.world_to_ndc(gt, at).filter(|n| n.z > 0.0 && n.z < 1.0)?;
            // fades out as the sun leaves the screen
            let off = ndc.x.abs().max(ndc.y.abs());
            let on = 1.0 - ((off - 1.0) / 0.5).clamp(0.0, 1.0);
            // strongest with the sun low, as at dawn
            let low = 1.0 - 0.6 * ((sun.direction.y - 0.2) / 0.6).clamp(0.0, 1.0);
            let strength = STRENGTH * on * low * (sun.day_gate * 4.0).min(1.0);
            let size = cam.logical_viewport_size().unwrap_or(Vec2::ONE);
            (strength > 0.01).then_some(GodRays {
                sun_uv: Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5),
                strength,
                threshold: THRESHOLD,
                aspect: size.x / size.y.max(1.0),
                _pad: Vec3::ZERO,
            })
        }).flatten();
        match (rays, had.is_some()) {
            (Some(r), _) => { commands.entity(e).insert(r); }
            (None, true) => { commands.entity(e).remove::<GodRays>(); }
            (None, false) => {}
        }
    }
}
