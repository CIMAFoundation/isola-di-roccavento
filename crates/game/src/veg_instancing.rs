//! GPU instancing of the plants (2026-10-31): one mesh per variety and level
//! of detail, drawn once per tile with a static per-instance vertex buffer
//! (position, yaw, scale, tint: [`PlantInstance`]) written when the scene is
//! built. The plants are not entities: an entity is a tile × variety × tier
//! (`vegetation.rs`), with the tile's bounds for frustum culling.
//!
//! The vegetation stays a normal `Material` (`FoliageMaterial`: its bind
//! groups, pipelines, prepass and shadows): only the draw changes. A
//! material's draw functions are Bevy's (`DrawMaterial`, `DrawPrepass`, which
//! end in `DrawMesh` and bind one vertex buffer); right after the material is
//! prepared, [`patch_draw_functions`] swaps them for the same commands ending
//! in [`DrawVegInstanced`], which binds the instance buffer too and draws the
//! mesh once per plant. The material's `specialize` adds the instance buffer's
//! layout to every pipeline (`materials/foliage.rs`), and the vertex stages
//! read it (`shaders/veg_vertex.wesl`, `veg_prepass.wesl`); they never read
//! the per-mesh uniform (whose index the instances do not have), the tile's
//! transform is the identity.

use std::any::TypeId;
use std::sync::Arc;

use bevy::core_pipeline::core_3d::{AlphaMask3d, Opaque3d};
use bevy::core_pipeline::prepass::{AlphaMask3dPrepass, Opaque3dPrepass};
use bevy::ecs::query::QueryItem;
use bevy::ecs::system::{lifetimeless::*, SystemParamItem};
use bevy::extract::extract_component::ExtractComponent;
use bevy::render::extract_component::ExtractComponentPlugin;
use bevy::material::labels::{DrawFunctionLabel, InternedDrawFunctionLabel};
use bevy::material::MaterialProperties;
use bevy::extract::sync_component::SyncComponent;
use bevy::mesh::VertexBufferLayout;
use bevy::pbr::{
    MainPassAlphaMaskDrawFunction, MainPassOpaqueDrawFunction, PreparedMaterial,
    PrepassAlphaMaskDrawFunction, PrepassOpaqueDepthOnlyDrawFunction, PrepassOpaqueDrawFunction,
    RenderMeshInstances, SetMaterialBindGroup, SetMeshBindGroup, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup, SetPrepassEmptyMaterialBindGroup, SetPrepassViewBindGroup,
    SetPrepassViewEmptyBindGroup, Shadow, ShadowsDepthOnlyDrawFunction, ShadowsDrawFunction,
};
use bevy::prelude::*;
use bevy::render::{
    erased_render_asset::ErasedRenderAssets,
    sync_world::MainEntity,
    mesh::{allocator::MeshAllocator, RenderMesh, RenderMeshBufferInfo},
    render_asset::RenderAssets,
    render_phase::{
        AddRenderCommand, DrawFunctionId, DrawFunctions, PhaseItem, RenderCommand, RenderCommandResult,
        SetItemPipeline, TrackedRenderPass,
    },
    render_resource::{Buffer, BufferInitDescriptor, BufferUsages, VertexAttribute, VertexFormat, VertexStepMode},
    renderer::RenderDevice,
    Render, RenderApp, RenderSystems,
};

use crate::materials::FoliageMaterial;

/// One plant: where it stands (world metres, Bevy frame), its turn about the
/// vertical, its scale, and its tint (leaf red, green, blue, wood: each the
/// multiplier / 2, as bytes). 32 bytes.
#[derive(Clone, Copy, Debug)]
pub struct PlantInstance {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub scale: [f32; 3],
    pub tint: [u8; 4],
}

const STRIDE: u64 = 32;

impl PlantInstance {
    fn write(&self, out: &mut Vec<u8>) {
        for v in self.pos.iter().chain([&self.yaw]).chain(self.scale.iter()) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.tint);
    }
}

/// First shader location of the instance attributes (`veg_vertex.wesl`).
pub const INSTANCE_LOCATION: u32 = 10;

/// The per-instance vertex buffer's layout, slot 1 of every vegetation
/// pipeline.
pub fn instance_layout() -> VertexBufferLayout {
    VertexBufferLayout {
        array_stride: STRIDE,
        step_mode: VertexStepMode::Instance,
        attributes: vec![
            VertexAttribute { format: VertexFormat::Float32x3, offset: 0, shader_location: INSTANCE_LOCATION },
            VertexAttribute { format: VertexFormat::Float32, offset: 12, shader_location: INSTANCE_LOCATION + 1 },
            VertexAttribute { format: VertexFormat::Float32x3, offset: 16, shader_location: INSTANCE_LOCATION + 2 },
            VertexAttribute { format: VertexFormat::Unorm8x4, offset: 28, shader_location: INSTANCE_LOCATION + 3 },
        ],
    }
}

/// The plants a tile entity draws with its mesh.
#[derive(Component, Clone)]
pub struct VegInstances(pub Arc<Vec<PlantInstance>>);

impl SyncComponent<RenderApp> for VegInstances {
    type Target = Self;
}

impl ExtractComponent<RenderApp> for VegInstances {
    type QueryData = &'static VegInstances;
    type QueryFilter = ();
    type Out = Self;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self> {
        Some(item.clone())
    }
}

/// The instances on the GPU, made once per tile entity, by its main-world
/// entity (a binned phase item knows only that one).
#[derive(Resource, Default)]
struct VegInstanceBuffers(bevy::platform::collections::HashMap<MainEntity, (Buffer, u32)>);

pub struct VegInstancingPlugin;

impl Plugin for VegInstancingPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<VegInstances>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<VegInstanceBuffers>()
            .add_render_command::<Opaque3d, DrawVegMain>()
            .add_render_command::<AlphaMask3d, DrawVegMain>()
            .add_render_command::<Opaque3dPrepass, DrawVegPrepass>()
            .add_render_command::<AlphaMask3dPrepass, DrawVegPrepass>()
            .add_render_command::<Opaque3dPrepass, DrawVegDepthOnly>()
            .add_render_command::<Shadow, DrawVegPrepass>()
            .add_render_command::<Shadow, DrawVegDepthOnly>()
            .add_systems(
                Render,
                (
                    patch_draw_functions.in_set(RenderSystems::PrepareMeshes),
                    prepare_instance_buffers.in_set(RenderSystems::PrepareResources),
                ),
            );
    }
}

fn prepare_instance_buffers(
    mut buffers: ResMut<VegInstanceBuffers>,
    query: Query<(&MainEntity, &VegInstances)>,
    device: Res<RenderDevice>,
) {
    for (entity, instances) in &query {
        if buffers.0.contains_key(entity) {
            continue;
        }
        let mut bytes = Vec::with_capacity(instances.0.len() * STRIDE as usize);
        for i in instances.0.iter() {
            i.write(&mut bytes);
        }
        let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("plant instances"),
            contents: &bytes,
            usage: BufferUsages::VERTEX,
        });
        buffers.0.insert(*entity, (buffer, instances.0.len() as u32));
    }
}

type DrawVegMain = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetMeshBindGroup<2>,
    SetMaterialBindGroup<3>,
    DrawVegInstanced,
);

type DrawVegPrepass = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    SetMeshBindGroup<2>,
    SetMaterialBindGroup<3>,
    DrawVegInstanced,
);

type DrawVegDepthOnly = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    SetMeshBindGroup<2>,
    SetPrepassEmptyMaterialBindGroup<3>,
    DrawVegInstanced,
);

/// Every prepared `FoliageMaterial` draws its meshes as instances: its draw
/// functions are swapped for the ones above (a material is prepared again
/// whenever it changes, every frame the clock moves, so this runs each time).
#[allow(clippy::too_many_arguments)]
fn patch_draw_functions(
    mut materials: ResMut<ErasedRenderAssets<PreparedMaterial>>,
    opaque: Res<DrawFunctions<Opaque3d>>,
    alpha_mask: Res<DrawFunctions<AlphaMask3d>>,
    opaque_prepass: Res<DrawFunctions<Opaque3dPrepass>>,
    alpha_mask_prepass: Res<DrawFunctions<AlphaMask3dPrepass>>,
    shadow: Res<DrawFunctions<Shadow>>,
) {
    let main_opaque = opaque.read().id::<DrawVegMain>();
    let ours: [(InternedDrawFunctionLabel, DrawFunctionId); 7] = [
        (MainPassOpaqueDrawFunction.intern(), main_opaque),
        (MainPassAlphaMaskDrawFunction.intern(), alpha_mask.read().id::<DrawVegMain>()),
        (PrepassOpaqueDrawFunction.intern(), opaque_prepass.read().id::<DrawVegPrepass>()),
        (PrepassAlphaMaskDrawFunction.intern(), alpha_mask_prepass.read().id::<DrawVegPrepass>()),
        (PrepassOpaqueDepthOnlyDrawFunction.intern(), opaque_prepass.read().id::<DrawVegDepthOnly>()),
        (ShadowsDrawFunction.intern(), shadow.read().id::<DrawVegPrepass>()),
        (ShadowsDepthOnlyDrawFunction.intern(), shadow.read().id::<DrawVegDepthOnly>()),
    ];
    for (id, material) in materials.iter_mut() {
        if id.type_id() != TypeId::of::<FoliageMaterial>() {
            continue;
        }
        let p = &material.properties;
        if p.get_draw_function(MainPassOpaqueDrawFunction) == Some(main_opaque) {
            continue;
        }
        let mut draw_functions = p.draw_functions.clone();
        for (label, f) in draw_functions.iter_mut() {
            if let Some((_, ours)) = ours.iter().find(|(l, _)| l == label) {
                *f = *ours;
            }
        }
        material.properties = Arc::new(MaterialProperties {
            render_method: p.render_method,
            alpha_mode: p.alpha_mode,
            mesh_pipeline_key_bits: p.mesh_pipeline_key_bits,
            depth_bias: p.depth_bias,
            reads_view_transmission_texture: p.reads_view_transmission_texture,
            render_phase_type: p.render_phase_type,
            material_layout: p.material_layout.clone(),
            draw_functions,
            shaders: p.shaders.clone(),
            bindless: p.bindless,
            base_specialize: p.base_specialize,
            prepass_specialize: p.prepass_specialize,
            user_specialize: p.user_specialize,
            material_key: p.material_key.clone(),
            shadows_enabled: p.shadows_enabled,
            prepass_enabled: p.prepass_enabled,
            oit_enabled: p.oit_enabled,
        });
    }
}

/// `DrawMesh` with the instance buffer in slot 1 and one instance per plant.
struct DrawVegInstanced;

impl<P: PhaseItem> RenderCommand<P> for DrawVegInstanced {
    type Param = (SRes<RenderAssets<RenderMesh>>, SRes<RenderMeshInstances>, SRes<MeshAllocator>, SRes<VegInstanceBuffers>);
    type ViewQuery = ();
    type ItemQuery = ();

    #[inline]
    fn render<'w>(
        item: &P,
        _view: (),
        _item: Option<()>,
        (meshes, mesh_instances, allocator, buffers): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let allocator = allocator.into_inner();
        let Some((buffer, length)) = buffers.into_inner().0.get(&item.main_entity()) else {
            return RenderCommandResult::Skip;
        };
        let Some(mesh_id) = mesh_instances.mesh_asset_id(item.main_entity()) else {
            return RenderCommandResult::Skip;
        };
        let Some(gpu_mesh) = meshes.into_inner().get(mesh_id) else { return RenderCommandResult::Skip };
        let Some(vertices) = allocator.mesh_vertex_slice(&mesh_id) else { return RenderCommandResult::Skip };
        pass.set_vertex_buffer(0, vertices.buffer.slice(..));
        pass.set_vertex_buffer(1, buffer.slice(..));
        match &gpu_mesh.buffer_info {
            RenderMeshBufferInfo::Indexed { index_format, count } => {
                let Some(indices) = allocator.mesh_index_slice(&mesh_id) else { return RenderCommandResult::Skip };
                pass.set_index_buffer(indices.buffer.slice(..), *index_format);
                pass.draw_indexed(
                    indices.range.start..indices.range.start + count,
                    vertices.range.start as i32,
                    0..*length,
                );
            }
            RenderMeshBufferInfo::NonIndexed => pass.draw(vertices.range, 0..*length),
        }
        RenderCommandResult::Success
    }
}
