//! The vegetation's material (`shaders/foliage.wgsl`) and its vertex stages,
//! which apply each plant's fire state from a small per-chunk image
//! (`shaders/veg_state.wgsl`, `vegetation.rs`).

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::shader::ShaderRef;
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError, VertexFormat,
};

const SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f15");
const STATE_SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f11");
const VERTEX_SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f12");
const PREPASS_SHADER_HANDLE: Handle<Shader> = uuid_handle!("91a744d2-6b0e-4c71-aa90-220188c42f13");

/// Which plant of its chunk a vertex belongs to (bit 31: the burnt-remains
/// layer), so the vertex stage finds the plant's fire state.
pub const ATTRIBUTE_PLANT: MeshVertexAttribute = MeshVertexAttribute::new("Plant", 0x91A7_4402, VertexFormat::Uint32);
/// Vertex input location of [`ATTRIBUTE_PLANT`] (`shaders/veg_*.wgsl`).
const PLANT_LOCATION: u32 = 14;

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct FoliageMaterial {
    /// x: tactical desaturation (`tactical.rs`); y: the simulated clock, s;
    /// z: how long a plant flames, s (`vegetation::fire_clock`). One vec4:
    /// WebGL2 wants uniform bindings in multiples of 16 bytes.
    #[uniform(0)]
    pub fire: Vec4,
    /// One texel per plant of the chunk (x = fire state, y = ignition time),
    /// read in the vertex stage. The plants burn by rewriting this small
    /// image, never their meshes.
    #[texture(1, sample_type = "float", filterable = false)]
    pub plant_state: Handle<Image>,
}

impl FoliageMaterial {
    pub fn new(plant_state: Handle<Image>) -> Self {
        FoliageMaterial { fire: Vec4::ZERO, plant_state }
    }

    pub fn tactical(&self) -> f32 {
        self.fire.x
    }

    pub fn set_tactical(&mut self, amount: f32) {
        self.fire.x = amount;
    }
}

impl Material for FoliageMaterial {
    fn vertex_shader() -> ShaderRef {
        VERTEX_SHADER_HANDLE.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_HANDLE.into()
    }

    /// Depth and shadow passes too: a plant that has collapsed casts nothing.
    fn prepass_vertex_shader() -> ShaderRef {
        PREPASS_SHADER_HANDLE.into()
    }

    /// Foliage is thin: both faces drawn, in every pass, and light through the
    /// leaves (Bevy's diffuse transmission, set per pixel in `foliage.wgsl`).
    /// The plant index joins the mesh's one interleaved vertex buffer.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        // leaves let the sun through: lit from behind, a crown glows
        // (`?trasmissione=0` in the browser leaves it out, `?stato=0` the
        // plants' fire state, `?trama=0` the leaves' and bark's pattern:
        // switches to find what kills an iPhone's page)
        if crate::quality::web_off("stato") {
            descriptor.vertex.shader_defs.push("NO_PLANT_STATE".into());
        }
        if let Some(fragment) = descriptor.fragment.as_mut() {
            if !crate::quality::web_off("trasmissione") {
                fragment.shader_defs.push("STANDARD_MATERIAL_DIFFUSE_TRANSMISSION".into());
            }
            if crate::quality::web_off("trama") {
                fragment.shader_defs.push("PLAIN_FOLIAGE".into());
            }
        }
        let plant = layout.0.get_layout(&[ATTRIBUTE_PLANT.at_shader_location(PLANT_LOCATION)])?;
        descriptor.vertex.buffers[0].attributes.extend(plant.attributes);
        Ok(())
    }
}

pub(super) struct FoliagePlugin;

impl Plugin for FoliagePlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, STATE_SHADER_HANDLE, "../shaders/veg_state.wesl");
        crate::load_wesl!(app, VERTEX_SHADER_HANDLE, "../shaders/veg_vertex.wesl");
        crate::load_wesl!(app, PREPASS_SHADER_HANDLE, "../shaders/veg_prepass.wesl");
        crate::load_wesl!(app, SHADER_HANDLE, "../shaders/foliage.wesl");
        app.add_plugins(MaterialPlugin::<FoliageMaterial>::default());
    }
}
