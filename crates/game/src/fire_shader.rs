//! Custom shader materials for the fire.
//!
//! Everything in [`crate::fire_view`] decides *where* fire geometry goes —
//! flame tongue placement, tongue count, the fire map the terrain's shader
//! draws the scar and the glow band from (`shaders/fire_ground.wesl`). What
//! a `StandardMaterial` cannot do is make that geometry look like it is
//! burning rather than merely lit: a billboard with a static texture reads as
//! a sprite the moment the camera holds still. These two materials add a
//! per-pixel animated domain warp and flicker, driven by `globals.time` and
//! world position, so flame edges roil and the ground's glow band breathes
//! even between the fire map's ~150 ms rewrites.

use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::shader::ShaderRef;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};

const FLAME_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("9f3d2c61-88aa-11ee-a0b1-223344556677");
const SMOKE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("6c025d4f-93b1-47a8-8e2d-0f19a3c7b456");
const GROUND_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("4b7e119a-2cdd-44ff-9021-aabbccddeeff");

/// Flame tongues and embers: a textured, additive billboard whose lookup UV
/// is warped by animated noise. See `shaders/flame.wgsl`.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct FireMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    /// x = 1 for flame tongues (rising, eroding, heat ramp); 0 for embers,
    /// which keep the plain warped-texture look.
    #[uniform(2)]
    pub mode: Vec4,
}

impl Material for FireMaterial {
    fn fragment_shader() -> ShaderRef {
        FLAME_SHADER_HANDLE.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Billboards are built double-sided (see QuadBuilder) precisely so
        // they read from any camera angle.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Smoke puffs: unlit, alpha-blended, fogged by hand. See `shaders/smoke.wgsl`.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct SmokeMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
}

impl Material for SmokeMaterial {
    fn vertex_shader() -> ShaderRef {
        SMOKE_SHADER_HANDLE.into()
    }

    fn fragment_shader() -> ShaderRef {
        SMOKE_SHADER_HANDLE.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Built double-sided, like the other billboards.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub struct FireShaderPlugin;

impl Plugin for FireShaderPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, FLAME_SHADER_HANDLE, "shaders/flame.wesl");
        // a module of the terrain's shader (`game::shaders::fire_ground`)
        crate::load_wesl!(app, GROUND_SHADER_HANDLE, "shaders/fire_ground.wesl");
        crate::load_wesl!(app, SMOKE_SHADER_HANDLE, "shaders/smoke.wesl");
        app.add_plugins((
            MaterialPlugin::<FireMaterial>::default(),
            MaterialPlugin::<SmokeMaterial>::default(),
        ));
    }
}
