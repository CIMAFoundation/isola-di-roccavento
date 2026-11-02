//! Adaptive resolution (user, 2026-11-02): the 3D scene is drawn at a share
//! of the screen's resolution ([`Graphics::render_scale`], per side) and
//! scaled up before the post effects, so the interface, its text and the
//! effects stay at full resolution while the GPU colours fewer pixels.
//! `adaptive` moves the share with the frame time; `KIOSK_GFX` / `?gfx=`
//! `scale=0.5` fixes it.
//!
//! Bevy draws the main pass (prepass, opaque, transparent) into the top-left
//! corner of the view at the size of `MainPassResolutionOverride`; this module
//! puts that component on the camera in the render world and, right after the
//! main pass, stretches the corner over the whole view (bilinear). What reads
//! the depth at screen resolution would not match it: the depth of field is
//! off below 1 (`quality_live`), and Mario's outline keeps the full resolution.
//!
//! [`Graphics::render_scale`]: crate::quality::Graphics::render_scale

use bevy::asset::uuid_handle;
use bevy::camera::MainPassResolutionOverride;
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::core_pipeline::Core3dSystems;
use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::BoxedSystem;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::render::sync_world::RenderEntity;
use bevy::render::{Extract, ExtractSchedule, RenderApp};
use bevy::shader::ShaderRef;

use crate::camera::OrbitCamera;

const UPSCALE_SHADER_HANDLE: Handle<Shader> = uuid_handle!("2686744e-44c2-41d7-9a65-b8d9da6b7179");

/// Below this share the scene is scaled; at or above it, drawn whole.
const WHOLE: f32 = 0.999;

pub struct RenderScalePlugin;

impl Plugin for RenderScalePlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, UPSCALE_SHADER_HANDLE, "shaders/upscale.wesl");
        app.add_plugins(FullscreenMaterialPlugin::<Upscale>::default())
            .add_systems(PostUpdate, aim.after(bevy::camera::CameraUpdateSystems));
        app.sub_app_mut(RenderApp).add_systems(ExtractSchedule, extract_override);
    }
}

/// Whether the scene of a camera is drawn whole at this share.
pub fn whole(scale: f32) -> bool {
    scale >= WHOLE
}

/// The stretch, as the shader reads it; on the camera only while it scales.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, PartialEq)]
#[extract_app(RenderApp)]
pub struct Upscale {
    /// Main-pass pixels per screen pixel, per side.
    ratio: Vec2,
    /// The last main-pass texel, so the filter never reads past the corner.
    last: Vec2,
    /// The main pass's size, physical pixels.
    size: Vec2,
    _pad: Vec2,
}

impl FullscreenMaterial for Upscale {
    fn fragment_shader() -> ShaderRef {
        UPSCALE_SHADER_HANDLE.into()
    }

    /// Right after the main pass, before every post effect: they all see the
    /// scene at the full size.
    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system.in_set(Core3dSystems::EarlyPostProcess)
    }
}

/// Size the main pass from the share and the camera's viewport, or take the
/// stretch away when the scene is drawn whole.
fn aim(
    mut commands: Commands,
    gfx: Res<crate::quality::Graphics>,
    mario: Option<Res<crate::mario::Playground>>,
    cams: Query<(Entity, &Camera, Option<&Upscale>), With<OrbitCamera>>,
) {
    // Mario's outline reads the depth at screen resolution
    let scale = if mario.is_some_and(|m| m.active) { 1.0 } else { gfx.render_scale };
    for (e, cam, had) in &cams {
        let want = (!whole(scale)).then(|| cam.physical_viewport_size()).flatten().map(|full| {
            let full = full.as_vec2().max(Vec2::ONE);
            let size = (full * scale).round().clamp(Vec2::ONE, full);
            Upscale { ratio: size / full, last: size - 1.0, size, _pad: Vec2::ZERO }
        });
        match (want, had) {
            (Some(u), Some(h)) if u == *h => {}
            (Some(u), _) => {
                commands.entity(e).insert(u);
            }
            (None, Some(_)) => {
                commands.entity(e).remove::<Upscale>();
            }
            (None, None) => {}
        }
    }
}

/// Bevy reads the main pass's size from the camera in the render world.
fn extract_override(mut commands: Commands, cams: Extract<Query<(&RenderEntity, Option<&Upscale>), With<OrbitCamera>>>) {
    for (e, up) in &cams {
        let mut cam = commands.entity(e.id());
        match up {
            Some(u) => {
                cam.insert(MainPassResolutionOverride(u.size.as_uvec2()));
            }
            None => {
                cam.remove::<MainPassResolutionOverride>();
            }
        }
    }
}
