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
    ) -> Self {
        TerrainMaterial {
            uniform: TerrainUniform { mask_rect, ground_rect, ..Default::default() },
            road_mask,
            grass_map,
            ground_colour,
            ground_normal,
        }
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
}

pub(super) struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, SHADER_HANDLE, "../shaders/terrain.wesl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default());
    }
}
