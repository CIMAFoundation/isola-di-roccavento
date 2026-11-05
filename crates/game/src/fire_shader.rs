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
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};

const FLAME_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("9f3d2c61-88aa-11ee-a0b1-223344556677");
const SMOKE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("6c025d4f-93b1-47a8-8e2d-0f19a3c7b456");
const GROUND_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("4b7e119a-2cdd-44ff-9021-aabbccddeeff");
const VOLUME_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("7e2a9c40-3b5f-4d61-9a8e-5c0f1d2b3a47");
const SMOKE_VOLUME_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("c3d81f6e-2a74-4b05-9e17-8f6a0d2c5b39");

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

    /// Drawn after the smoke, every frame (2026-11-04): Bevy sorts the
    /// transparent meshes by the centre of their bounds, and the flames',
    /// embers' and smoke's are rebuilt each frame, so the order flipped
    /// and the flames blinked behind the smoke. Added light on top of the
    /// smoke; far more than the island across, in view-depth metres.
    fn depth_bias(&self) -> f32 {
        if self.mode.x > 1.5 { 2.0e4 } else { 3.0e4 }
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

/// The volume shader's uniform (`shaders/fire_volume.wesl`, `Params`).
#[derive(ShaderType, Clone, Copy, Default, Debug)]
pub struct VolumeParams {
    /// 1 / world width, 1 / world height, the terrain's posting, its height_m.
    pub rect: Vec4,
    /// Seconds a cell flames, fade start and end (m from the eye), spare.
    pub fade: Vec4,
    /// Emission gain, noise period (m), rise (periods / s), absorption (/ m).
    pub look: Vec4,
    /// The height texture's offset (m) and scale (m per code), spare.
    pub elev: Vec4,
}

/// The flames as volumes (2026-11-04): boxes over the flaming band near the
/// camera, ray-marched. See `shaders/fire_volume.wesl`. Additive like the
/// tongues, with the same draw order against the smoke.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct FireVolumeMaterial {
    #[uniform(0)]
    pub params: VolumeParams,
    /// Tileable fbm, `textures::noise_volume`.
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub noise: Handle<Image>,
    /// The fire map (`ground_mask::GRASS_MAP`).
    #[texture(3)]
    #[sampler(4)]
    pub fire_map: Handle<Image>,
    /// The terrain's heights, `Rg8Unorm` (high byte, low byte), read with
    /// `textureLoad`.
    #[texture(5, filterable = false)]
    pub heights: Handle<Image>,
    /// The flame length of every cell (`fire_view::FLAME_MAP`).
    #[texture(6)]
    #[sampler(7)]
    pub flame_map: Handle<Image>,
}

impl Material for FireVolumeMaterial {
    fn vertex_shader() -> ShaderRef {
        VOLUME_SHADER_HANDLE.into()
    }

    fn fragment_shader() -> ShaderRef {
        VOLUME_SHADER_HANDLE.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    /// After the smoke, like the tongues (`FireMaterial::depth_bias`).
    fn depth_bias(&self) -> f32 {
        3.0e4
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Front faces only: the march starts on the face the eye sees and
        // the depth test hides what stands in front of the box. A box
        // around the eye is sent as a quad across the view instead
        // (`fire_view::update_volumes`).
        descriptor.primitive.cull_mode = Some(bevy::render::render_resource::Face::Back);
        Ok(())
    }
}

/// The smoke shader's uniform (`shaders/smoke_volume.wesl`, `Params`).
#[derive(ShaderType, Clone, Copy, Default, Debug)]
pub struct SmokeParams {
    /// 1 / world width, 1 / world height, the terrain's posting, its height_m.
    pub rect: Vec4,
    /// The plume's top (m), its rise (m/s), seconds a cell smokes, metres a step.
    pub plume: Vec4,
    /// The wind's drift (Bevy x, z, m/s), its speed, the lateral margin (m).
    pub wind: Vec4,
    /// Extinction (/ m), noise period (m), gain, seconds a cell flames.
    pub look: Vec4,
    /// The height texture's offset (m) and scale (m per code), spare.
    pub elev: Vec4,
}

/// The smoke as a volume (2026-11-04): boxes over the smoking ground,
/// sheared downwind, ray-marched. See `shaders/smoke_volume.wesl`.
/// Blended like the puffs, and before the flames (their `depth_bias`).
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct SmokeVolumeMaterial {
    #[uniform(0)]
    pub params: SmokeParams,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub noise: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub fire_map: Handle<Image>,
    #[texture(5, filterable = false)]
    pub heights: Handle<Image>,
    #[texture(6)]
    #[sampler(7)]
    pub flame_map: Handle<Image>,
}

impl Material for SmokeVolumeMaterial {
    fn vertex_shader() -> ShaderRef {
        SMOKE_VOLUME_SHADER_HANDLE.into()
    }

    fn fragment_shader() -> ShaderRef {
        SMOKE_VOLUME_SHADER_HANDLE.into()
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
        // front faces only, as the flame volumes
        descriptor.primitive.cull_mode = Some(bevy::render::render_resource::Face::Back);
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
        // an experiment, off (`fire_view::volumes_on`): nothing of it is
        // loaded unless asked for
        if crate::fire_view::volumes_on() {
            crate::load_wesl!(app, VOLUME_SHADER_HANDLE, "shaders/fire_volume.wesl");
            crate::load_wesl!(app, SMOKE_VOLUME_SHADER_HANDLE, "shaders/smoke_volume.wesl");
            app.add_plugins((MaterialPlugin::<FireVolumeMaterial>::default(), MaterialPlugin::<SmokeVolumeMaterial>::default()));
        }
    }
}
