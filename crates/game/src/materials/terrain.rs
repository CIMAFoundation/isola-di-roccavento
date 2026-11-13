//! The terrain's material (`shaders/terrain.wgsl`): the same on the desktop and
//! in the browser.

use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

const SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f10");

#[derive(Clone, Copy, Default, ShaderType)]
struct TerrainUniform {
    mask_rect: Vec4,
    ground_rect: Vec4,
    /// x: desaturation; y, z: the canopy's fade from shade to crowns, metres
    /// from the eye; w: the forest's occlusion, 0 (off) to 1
    /// (`vegetation::canopy::forest_shade`).
    tactical: Vec4,
    /// Mean crown colour (linear) of conifers, broadleaves and macchia; in
    /// the conifers' w, the terrain samples' posting (m).
    canopy_conifer: Vec4,
    canopy_broadleaf: Vec4,
    canopy_macchia: Vec4,
    /// x: the drawing clock less the front map's base (s,
    /// `fire_ahead::FireAhead::base`); y: 1 when the front map is read; z: 1
    /// when the flames are the volumes' (`fire_volumetric`): the ground
    /// under them is char and embers, not the fire itself.
    fire_front: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct TerrainMaterial {
    #[uniform(0)]
    uniform: TerrainUniform,
    /// Road distance field (`ground_mask`).
    #[texture(1)]
    #[sampler(2)]
    pub road_mask: Handle<Image>,
    /// Meadow cover and the fire, one texel per fire cell
    /// (`ground_mask::grass_map`, `fire_view::update_fire_map`).
    #[texture(3)]
    #[sampler(4)]
    pub grass_map: Handle<Image>,
    /// Ground colour (sRGB) with the canopy's cover in alpha, and surface
    /// normal (Bevy frame: x, z; y is rebuilt) with the canopy's kind and
    /// brightness, one texel per terrain sample, so the look does not depend
    /// on the mesh density.
    #[texture(5)]
    #[sampler(6)]
    pub ground_colour: Handle<Image>,
    #[texture(7)]
    #[sampler(8)]
    pub ground_normal: Handle<Image>,
    /// The realistic ground's photo layers (2026-11-06,
    /// `textures::ground_detail_image`), read only with the scanned plants
    /// (`REAL_GROUND`); a stand-in otherwise. In the browser only in the
    /// WebGPU build: on WebGL2 every texture of the layout takes one of its
    /// 16 units, and there are no scanned plants.
    #[cfg(any(not(target_arch = "wasm32"), feature = "webgpu"))]
    #[texture(9, dimension = "2d_array")]
    #[sampler(10)]
    pub ground_detail: Handle<Image>,
    /// The front drawn ahead of the game's (`fire_ahead::FRONT_MAP`), read
    /// texel by texel.
    #[texture(11)]
    front_map: Handle<Image>,
}

impl TerrainMaterial {
    /// `mask_rect`: world metres to mask coordinates
    /// (`ground_mask::RoadMask::rect`). `ground_rect`: world metres to the
    /// ground textures' uv: x, y = 1 / (posting * size), z = half a texel in
    /// u, w = the north edge plus half a texel in v.
    pub fn new(
        mask_rect: Vec4,
        ground_rect: Vec4,
        road_mask: Handle<Image>,
        grass_map: Handle<Image>,
        ground_colour: Handle<Image>,
        ground_normal: Handle<Image>,
        #[cfg_attr(all(target_arch = "wasm32", not(feature = "webgpu")), allow(unused_variables))] ground_detail: Handle<Image>,
    ) -> Self {
        TerrainMaterial {
            uniform: TerrainUniform {
                mask_rect,
                ground_rect,
                fire_front: Vec4::new(0.0, 0.0, f32::from(crate::fire_volumetric::on()), 0.0),
                ..Default::default()
            },
            road_mask,
            grass_map,
            ground_colour,
            ground_normal,
            #[cfg(any(not(target_arch = "wasm32"), feature = "webgpu"))]
            ground_detail,
            front_map: crate::fire_ahead::FRONT_MAP,
        }
    }

    /// The drawing clock against the front map's base (`fire_ahead`).
    pub fn front_clock(&self) -> f32 {
        self.uniform.fire_front.x
    }

    pub fn front_on(&self) -> bool {
        self.uniform.fire_front.y > 0.5
    }

    pub fn set_front_clock(&mut self, rel: f32, on: bool) {
        self.uniform.fire_front.x = rel;
        self.uniform.fire_front.y = f32::from(on);
    }

    /// How far the colours are drawn toward grey, 0..1 (`tactical.rs`).
    pub fn tactical(&self) -> f32 {
        self.uniform.tactical.x
    }

    pub fn set_tactical(&mut self, amount: f32) {
        self.uniform.tactical.x = amount;
    }

    /// The crowns' colours by kind and the distances of their fade
    /// (`vegetation::canopy`).
    pub fn set_canopy(&mut self, colours: [[f32; 3]; 3], fade: [f32; 2]) {
        let v = |c: [f32; 3]| Vec4::new(c[0], c[1], c[2], 1.0);
        self.uniform.canopy_conifer = v(colours[0]);
        self.uniform.canopy_broadleaf = v(colours[1]);
        self.uniform.canopy_macchia = v(colours[2]);
        self.uniform.tactical.y = fade[0];
        self.uniform.tactical.z = fade[1];
    }

    /// The forest's occlusion on the ground: `amount` 0 (off) to 1, read from
    /// the mips of the canopy's cover, `posting` metres a sample.
    pub fn set_forest_shade(&mut self, amount: f32, posting: f32) {
        self.uniform.tactical.w = amount;
        self.uniform.canopy_conifer.w = posting;
    }
}

impl Material for TerrainMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_HANDLE.into()
    }

    /// The realistic ground (photo layers under the scanned plants) is a
    /// shader def, so the painted ground compiles to the shader it always was.
    fn specialize(
        _: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        _: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        if !crate::quality::WEBGL2 && crate::textures::ground_detail().is_some() {
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader_defs.push("REAL_GROUND".into());
            }
        }
        Ok(())
    }
}

pub(super) struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, SHADER_HANDLE, "../shaders/terrain.wesl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default());
    }
}
