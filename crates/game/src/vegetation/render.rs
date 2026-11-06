//! Drawing the plants: a render pipeline of their own, outside Bevy's
//! material system.
//!
//! The render world receives the scene once (`PlantScene`: meshes, instance
//! records, atlas) and makes its GPU resources once: a vertex and an index
//! buffer per mesh, a static instance buffer and a fire buffer per group, the
//! atlas texture, one uniform buffer and one bind group. Per frame it writes
//! the 48 bytes of uniforms, and a group's fire buffer when its version moved.
//!
//! Each frame, for every camera, the tiles in its frustum are listed with
//! the level of detail their distance asks for, and one item goes into the
//! camera's opaque phase (and into its depth prepass, when it has a bare
//! one: the depth of field's; there only the meshes the main pass draws
//! whole, see [`Pass::Prepass`]); for every shadow cascade of the sun, the
//! tiles in the cascade's box, and one item into its shadow phase. The item's draw
//! command ([`DrawPlants`]) walks the view's list: per (variety, tier) one
//! instanced draw. Bevy's binned phases take such items as
//! [`BinnedRenderPhaseType::NonMesh`]: no mesh uniform, no mesh allocator.
//!
//! The main pipeline is Bevy's mesh pipeline for the view (its view
//! bindings, lights, shadows, fog, tonemapping, MSAA, HDR), with the plants'
//! shader, vertex buffers and bind group (`shaders/plants.wesl`); the shadow
//! pipeline is a depth-only one like Bevy's prepass for the sun's cascades
//! (`shaders/plant_shadow.wesl`).

use std::num::NonZeroU64;
use std::sync::Arc;

use bevy::asset::{uuid_handle, RenderAssetUsages};
use bevy::core_pipeline::core_3d::{Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey, CORE_3D_DEPTH_FORMAT};
use bevy::core_pipeline::prepass::{Opaque3dPrepass, OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey};
use bevy::ecs::system::{lifetimeless::SRes, SystemParamItem};
use bevy::mesh::{MeshVertexBufferLayoutRef, MeshVertexBufferLayouts, PrimitiveTopology, VertexBufferLayout};
use bevy::pbr::{
    ExtractedDirectionalLight, LightEntity, MeshPipeline, MeshPipelineKey, PrepassPipeline, SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup,
    SetPrepassViewBindGroup, SetPrepassViewEmptyBindGroup, Shadow, ShadowBatchSetKey, ShadowBinKey, ViewKeyCache,
    ViewLightEntities,
};
use bevy::platform::collections::HashMap;
use bevy::shader::ShaderDefVal;
use bevy::render::render_resource::SpecializedMeshPipeline;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::{
    mesh::allocator::MeshSlabs,
    render_phase::{
        AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem, RenderCommand,
        RenderCommandResult, TrackedRenderPass, ViewBinnedRenderPhases,
    },
    render_resource::{
        binding_types::{sampler, texture_2d, uniform_buffer_sized},
        BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer, BufferInitDescriptor,
        BufferUsages, CachedRenderPipelineId, CompareFunction, DepthStencilState, Extent3d, Face, FragmentState,
        IndexFormat, MultisampleState, PipelineCache, PrimitiveState, RenderPipelineDescriptor, SamplerBindingType,
        SamplerDescriptor, ShaderStages, SpecializedRenderPipeline, SpecializedRenderPipelines,
        TextureDataOrder, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
        TextureViewDescriptor, VertexAttribute, VertexFormat, VertexState, VertexStepMode,
    },
    renderer::{RenderDevice, RenderQueue},
    sync_world::{MainEntity, RenderEntity},
    view::{ExtractedView, RetainedViewEntity},
    Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
};

use super::meshes::{Tier, VERTEX_BYTES};
use super::{PlantScene, PlantsDrawable, Vegetation, INSTANCE_BYTES};

const SHADER: Handle<Shader> = uuid_handle!("3f1c9a52-7d0e-4b6a-9c81-5e2f04a7d310");
const SHADOW_SHADER: Handle<Shader> = uuid_handle!("3f1c9a52-7d0e-4b6a-9c81-5e2f04a7d311");
const COMMON_SHADER: Handle<Shader> = uuid_handle!("3f1c9a52-7d0e-4b6a-9c81-5e2f04a7d312");

/// Bytes of the uniforms (`PlantUniform` in `plant_common.wesl`).
const UNIFORM_BYTES: u64 = 48;

/// Within this distance of the camera the full models cast their own
/// shadow; beyond it the proxies do. The sun's first cascade ends at 80 m
/// and the second at about 300 m, where a texel is a hand's width: a crown's
/// lobes are gone from the shadow map, and its outline is the proxy's.
const FULL_SHADOW_M: f32 = 250.0;

/// Where a scanned plant changes level (2026-11-04,
/// `tools/vegetation/lod.py`): its `lod0` ([`Tier::Near`], built for
/// 15 m) up to `NEAR_M`, its `lod1` ([`Tier::Mid`], built for 40 m) up to
/// `MID_M`, then its `lod2` ([`Tier::Full`], built for 150 m) up to
/// `SCAN_FAR_M` (or the LOD distance, if nearer). Each a little farther
/// than it was built for: «vorrei dei modelli dettagliati da vicino».
/// Beyond, the plant is drawn in boxes made of its `lod2`
/// (`meshes::Builder::scan_voxels`): small ones ([`Tier::Proxy`]) up to
/// `SCAN_LUMP_M`, fewer and larger ([`Tier::Far`]) past it.
/// `KIOSK_SCAN_LOD=<near>,<mid>,<far>,<lumps>` (m) for trials.
///
/// The first three end a little short of where one of the sun's cascades
/// begins to be read (56, 206 and 760 m with four cascades,
/// [`cascade_begins`]; they were 60, 250 and 800 m until 2026-11-06): a
/// plant of a level that reaches past it reads that cascade and must cast
/// its own mesh in it ([`pick_caster`]), and the ring between was a third
/// to a half of the triangles a cascade drew.
const NEAR_M: f32 = 55.0;
const MID_M: f32 = 205.0;
const SCAN_FAR_M: f32 = 755.0;
const SCAN_LUMP_M: f32 = 1500.0;

fn scan_lod() -> [f32; 4] {
    static LOD: std::sync::OnceLock<[f32; 4]> = std::sync::OnceLock::new();
    *LOD.get_or_init(|| {
        let v: Vec<f32> = std::env::var("KIOSK_SCAN_LOD").unwrap_or_default().split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() == 4 { [v[0], v[1], v[2], v[3]] } else { [NEAR_M, MID_M, SCAN_FAR_M, SCAN_LUMP_M] }
    })
}

/// The depth from the camera (m) at which cascade `index` of a sun begins
/// to be read: the far bound of the cascade before, less the overlap
/// (`bevy_light::build_directional_light_cascades`); 0 for the first.
fn cascade_begins(config: &bevy::light::CascadeShadowConfig, index: usize) -> f32 {
    match index.checked_sub(1).and_then(|before| config.bounds.get(before)) {
        Some(far) => far * (1.0 - config.overlap_proportion),
        None => 0.0,
    }
}

/// How far from itself a plant's shadow may fall (m), for a sun `up` high
/// (the sine of its elevation): a 30 m plant's on ground that falls away
/// from the sun two thirds as steeply as the sun's rays, and a crown's
/// width; never under 250 m (a tree on a cliff's edge shades the valley
/// floor farther than any slope does). Past the horizon, no end.
fn shadow_reach_m(up: f32) -> f32 {
    if up <= 0.01 {
        return f32::INFINITY;
    }
    let cot = (1.0 - up * up).max(0.0).sqrt() / up;
    (30.0 * cot * 3.0 + 40.0).max(250.0)
}

/// The pick of a cascade that begins to be read `begins` metres from the
/// camera ([`cascade_begins`]), for plants drawn by `band`. A node with a
/// plant that may read the cascade (its farthest point at `begins` or
/// past it) casts as the main pass draws it, by its nearest point; one
/// wholly nearer casts the mesh drawn `coarse` metres away, if that is
/// coarser, and one wholly nearer than `beyond` nothing. A node across
/// `begins` is split, down to its leaves.
fn pick_caster(band: impl Fn(f32) -> u8, begins: f32, beyond: f32, coarse: f32) -> impl Fn(f32, f32, bool) -> Pick {
    move |near, far, leaf| {
        if far < beyond {
            return Pick::Skip;
        }
        if far < begins {
            let b = band(near.max(coarse));
            return if leaf || b == band(far.max(coarse)) { Pick::Whole(b) } else { Pick::Split };
        }
        let b = band(near);
        if leaf || (near >= begins && b == band(far)) { Pick::Whole(b) } else { Pick::Split }
    }
}

/// The band of a plant at `d`: 3 near, 4 mid, 0 full (an authored plant
/// draws its model in all three), 1 proxy, 2 far. A scanned plant
/// (`scanned`) changes to its `lod3` (the proxy band) and to its lumps
/// (the far band) at its own distances.
fn plant_band(d: f32, lod_m: f32, far_m: f32, scanned: bool) -> u8 {
    let [near, mid, scan_far, lumps] = scan_lod();
    let full_m = if scanned { scan_far.min(lod_m) } else { lod_m };
    let far_m = if scanned { lumps.max(full_m) } else { far_m };
    if lod_m > 0.0 && d < full_m {
        if d < near { 3 } else if d < mid { 4 } else { 0 }
    } else if d > far_m {
        2
    } else {
        1
    }
}

/// Beyond this multiple of the LOD distance (and at least
/// [`FAR_TIER_MIN_M`]) a tile draws its cheapest crowns.
const FAR_TIER: f32 = 2.0;
const FAR_TIER_MIN_M: f32 = 2000.0;

pub struct PlantRenderPlugin;

impl Plugin for PlantRenderPlugin {
    fn build(&self, app: &mut App) {
        crate::load_wesl!(app, COMMON_SHADER, "../shaders/plant_common.wesl");
        crate::load_wesl!(app, SHADER, "../shaders/plants.wesl");
        crate::load_wesl!(app, SHADOW_SHADER, "../shaders/plant_shadow.wesl");
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<ExtractedPlants>()
            .init_resource::<PlantPipelineSlot>()
            .init_resource::<SpecializedRenderPipelines<PlantPipeline>>()
            .init_resource::<PlantGpu>()
            .init_resource::<PlantDraws>()
            .init_resource::<PlantLod>()
            .add_render_command::<Opaque3d, DrawPlantsMain>()
            .add_render_command::<Opaque3dPrepass, DrawPlantsPrepass>()
            .add_render_command::<Shadow, DrawPlantsShadow>()
            .add_systems(ExtractSchedule, extract_plants)
            .add_systems(
                Render,
                (
                    (init_pipeline, queue_plants).chain().in_set(RenderSystems::Queue),
                    prepare_plants.in_set(RenderSystems::PrepareResources),
                ),
            );
    }
}

// --- extraction --------------------------------------------------------------

/// A group's fire as extracted: version, per-plant state, plants caught.
type ExtractedFire = (u32, Arc<Vec<[f32; 2]>>, Arc<Vec<u32>>);

/// What the main world says this frame.
#[derive(Resource, Default)]
struct ExtractedPlants {
    scene: Option<Arc<PlantScene>>,
    /// Per group: version and state of its fire, the plants it has come to.
    fire: Vec<ExtractedFire>,
    uniform: [f32; 12],
    lod_m: f32,
    /// The sun casts shadows (`quality::Graphics::shadows`, live): the
    /// plants look their shadow up only then (`PLANT_SHADOWS`). Bevy's fog
    /// still brings the shadow filters into the program (its in-scattering
    /// samples the cascades), so the phone's GLSL barely shrinks.
    shadows: bool,
    /// The entity the draws hang on: render and main world.
    item: Option<(Entity, MainEntity)>,
    /// Cameras that do not see the default render layer, where the plants
    /// are, as Bevy keeps every mesh of that layer out of them: Mario's
    /// silhouette camera drew the whole forest into his mask (2026-11-02).
    blind: Vec<MainEntity>,
}

fn extract_plants(
    mut out: ResMut<ExtractedPlants>,
    veg: Extract<Option<Res<Vegetation>>>,
    gfx: Extract<Option<Res<crate::quality::Graphics>>>,
    item: Extract<Query<(Entity, &RenderEntity), With<PlantsDrawable>>>,
    cameras: Extract<Query<(Entity, &RenderLayers), With<Camera>>>,
) {
    out.blind.clear();
    out.blind.extend(cameras.iter().filter(|(_, l)| !l.intersects(&RenderLayers::default())).map(|(e, _)| MainEntity::from(e)));
    let (Some(veg), Ok((main, render))) = (veg.as_ref(), item.single()) else {
        out.scene = None;
        out.item = None;
        return;
    };
    out.scene = Some(veg.scene.clone());
    out.fire.clear();
    out.fire.extend(veg.fire.iter().map(|f| (f.version, f.state.clone(), f.caught.clone())));
    out.uniform = veg.uniform;
    out.lod_m = veg.lod_m;
    out.shadows = gfx.as_ref().is_none_or(|g| g.shadows);
    out.item = Some((render.id(), MainEntity::from(main)));
}

// --- pipelines ---------------------------------------------------------------

#[derive(Resource)]
struct PlantPipeline {
    mesh_pipeline: MeshPipeline,
    /// The prepass's view layout (no motion vectors) and its empty group,
    /// as Bevy's shadow passes bind them.
    shadow_view_layout: BindGroupLayoutDescriptor,
    empty_layout: BindGroupLayoutDescriptor,
    /// Group 2: uniforms, atlas, sampler.
    plants_layout: BindGroupLayoutDescriptor,
    /// A vertex layout with position, normal, uv and colour, for the shader
    /// defs Bevy's mesh pipeline derives from a mesh (the plants bring
    /// their own vertex buffers).
    probe: MeshVertexBufferLayoutRef,
    depth_clip_control: bool,
}

/// Made on the first frame both of Bevy's pipelines it builds on exist.
#[derive(Resource, Default)]
struct PlantPipelineSlot(Option<PlantPipeline>);

/// The passes the plants are drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Pass {
    /// The camera's opaque pass, lit (`plants.wesl`).
    Main,
    /// The camera's depth prepass, when it has one (depth of field, Mario):
    /// depth only, so the ground under the crowns is not shaded in the main
    /// pass (`plant_shadow.wesl`). Only the meshes the main pass draws whole,
    /// the proxies: behind a depth written here nothing else can draw, so
    /// where the main pass then leaves a sample bare the clear colour shows.
    /// The cut-out meshes (leaf cards, remains) are left to the main pass
    /// alone, which smooths their edges with alpha to coverage and eats them
    /// as they burn: with them in the prepass the crowns near the camera
    /// shimmered grey at every edge and a burning crown turned into a grey
    /// silhouette (2026-11-02). The depth of field reads the depth after the
    /// main pass, plants and all.
    Prepass,
    /// The sun's cascades, depth only (`plant_shadow.wesl`).
    Shadow,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PlantKey {
    /// The view's key (main pass and prepass); empty for the shadow pass.
    view: MeshPipelineKey,
    pass: Pass,
    cutout: bool,
    double_sided: bool,
    /// The leaves are painted with the canopy tile (`PLANT_CANOPY`).
    canopy: bool,
    /// The main pass looks the sun's shadows up (`PLANT_SHADOWS`).
    receive_shadows: bool,
}

/// The three vertex buffers: the mesh, the plants' static records, their fire.
fn vertex_buffers() -> Vec<VertexBufferLayout> {
    let attr = |format, offset, shader_location| VertexAttribute { format, offset, shader_location };
    vec![
        VertexBufferLayout {
            array_stride: VERTEX_BYTES,
            step_mode: VertexStepMode::Vertex,
            attributes: vec![
                attr(VertexFormat::Float32x3, 0, 0),
                attr(VertexFormat::Snorm8x4, 12, 1),
                attr(VertexFormat::Unorm16x2, 16, 2),
                attr(VertexFormat::Unorm16x4, 20, 3),
                attr(VertexFormat::Snorm8x4, 28, 9),
                attr(VertexFormat::Snorm8x4, 32, 10),
            ],
        },
        VertexBufferLayout {
            array_stride: INSTANCE_BYTES,
            step_mode: VertexStepMode::Instance,
            attributes: vec![
                attr(VertexFormat::Float32x4, 0, 4),
                attr(VertexFormat::Float32x2, 16, 5),
                attr(VertexFormat::Unorm8x4, 24, 6),
                attr(VertexFormat::Unorm8x4, 28, 7),
            ],
        },
        VertexBufferLayout {
            array_stride: 8,
            step_mode: VertexStepMode::Instance,
            attributes: vec![attr(VertexFormat::Float32x2, 0, 8)],
        },
    ]
}

impl SpecializedRenderPipeline for PlantPipeline {
    type Key = PlantKey;

    fn specialize(&self, key: PlantKey) -> RenderPipelineDescriptor {
        // `pbr_functions` (imported for fog and tonemapping) declares
        // StandardMaterial's bindings in this group: never read, never bound
        let mut defs: Vec<ShaderDefVal> = vec![ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3)];
        if key.cutout {
            defs.push("PLANT_CUTOUT".into());
        }
        if key.pass != Pass::Main {
            let shadow = key.pass == Pass::Shadow;
            let emulate = shadow && !self.depth_clip_control;
            if emulate {
                defs.push("UNCLIPPED_DEPTH_ORTHO_EMULATION".into());
            }
            if crate::quality::WEBGL2 {
                defs.push("WEBGL2".into());
            }
            let fragment = (emulate || key.cutout).then(|| FragmentState {
                shader: SHADOW_SHADER,
                shader_defs: defs.clone(),
                entry_point: Some("fragment".into()),
                targets: vec![],
                ..default()
            });
            return RenderPipelineDescriptor {
                label: Some(if shadow { "plant_shadows" } else { "plant_prepass" }.into()),
                layout: vec![self.shadow_view_layout.clone(), self.empty_layout.clone(), self.plants_layout.clone()],
                vertex: VertexState {
                    shader: SHADOW_SHADER,
                    shader_defs: defs,
                    entry_point: Some("vertex".into()),
                    buffers: vertex_buffers(),
                    ..default()
                },
                fragment,
                primitive: PrimitiveState {
                    // both faces cast a shadow; the prepass culls as the main pass does
                    cull_mode: if shadow || key.double_sided { None } else { Some(Face::Back) },
                    unclipped_depth: shadow && !emulate,
                    ..default()
                },
                depth_stencil: Some(DepthStencilState {
                    format: CORE_3D_DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(CompareFunction::GreaterEqual),
                    stencil: default(),
                    bias: default(),
                }),
                multisample: MultisampleState { count: if shadow { 1 } else { key.view.msaa_samples() }, ..default() },
                ..default()
            };
        }
        // the view's key says nothing of the mesh: without the topology's
        // bits it means a point list (2026-11-01: every plant drawn as
        // scattered pixels while its shadow, from a pipeline of its own, was whole)
        let mesh_key = key.view | MeshPipelineKey::from_primitive_topology_and_strip_index(PrimitiveTopology::TriangleList, None);
        let mut d = self
            .mesh_pipeline
            .specialize(mesh_key, &self.probe)
            .expect("the probe layout has every attribute the mesh pipeline asks for");
        d.label = Some("plants".into());
        d.layout.truncate(2);
        d.layout.push(self.plants_layout.clone());
        // leaf edges smoothed by the samples rather than cut on the pixel grid
        let coverage = key.cutout && key.view.msaa_samples() > 1;
        if coverage {
            defs.push("PLANT_A2C".into());
        }
        if key.receive_shadows {
            defs.push("PLANT_SHADOWS".into());
        }
        if key.canopy {
            defs.push("PLANT_CANOPY".into());
        }
        d.vertex.shader = SHADER;
        d.vertex.entry_point = Some("vertex".into());
        d.vertex.buffers = vertex_buffers();
        d.vertex.shader_defs.extend(defs.iter().cloned());
        if let Some(f) = d.fragment.as_mut() {
            f.shader = SHADER;
            f.entry_point = Some("fragment".into());
            f.shader_defs.extend(defs);
        }
        d.primitive.cull_mode = if key.double_sided { None } else { Some(Face::Back) };
        d.multisample.alpha_to_coverage_enabled = coverage;
        d
    }
}

fn init_pipeline(
    mut slot: ResMut<PlantPipelineSlot>,
    mesh_pipeline: Option<Res<MeshPipeline>>,
    prepass: Option<Res<PrepassPipeline>>,
    mut layouts: ResMut<MeshVertexBufferLayouts>,
) {
    if slot.0.is_some() {
        return;
    }
    let (Some(mesh_pipeline), Some(prepass)) = (mesh_pipeline, prepass) else { return };
    let mut probe = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    probe.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3]);
    probe.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 3]);
    probe.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 3]);
    probe.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; 3]);
    let probe = probe.get_mesh_vertex_buffer_layout(&mut layouts);
    let plants_layout = BindGroupLayoutDescriptor::new(
        "plants",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                uniform_buffer_sized(false, NonZeroU64::new(UNIFORM_BYTES)),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    slot.0 = Some(PlantPipeline {
        mesh_pipeline: mesh_pipeline.clone(),
        shadow_view_layout: prepass.view_layout_no_motion_vectors.clone(),
        empty_layout: prepass.empty_layout.clone(),
        plants_layout,
        probe,
        depth_clip_control: prepass.depth_clip_control_supported,
    });
}

// --- GPU resources -----------------------------------------------------------

struct GpuMesh {
    vertices: Buffer,
    indices: Buffer,
    count: u32,
}

struct GpuGroup {
    instances: Buffer,
    fire: Buffer,
    count: u32,
    version: u32,
    /// The plants the fire has come to, their records and fire copied out:
    /// the remains are drawn for these alone (a group of 300 shrubs with
    /// three burning drew 300 skeletons in every pass, 2026-11-01).
    remains: Option<GpuRemains>,
}

struct GpuRemains {
    instances: Buffer,
    fire: Buffer,
    count: u32,
    /// Plants the buffers hold room for (doubled when outgrown).
    capacity: u32,
}

/// The records and fire of `group`'s plants `caught`, packed.
fn remains_bytes(scene: &PlantScene, group: usize, state: &[[f32; 2]], caught: &[u32]) -> (Vec<u8>, Vec<u8>) {
    let records = &scene.groups[group].instances;
    let n = INSTANCE_BYTES as usize;
    let mut instances = Vec::with_capacity(caught.len() * n);
    let mut fire = Vec::with_capacity(caught.len() * 8);
    for &i in caught {
        let i = i as usize;
        instances.extend_from_slice(&records[i * n..(i + 1) * n]);
        fire.extend(state[i].iter().flat_map(|v| v.to_le_bytes()));
    }
    (instances, fire)
}

struct GpuScene {
    id: u64,
    meshes: Vec<GpuMesh>,
    groups: Vec<GpuGroup>,
    uniform: Buffer,
    bind_group: BindGroup,
}

#[derive(Resource, Default)]
struct PlantGpu(Option<GpuScene>);

fn fire_bytes(state: &[[f32; 2]]) -> Vec<u8> {
    state.iter().flat_map(|s| s.iter().flat_map(|v| v.to_le_bytes())).collect()
}

fn uniform_bytes(u: &[f32; 12]) -> Vec<u8> {
    u.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn build_scene(scene: &PlantScene, plants: &ExtractedPlants, pipeline: &PlantPipeline, device: &RenderDevice, queue: &RenderQueue, cache: &PipelineCache) -> GpuScene {
    let buffer = |label: &'static str, contents: &[u8], usage: BufferUsages| {
        device.create_buffer_with_data(&BufferInitDescriptor { label: Some(label), contents, usage })
    };
    let meshes = scene
        .meshes
        .iter()
        .map(|m| {
            let indices: Vec<u8> = m.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            GpuMesh {
                vertices: buffer("plant vertices", &m.vertices, BufferUsages::VERTEX),
                indices: buffer("plant indices", &indices, BufferUsages::INDEX),
                count: m.indices.len() as u32,
            }
        })
        .collect();
    let groups = scene
        .groups
        .iter()
        .zip(&plants.fire)
        .map(|(g, (version, state, _))| GpuGroup {
            instances: buffer("plant instances", &g.instances, BufferUsages::VERTEX),
            fire: buffer("plant fire", &fire_bytes(state), BufferUsages::VERTEX | BufferUsages::COPY_DST),
            count: g.count,
            // `prepare_plants` fills it in from the next frame
            version: version.wrapping_sub(1),
            remains: None,
        })
        .collect();
    let uniform = buffer("plant uniforms", &uniform_bytes(&plants.uniform), BufferUsages::UNIFORM | BufferUsages::COPY_DST);
    let a = &scene.atlas;
    let texture = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some("foliage atlas"),
            size: Extent3d { width: a.width, height: a.height, depth_or_array_layers: 1 },
            mip_level_count: a.levels,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &a.data,
    );
    let view = texture.create_view(&TextureViewDescriptor::default());
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("foliage atlas"),
        mag_filter: bevy::render::render_resource::FilterMode::Linear,
        min_filter: bevy::render::render_resource::FilterMode::Linear,
        mipmap_filter: bevy::render::render_resource::MipmapFilterMode::Linear,
        ..default()
    });
    let bind_group = device.create_bind_group(
        "plants",
        &cache.get_bind_group_layout(&pipeline.plants_layout),
        &BindGroupEntries::sequential((uniform.as_entire_binding(), &view, &sampler)),
    );
    GpuScene { id: scene.id, meshes, groups, uniform, bind_group }
}

fn prepare_plants(
    plants: Res<ExtractedPlants>,
    slot: Res<PlantPipelineSlot>,
    mut gpu: ResMut<PlantGpu>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
) {
    let (Some(scene), Some(pipeline)) = (&plants.scene, &slot.0) else {
        gpu.0 = None;
        return;
    };
    if gpu.0.as_ref().is_none_or(|g| g.id != scene.id) {
        gpu.0 = Some(build_scene(scene, &plants, pipeline, &device, &queue, &cache));
        return;
    }
    let Some(g) = gpu.0.as_mut() else { return };
    queue.write_buffer(&g.uniform, 0, &uniform_bytes(&plants.uniform));
    for (index, (group, (version, state, caught))) in g.groups.iter_mut().zip(&plants.fire).enumerate() {
        if group.version == *version {
            continue;
        }
        queue.write_buffer(&group.fire, 0, &fire_bytes(state));
        group.version = *version;
        if !scene.remains || caught.is_empty() {
            group.remains = None;
            continue;
        }
        let (instances, fire) = remains_bytes(scene, index, state, caught);
        let count = caught.len() as u32;
        match group.remains.as_mut() {
            Some(r) if r.capacity >= count => {
                queue.write_buffer(&r.instances, 0, &instances);
                queue.write_buffer(&r.fire, 0, &fire);
                r.count = count;
            }
            _ => {
                let capacity = count.next_power_of_two().max(16);
                let make = |label, bytes: &[u8], per: u64| {
                    let b = device.create_buffer(&bevy::render::render_resource::BufferDescriptor {
                        label: Some(label),
                        size: capacity as u64 * per,
                        usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    queue.write_buffer(&b, 0, bytes);
                    b
                };
                group.remains = Some(GpuRemains {
                    instances: make("plant remains instances", &instances, INSTANCE_BYTES),
                    fire: make("plant remains fire", &fire, 8),
                    count,
                    capacity,
                });
            }
        }
    }
}

// --- queueing ----------------------------------------------------------------

/// One instanced draw: a mesh, once per plant of a group.
#[derive(Clone, Copy)]
struct Draw {
    pipeline: CachedRenderPipelineId,
    mesh: u32,
    group: u32,
    /// Drawn for the plants the fire has come to ([`GpuRemains`]), not the whole group.
    remains: bool,
    /// The group's plants drawn: `count` from `first` (a run of its
    /// quadtree, [`select`]; `Group::kept` when thinned by count); the
    /// remains draw all theirs.
    first: u32,
    count: u32,
}

/// `KIOSK_LOD_TILE=1`: the level of detail per 1280 m tile, as until
/// 2026-11-04, for comparisons.
fn lod_per_tile() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KIOSK_LOD_TILE").is_ok_and(|v| v == "1"))
}

/// `KIOSK_VEG_STATS=1`: once a second, the triangles the plants send to
/// each camera's main pass, by band of distance, and to each of the sun's
/// cascades (millions, a line in the log).
fn stats_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KIOSK_VEG_STATS").is_ok_and(|v| v == "1"))
}

/// What [`select`] does with a node of a group's quadtree.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Pick {
    /// All its plants in this band.
    Whole(u8),
    /// Its children, each on its own (never asked of a leaf).
    Split,
    /// None of its plants.
    Skip,
}

/// The pick of a pass that draws each plant by its distance alone (`band`):
/// a node whole when all of it is in one band, a leaf in the band of its
/// nearest point.
fn pick_by_band(band: impl Fn(f32) -> u8) -> impl Fn(f32, f32, bool) -> Pick {
    move |near, far, leaf| {
        let b = band(near);
        if leaf || b == band(far) { Pick::Whole(b) } else { Pick::Split }
    }
}

/// The runs of `group`'s plants and the band each is drawn in, in the
/// order of the instance buffer, neighbours of one band merged
/// (2026-11-04: the level of detail per node of the group's quadtree, no
/// longer per 1280 m tile). `pick` says what becomes of a node, from the
/// distances of its nearest and farthest points and whether it is a leaf:
/// whole in one band, its children in its stead, or nothing. Nodes out of
/// `sight` are left out. `limit`: no plant at or past it (the thinning by
/// count).
fn select(
    group: &super::Group,
    eye: Vec3,
    sight: &Sight,
    limit: u32,
    pick: &dyn Fn(f32, f32, bool) -> Pick,
    out: &mut Vec<(u8, u32, u32)>,
) {
    fn walk(
        nodes: &[super::Node],
        id: u32,
        eye: Vec3,
        sight: &Sight,
        limit: u32,
        pick: &dyn Fn(f32, f32, bool) -> Pick,
        out: &mut Vec<(u8, u32, u32)>,
    ) {
        let n = &nodes[id as usize];
        if n.start >= limit || !sight.sees(n.lo, n.hi) {
            return;
        }
        let near = eye.distance(eye.clamp(n.lo, n.hi));
        let far = (eye - n.lo).abs().max((eye - n.hi).abs()).length();
        let leaf = n.kids.iter().all(|&k| k == super::NO_NODE);
        match pick(near, far, leaf) {
            Pick::Skip => {}
            Pick::Whole(band) => {
                let end = n.end.min(limit);
                match out.last_mut() {
                    Some(last) if last.0 == band && last.2 == n.start => last.2 = end,
                    _ => out.push((band, n.start, end)),
                }
            }
            Pick::Split => {
                for &k in &n.kids {
                    if k != super::NO_NODE {
                        walk(nodes, k, eye, sight, limit, pick, out);
                    }
                }
            }
        }
    }
    if !group.nodes.is_empty() {
        walk(&group.nodes, 0, eye, sight, limit, pick, out);
    }
}

/// What each view draws this frame, by render-world view entity.
#[derive(Resource, Default)]
struct PlantDraws(HashMap<(Entity, Pass), Vec<Draw>>);

/// Each tile's level of detail as last chosen for each camera (near, far),
/// so a tile on the edge does not flicker while the camera drifts.
#[derive(Resource, Default)]
struct PlantLod(HashMap<(RetainedViewEntity, u32), (bool, bool)>);

fn clip_from_world(view: &ExtractedView) -> Mat4 {
    view.clip_from_world
        .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse())
}

/// What a pass may draw of the world: the boxes that reach into its clip
/// space, and for a cascade, of those, the ones over what reads it.
struct Sight {
    clip: Mat4,
    /// Test the near plane too (not for the sun's cascades: their depth is
    /// unclipped).
    near: bool,
    /// A cascade's: the outline of what reads it ([`slice_outline`]), as
    /// half planes of its clip space, `n . (x, y) <= c` inside. A box
    /// wholly out of one of them shades nothing on the screen.
    outline: Vec<(Vec2, f32)>,
}

impl Sight {
    fn sees(&self, lo: Vec3, hi: Vec3) -> bool {
        let mut outside = [true; 5];
        let mut at = [Vec2::ZERO; 8];
        for (i, at) in at.iter_mut().enumerate() {
            let c = Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let p = self.clip * c.extend(1.0);
            outside[0] &= p.x < -p.w;
            outside[1] &= p.x > p.w;
            outside[2] &= p.y < -p.w;
            outside[3] &= p.y > p.w;
            outside[4] &= self.near && p.z > p.w;
            *at = Vec2::new(p.x, p.y);
        }
        !outside.iter().any(|&o| o) && self.outline.iter().all(|&(n, c)| at.iter().any(|p| n.dot(*p) <= c))
    }
}

/// The outline, in a cascade's clip space (`cascade`, from the world), of
/// the part of a camera's view between the depths `from` and `to` (m): what
/// reads the cascade is on the screen and between where the cascade begins
/// and its far bound, and a plant shades it only from over it, as the sun
/// sees them. The cascade's own box is a square round that slice, more
/// than twice its area. `margin`: so much wider, in clip units (the
/// shadow's filter).
fn slice_outline(world_from_view: Mat4, clip_from_view: Mat4, cascade: &Mat4, from: f32, to: f32, margin: f32) -> Vec<(Vec2, f32)> {
    let p = clip_from_view;
    // a perspective camera's: half its width and height a metre of depth
    if p.w_axis.w != 0.0 || p.x_axis.x == 0.0 || p.y_axis.y == 0.0 {
        return Vec::new();
    }
    let (wide, tall) = (1.0 / p.x_axis.x, 1.0 / p.y_axis.y);
    let mut points = Vec::with_capacity(8);
    for d in [from, to] {
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let at = *cascade * (world_from_view * Vec4::new(x * d * wide, y * d * tall, -d, 1.0));
            points.push(Vec2::new(at.x, at.y));
        }
    }
    // the convex hull, counterclockwise (Andrew's chain)
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Vec2> = Vec::with_capacity(16);
    for pass in 0..2 {
        let start = hull.len();
        let order: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 { Box::new(points.iter()) } else { Box::new(points.iter().rev()) };
        for &q in order {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0 {
                hull.pop();
            }
            hull.push(q);
        }
        hull.pop();
    }
    if hull.len() < 3 {
        return Vec::new();
    }
    (0..hull.len())
        .filter_map(|i| {
            let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
            let edge = b - a;
            // counterclockwise: the outside is to the right of an edge
            let n = Vec2::new(edge.y, -edge.x).try_normalize()?;
            Some((n, n.dot(a) + margin))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn queue_plants(
    plants: Res<ExtractedPlants>,
    slot: Res<PlantPipelineSlot>,
    mut specialized: ResMut<SpecializedRenderPipelines<PlantPipeline>>,
    cache: Res<PipelineCache>,
    view_keys: Res<ViewKeyCache>,
    (opaque_draws, prepass_draws, shadow_draws): (Res<DrawFunctions<Opaque3d>>, Res<DrawFunctions<Opaque3dPrepass>>, Res<DrawFunctions<Shadow>>),
    mut opaque_phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    mut prepass_phases: ResMut<ViewBinnedRenderPhases<Opaque3dPrepass>>,
    mut shadow_phases: ResMut<ViewBinnedRenderPhases<Shadow>>,
    views: Query<(Entity, &ExtractedView, Option<&ViewLightEntities>)>,
    light_views: Query<(&ExtractedView, &LightEntity)>,
    suns: Query<&ExtractedDirectionalLight>,
    mut draws: ResMut<PlantDraws>,
    mut lod: ResMut<PlantLod>,
    mut frame: Local<u32>,
) {
    draws.0.clear();
    *frame += 1;
    let stats = stats_on() && *frame % 60 == 0;
    let (Some(scene), Some(pipeline), Some(item)) = (&plants.scene, &slot.0, plants.item) else {
        return;
    };
    let main_draw = opaque_draws.read().id::<DrawPlantsMain>();
    let prepass_draw = prepass_draws.read().id::<DrawPlantsPrepass>();
    let shadow_draw = shadow_draws.read().id::<DrawPlantsShadow>();
    let lod_m = plants.lod_m;
    let far_m = (lod_m * FAR_TIER).max(FAR_TIER_MIN_M);
    // the plants of each group drawn: all, or those the thinning keeps
    let keep = plants.uniform[2];
    let drawn = |g: usize| {
        let group = &scene.groups[g];
        if scene.thin_by_count { group.kept(keep) } else { group.count }
    };
    let mut pipeline_for = |view: MeshPipelineKey, pass: Pass, mesh: u32| {
        let m = &scene.meshes[mesh as usize];
        let receive_shadows = pass == Pass::Main && plants.shadows;
        // the depth-only passes paint nothing
        let canopy = m.canopy && pass == Pass::Main;
        let key = PlantKey { view, pass, cutout: m.cutout, double_sided: m.double_sided, canopy, receive_shadows };
        specialized.specialize(&cache, pipeline, key)
    };
    for (view_entity, view, lights) in &views {
        if plants.blind.contains(&view.retained_view_entity.main_entity) {
            continue;
        }
        let Some(opaque) = opaque_phases.get_mut(&view.retained_view_entity) else { continue };
        let Some(&view_key) = view_keys.get(&view.retained_view_entity) else { continue };
        opaque.remove(item.1);
        let eye = view.world_from_view.translation();
        let frustum = Sight { clip: clip_from_world(view), near: true, outline: Vec::new() };
        // per tile: the level of detail for this camera (full, far)
        let mut tiers = Vec::with_capacity(scene.tiles.len());
        for (t, tile) in scene.tiles.iter().enumerate() {
            let d = eye.distance(eye.clamp(tile.lo, tile.hi));
            let key = (view.retained_view_entity, t as u32);
            let (was_near, was_far) = lod.0.get(&key).copied().unwrap_or((false, false));
            let near = lod_m > 0.0 && if was_near { d < lod_m * 1.1 } else { d < lod_m };
            let far = !near && if was_far { d > far_m / 1.1 } else { d > far_m };
            lod.0.insert(key, (near, far));
            tiers.push((near, far, d));
        }
        // per node: the models within the LOD distance, the cheapest crowns
        // beyond the far one, the proxies between
        let mut list = Vec::new();
        let mut runs = Vec::new();
        // triangles by band (near, mid, full, proxy, far) and draws
        let mut sent = ([0u64; 5], 0u32);
        for (t, tile) in scene.tiles.iter().enumerate() {
            if !frustum.sees(tile.lo, tile.hi) {
                continue;
            }
            let (near, far, _) = tiers[t];
            let whole = |_: f32| if near { 0 } else if far { 2 } else { 1 };
            for g in tile.groups.clone() {
                let group = &scene.groups[g];
                let tier = |t: Tier| group.tiers[t as usize];
                let scanned = tier(Tier::Near).is_some();
                let band = |d: f32| plant_band(d, lod_m, far_m, scanned);
                runs.clear();
                if lod_per_tile() {
                    runs.push((whole(0.0), 0, drawn(g)));
                } else {
                    select(group, eye, &frustum, drawn(g), &pick_by_band(band), &mut runs);
                }
                for &(b, first, end) in &runs {
                    let plant = match b { 3 => tier(Tier::Near).or(tier(Tier::Full)), 4 => tier(Tier::Mid).or(tier(Tier::Full)), 0 => tier(Tier::Full), 2 => tier(Tier::Far), _ => None }.or(tier(Tier::Proxy));
                    if let Some(mesh) = plant.filter(|_| end > first) {
                        list.push(Draw { pipeline: pipeline_for(view_key, Pass::Main, mesh), mesh, group: g as u32, remains: false, first, count: end - first });
                        sent.0[[2, 3, 4, 0, 1][b as usize]] += scene.meshes[mesh as usize].triangles() as u64 * (end - first) as u64;
                        sent.1 += 1;
                    }
                }
                if plants.fire.get(g).is_some_and(|f| !f.2.is_empty()) {
                    if let Some(mesh) = if near { tier(Tier::RemainsNear) } else { None }.or(tier(Tier::RemainsFar)) {
                        list.push(Draw { pipeline: pipeline_for(view_key, Pass::Main, mesh), mesh, group: g as u32, remains: true, first: 0, count: 0 });
                    }
                }
            }
        }
        if stats {
            let m = sent.0.map(|t| t as f32 / 1e6);
            info!("plants, main pass: {:.2} M triangles in {} draws (near {:.2}, mid {:.2}, full {:.2}, proxy {:.2}, far {:.2})", m.iter().sum::<f32>(), sent.1, m[0], m[1], m[2], m[3], m[4]);
        }
        list.sort_by_key(|d| (d.pipeline.id(), d.mesh));
        if let Some(first) = list.first() {
            opaque.add(
                Opaque3dBatchSetKey {
                    pipeline: first.pipeline,
                    draw_function: main_draw,
                    material_bind_group_index: None,
                    slabs: MeshSlabs::default(),
                    lightmap_slab: None,
                },
                Opaque3dBinKey { asset_id: AssetId::<Mesh>::default().untyped() },
                item,
                InputUniformIndex::default(),
                BinnedRenderPhaseType::NonMesh,
            );
            // the depth prepass, when the camera has a bare one (its
            // normal, motion or deferred targets would want outputs of
            // their own): the plants the main pass draws whole, depth only
            let others = MeshPipelineKey::NORMAL_PREPASS | MeshPipelineKey::MOTION_VECTOR_PREPASS | MeshPipelineKey::DEFERRED_PREPASS;
            if view_key.contains(MeshPipelineKey::DEPTH_PREPASS) && !view_key.intersects(others) {
                if let Some(prepass) = prepass_phases.get_mut(&view.retained_view_entity) {
                    prepass.remove(item.1);
                    let mut pre: Vec<Draw> = list
                        .iter()
                        .filter(|d| !scene.meshes[d.mesh as usize].cutout)
                        .map(|d| Draw { pipeline: pipeline_for(view_key, Pass::Prepass, d.mesh), ..*d })
                        .collect();
                    pre.sort_by_key(|d| (d.pipeline.id(), d.mesh));
                    if let Some(first) = pre.first() {
                        prepass.add(
                            OpaqueNoLightmap3dBatchSetKey {
                                pipeline: first.pipeline,
                                draw_function: prepass_draw,
                                material_bind_group_index: None,
                                slabs: MeshSlabs::default(),
                            },
                            OpaqueNoLightmap3dBinKey { asset_id: AssetId::<Mesh>::default().untyped() },
                            item,
                            InputUniformIndex::default(),
                            BinnedRenderPhaseType::NonMesh,
                        );
                        draws.0.insert((view_entity, Pass::Prepass), pre);
                    }
                }
            }
            draws.0.insert((view_entity, Pass::Main), list);
        }

        // the sun's cascades of this camera: the plants cast their shadows
        for &light_entity in lights.map(|l| l.lights.as_slice()).unwrap_or_default() {
            let Ok((light_view, &LightEntity::Directional { light_entity: sun, cascade_index })) = light_views.get(light_entity) else { continue };
            let Some(shadow) = shadow_phases.get_mut(&light_view.retained_view_entity) else { continue };
            shadow.remove(item.1);
            let mut cascade = Sight { clip: clip_from_world(light_view), near: false, outline: Vec::new() };
            // a plant casts the shadow of the mesh it is drawn with: a
            // proxy's closed crown over a full model darkened the model
            // itself (2026-11-04, per node; per tile the camera's own tile
            // cast with its full models up to its edge, `FULL_SHADOW_M`).
            //
            // It must only in the cascades it reads. A fragment reads the
            // cascade its depth from the camera falls in and, in the
            // overlap, the next (`shadows::fetch_directional_shadow`): what
            // is nearer the camera than where a cascade begins never reads
            // it, and in it only casts on what is farther. There a nearer
            // plant casts the mesh drawn where a pixel of the screen is as
            // wide as a texel of the cascade (finer would not show), and one
            // whose shadow cannot reach that far casts nothing
            // (2026-11-06: the nearest plants went whole into all four
            // cascades; with the scanned ones, 300 m from the front, 88
            // million triangles of shadows a frame for 13 drawn).
            let config = suns.get(sun).ok().map(|s| &s.cascade_shadow_config);
            let begins = config.map_or(0.0, |c| cascade_begins(c, cascade_index));
            // and what reads it is on the screen, this side of its far bound
            if let Some(ends) = config.and_then(|c| c.bounds.get(cascade_index)) {
                let filter = 6.0 * 2.0 / light_view.viewport.z.max(1) as f32;
                cascade.outline = slice_outline(view.world_from_view.to_matrix(), view.clip_from_view, &cascade.clip, begins, *ends, filter);
            }
            let beyond = begins - shadow_reach_m(light_view.world_from_view.back().y);
            let texel = 2.0 / light_view.clip_from_view.x_axis.x.abs().max(1e-9) / light_view.viewport.z.max(1) as f32;
            let pixel = 2.0 / (view.clip_from_view.y_axis.y.abs().max(1e-3) * view.viewport.w.max(1) as f32);
            let coarse = texel / pixel;
            let mut list = Vec::new();
            let mut runs = Vec::new();
            let mut sent = ([0u64; 5], 0u32);
            for (t, tile) in scene.tiles.iter().enumerate() {
                if !cascade.sees(tile.lo, tile.hi) {
                    continue;
                }
                let (near, far, d) = tiers[t];
                let full = near && d < FULL_SHADOW_M;
                for g in tile.groups.clone() {
                    let group = &scene.groups[g];
                    let tier = |t: Tier| group.tiers[t as usize];
                    let scanned = tier(Tier::Near).is_some();
                    let band = |d: f32| plant_band(d, lod_m, far_m, scanned);
                    runs.clear();
                    if lod_per_tile() {
                        runs.push((if full { 0 } else if far { 2 } else { 1 }, 0, drawn(g)));
                    } else {
                        // the distance from the camera, the box from the cascade
                        select(group, eye, &cascade, drawn(g), &pick_caster(band, begins, beyond, coarse), &mut runs);
                    }
                    for &(b, first, end) in &runs {
                        let plant = match b { 3 => tier(Tier::Near).or(tier(Tier::Full)), 4 => tier(Tier::Mid).or(tier(Tier::Full)), 0 => tier(Tier::Full), 2 => tier(Tier::Far), _ => None }.or(tier(Tier::Proxy));
                        if let Some(mesh) = plant.filter(|_| end > first) {
                            list.push(Draw { pipeline: pipeline_for(MeshPipelineKey::empty(), Pass::Shadow, mesh), mesh, group: g as u32, remains: false, first, count: end - first });
                            sent.0[[2, 3, 4, 0, 1][b as usize]] += scene.meshes[mesh as usize].triangles() as u64 * (end - first) as u64;
                            sent.1 += 1;
                        }
                    }
                    if full && plants.fire.get(g).is_some_and(|f| !f.2.is_empty()) {
                        if let Some(mesh) = tier(Tier::RemainsNear) {
                            list.push(Draw { pipeline: pipeline_for(MeshPipelineKey::empty(), Pass::Shadow, mesh), mesh, group: g as u32, remains: true, first: 0, count: 0 });
                        }
                    }
                }
            }
            if stats {
                let m = sent.0.map(|t| t as f32 / 1e6);
                info!("plants, a cascade: {:.2} M triangles in {} draws (near {:.2}, mid {:.2}, full {:.2}, proxy {:.2}, far {:.2})", m.iter().sum::<f32>(), sent.1, m[0], m[1], m[2], m[3], m[4]);
            }
            list.sort_by_key(|d| (d.pipeline.id(), d.mesh));
            if let Some(first) = list.first() {
                shadow.add(
                    ShadowBatchSetKey {
                        pipeline: first.pipeline,
                        draw_function: shadow_draw,
                        material_bind_group_index: None,
                        slabs: MeshSlabs::default(),
                    },
                    ShadowBinKey { asset_id: AssetId::<Mesh>::default().untyped() },
                    item,
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                );
                draws.0.insert((light_entity, Pass::Shadow), list);
            }
        }
    }
}

// --- drawing -----------------------------------------------------------------

type DrawPlantsMain = (SetMeshViewBindGroup<0>, SetMeshViewBindingArrayBindGroup<1>, DrawPlants<MAIN>);
type DrawPlantsPrepass = (SetPrepassViewBindGroup<0>, SetPrepassViewEmptyBindGroup<1>, DrawPlants<PREPASS>);
type DrawPlantsShadow = (SetPrepassViewBindGroup<0>, SetPrepassViewEmptyBindGroup<1>, DrawPlants<SHADOW>);

const MAIN: u8 = 0;
const PREPASS: u8 = 1;
const SHADOW: u8 = 2;

/// The view's list of instanced draws ([`PlantDraws`]): the pipeline when it
/// changes, the plants' bind group, the three vertex buffers, one draw.
struct DrawPlants<const PASS: u8>;

impl<P: PhaseItem, const PASS: u8> RenderCommand<P> for DrawPlants<PASS> {
    type Param = (SRes<PlantGpu>, SRes<PlantDraws>, SRes<PipelineCache>);
    type ViewQuery = Entity;
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        view: Entity,
        _: Option<()>,
        (gpu, draws, cache): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (Some(gpu), Some(list)) = (gpu.into_inner().0.as_ref(), draws.into_inner().0.get(&(view, [Pass::Main, Pass::Prepass, Pass::Shadow][PASS as usize]))) else {
            return RenderCommandResult::Skip;
        };
        let cache = cache.into_inner();
        let mut current = None;
        for d in list {
            if current != Some(d.pipeline) {
                let Some(p) = cache.get_render_pipeline(d.pipeline) else { continue };
                pass.set_render_pipeline(p);
                pass.set_bind_group(2, &gpu.bind_group, &[]);
                current = Some(d.pipeline);
            }
            let (Some(mesh), Some(group)) = (gpu.meshes.get(d.mesh as usize), gpu.groups.get(d.group as usize)) else { continue };
            let (instances, fire, first, count) = if d.remains {
                let Some(r) = group.remains.as_ref() else { continue };
                (&r.instances, &r.fire, 0, r.count)
            } else {
                let first = d.first.min(group.count);
                (&group.instances, &group.fire, first, d.count.min(group.count - first))
            };
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_vertex_buffer(1, instances.slice(..));
            pass.set_vertex_buffer(2, fire.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), IndexFormat::Uint16);
            pass.draw_indexed(0..mesh.count, 0, first..first + count);
        }
        RenderCommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    /// The scanned plants' levels end a little short of where the sun's
    /// cascades begin to be read, at the highest quality.
    #[test]
    fn the_scanned_levels_end_where_the_cascades_begin() {
        let config = crate::quality::Graphics::ALTA.sun_cascades();
        let begins: Vec<f32> = (0..config.bounds.len()).map(|i| cascade_begins(&config, i)).collect();
        assert_eq!(begins.len(), 4);
        assert_eq!(begins[0], 0.0);
        for (level, begin) in [NEAR_M, MID_M, SCAN_FAR_M].into_iter().zip(&begins[1..]) {
            assert!(level < *begin && begin - level < 0.05 * begin, "a level ends at {level} m, a cascade begins at {begin} m");
        }
        assert!(SCAN_LUMP_M >= SCAN_FAR_M);
    }

    /// A box between a point of the screen and the sun is never left out of
    /// the cascade that point reads, and the outline does leave out boxes
    /// of the cascade's square.
    #[test]
    fn a_cascade_keeps_what_shades_the_screen() {
        // numbers in 0..1, the same every run
        let mut seed = 0x2545_f491u32;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        let clip_from_view = Mat4::perspective_infinite_reverse_rh(0.75, 1.6, 0.1);
        let (wide, tall) = (1.0 / clip_from_view.x_axis.x, 1.0 / clip_from_view.y_axis.y);
        let (mut shading, mut left_out, mut tried) = (0, 0, 0);
        for case in 0..40 {
            let eye = Vec3::new(4000.0 + 2000.0 * rnd(), 150.0 + 800.0 * rnd(), -4000.0 - 2000.0 * rnd());
            let target = eye + Vec3::new(rnd() - 0.5, -0.1 - 0.8 * rnd(), rnd() - 0.5) * 1000.0;
            let world_from_view = Transform::from_translation(eye).looking_at(target, Vec3::Y).to_matrix();
            let toward_sun = Vec3::new(rnd() - 0.5, 0.3 + rnd(), rnd() - 0.5).normalize();
            // a cascade: a square of the sun's view round the slice, as Bevy's
            let (from, to) = [(0.0, 80.0), (56.0, 295.0), (206.0, 1086.0), (760.0, 4000.0)][case % 4];
            let light_from_world = Mat4::look_to_rh(Vec3::ZERO, -toward_sun, Vec3::Y);
            let middle = light_from_world.transform_point3(world_from_view.transform_point3(Vec3::new(0.0, 0.0, -0.5 * (from + to))));
            let half = to * 1.6;
            let cascade = Mat4::orthographic_rh(middle.x - half, middle.x + half, middle.y - half, middle.y + half, -1.0e5, 1.0e5) * light_from_world;
            let sight = Sight { clip: cascade, near: false, outline: slice_outline(world_from_view, clip_from_view, &cascade, from, to, 0.0) };
            assert!(sight.outline.len() >= 3, "case {case}");
            for _ in 0..500 {
                // a point of the screen that reads the cascade
                let d = from + (to - from) * rnd();
                let point = world_from_view.transform_point3(Vec3::new((2.0 * rnd() - 1.0) * d * wide, (2.0 * rnd() - 1.0) * d * tall, -d));
                // a box between it and the sun
                let size = Vec3::new(0.5 + 20.0 * rnd(), 0.5 + 20.0 * rnd(), 0.5 + 20.0 * rnd());
                let caster = point + toward_sun * (1.0 + 400.0 * rnd());
                assert!(sight.sees(caster - size, caster + size), "case {case}: a box over a point at depth {d} was left out");
                shading += 1;
                // and a box anywhere in the cascade's square
                let anywhere = light_from_world.inverse().transform_point3(Vec3::new(middle.x + half * (2.0 * rnd() - 1.0), middle.y + half * (2.0 * rnd() - 1.0), middle.z));
                tried += 1;
                left_out += usize::from(!sight.sees(anywhere - Vec3::splat(5.0), anywhere + Vec3::splat(5.0)));
            }
        }
        assert_eq!(shading, 20_000);
        // the slice is well under half of its square
        assert!(left_out * 2 > tried, "{left_out} of {tried} left out");
    }

    /// What a cascade's pick does with the nodes round where it begins.
    #[test]
    fn a_cascade_casts_what_reads_it_as_drawn() {
        let band = |d: f32| plant_band(d, 1400.0, 2800.0, true);
        // a cascade read from 206 m, shadows no longer than 250 m, a texel as a pixel at 900 m
        let pick = pick_caster(band, 206.0, -44.0, 900.0);
        // read by its far part: as drawn by its nearest point, whole only past the beginning
        assert_eq!(pick(210.0, 230.0, false), Pick::Whole(band(210.0)));
        assert_eq!(pick(150.0, 230.0, false), Pick::Split);
        assert_eq!(pick(150.0, 230.0, true), Pick::Whole(band(150.0)));
        // wholly nearer: the coarse mesh
        assert_eq!(pick(20.0, 60.0, false), Pick::Whole(band(900.0)));
        assert_ne!(band(900.0), band(20.0));
        // the last cascade, read from 760 m: what is nearer than 510 m casts nothing
        let last = pick_caster(band, 760.0, 510.0, 3000.0);
        assert_eq!(last(100.0, 400.0, false), Pick::Skip);
        assert_eq!(last(400.0, 600.0, false), Pick::Whole(band(3000.0)));
        assert_eq!(last(700.0, 900.0, false), Pick::Split);
        // and the main pass's pick
        let main = pick_by_band(band);
        assert_eq!(main(10.0, 40.0, false), Pick::Whole(band(10.0)));
        assert_eq!(main(10.0, 100.0, false), Pick::Split);
        assert_eq!(main(10.0, 100.0, true), Pick::Whole(band(10.0)));
    }

    /// Draws and triangles of the plants for a few cameras, the level of
    /// detail chosen per 1280 m tile (until 2026-11-04) and per node of the
    /// groups' quadtrees: printed, run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    #[allow(deprecated)] // glam's camera matrices, renamed in its new API
    fn plant_lod_per_tile_and_per_node() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let sim = crate::sim::Sim::new(&data, "medium", 2).unwrap();
        let (wm, hm) = (sim.scn.world.width_m, sim.scn.world.height_m);
        for (label, gfx) in [("alta", crate::quality::Graphics::ALTA), ("media", crate::quality::Graphics::MEDIA)] {
            let mut world = World::new();
            world.insert_resource(crate::sim::Sim::new(&data, "medium", 2).unwrap());
            world.insert_resource(gfx.clone());
            world.run_system_once(super::super::spawn).unwrap();
            let scene = world.resource::<Vegetation>().scene.clone();
            let lod_m = gfx.lod_m;
            let far_m = (lod_m * FAR_TIER).max(FAR_TIER_MIN_M);
            let tris = |m: Option<u32>| m.map_or(0, |m| scene.meshes[m as usize].triangles());
            println!("{label}: lod {lod_m} m, far {far_m} m");
            println!("{:>28} {:>9} {:>9} {:>11} {:>11}", "camera", "draws", "draws", "M tris", "M tris");
            println!("{:>28} {:>9} {:>9} {:>11} {:>11}", "", "tile", "node", "tile", "node");
            for (name, at, dist, pitch) in [
                ("centre, 1600 m, -32°", Vec2::new(wm * 0.5, hm * 0.5), 1600.0f32, -32.0f32),
                ("centre, 500 m, -25°", Vec2::new(wm * 0.5, hm * 0.5), 500.0, -25.0),
                ("west, 900 m, -20°", Vec2::new(wm * 0.3, hm * 0.5), 900.0, -20.0),
                ("north, 250 m, -15°", Vec2::new(wm * 0.5, hm * 0.7), 250.0, -15.0),
            ] {
                let ground = sim.scn.terrain.height_at(scenario::Pos { x: at.x, y: at.y });
                let target = Vec3::new(at.x, ground, -at.y);
                let p = pitch.to_radians();
                let eye = target + Vec3::new(0.0, -p.sin() * dist, p.cos() * dist);
                let clip = Mat4::perspective_rh(45f32.to_radians(), 1.6, 1.0, 30_000.0) * Mat4::look_at_rh(eye, target, Vec3::Y);
                let (mut dt, mut dn, mut tt, mut tn) = (0usize, 0usize, 0usize, 0usize);
                let mut runs = Vec::new();
                for tile in &scene.tiles {
                    let sight = Sight { clip, near: true, outline: Vec::new() };
                    if !sight.sees(tile.lo, tile.hi) {
                        continue;
                    }
                    let d = eye.distance(eye.clamp(tile.lo, tile.hi));
                    let (near, far) = (d < lod_m, d >= lod_m && d > far_m);
                    let band = |d: f32| if lod_m > 0.0 && d < lod_m { 0 } else if d > far_m { 2 } else { 1 };
                    for g in tile.groups.clone() {
                        let group = &scene.groups[g];
                        let tier = |t: Tier| group.tiers[t as usize];
                        let mesh = |b: u8| match b { 0 => tier(Tier::Full), 2 => tier(Tier::Far), _ => None }.or(tier(Tier::Proxy));
                        dt += 1;
                        tt += group.count as usize * tris(mesh(if near { 0 } else if far { 2 } else { 1 }));
                        runs.clear();
                        select(group, eye, &sight, group.count, &pick_by_band(band), &mut runs);
                        dn += runs.len();
                        tn += runs.iter().map(|&(b, s, e)| (e - s) as usize * tris(mesh(b))).sum::<usize>();
                    }
                }
                println!("{name:>28} {dt:>9} {dn:>9} {:>11.2} {:>11.2}", tt as f32 / 1e6, tn as f32 / 1e6);
            }
        }
    }
}
